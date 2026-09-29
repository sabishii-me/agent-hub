//! The session runtime: one real adapter process **per session** (`ARCHITECTURE`
//! §6), started with `session/start` then configured with `config/set`.
//!
//! A session is NOT the harness-level adapter the `adapter` crate manages. That
//! manager keeps one process per *harness*; a session owns its **own** process,
//! with its own `AGENT_HUB_SESSION_ID`, cwd and native `ref`. Close kills that
//! session's process (the adapter exits on stdin EOF / SIGKILL); the record stays
//! and the session becomes `readonly`.
//!
//! `active` is reported **only** when the handshake AND the config both really
//! succeeded. There is no path that writes `active` on a row alone (TASK-048 F01).

use std::path::PathBuf;
use std::sync::Mutex;

use agent_hub_adapter::AgentBus;
use serde_json::{json, Value};

/// A running session's process and its native ref.
pub struct SessionProcess {
    pub sid: String,
    pub harness_id: String,
    pub native_ref: String,
    bus: Option<AgentBus>,
    /// What the adapter reported as applied by `config/set` (proof, not a claim).
    pub applied: Value,
}

impl SessionProcess {
    /// Stop the session's process and CONFIRM it exited. Idempotent (a second
    /// stop is a no-op success). Returns an error if the process could not be
    /// confirmed stopped, so a caller never claims a release that did not happen.
    pub async fn stop(&mut self) -> Result<(), StartError> {
        if let Some(bus) = self.bus.take() {
            bus.shutdown().await.map_err(|e| StartError::Spawn(e.to_string()))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct StartSpec {
    pub sid: String,
    pub harness_id: String,
    /// The adapter argv (the plugin manifest's `command`).
    pub command: Vec<String>,
    /// The plugin directory (the adapter's cwd for `command`).
    pub plugin_dir: PathBuf,
    /// The session's working directory (`AGENT_HUB_CWD`).
    pub cwd: PathBuf,
    /// `AGENT_HUB_HARNESS_DIR` (shared per harness).
    pub harness_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub extensions_dir: PathBuf,
    /// The runtime argv, absolute (`AGENT_HUB_RUNTIME_COMMAND`).
    pub runtime_argv: Option<Vec<String>>,
    /// A native ref to resume, when reopening.
    pub resume: Option<String>,
    /// The `config/set` payload (may be `{}`).
    pub config: Value,
}

/// The session runtime: the running session processes, keyed by sid.
pub struct Sessions {
    running: Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<SessionProcess>>>>,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Sessions {
    pub fn new() -> Self {
        Sessions { running: Mutex::new(std::collections::HashMap::new()) }
    }

    /// Start a session against a real adapter: spawn the process, run
    /// `session/start`, then `config/set`. Only a fully successful start returns
    /// `Ok`; any failure stops the process and returns an error, so a caller can
    /// never see `active` without a real process behind it.
    pub async fn start(&self, spec: StartSpec) -> Result<SessionProcess, StartError> {
        let env = build_env(&spec);
        let bus = AgentBus::spawn(&spec.command, &spec.plugin_dir, &env)
            .map_err(|e| StartError::Spawn(e.to_string()))?;

        // session/start
        let mut params = json!({ "sid": spec.sid });
        if let Some(r) = &spec.resume {
            params["resume"] = json!(r);
        }
        let started = bus
            .requests
            .request("session/start", params)
            .await
            .map_err(|e| StartError::Protocol(e.to_string()))?;
        let native_ref = match started.get("ref").and_then(Value::as_str) {
            Some(r) => r.to_string(),
            None => {
                let _ = bus.shutdown().await;
                return Err(StartError::Protocol("the adapter did not return a native ref".into()));
            }
        };

        // config/set (the hub's materialization must be applied before a turn)
        let applied = match bus
            .requests
            .request("config/set", json!({ "sid": spec.sid, "config": spec.config }))
            .await
        {
            Ok(v) => v.get("applied").cloned().unwrap_or(Value::Null),
            Err(e) => {
                let _ = bus.shutdown().await;
                return Err(StartError::Protocol(format!("config/set failed: {e}")));
            }
        };

        let sid = spec.sid.clone();
        let harness_id = spec.harness_id.clone();
        let applied_out = applied.clone();
        let process = SessionProcess {
            sid: sid.clone(),
            harness_id,
            native_ref: native_ref.clone(),
            bus: Some(bus),
            applied,
        };
        self.running
            .lock()
            .expect("running")
            .insert(sid, std::sync::Arc::new(tokio::sync::Mutex::new(process)));
        // Return a description (the process itself lives in the map). The bus
        // stays in the map entry; this value is the caller's record of it.
        Ok(SessionProcess {
            sid: spec.sid.clone(),
            harness_id: spec.harness_id.clone(),
            native_ref,
            bus: None,
            applied: applied_out,
        })
    }

    /// Whether a process is running for this session.
    pub fn is_running(&self, sid: &str) -> bool {
        self.running.lock().expect("running").contains_key(sid)
    }

    /// Stop and drop a session's process. Idempotent. Returns an error if the
    /// process could not be confirmed stopped (the entry is kept so a retry can
    /// try again).
    pub async fn stop(&self, sid: &str) -> Result<(), StartError> {
        let handle = self.running.lock().expect("running").get(sid).cloned();
        match handle {
            None => Ok(()),
            Some(handle) => {
                let mut guard = handle.lock().await;
                guard.stop().await?;
                drop(guard);
                self.running.lock().expect("running").remove(sid);
                Ok(())
            }
        }
    }
}

fn build_env(spec: &StartSpec) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| k != "AGENT_HUB_SECRET_KEY")
        .collect();
    env.push(("AGENT_HUB_HARNESS_DIR".into(), spec.harness_dir.to_string_lossy().into()));
    env.push(("AGENT_HUB_CWD".into(), spec.cwd.to_string_lossy().into()));
    env.push(("AGENT_HUB_SESSION_ID".into(), spec.sid.clone()));
    env.push(("AGENT_HUB_INSTALLED_SKILLS_DIR".into(), spec.skills_dir.to_string_lossy().into()));
    env.push(("AGENT_HUB_INSTALLED_EXTENSIONS_DIR".into(), spec.extensions_dir.to_string_lossy().into()));
    if let Some(argv) = &spec.runtime_argv {
        env.push(("AGENT_HUB_RUNTIME_COMMAND".into(), serde_json::to_string(argv).unwrap()));
    }
    env
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("could not start the adapter: {0}")]
    Spawn(String),
    #[error("adapter protocol: {0}")]
    Protocol(String),
}

impl StartError {
    pub fn code(&self) -> &'static str {
        match self {
            StartError::Spawn(_) => "adapter_unreachable",
            StartError::Protocol(_) => "adapter_crash",
        }
    }
}
