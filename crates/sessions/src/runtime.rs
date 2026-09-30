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
    /// A cloneable handle to send requests on this session's process (a turn's
    /// `session/prompt`/`session/abort`). Kept even while the bus lives.
    requests: agent_hub_adapter::RequestHandle,
    /// What the adapter reported as applied by `config/set` (proof, not a claim).
    pub applied: Value,
    /// The provider identity the adapter confirmed it applied (`applied
    /// .modelProviderId` / its resolved native route), read from the proof.
    pub applied_provider: Option<String>,
    /// The model the adapter confirmed (from the applied proof, not the request).
    pub applied_model: Option<String>,
    /// The resolved native route the adapter confirmed (`applied.connectionId` /
    /// native route), when it reports one.
    pub applied_route: Option<String>,
    /// The preset the adapter confirmed (`applied.preset`).
    pub applied_preset: Option<String>,
    /// The plan state the adapter confirmed (`applied.plan`).
    pub applied_plan: Option<bool>,
    /// The review state the adapter confirmed (`applied.review`).
    pub applied_review: Option<bool>,
}

impl SessionProcess {
    /// Send a request on this session's process (a turn's prompt/abort).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, StartError> {
        self.requests
            .request(method, params)
            .await
            .map_err(|e| StartError::Protocol(e.to_string()))
    }

    /// Stop the session's process and CONFIRM it exited. Idempotent. On a failure
    /// the handle is **kept**, so a retry can try again - a failed stop never
    /// leaves an unrecoverable entry (N3).
    pub async fn stop(&mut self) -> Result<(), StartError> {
        if self.bus.is_none() {
            return Ok(()); // already stopped
        }
        let bus = self.bus.as_mut().expect("checked above");
        match bus.shutdown().await {
            Ok(()) => {
                self.bus = None; // released: drop the handle
                Ok(())
            }
            // The handle is KEPT; a second stop can retry (N3).
            Err(e) => Err(StartError::Spawn(e.to_string())),
        }
    }
}

/// A fork source: the source session's native ref and an optional 1-based
/// completed-turn anchor.
#[derive(Debug, Clone)]
pub struct ForkFrom {
    pub source_ref: String,
    pub through_turn: Option<u32>,
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
    /// The plugin's presets dir (`AGENT_HUB_PRESETS_DIR`), when declared.
    pub presets_dir: Option<PathBuf>,
    /// The runtime argv, absolute (`AGENT_HUB_RUNTIME_COMMAND`).
    pub runtime_argv: Option<Vec<String>>,
    /// A native ref to resume, when reopening.
    pub resume: Option<String>,
    /// The `config/set` payload (may be `{}`).
    pub config: Value,
    /// When set, the process is spawned as a FORK of another session: the first
    /// request is `session/fork {sid, from, throughTurn}` instead of
    /// `session/start`. `from` is the SOURCE's native ref; the returned ref becomes
    /// THIS session's own ref (the source is untouched).
    pub fork_from: Option<ForkFrom>,
    /// The preset that was requested (confirmed against `applied.preset`).
    pub requested_preset_id: Option<String>,
    /// The plan mode requested (confirmed against `applied.plan`), if any.
    pub requested_plan: Option<bool>,
    /// The review switch requested (confirmed against `applied.review`), if any.
    pub requested_review: Option<bool>,
    /// The credential grant for a hub-managed provider (`credentials/grant`),
    /// sent AFTER `session/start` and BEFORE `config/set` (`adapter-v1:345-353`).
    /// Memory-only: the adapter does not persist it, so it is re-sent on every
    /// start/reopen (a restart re-grants).
    pub grant: Option<Grant>,
}

/// A resolved credential for one hub-managed provider: the adapter receives the
/// value and its endpoint/declarations and materialises a `hub-<id>` provider
/// entry for its own harness. The hub owns the secret; the adapter never reads
/// the keychain.
#[derive(Debug, Clone)]
pub struct Grant {
    /// The hub's provider id (the adapter names it `hub-<connectionId>`).
    pub connection_id: String,
    /// The credential value (from the OS secret store; never logged).
    pub value: String,
    pub url: Option<String>,
    pub api: Option<String>,
    pub declarations: Option<Value>,
    /// The provider id AS THE HUB requested it, recorded from the adapter's
    /// `applied` proof (never assumed).
    pub requested_provider_id: String,
    /// The model to select within that provider's route.
    pub requested_model_id: Option<String>,
}

