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
            PluginError::NotInstalledByHub(_) => "plugin_remove_failed",
            PluginError::InUse(_, _) => "plugin_in_use",
            PluginError::Busy(_, _) => "plugin_dir_busy",
            PluginError::InvalidManifest(_) => "plugin_archive_invalid",
            PluginError::Conflict(_) => "idempotency_conflict",
            PluginError::InstallFailed(_) => "plugin_install_failed",
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
                        "package": r.package, "version": r.version, "target": r.target
                    })
                }),
            runtime_ready: false,
            state: self.state_of(id),
            prepare: None,
            invalid,
            artifact: None,
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
        source_dir: &Path,
    ) -> Result<InstallIntent, PluginError> {
        let raw = std::fs::read_to_string(source_dir.join("manifest.json"))
            .map_err(|_| PluginError::InvalidManifest("no manifest.json".into()))?;
        let manifest = Manifest::parse(&raw).map_err(PluginError::InvalidManifest)?;
        if let Some(reason) = manifest.invalid_reason() {
            return Err(PluginError::InvalidManifest(reason));
        }
        let id = manifest
            .id
            .clone()
            .ok_or_else(|| PluginError::InvalidManifest("manifest declares no id".into()))?;

        // Same identity + same request (install of this id) is a retry.
        match self.ids.present(command_id, &format!("install:{id}")) {
            Idempotency::Replay => {
                let row = self.row_for_dir(&id);
                return Ok(InstallIntent::Replay(self.view(&id, row)));
            }
            Idempotency::Conflict => return Err(PluginError::Conflict(command_id.into())),
            Idempotency::New => {}
        }

        // A conflicting command on the same plugin is refused while one runs.
        if let Some(op) = self.current_op(&id) {
            return Err(PluginError::Busy(id.clone(), op.as_str().into()));
        }

        self.enter(&id, Op::Installing);
        self.announce(&id);
        Ok(InstallIntent::Proceed { id })
    }

    /// The detached half of an install: land the tree and commit the row. The
    /// caller has already answered `202`; failures are reported on the event
    /// stream (the resource's `state` becomes `failed`).
    pub fn finish_install(&self, id: &str, source_dir: &Path) -> Result<(), PluginError> {
        let manifest = std::fs::read_to_string(source_dir.join("manifest.json"))
            .ok()
            .and_then(|raw| Manifest::parse(&raw).ok());
        let layout = Layout::for_plugin(&self.root, id);
        std::fs::create_dir_all(&self.root)?;

        std::fs::create_dir_all(&self.root)?;
        let result: Result<(), PluginError> =
            agent_hub_db::install(&self.db, id, &layout, source_dir, None)
                .map(|_| ())
                .map_err(|e| PluginError::InstallFailed(e.to_string()));

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

    fn current_op(&self, id: &str) -> Option<Op> {
        let ops = self.ops.lock().expect("ops mutex");
        ops.get(id).and_then(|s| s.verdict())
    }
}

/// What `begin_install` decided.
pub enum InstallIntent {
    /// A retry: return the original result, run nothing.
    Replay(PluginView),
    /// First time: run the install detached for this id.
    Proceed { id: String },
}

/// What `begin_remove` decided.
pub enum RemoveIntent {
    Replay,
    Proceed,
}
