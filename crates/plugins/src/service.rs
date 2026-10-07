//! The plugins domain: install, remove, prepare, and the listing
//! (`ARCHITECTURE` §10, task T3, wired to the frame).

use std::path::{Path, PathBuf};

use agent_hub_db::{Db, InstallStep, Layout, PluginRow, PluginState};
use agent_hub_events::Bus;

use crate::identity::{CommandIds, Idempotency};
use crate::manifest::{Manifest, PluginView};
use crate::state::{Op, Ops};

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("plugin `{0}` not found")]
    NotFound(String),
    #[error("plugin `{0}` is a deployment directory, not installed by this hub")]
    NotInstalledByHub(String),
    #[error("plugin `{0}` is in use by sessions: {1}")]
    InUse(String, String),
    #[error("plugin `{0}` is {1}; retry when the operation finishes")]
    Busy(String, String),
    #[error("manifest invalid: {0}")]
    InvalidManifest(String),
    #[error("idempotency conflict for `{0}`")]
    Conflict(String),
    /// A conflicting command on the SAME plugin while an operation is in flight (contract: a
    /// conflicting command on the same plugin is refused). Distinct from `Conflict` (idempotency)
    /// so the code is `conflict`, not `idempotency_conflict`.
    #[error("plugin `{0}` already has an operation in progress")]
    InProgress(String),
    /// The source is not in the official registry (docs/issues/20261005-130000): the trust
    /// anchor does not list it, so it is refused before anything is fetched.
    #[error("the source is not in the official registry: {0}")]
    NotInRegistry(String),
    #[error("install failed: {0}")]
    InstallFailed(String),
    #[error(transparent)]
    Source(#[from] crate::source::SourceError),
    // The detail MUST name WHICH registry failure happened: a caller and a test can
    // only distinguish "not set" from "the address did not answer" from "the body is
    // not a registry" by this string, and the code is the same for all of them
    // (docs/issues/20261005-110000).
    #[error("{0}")]
    RegistryUrlMissing(String),
    #[error(transparent)]
    Db(#[from] agent_hub_db::DbError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl PluginError {
    /// The **contract** error code for this failure (`contract/errors.json`).
    /// The transport maps the code to an HTTP status; a domain never does.
    pub fn code(&self) -> &'static str {
        match self {
            PluginError::NotFound(_) => "not_found",
            // A deployment directory is READ-ONLY to the hub, not a failed remove: the
            // contract names 409 conflict (nothing was attempted; a 500 would be a lie
            // and, being retryable, would loop).
            PluginError::NotInstalledByHub(_) => "conflict",
            PluginError::InUse(_, _) => "plugin_in_use",
            PluginError::Busy(_, _) => "plugin_dir_busy",
            PluginError::InvalidManifest(_) => "plugin_archive_invalid",
            PluginError::Conflict(_) => "idempotency_conflict",
            PluginError::InProgress(_) => "conflict",
            PluginError::NotInRegistry(_) => "plugin_not_in_registry",
            PluginError::InstallFailed(_) => "plugin_install_failed",
            PluginError::Source(e) => e.code(),
            // A registry failure is NOT an install failure: nothing was installed, the
            // registry could not be read. A false, retryable code here would make a
            // client retry a request that can never succeed (docs/issues/20261005-110000).
            PluginError::RegistryUrlMissing(_) => "registry_unavailable",
            PluginError::Db(_) | PluginError::Io(_) => "internal_error",
        }
    }

    /// A contract error for the transport: the code plus a detail.
    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// The plugins domain. Holds the DB, the hub's own writable root, and the event
/// bus. Runtime prepare is delegated to the adapter (a later domain); here the
/// install/remove lifecycle and its recovery are the concern.
pub struct Plugins {
    pub db: Db,
    /// `<DATA_DIR>/plugins`: where POST installs. Deployment roots are read-only.
    pub root: PathBuf,
    bus: Bus,
    ids: CommandIds,
    /// Per-plugin in-flight operations (the one verdict).
    ops: std::sync::Mutex<std::collections::HashMap<String, Ops>>,
    /// PER-ID install/remove locks. The old design held ONE global lock, so an install of pi and
    /// an install of deepseek - DIFFERENT plugins - were wrongly refused 409 each other, reducing
    /// concurrency to one op at a time hub-wide (a T0). The staging dir is now per-id too, so two
    /// DIFFERENT ids proceed concurrently; two ops on the SAME id conflict (contract: refused or
    /// joined).
    op_locks: std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<()>>>>,
    /// The loaded official registry (docs/issues/20261005-130000). Loaded once at construction
    /// and replaced by a refresh; the INSTALL path reconciles a caller's source against this, so
    /// a source the registry does not list is refused before anything is fetched. `None` = not
    /// loaded (no registry available): then NO install is authorized.
    registry: std::sync::Mutex<Option<crate::registry::Registry>>,
    /// Why the registry could not be loaded, when that is the reason an install is refused.
    /// `None` = no fault recorded (a registry may simply not be configured yet). `Some(reason)` =
    /// the registry EXISTS but could not be read/parsed, or a refresh failed: an install is then
    /// refused with `registry_unavailable` (the REGISTRY is the problem), NOT
    /// `plugin_not_in_registry` (the SOURCE is not listed) - a different fact, a different code.
    registry_fault: std::sync::Mutex<Option<String>>,
}

impl Plugins {
    /// The registry file the hub reads: `<DATA_DIR>/registry.json` (beside the plugin root's
    /// parent). The path is FIXED - a caller must not be able to point the hub at an arbitrary
    /// local file as its registry. In a dev build only, `AGENT_HUB_REGISTRY_FILE` may override it
    /// so tests can supply a registry; that override is compiled out of a release binary.
    fn registry_file(&self) -> PathBuf {
        #[cfg(debug_assertions)]
        {
            if let Ok(p) = std::env::var("AGENT_HUB_REGISTRY_FILE") {
                if !p.trim().is_empty() {
                    return PathBuf::from(p);
                }
            }
        }
        self.root
            .parent()
            .map(|p| p.join("registry.json"))
            .unwrap_or_else(|| PathBuf::from("registry.json"))
    }

    /// The plugin catalog: the registry file restated VERBATIM (the hub does not
    /// resolve, rank or rewrite it). A missing or invalid file is a `fault`, never a
    /// 500: the catalog is data, and its absence is a fact a caller needs.
    pub fn catalog(&self) -> serde_json::Value {
        let file = self.registry_file();
        let source = file.to_string_lossy().to_string();
        if !file.exists() {
            return serde_json::json!({
                "schema": 1, "source": null, "plugins": [],
                "fault": format!("no registry at {source}")
            });
        }
        let raw = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                return serde_json::json!({
                    "schema": 1, "source": source, "plugins": [],
                    "fault": format!("the registry file is not readable: {e}")
                })
            }
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                return serde_json::json!({
                    "schema": 1, "source": source, "plugins": [],
                    "fault": format!("the registry file is not readable JSON: {e}")
                })
            }
        };
        let plugins = parsed.get("plugins").and_then(|v| v.as_array()).cloned();
        match plugins {
            Some(plugins) => serde_json::json!({
                "schema": parsed.get("schema").and_then(|v| v.as_i64()).unwrap_or(1),
                "source": source,
                "note": parsed.get("note").cloned().unwrap_or(serde_json::Value::Null),
                "plugins": plugins,
            }),
            None => serde_json::json!({
                "schema": 1, "source": source, "plugins": [],
                "fault": "the registry file has no plugins array"
            }),
        }
    }

    /// Refresh the catalog: fetch the registry from the hub's FIXED address (see
    /// `registry.rs`: the official URL in a release build; `AGENT_HUB_REGISTRY_URL` may
    /// only override it in a dev build) and write it where the hub reads the catalog.
    pub async fn refresh_registry(&self) -> Result<serde_json::Value, PluginError> {
        let url = crate::registry::source_url();
        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .map_err(|e| self.fail_refresh(format!("registry fetch failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(self.fail_refresh(format!("registry fetch failed: HTTP {}", resp.status())));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| self.fail_refresh(format!("registry read failed: {e}")))?;
        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| self.fail_refresh(format!("registry is not JSON: {e}")))?;
        let count = parsed
            .get("plugins")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .ok_or_else(|| self.fail_refresh("registry has no plugins array".into()))?;
        let file = self.registry_file();
        std::fs::write(&file, &text)?;
        // Make the fetched registry the one installs reconcile against, and clear any fault.
        if let Ok(reg) = crate::registry::Registry::parse(&text) {
            *self.registry.lock().expect("registry mutex") = Some(reg);
            *self.registry_fault.lock().expect("registry fault mutex") = None;
        }
        Ok(serde_json::json!({ "source": file.to_string_lossy(), "plugins": count }))
    }

    /// Record a refresh failure as the current registry fault, and return it as a
    /// `registry_unavailable` error. A failed refresh means the REGISTRY is unavailable, so a
    /// later install is refused with `registry_unavailable` (not `plugin_not_in_registry`).
    fn fail_refresh(&self, reason: String) -> PluginError {
        *self.registry.lock().expect("registry mutex") = None;
        *self.registry_fault.lock().expect("registry fault mutex") = Some(reason.clone());
        PluginError::RegistryUrlMissing(reason)
    }

    pub fn new(db: Db, root: impl Into<PathBuf>, bus: Bus) -> Self {
        Plugins {
            db,
            root: root.into(),
            bus,
            ids: CommandIds::new(4096),
            ops: std::sync::Mutex::new(std::collections::HashMap::new()),
            op_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
            registry: std::sync::Mutex::new(None),
            registry_fault: std::sync::Mutex::new(None),
        }
    }

    /// Load the registry into memory from `registry_file()` (at boot, and after a refresh wrote
    /// it). In a DEV build `registry_file()` honours `AGENT_HUB_REGISTRY_FILE` (how a test injects
    /// a registry); in a RELEASE build there is no override and the file is `<DATA_DIR>/registry.json`.
    /// A missing or unparseable file loads NOTHING, so every install is refused.
    pub fn load_registry_from_file(&self) {
        let file = self.registry_file();
        if !file.exists() {
            // No registry configured: not a fault, just nothing to reconcile against.
            *self.registry.lock().expect("registry mutex") = None;
            *self.registry_fault.lock().expect("registry fault mutex") = None;
            return;
        }
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                *self.registry.lock().expect("registry mutex") = None;
                *self.registry_fault.lock().expect("registry fault mutex") =
                    Some(format!("the registry at {file:?} is not readable: {e}"));
                return;
            }
        };
        match crate::registry::Registry::parse(&text) {
            Ok(reg) => {
                *self.registry.lock().expect("registry mutex") = Some(reg);
                *self.registry_fault.lock().expect("registry fault mutex") = None;
            }
            Err(e) => {
                *self.registry.lock().expect("registry mutex") = None;
                *self.registry_fault.lock().expect("registry fault mutex") =
                    Some(format!("the registry at {file:?} cannot be parsed: {e}"));
            }
        }
    }

    /// Reconcile a caller's source against the loaded registry. Returns Ok(()) when the source
    /// is authorized (an artifact whose url AND sha256 match a registry release), or an error
    /// naming why it is not. A git source carries no sha256, so it cannot match a release-zip
    /// registry -> refused (docs/issues/20261005-130000).
    fn authorized(&self, source: &crate::source::Source) -> Result<(), PluginError> {
        // If the registry EXISTS but could not be read/parsed, the problem is the REGISTRY, not
        // the SOURCE: answer `registry_unavailable` (502), never `plugin_not_in_registry` (403).
        if self.registry.lock().expect("registry mutex").is_none() {
            if let Some(reason) = self.registry_fault.lock().expect("registry fault mutex").clone() {
                return Err(PluginError::RegistryUrlMissing(reason));
            }
        }
        let guard = self.registry.lock().expect("registry mutex");
        let Some(reg) = guard.as_ref() else {
            return Err(PluginError::NotInRegistry(
                "no registry is loaded, so no source is authorized".into(),
            ));
        };
        match source {
            crate::source::Source::Artifact(spec) => {
                if reg.authorizes(&spec.url, &spec.sha256) {
                    Ok(())
                } else {
                    Err(PluginError::NotInRegistry(format!(
                        "url `{}` with sha256 `{}` is not a release the registry lists",
                        spec.url, spec.sha256
                    )))
                }
            }
            crate::source::Source::Git { url, .. } => Err(PluginError::NotInRegistry(format!(
                "a git source (`{url}`) is not a registered release: the registry publishes release zips with a pinned sha256"
            ))),
        }
    }

    fn enter(&self, id: &str, op: Op) {
        let mut ops = self.ops.lock().expect("ops mutex");
        ops.entry(id.to_string()).or_default().enter(op);
    }

    /// The id has an operation in flight RIGHT NOW. The contract: a conflicting command on the
    /// SAME plugin is refused (409). This covers BOTH "install while installing" and "delete
    /// while installing" - an in-flight install owns its id, so neither a second install nor a
    /// remove may proceed. It is called BEFORE any lock/staging so the answer is a clean 409,
    /// never a 404 ("not found") or a 502 (a staging collision).
    fn busy(&self, id: &str) -> bool {
        let ops = self.ops.lock().expect("ops mutex");
        ops.get(id).map(|o| !o.is_empty()).unwrap_or(false)
    }

    fn leave(&self, id: &str, op: Op) {
        let mut ops = self.ops.lock().expect("ops mutex");
        if let Some(set) = ops.get_mut(id) {
            set.leave(op);
        }
    }

    fn has_dir(&self, id: &str) -> bool {
        Layout::for_plugin(&self.root, id).target.join("manifest.json").exists()
    }

    /// The current `state` for a plugin: the in-flight verdict, else the disk fact.
    pub fn state_of(&self, id: &str) -> String {
        let ops = self.ops.lock().expect("ops mutex");
        let verdict = ops.get(id).cloned().unwrap_or_default();
        verdict.state_str(self.has_dir(id)).to_string()
    }

    /// Publish a `hub.plugins.changed` frame and return the new state.
    fn announce(&self, id: &str) -> String {
        let state = self.state_of(id);
        self.bus.publish(
            "hub.plugins.changed",
            serde_json::json!({ "id": id, "state": state, "at": agent_hub_db::now_utc() }),
        );
        state
    }

    /// List installed and deployment plugins, each rendered as the contract's
    /// `plugin` object.
    pub fn list(&self) -> Result<Vec<PluginView>, PluginError> {
        let mut views = Vec::new();
        // Rows the hub installed.
        for row in self.db.list_plugins()? {
            let id = row.id.clone();
            views.push(self.view(&id, Some(row)));
        }
        // Directories a deployment placed on the search path (read-only to us).
        if self.root.exists() {
            for entry in std::fs::read_dir(&self.root)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || views.iter().any(|v| v.id == name) {
                    continue;
                }
                if entry.path().join("manifest.json").exists() {
                    views.push(self.view(&name, Some(self.row_for_dir(&name))));
                }
            }
        }
        views.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(views)
    }

    /// Read one plugin (the resource a `202 Location` names).
    pub fn get(&self, id: &str) -> Result<PluginView, PluginError> {
        // A hub-installed plugin has a row; a deployment directory does not. But an operation in
        // flight (docs/issues/20261007-183000) means the resource is REAL before the bytes land:
        // a 202 Location names it and a client must be able to read it, as `installing`, WHILE it
        // fetches. Without this the resource was 404 during the whole download and then jumped to
        // `ready` - the state machine lied.
        match self.db.plugin(id)? {
            Some(row) => Ok(self.view(id, Some(row))),
            None if self.has_dir(id) => Ok(self.view(id, Some(self.row_for_dir(id)))),
            None if self.busy(id) => Ok(self.view(id, None)),
            None => Err(PluginError::NotFound(id.into())),
        }
    }

    fn row_for_dir(&self, id: &str) -> PluginRow {
        PluginRow {
            id: id.into(),
            name: None,
            summary: None,
            plugin_type: None,
            source: None,
            reference: None,
            commit: None,
            state: PluginState::Ready,
            detail: None,
            installed_at: None,
            artifact: None,
        }
    }

    fn view(&self, id: &str, row: Option<PluginRow>) -> PluginView {
        let row = row.unwrap_or_else(|| PluginRow {
            id: id.to_string(), name: None, summary: None, plugin_type: None, source: None,
            reference: None, commit: None, state: PluginState::Installing, installed_at: None,
            detail: None, artifact: None,
        });
        let target = Layout::for_plugin(&self.root, id).target;
        let manifest = std::fs::read_to_string(target.join("manifest.json"))
            .ok()
            .and_then(|raw| Manifest::parse(&raw).ok());
        let invalid = manifest
            .as_ref()
            .and_then(|m| m.invalid_reason())
            .or_else(|| row.detail.clone().filter(|_| row.state == PluginState::Failed));

        PluginView {
            id: id.to_string(),
            name: row.name.clone().or_else(|| manifest.as_ref().and_then(|m| m.name.clone())),
            summary: row.summary.clone(),
            capabilities: manifest.as_ref().map(|m| m.capabilities.clone()).unwrap_or_default(),
            plugin_type: row
                .plugin_type
                .clone()
                .or_else(|| manifest.as_ref().and_then(|m| m.plugin_type.clone()))
                .unwrap_or_else(|| "invalid".into()),
            origin: if row.installed_at.is_some() { "hub".into() } else { "deployment".into() },
            path: target.to_string_lossy().to_string(),
            source: row.source.clone(),
            reference: row.reference.clone(),
            commit: row.commit.clone(),
            installed_at: row.installed_at.clone(),
            runtime: manifest
                .as_ref()
                .and_then(|m| m.runtime.as_ref())
                .map(|r| {
                    serde_json::json!({
                        "package": r.package, "version": r.version,
                        "target": r.target.clone().or_else(|| runtime_script(&target, manifest.as_ref()))
                    })
                }),
            // The contract: `runtimeReady` is a FACT about the filesystem - the
            // manifest's command exists under the plugin dir - not a claim about the
            // harness working. It was hardcoded `false`, so a SUCCESSFUL prepare read
            // as not-ready (docs/issues/20261005-090000).
            runtime_ready: runtime_command_present(&target, manifest.as_ref()),
            state: self.state_of(id),
            prepare: None,
            invalid,
            artifact: row
                .artifact
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok()),
        }
    }
}