/// The session runtime: the running session processes, keyed by sid.
pub struct Sessions {
    running: Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<SessionProcess>>>>,
    /// A cloneable request handle per session, OUTSIDE the process mutex, so a
    /// turn's prompt/abort never contends with the lifecycle lock.
    requests: Mutex<std::collections::HashMap<String, agent_hub_adapter::RequestHandle>>,
    /// Where an adapter's notifications become events.
    events: agent_hub_events::Bus,
}

impl Sessions {
    pub fn new(events: agent_hub_events::Bus) -> Self {
        Sessions {
            running: Mutex::new(std::collections::HashMap::new()),
            requests: Mutex::new(std::collections::HashMap::new()),
            events,
        }
    }

    /// Start a session against a real adapter: spawn the process, run
    /// `session/start`, then `config/set`. Only a fully successful start returns
    /// `Ok`; any failure stops the process and returns an error, so a caller can
    /// never see `active` without a real process behind it.
    pub async fn start(&self, spec: StartSpec) -> Result<SessionProcess, StartError> {
        let env = build_env(&spec);
        let mut bus = AgentBus::spawn(&spec.command, &spec.plugin_dir, &env)
            .map_err(|e| StartError::Spawn(e.to_string()))?;

        // The FIRST request: `session/fork` for a fork (this process becomes the
        // child conversation), else `session/start` (open or resume).
        let (method, params) = match &spec.fork_from {
            Some(f) => {
                let mut p = json!({ "sid": spec.sid, "from": f.source_ref });
                if let Some(n) = f.through_turn {
                    p["throughTurn"] = json!(n);
                }
                ("session/fork", p)
            }
            None => {
                let mut p = json!({ "sid": spec.sid });
                if let Some(r) = &spec.resume {
                    p["resume"] = json!(r);
                }
                ("session/start", p)
            }
        };
        let started = bus
            .requests
            .request(method, params)
            .await
            .map_err(|e| StartError::Protocol(e.to_string()))?;
        let native_ref = match started.get("ref").and_then(Value::as_str) {
            Some(r) => r.to_string(),
            None => {
                let _ = bus.shutdown().await;
                return Err(StartError::Protocol("the adapter did not return a native ref".into()));
            }
        };

        // credentials/grant: the hub-managed provider's credential (memory-only in
        // the adapter). Sent before config/set so the provider exists when the
        // config selects it. A grant failure is a start failure - we do NOT
        // pretend a session is active with a provider it did not receive.
        if let Some(grant) = &spec.grant {
            let params = json!({
                "connectionId": grant.connection_id,
                "value": grant.value,
                "url": grant.url,
                "declarations": grant.declarations,
            });
            if let Err(e) = bus.requests.request("credentials/grant", params).await {
                let _ = bus.shutdown().await;
                return Err(StartError::Protocol(format!("credentials/grant failed: {e}")));
            }
        }

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
        let applied_provider = applied
            .get("modelProviderId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let applied_model = applied
            .get("model")
            .or_else(|| applied.get("modelId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let applied_route = applied
            .get("connectionId")
            .or_else(|| applied.get("nativeRoute"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let applied_preset = applied
            .get("preset")
            .and_then(Value::as_str)
            .map(str::to_string);
        // Confirm the requested preset was applied. A requested preset that the
        // adapter did not confirm applied is a start failure (never a silent
        // default): the session would otherwise run the wrong composition.
        if let Some(want) = spec.requested_preset_id.as_deref() {
            if applied_preset.as_deref() != Some(want) {
                let _ = bus.shutdown().await;
                return Err(StartError::Protocol(format!(
                    "the adapter did not confirm the requested preset `{want}`: applied={applied}"
                )));
            }
        }
        let applied_plan = applied.get("plan").and_then(Value::as_bool);
        let applied_review = applied.get("review").and_then(Value::as_bool);
        // plan/review are confirmed like the preset: a requested value the adapter
        // did not confirm is not recorded as applied. (These are advisory - a
        // harness that does not report the switch is not a start failure, but we
        // must not claim the requested value either: applied stays null.)
        let plan_confirmed = spec.requested_plan.is_none() || applied_plan == spec.requested_plan;
        let review_confirmed = spec.requested_review.is_none() || applied_review == spec.requested_review;
        if !plan_confirmed || !review_confirmed {
            let _ = bus.shutdown().await;
            return Err(StartError::Protocol(format!(
                "the adapter did not confirm the requested plan/review: applied={applied}"
            )));
        }

        // Confirm the adapter applied WHAT WE ASKED (TASK-048 F3). If the adapter
        // answers a different provider/model - or no `applied` at all - the
        // session is NOT started: an unconfirmed identity is never recorded as
        // applied.
        if let Some(grant) = &spec.grant {
            let want_provider = Some(grant.requested_provider_id.as_str());
            let want_model = grant.requested_model_id.as_deref();
            let ok_provider = applied_provider.as_deref() == want_provider;
            let ok_model = match want_model {
                Some(m) => applied_model.as_deref() == Some(m),
                None => true,
            };
            if !applied.is_object() || !ok_provider || !ok_model {
                let _ = bus.shutdown().await;
                return Err(StartError::Protocol(format!(
                    "the adapter did not confirm the requested provider/model: requested provider={:?} model={:?}, applied={}",
                    want_provider, want_model, applied
                )));
            }
        }
        let applied_out = applied.clone();
        let applied_provider_out = applied_provider.clone();
        let applied_model_out = applied_model.clone();
        let applied_route_out = applied_route.clone();
        let applied_preset_out = applied_preset.clone();
        let applied_plan_out = applied_plan;
        let applied_review_out = applied_review;
        let bus_requests = bus.requests.clone();
        let requests_out = bus_requests.clone();
        // Pump this session's notifications into the event bus. Every frame names
        // its session, so one stream carries every session's turn events.
        let events = self.events.clone();
        let pump_sid = sid.clone();
        let mut notifications = std::mem::replace(
            &mut bus.notifications,
            agent_hub_adapter::Notifications::closed(),
        );
        tokio::spawn(async move {
            while let Some(n) = notifications.recv().await {
                let mut payload = n.params.clone();
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("sessionId".into(), json!(pump_sid));
                } else {
                    payload = json!({ "sessionId": pump_sid, "data": payload });
                }
                events.publish(n.method, payload);
            }
        });
        let process = SessionProcess {
            sid: sid.clone(),
            harness_id,
            native_ref: native_ref.clone(),
            bus: Some(bus),
            requests: requests_out,
            applied,
            applied_provider,
            applied_model,
            applied_route,
            applied_preset,
            applied_plan,
            applied_review,
        };
        self.requests.lock().expect("requests").insert(sid.clone(), bus_requests.clone());
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
            requests: bus_requests,
            applied: applied_out,
            applied_provider: applied_provider_out,
            applied_model: applied_model_out,
            applied_route: applied_route_out,
            applied_preset: applied_preset_out,
            applied_plan: applied_plan_out,
            applied_review: applied_review_out,
        })
    }

    /// Send a request on a session's own process (a turn's prompt/abort). Fails
    /// if the session has no running process.
    pub async fn request(&self, sid: &str, method: &str, params: Value) -> Result<Value, StartError> {
        // Take the CLONEABLE request handle under a SHORT lock, then release it.
        // Holding the session mutex across a full prompt (which ends only when the
        // turn does) would make abort/stop wait for the turn - the turn could
        // never be interrupted (TASK-048 P1).
        let requests = self
            .requests
            .lock()
            .expect("requests")
            .get(sid)
            .cloned()
            .ok_or_else(|| StartError::Protocol(format!("session `{sid}` has no running process")))?;
        requests
            .request(method, params)
            .await
            .map_err(|e| StartError::Protocol(e.to_string()))
    }

    /// Deliver a credential grant to a session's live adapter (a mid-session
    /// provider switch: `credentials/grant` precedes `config/set`).
    pub async fn grant(&self, sid: &str, grant: &Grant) -> Result<(), StartError> {
        let params = json!({
            "connectionId": grant.connection_id,
            "value": grant.value,
            "url": grant.url,
            "declarations": grant.declarations,
        });
        self.request(sid, "credentials/grant", params).await.map(|_| ())
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
                self.requests.lock().expect("requests").remove(sid);
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
    if let Some(p) = &spec.presets_dir {
        env.push(("AGENT_HUB_PRESETS_DIR".into(), p.to_string_lossy().into()));
    }
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
