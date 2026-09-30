//! The adapter domain (`ARCHITECTURE` §4): spawn out-of-process adapters, route
//! capability calls, and map their notifications to events.
//!
//! A harness is a plugin that declares the adapter protocol. The hub registers
//! it on first sight, starts its adapter when a session needs it, and translates
//! its notifications into the contract's events. No adapter code runs in-process.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::bus::{AgentBus, Notification, RequestHandle};
use crate::manifest::AdapterManifest;
use agent_hub_events::Bus;

/// A registered harness (a present adapter plugin).
#[derive(Debug, Clone)]
pub struct Harness {
    pub id: String,
    pub directory: std::path::PathBuf,
    pub manifest: AdapterManifest,
    pub status: HarnessStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessStatus {
    Enabled,
    Disabled,
}

impl HarnessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            HarnessStatus::Enabled => "enabled",
            HarnessStatus::Disabled => "disabled",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("harness `{0}` not found")]
    NotFound(String),
    #[error("this harness does not support `{0}`")]
    Unsupported(String),
    #[error(transparent)]
    Manifest(#[from] crate::manifest::ManifestError),
    #[error(transparent)]
    Bus(#[from] crate::bus::BusError),
    #[error("extension placement failed: {0}")]
    Placement(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Where the hub's data for one harness lives, so the adapter can be pointed at
/// it. The hub owns this layout; the adapter receives it as `AGENT_HUB_*`
/// variables (`contract/adapter-v1.json`, `contract/v1.json`).
#[derive(Debug, Clone)]
pub struct HarnessEnv {
    /// `AGENT_HUB_HARNESS_DIR`: per-harness data (sessions, credentials state).
    pub harness_dir: std::path::PathBuf,
    /// `AGENT_HUB_INSTALLED_SKILLS_DIR`: skills the hub installed.
    pub skills_dir: std::path::PathBuf,
    /// `AGENT_HUB_INSTALLED_EXTENSIONS_DIR`: extensions the hub installed.
    pub extensions_dir: std::path::PathBuf,
    /// The working directory for a session (`AGENT_HUB_CWD`).
    pub cwd: std::path::PathBuf,
    /// The session id (`AGENT_HUB_SESSION_ID`), when the spawn is per session.
    pub session_id: Option<String>,
    /// Extra roots (`AGENT_HUB_ADDITIONAL_DIRS`).
    pub additional_dirs: Vec<String>,
    /// The preset directory (`AGENT_HUB_PRESETS_DIR`), when presets are active.
    pub presets_dir: Option<std::path::PathBuf>,
    /// Connection credentials keyed by env name (`envName`), never written down.
    pub connection_env: Vec<(String, String)>,
}

/// The adapter domain: the harness registry and the running adapters.
pub struct Adapters {
    /// The plugin roots the hub searches, in order.
    roots: Vec<std::path::PathBuf>,
    /// The hub's data root; per-harness dirs are derived from it.
    data_dir: std::path::PathBuf,
    harnesses: Mutex<HashMap<String, Harness>>,
    running: Mutex<HashMap<String, RequestHandle>>,
    events: Bus,
}

impl Adapters {
    pub fn new(roots: Vec<std::path::PathBuf>, data_dir: impl Into<std::path::PathBuf>, events: Bus) -> Self {
        Adapters {
            roots,
            data_dir: data_dir.into(),
            harnesses: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            events,
        }
    }

    /// The environment a harness's adapter is started with, derived from the
    /// hub's own layout. The hub creates the directories it hands over; an
    /// adapter refuses to invent its own (the real ones do).
    ///
    /// A **declared** extension whose source directory is missing, or whose copy
    /// fails, is an **error** that must stop the start (TASK-048 C3). Placement
    /// has one implementation (the extensions crate's rule); a missing trust
    /// component is never a warning that still spawns.
    pub fn harness_env(
        &self,
        id: &str,
        session_id: Option<String>,
    ) -> Result<HarnessEnv, AdapterError> {
        let base = self.data_dir.join("agents").join(id);
        let harness_dir = base.clone();
        let skills_dir = base.join("skills");
        let extensions_dir = base.join("extensions");
        std::fs::create_dir_all(&harness_dir)?;
        std::fs::create_dir_all(&skills_dir)?;

        // Replace the installed set (a removed extension is gone next run).
        let _ = std::fs::remove_dir_all(&extensions_dir);
        std::fs::create_dir_all(&extensions_dir)?;

        // A harness that declares `presets` reads definitions from its own
        // `<plugin>/presets/` dir; point the adapter there.
        let mut presets_dir = None;
        if let Ok(harness) = self.get(id) {
            if harness.manifest.capabilities.iter().any(|c| c == "presets") {
                let p = harness.directory.join("presets");
                if p.is_dir() {
                    presets_dir = Some(p);
                }
            }
            // ONE placement implementation: the extensions crate's rule. It
            // refuses a declared id with no directory (SourceMissing); the start
            // does not swallow it.
            let shipped: Vec<(String, std::path::PathBuf)> = harness
                .manifest
                .shipped_extensions(&harness.directory)
                .into_iter()
                .map(|e| (e.id, e.dir))
                .collect();
            let selected: Vec<String> = shipped.iter().map(|(id, _)| id.clone()).collect();
            agent_hub_extensions::install_for_harness(
                &self.data_dir.join("agents"),
                id,
                &shipped,
                &selected,
            )
            .map_err(|e| AdapterError::Placement(e.to_string()))?;
        }

        Ok(HarnessEnv {
            harness_dir,
            skills_dir,
            extensions_dir,
            cwd: self.data_dir.clone(),
            session_id,
            additional_dirs: Vec::new(),
            presets_dir,
            connection_env: Vec::new(),
        })
    }

    /// Scan the roots and register every adapter plugin found. A directory with
    /// no manifest, or a manifest that fails validation, is skipped (and the
    /// reason logged) - never a hard failure of startup.
    pub fn scan(&self) -> Vec<Harness> {
        let mut found = Vec::new();
        for root in &self.roots {
            let Ok(entries) = std::fs::read_dir(root) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                match AdapterManifest::load(&path, &name) {
                    Ok(manifest) => {
                        if manifest.plugin_type.as_deref() != Some("harness-adapter") {
                            continue;
                        }
                        let harness = Harness {
                            id: name.clone(),
                            directory: path,
                            manifest,
                            status: HarnessStatus::Enabled,
                        };
                        self.harnesses
                            .lock()
                            .expect("harnesses")
                            .insert(name.clone(), harness.clone());
                        found.push(harness);
                    }
                    Err(e) => tracing::debug!(plugin = %name, error = %e, "not an adapter plugin"),
                }
            }
        }
        found.sort_by(|a, b| a.id.cmp(&b.id));
        found
    }

    pub fn list(&self) -> Vec<Harness> {
        let mut out: Vec<Harness> = self.harnesses.lock().expect("harnesses").values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn get(&self, id: &str) -> Result<Harness, AdapterError> {
        self.harnesses
            .lock()
            .expect("harnesses")
            .get(id)
            .cloned()
            .ok_or_else(|| AdapterError::NotFound(id.into()))
    }

    pub fn set_status(&self, id: &str, status: HarnessStatus) -> Result<Harness, AdapterError> {
        let mut map = self.harnesses.lock().expect("harnesses");
        let harness = map.get_mut(id).ok_or_else(|| AdapterError::NotFound(id.into()))?;
        harness.status = status;
        self.events.publish(
            "harness.changed",
            json!({ "id": id, "status": status.as_str() }),
        );
        Ok(harness.clone())
    }

    /// Start an adapter for a harness (idempotent: one process per harness).
    pub async fn ensure_started(&self, id: &str) -> Result<RequestHandle, AdapterError> {
        if let Some(handle) = self.running.lock().expect("running").get(id) {
            return Ok(handle.clone());
        }
        let harness = self.get(id)?;
        if harness.status == HarnessStatus::Disabled {
            return Err(AdapterError::Unsupported(format!("harness `{id}` is disabled")));
        }
        let command = harness.manifest.command.clone().unwrap_or_default();
        let env = self.adapter_env(&harness)?;
        let bus = AgentBus::spawn(&command, &harness.directory, &env)?;
        let handle = bus.requests.clone();
        self.running
            .lock()
            .expect("running")
            .insert(id.to_string(), handle.clone());

        // Pump notifications -> events in the background. Requests and
        // notifications never contend (the handle is separate).
        let events = self.events.clone();
        let harness_id = id.to_string();
        tokio::spawn(async move {
            // Keep the child alive for the adapter's lifetime.
            let mut bus = bus;
            while let Some(n) = bus.notifications.recv().await {
                publish_notification(&events, &harness_id, n);
            }
        });
        Ok(handle)
    }

    /// The `AGENT_HUB_*` environment for an adapter, plus the connection
    /// credentials, exactly as the contract and the old hub define it: the
    /// process environment with `AGENT_HUB_SECRET_KEY` removed, then the hub's
    /// own variables (`server.mjs:802/2964`).
    pub fn adapter_env(&self, harness: &Harness) -> Result<Vec<(String, String)>, AdapterError> {
        let mut env: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k != "AGENT_HUB_SECRET_KEY")
            .collect();
        let he = self.harness_env(&harness.id, None)?;
        env.push(("AGENT_HUB_HARNESS_DIR".into(), he.harness_dir.to_string_lossy().into()));
        env.push(("AGENT_HUB_CWD".into(), he.cwd.to_string_lossy().into()));
        env.push(("AGENT_HUB_INSTALLED_SKILLS_DIR".into(), he.skills_dir.to_string_lossy().into()));
        env.push(("AGENT_HUB_INSTALLED_EXTENSIONS_DIR".into(), he.extensions_dir.to_string_lossy().into()));
        if !he.additional_dirs.is_empty() {
            env.push(("AGENT_HUB_ADDITIONAL_DIRS".into(), serde_json::to_string(&he.additional_dirs).unwrap()));
        }
        if let Some(p) = &he.presets_dir {
            env.push(("AGENT_HUB_PRESETS_DIR".into(), p.to_string_lossy().into()));
        }
        if let Some(argv) = harness.manifest.runtime_argv(&harness.directory) {
            env.push(("AGENT_HUB_RUNTIME_COMMAND".into(), serde_json::to_string(&argv).unwrap()));
        }
        Ok(env)
    }

    /// A capability call, gated on the harness declaring it.
    pub async fn call(
        &self,
        id: &str,
        capability: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, AdapterError> {
        let harness = self.get(id)?;
        if !harness.manifest.declares(capability) {
            return Err(AdapterError::Unsupported(capability.into()));
        }
        let handle = self.ensure_started(id).await?;
        Ok(handle.request(method, params).await?)
    }
}

/// Map an adapter notification to the contract's event name and payload.
fn publish_notification(events: &Bus, harness_id: &str, n: Notification) {
    // The adapter's method names map onto the contract's event names; the
    // harness id is always attached so a subscriber can route.
    let mut payload = n.params.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("harnessId".into(), json!(harness_id));
    }
    events.publish(n.method, payload);
}
