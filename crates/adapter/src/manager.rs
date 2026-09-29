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
}

/// The adapter domain: the harness registry and the running adapters.
pub struct Adapters {
    /// The plugin roots the hub searches, in order.
    roots: Vec<std::path::PathBuf>,
    harnesses: Mutex<HashMap<String, Harness>>,
    running: Mutex<HashMap<String, RequestHandle>>,
    events: Bus,
}

impl Adapters {
    pub fn new(roots: Vec<std::path::PathBuf>, events: Bus) -> Self {
        Adapters {
            roots,
            harnesses: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            events,
        }
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
        let bus = AgentBus::spawn(&command, &harness.directory, &[])?;
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