impl Plugins {
    /// Validate an install **before any work** (cheap, synchronous): parse the
    /// manifest, resolve the id, and register the logical command identity (R1).
    /// Returns [`InstallIntent::Replay`] for a retry (no work), or `Proceed` with
    /// the id to run detached.
    pub fn begin_install(
        &self,
        command_id: &str,
        source: &crate::source::Source,
    ) -> Result<InstallIntent, PluginError> {
        // AUTHORIZATION FIRST (docs/issues/20261005-130000): a source must be listed by the
        // official registry (url AND sha256) before ANY work - no clone, no download, no
        // staging. This is what stops a caller installing code the registry never approved.
        self.authorized(source)?;
        // The plugin id FIRST. For an artifact it is in the spec; for a git source we read the
        // source manifest (git is refused by the registry anyway). The lock and staging dir are
        // PER-ID, so DIFFERENT plugins install CONCURRENTLY and only the SAME id conflicts.
        let id: String = match source {
            crate::source::Source::Artifact(spec) => spec.id.clone(),
            crate::source::Source::Git { url, .. } => {
                let p = PathBuf::from(url);
                let mf = std::fs::read_to_string(p.join("manifest.json"))
                    .map_err(|_| PluginError::InvalidManifest("no manifest.json in the source".into()))?;
                crate::manifest::Manifest::parse(&mf)
                    .map_err(PluginError::InvalidManifest)?
                    .id
                    .ok_or_else(|| PluginError::InvalidManifest("the source manifest has no id".into()))?
            }
        };
        // A conflicting command on the SAME plugin is refused 409 (contract): an in-flight install
        // owns its id, so neither a second install nor a remove may proceed.
        if self.busy(&id) {
            return Err(PluginError::InProgress(id.clone()));
        }
        // A DEPLOYMENT directory is READ-ONLY to the hub: if a directory for this id already exists
        // under the hub's own root but the hub has no row installing it, it belongs to a deployment.
        // Refuse it 409 `conflict` (never touch the tree) - the exact twin of `begin_remove`.
        {
            let target = Layout::for_plugin(&self.root, &id).target;
            let owns = self
                .db
                .plugin(&id)?
                .map(|r| r.installed_at.is_some())
                .unwrap_or(false);
            if !owns && target.join("manifest.json").exists() {
                return Err(PluginError::NotInstalledByHub(id.clone()));
            }
        }
        // Same identity + same request (install of this id) is a retry.
        match self.ids.present(command_id, &format!("install:{id}")) {
            Idempotency::Replay => {
                let row = self.row_for_dir(&id);
                return Ok(InstallIntent::Replay(self.view(&id, Some(row))));
            }
            Idempotency::Conflict => {
                return Err(PluginError::Conflict(command_id.into()));
            }
            Idempotency::New => {}
        }
        let artifact_json = match source {
            crate::source::Source::Artifact(spec) => serde_json::to_string(&serde_json::json!({
                "id": spec.id, "version": spec.version, "url": spec.url,
                "sha256": spec.sha256, "size": spec.size,
                "pluginType": spec.plugin_type,
            }))
            .ok(),
            crate::source::Source::Git { .. } => None,
        };
        // ENTER `installing` and ANNOUNCE **BEFORE the download** (docs/issues/20261007-183000):
        // the contract says a 202 Location names a resource readable, in `installing`, WHILE it
        // fetches. The download runs in the detached `finish_install`, so `installing` is a real,
        // visible duration instead of a ~0ms blip announced only after the bytes had landed.
        self.enter(&id, Op::Installing);
        self.announce(&id);
        Ok(InstallIntent::Proceed { id, source: source.clone(), artifact_json })
    }

