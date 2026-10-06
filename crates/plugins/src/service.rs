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
    #[error("install failed: {0}")]
    InstallFailed(String),
    #[error(transparent)]
    Source(#[from] crate::source::SourceError),
    #[error("no registry URL is configured")]
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
            PluginError::InstallFailed(_) => "plugin_install_failed",
            PluginError::Source(e) => e.code(),
            PluginError::RegistryUrlMissing(_) => "plugin_install_failed",
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
    /// Serializes install/remove: the staging dir is ONE per root (the comment on
    /// `staging_dir` says "one at a time per root"), so two installs landing at once
    /// trample it and fail with os error 5. Holding this lock makes install/remove
    /// ONE at a time; a second concurrent call is refused 409, never a 500
    /// (docs/issues/20261005-070000).
    op_lock: std::sync::Arc<std::sync::Mutex<()>>,
}

impl Plugins {
    /// The registry file the hub reads: `AGENT_HUB_REGISTRY_FILE`, else
    /// `<DATA_DIR>/registry.json` (beside the plugin root's parent).
    fn registry_file(&self) -> PathBuf {
        std::env::var("AGENT_HUB_REGISTRY_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                self.root
                    .parent()
                    .map(|p| p.join("registry.json"))
                    .unwrap_or_else(|| PathBuf::from("registry.json"))
            })
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

    /// Refresh the catalog: read `AGENT_HUB_REGISTRY_URL` and write it where the hub
    /// reads the catalog. This is the ONLY way the URL is contacted - never at
    /// startup, never silently.
    pub async fn refresh_registry(&self) -> Result<serde_json::Value, PluginError> {
        let url = std::env::var("AGENT_HUB_REGISTRY_URL")
            .map_err(|_| PluginError::RegistryUrlMissing("AGENT_HUB_REGISTRY_URL is not set".into()))?;
        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .map_err(|e| PluginError::RegistryUrlMissing(format!("registry fetch failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(PluginError::RegistryUrlMissing(format!(
                "registry fetch failed: HTTP {}",
                resp.status()
            )));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| PluginError::RegistryUrlMissing(format!("registry read failed: {e}")))?;
        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| PluginError::RegistryUrlMissing(format!("registry is not JSON: {e}")))?;
        let count = parsed
            .get("plugins")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .ok_or_else(|| PluginError::RegistryUrlMissing("registry has no plugins array".into()))?;
        let file = self.registry_file();
        std::fs::write(&file, &text)?;
        Ok(serde_json::json!({ "source": file.to_string_lossy(), "plugins": count }))
    }

    pub fn new(db: Db, root: impl Into<PathBuf>, bus: Bus) -> Self {
        Plugins {
            db,
            root: root.into(),
            bus,
            ids: CommandIds::new(4096),
            ops: std::sync::Mutex::new(std::collections::HashMap::new()),
            op_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
        }
    }

    fn enter(&self, id: &str, op: Op) {
        let mut ops = self.ops.lock().expect("ops mutex");
        ops.entry(id.to_string()).or_default().enter(op);
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
            views.push(self.view(&id, row));
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
                    views.push(self.view(&name, self.row_for_dir(&name)));
                }
            }
        }
        views.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(views)
    }

    /// Read one plugin (the resource a `202 Location` names).
    pub fn get(&self, id: &str) -> Result<PluginView, PluginError> {
        // A hub-installed plugin has a row; a deployment directory does not.
        match self.db.plugin(id)? {
            Some(row) => Ok(self.view(id, row)),
            None if self.has_dir(id) => Ok(self.view(id, self.row_for_dir(id))),
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

    fn view(&self, id: &str, row: PluginRow) -> PluginView {
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
        // ONE install at a time per root: the staging dir is shared, so two installs
        // landing at once trample it (os error 5). A concurrent install is refused
        // 409, never a 500 (docs/issues/20261005-070000). The guard is held for the
        // synchronous staging below; the detached finish works on the per-id target,
        // which the `ops` map already serializes per id.
        let _op = self
            .op_lock
            .try_lock()
            .map_err(|_| PluginError::Conflict("another install/remove is in progress".into()))?;
        let staging = self.staging_dir();
        if let Some(parent) = staging.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        let artifact_json = match source {
            crate::source::Source::Artifact(spec) => serde_json::to_string(&serde_json::json!({
                "id": spec.id, "version": spec.version, "url": spec.url,
                "sha256": spec.sha256, "size": spec.size,
                "pluginType": spec.plugin_type,
            }))
            .ok(),
            crate::source::Source::Git { .. } => None,
        };
        let staged: Result<(), crate::source::SourceError> = match source {
            crate::source::Source::Git { url, .. } => {
                let p = PathBuf::from(url);
                if p.is_dir() && p.join("manifest.json").is_file() {
                    copy_dir(&p, &staging).map_err(crate::source::SourceError::Io)
                } else {
                    crate::source::resolve(source, &staging).map(|_| ())
                }
            }
            crate::source::Source::Artifact(_) => {
                crate::source::resolve(source, &staging).map(|_| ())
            }
        };
        if let Err(e) = staged {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(PluginError::Source(e));
        }
        let raw = std::fs::read_to_string(staging.join("manifest.json"))
            .map_err(|_| PluginError::InvalidManifest("no manifest.json".into()))?;
        let manifest = Manifest::parse(&raw).map_err(PluginError::InvalidManifest)?;
        if let Some(reason) = manifest.invalid_reason() {
            return Err(PluginError::InvalidManifest(reason));
        }
        // The minimum-host gate (ADR-0008): a plugin that needs a newer hub is
        // REFUSED here, before it lands, with a named reason. There is no
        // activation of a plugin the host cannot support.
        if let Some(reason) = agent_hub_adapter::version::refusal(manifest.min_hub_version.as_deref())
        {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(PluginError::InvalidManifest(reason));
        }
        let id = manifest
            .id
            .clone()
            .ok_or_else(|| PluginError::InvalidManifest("manifest declares no id".into()))?;

        // Same identity + same request (install of this id) is a retry.
        match self.ids.present(command_id, &format!("install:{id}")) {
            Idempotency::Replay => {
                let _ = std::fs::remove_dir_all(&staging);
                let row = self.row_for_dir(&id);
                return Ok(InstallIntent::Replay(self.view(&id, row)));
            }
            Idempotency::Conflict => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(PluginError::Conflict(command_id.into()));
            }
            Idempotency::New => {}
        }

        // A conflicting command on the same plugin is refused while one runs.
        if let Some(op) = self.current_op(&id) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(PluginError::Busy(id.clone(), op.as_str().into()));
        }

        self.enter(&id, Op::Installing);
        self.announce(&id);
        Ok(InstallIntent::Proceed { id, artifact_json })
    }

    /// The staging directory for the next install (one at a time per root).
    fn staging_dir(&self) -> PathBuf {
        self.root.join(".stage").join("install")
    }

    /// The detached half of an install: land the tree and commit the row. The
    /// caller has already answered `202`; failures are reported on the event
    /// stream (the resource's `state` becomes `failed`).
    pub fn finish_install(
        &self,
        id: &str,
        artifact_json: Option<String>,
    ) -> Result<(), PluginError> {
        let source_dir = self.staging_dir();
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
        // Same one-at-a-time rule as install (the staging/root is shared).
        let _op = self
            .op_lock
            .try_lock()
            .map_err(|_| PluginError::Conflict("another install/remove is in progress".into()))?;
        let row = self
            .db
            .plugin(id)?
            .ok_or_else(|| PluginError::NotInstalledByHub(id.into()))?;
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
    /// First time: run the install detached for this id, landing the staged tree.
    Proceed { id: String, artifact_json: Option<String> },
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