    /// The staging directory for the next install (one at a time per root).
    fn staging_dir(&self, id: &str) -> PathBuf {
        // PER-ID staging: two DIFFERENT plugins stage in their own dir and proceed concurrently.
        // (One shared `.stage/install` forced a global lock, which wrongly serialized different
        // ids - a T0.) The id is a plugin id, already validated as a path leaf by the source.
        self.root.join(".stage").join(id)
    }

    /// The per-id operation lock. Two ops on the SAME id conflict; different ids never do.
    fn id_lock(&self, id: &str) -> std::sync::Arc<std::sync::Mutex<()>> {
        let mut m = self.op_locks.lock().expect("op_locks mutex");
        m.entry(id.to_string()).or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(()))).clone()
    }

    /// The detached half of an install: land the tree and commit the row. The
    /// caller has already answered `202`; failures are reported on the event
    /// stream (the resource's `state` becomes `failed`).
    pub fn finish_install(
        &self,
        id: &str,
        source: &crate::source::Source,
        artifact_json: Option<String>,
    ) -> Result<(), PluginError> {
        // DOWNLOAD + STAGE here, in the detached task, so the route could already answer
        // 202 + `installing` (docs/issues/20261007-183000). The per-id lock guards the staging.
        let source_dir = self.staging_dir(id);
        let id_lock = self.id_lock(id);
        let _op = id_lock.lock().expect("id lock");
        if let Some(parent) = source_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_dir_all(&source_dir);
        std::fs::create_dir_all(&source_dir)?;
        let staged: Result<(), crate::source::SourceError> = match source {
            crate::source::Source::Git { url, .. } => {
                let p = PathBuf::from(url);
                if p.is_dir() && p.join("manifest.json").is_file() {
                    copy_dir(&p, &source_dir).map_err(crate::source::SourceError::Io)
                } else {
                    crate::source::resolve(source, &source_dir).map(|_| ())
                }
            }
            crate::source::Source::Artifact(_) => {
                crate::source::resolve(source, &source_dir).map(|_| ())
            }
        };
        if let Err(e) = staged {
            let _ = std::fs::remove_dir_all(&source_dir);
            self.leave(id, Op::Installing);
            self.enter(id, Op::Failed);
            self.announce(id);
            return Err(PluginError::Source(e));
        }
        // The manifest checks move here too (they need the download). A failure marks the plugin
        // `failed` and announces, so the state is never a silent lie.
        let checked: Result<(), PluginError> = (|| {
            let raw = std::fs::read_to_string(source_dir.join("manifest.json"))
                .map_err(|_| PluginError::InvalidManifest("no manifest.json".into()))?;
            let manifest = Manifest::parse(&raw).map_err(PluginError::InvalidManifest)?;
            if let Some(reason) = manifest.invalid_reason() {
                return Err(PluginError::InvalidManifest(reason));
            }
            if let Some(reason) =
                agent_hub_adapter::version::refusal(manifest.min_hub_version.as_deref())
            {
                return Err(PluginError::InvalidManifest(reason));
            }
            Ok(())
        })();
        if let Err(e) = checked {
            let _ = std::fs::remove_dir_all(&source_dir);
            self.leave(id, Op::Installing);
            self.enter(id, Op::Failed);
            self.announce(id);
            return Err(e);
        }
        let record_artifact = artifact_json.clone();
        let manifest = std::fs::read_to_string(source_dir.join("manifest.json"))
            .ok()
            .and_then(|raw| Manifest::parse(&raw).ok());
        let layout = Layout::for_plugin(&self.root, id);
        std::fs::create_dir_all(&self.root)?;
        let result: Result<(), PluginError> =
            agent_hub_db::install(&self.db, id, &layout, &source_dir, None)
                .map(|_| ())
                .map_err(|e| PluginError::InstallFailed(e.to_string()));
        let _ = std::fs::remove_dir_all(&source_dir);

        self.leave(id, Op::Installing);
        match result {
            Ok(_) => {
                let row = PluginRow {
                    id: id.into(),
                    name: manifest.as_ref().and_then(|m| m.name.clone()),
                    summary: manifest.as_ref().and_then(|m| m.summary.clone()),
                    plugin_type: manifest.as_ref().and_then(|m| m.plugin_type.clone()),
                    source: None,
                    reference: None,
                    commit: None,
                    state: PluginState::Ready,
                    detail: None,
                    installed_at: Some(agent_hub_db::now_utc()),
                    artifact: record_artifact.clone(),
                };
                self.db.upsert_plugin(&row)?;
                self.announce(id);
                Ok(())
            }
            Err(e) => {
                let mut row = self.row_for_dir(id);
                row.state = PluginState::Failed;
                row.detail = Some(e.to_string());
                let _ = self.db.upsert_plugin(&row);
                self.announce(id);
                Err(PluginError::InstallFailed(e.to_string()))
            }
        }
    }

    /// Validate a remove **before any work**: the plugin must be one the hub
    /// installed (a deployment directory is refused). Registers the identity.
    pub fn begin_remove(&self, command_id: &str, id: &str) -> Result<RemoveIntent, PluginError> {
        // PER-ID lock: only the SAME plugin conflicts; different plugins proceed concurrently.
        // A conflicting command on the SAME plugin is refused 409 (contract): an in-flight install
        // owns its id, so a remove while installing is refused HERE, cleanly, not as a 404.
        if self.busy(id) {
            return Err(PluginError::InProgress(id.to_string()));
        }
        let id_lock = self.id_lock(id);
        let _op = id_lock
            .try_lock()
            .map_err(|_| PluginError::Conflict("another install/remove of this plugin is in progress".into()))?;
        // Distinguish THREE cases, so the message never lies (docs/issues/20261005-140000):
        //  - a hub-installed plugin (a row with installed_at) -> allowed;
        //  - a DEPLOYMENT directory (no row, but a directory with a manifest exists) ->
        //    NotInstalledByHub (409 `conflict`), naming it a deployment directory;
        //  - NOTHING here (no row, no directory) -> NotFound (404 `not_found`).
        let row = match self.db.plugin(id)? {
            Some(row) => row,
            None => {
                if self.has_dir(id) {
                    return Err(PluginError::NotInstalledByHub(id.into()));
                }
                return Err(PluginError::NotFound(id.into()));
            }
        };
        if row.installed_at.is_none() {
            return Err(PluginError::NotInstalledByHub(id.into()));
        }
        match self.ids.present(command_id, &format!("remove:{id}")) {
            Idempotency::Replay => return Ok(RemoveIntent::Replay),
            Idempotency::Conflict => return Err(PluginError::Conflict(command_id.into())),
            Idempotency::New => {}
        }
        if let Some(op) = self.current_op(id) {
            return Err(PluginError::Busy(id.into(), op.as_str().into()));
        }
        self.enter(id, Op::Removing);
        self.announce(id);
        Ok(RemoveIntent::Proceed)
    }

    /// The detached half of a remove.
    pub fn finish_remove(&self, id: &str) -> Result<(), PluginError> {
        self.db.set_step(id, InstallStep::OldMovedAside)?;
        let layout = Layout::for_plugin(&self.root, id);
        if layout.target.exists() {
            std::fs::remove_dir_all(&layout.target)?;
        }
        self.db.delete_plugin(id)?;
        self.db.clear_step(id)?;
        self.leave(id, Op::Removing);
        self.announce(id);
        Ok(())
    }

    /// A remove that FAILED: the plugin could not be deleted (e.g. a live process
    /// still holds its files). It must NOT stay `removing` forever - the caller can
    /// never learn it failed and the resource lies. Leave `Removing`, record
    /// `failed` with the reason, and announce. The plugin dir is still on disk, so
    /// the verdict is `failed` (docs/issues/20261005-060000).
    pub fn fail_remove(&self, id: &str, reason: &str) {
        self.leave(id, Op::Removing);
        self.enter(id, Op::Failed);
        let mut row = self.row_for_dir(id);
        row.state = PluginState::Failed;
        row.detail = Some(reason.to_string());
        let _ = self.db.upsert_plugin(&row);
        self.announce(id);
    }

    fn current_op(&self, id: &str) -> Option<Op> {
        let ops = self.ops.lock().expect("ops mutex");
        ops.get(id).and_then(|s| s.verdict())
    }
}

/// What `begin_install` decided.
pub enum InstallIntent {
    /// A retry: return the original result, run nothing.
    Replay(PluginView),
    /// First time: run the install detached for this id - download, stage and land.
    Proceed { id: String, source: crate::source::Source, artifact_json: Option<String> },
}

/// What `begin_remove` decided.
pub enum RemoveIntent {
    Replay,
    Proceed,
}

/// Copy a directory tree (used for a LOCAL plugin source used in place).
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}


/// Whether the manifest's declared runtime command exists on disk, relative to the
/// plugin directory (the contract's `runtimeReady`: "the manifest's command exists").
fn runtime_command_present(plugin_dir: &std::path::Path, manifest: Option<&Manifest>) -> bool {
    let Some(m) = manifest else { return false };
    let Some(rt) = m.runtime.as_ref() else { return false };
    // The runtime command is `[program, script, ...]`; the SCRIPT (a relative path) is
    // what must exist. Prefer the last arg that looks like a relative path.
    let argv = match rt.command.as_ref() {
        Some(c) if !c.is_empty() => c,
        _ => return false,
    };
    for arg in argv.iter().rev() {
        if arg.starts_with('-') {
            continue;
        }
        let p = std::path::Path::new(arg);
        let full = if p.is_absolute() { p.to_path_buf() } else { plugin_dir.join(p) };
        if full.is_file() {
            return true;
        }
    }
    false
}


/// The absolute path of the manifest's runtime script (the last non-flag arg),
/// resolved against the plugin dir. `None` when the manifest declares no command.
fn runtime_script(plugin_dir: &std::path::Path, manifest: Option<&Manifest>) -> Option<String> {
    let argv = manifest?.runtime.as_ref()?.command.as_ref()?;
    for arg in argv.iter().rev() {
        if arg.starts_with('-') {
            continue;
        }
        let p = std::path::Path::new(arg);
        let full = if p.is_absolute() { p.to_path_buf() } else { plugin_dir.join(p) };
        if full.is_file() {
            return Some(full.to_string_lossy().to_string());
        }
    }
    None
}
