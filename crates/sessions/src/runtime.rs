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
            .map_err(|e| match e {
                agent_hub_adapter::BusError::Rpc { code, message, data } => {
                    StartError::Refused { code: code.to_string(), message, data }
                }
                other => StartError::Protocol(other.to_string()),
            })
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
    /// Extra roots the harness MAY activate (absolute; `AGENT_HUB_ADDITIONAL_DIRS`).
    pub additional_dirs: Vec<String>,
    /// Environment variables a session's adapter receives for the hub-managed
    /// connections (envName -> value). Populated only for ENABLED connections; a
    /// disabled connection contributes nothing (zero materialization).
    pub connection_env: Vec<(String, String)>,
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
    /// A per-session PROCESS GENERATION: bumped on every start and every stop.
    /// A caller that must act on a SPECIFIC process (a cancel's abort) captures
    /// the generation under the delivery lock and refuses to act if the process
    /// was replaced (stop/reopen) since (TASK-048 F4).
    generations: Mutex<std::collections::HashMap<String, u64>>,
    /// The REVERSE-REQUEST handler (the adapter asks the hub): given
    /// `(sid, method, params)`, it returns the JSON-RPC `result` to send back (or
    /// `None` to leave a plain notification alone). The composition root wires this
    /// to Humans for `approval_need`/`question_need` (TASK-048 G1).
    reverse: std::sync::Mutex<Option<ReverseHandler>>,
}

/// A boxed reverse-request handler.
pub type ReverseHandler = std::sync::Arc<
    dyn Fn(String, String, Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Value>> + Send>>
        + Send
        + Sync,
>;

impl Sessions {
    pub fn new(events: agent_hub_events::Bus) -> Self {
        Sessions {
            running: Mutex::new(std::collections::HashMap::new()),
            requests: Mutex::new(std::collections::HashMap::new()),
            events,
            generations: Mutex::new(std::collections::HashMap::new()),
            reverse: std::sync::Mutex::new(None),
        }
    }

    /// Install the reverse-request handler (the adapter -> hub requests). Wired
    /// once by the composition root.
    pub fn set_reverse_handler(&self, handler: ReverseHandler) {
        *self.reverse.lock().expect("reverse") = Some(handler);
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
            match tokio::time::timeout(control_request_timeout(), bus.requests.request("credentials/grant", params)).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    let _ = bus.shutdown().await;
                    return Err(bus_error(e, "credentials/grant"));
                }
                Err(_) => {
                    let _ = bus.shutdown().await;
                    return Err(StartError::Refused {
                        code: "credentials/grant:timeout".into(),
                        message: "the adapter did not answer credentials/grant in time".into(),
                        data: serde_json::Value::Null,
                    });
                }
            }
        }

        // config/set (the hub's materialization must be applied before a turn). A
        // bounded wait: an adapter that never answers must not hang the start.
        let applied = match tokio::time::timeout(
            control_request_timeout(),
            bus.requests
                .request("config/set", json!({ "sid": spec.sid, "config": spec.config })),
        )
        .await
        {
            Ok(Ok(v)) => v.get("applied").cloned().unwrap_or(Value::Null),
            Ok(Err(e)) => {
                let _ = bus.shutdown().await;
                return Err(bus_error(e, "config/set"));
            }
            Err(_) => {
                let _ = bus.shutdown().await;
                return Err(StartError::Refused {
                    code: "config/set:timeout".into(),
                    message: "the adapter did not answer config/set in time".into(),
                    data: serde_json::Value::Null,
                });
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
            // The adapter must also report the RESOLVED NATIVE ROUTE
            // (`applied.connectionId`). The contract (adapter-v1) is explicit: an
            // injected provider is named `hub-<id>` IN EVERY HARNESS, and
            // `applied.connectionId` is its RESOLVED NATIVE ROUTE (`session/start`
            // receives the resolved route from the hub). A BARE id is NOT the
            // resolved route, so accepting it would be an unauthorized alias
            // (TASK-048 F3): require the injected name.
            let want = want_provider.unwrap_or("");
            let ok_route = match applied_route.as_deref() {
                Some(r) => !r.is_empty() && r == format!("hub-{want}"),
                _ => false,
            };
            if !applied.is_object() || !ok_provider || !ok_model || !ok_route {
                let _ = bus.shutdown().await;
                return Err(StartError::Protocol(format!(
                    "the adapter did not confirm the requested provider/model/route: requested provider={:?} model={:?}, applied={}",
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
        let pump_events = events.clone();
        let pump_reply = bus.requests.clone();
        let pump_reverse = self.reverse.lock().expect("reverse").clone();
        let mut notifications = std::mem::replace(
            &mut bus.notifications,
            agent_hub_adapter::Notifications::closed(),
        );
        tokio::spawn(async move {
            while let Some(n) = notifications.recv().await {
                // A REVERSE REQUEST (`id` present, e.g. `approval_need`) must be
                // ANSWERED, not merely announced: run the handler and reply to the
                // SAME request. A plain notification is published as an event.
                if n.wants_reply() {
                    let id = n.id.clone().unwrap_or(Value::Null);
                    let method = n.method.clone();
                    if let Some(handler) = &pump_reverse {
                        let result = handler(pump_sid.clone(), method, n.params.clone()).await;
                        if let Some(result) = result {
                            let _ = pump_reply.reply(&id, result).await;
                        }
                    }
                    continue;
                }
                let mut payload = n.params.clone();
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("sessionId".into(), json!(pump_sid));
                } else {
                    payload = json!({ "sessionId": pump_sid, "data": payload });
                }
                pump_events.publish(n.method, payload);
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
        // Publish the handle AND the generation in ONE critical section: a
        // `send_if_generation` takes the SAME `requests` lock, so it can never
        // observe the new handle with the old generation (or vice versa)
        // (TASK-048 F4/S2).
        {
            let mut reqs = self.requests.lock().expect("requests");
            reqs.insert(sid.clone(), bus_requests.clone());
            self.bump_generation(&spec.sid);
        }
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
        requests.request(method, params).await.map_err(map_bus_err)
    }

    /// WRITE a request frame and return the receiver for its reply, WITHOUT
    /// awaiting it (the delivery half of a turn's prompt vs a cancel's abort).
    pub async fn send(
        &self,
        sid: &str,
        method: &str,
        params: Value,
    ) -> Result<
        tokio::sync::oneshot::Receiver<Result<Value, agent_hub_adapter::BusError>>,
        SendError,
    > {
        let requests = self
            .requests
            .lock()
            .expect("requests")
            .get(sid)
            .cloned()
            .ok_or_else(|| {
                SendError::NotDelivered(format!("session `{sid}` has no running process"))
            })?;
        match requests.send(method, params).await {
            Ok(rx) => Ok(rx),
            Err(e) => {
                // A write error with the process still alive proves NOTHING about
                // delivery: keep the unknown result. A dead process is a definite
                // non-delivery.
                if requests.is_alive() {
                    Err(SendError::Unknown(format!("the frame write failed with the adapter alive: {e}")))
                } else {
                    Err(SendError::NotDelivered(format!("the adapter is gone: {e}")))
                }
            }
        }
    }

    /// The current PROCESS GENERATION for a session (0 when none ever started).
    pub fn generation(&self, sid: &str) -> u64 {
        self.generations
            .lock()
            .expect("generations")
            .get(sid)
            .copied()
            .unwrap_or(0)
    }

    /// Bump the generation for `sid` (a new process starts, or a process stops).
    fn bump_generation(&self, sid: &str) -> u64 {
        let mut g = self.generations.lock().expect("generations");
        let e = g.entry(sid.to_string()).or_insert(0);
        *e = e.wrapping_add(1);
        *e
    }

    /// Write a request frame to a session's process ONLY IF its generation still
    /// matches `expected`. This binds a delivery to a SPECIFIC process: a cancel
    /// captured against process generation G refuses to send if the process was
    /// replaced since, so an old abort can never reach a NEW process (TASK-048 F4).
    ///
    /// The generation is read and the handle is taken under the SAME `requests`
    /// lock, and every `start`/`stop` bumps the generation AND mutates the handle
    /// map under that same lock, so the check and the lookup cannot interleave with
    /// a process replacement.
    pub async fn send_if_generation(
        &self,
        sid: &str,
        expected: u64,
        method: &str,
        params: Value,
    ) -> Result<
        tokio::sync::oneshot::Receiver<Result<Value, agent_hub_adapter::BusError>>,
        SendError,
    > {
        let requests = {
            // Take the requests lock FIRST, then read the generation; start/stop
            // mutate the generation AND the requests map under `requests`, so this
            // pair cannot interleave with a process replacement.
            let reqs = self.requests.lock().expect("requests");
            let current = self
                .generations
                .lock()
                .expect("generations")
                .get(sid)
                .copied()
                .unwrap_or(0);
            if current != expected {
                return Err(SendError::NotDelivered(
                    "the session process was replaced; the delivery target is stale".into(),
                ));
            }
            reqs.get(sid).cloned().ok_or_else(|| {
                SendError::NotDelivered(format!("session `{sid}` has no running process"))
            })?
        };
        match requests.send(method, params).await {
            Ok(rx) => Ok(rx),
            Err(e) => {
                if requests.is_alive() {
                    Err(SendError::Unknown(format!(
                        "the frame write failed with the adapter alive: {e}"
                    )))
                } else {
                    Err(SendError::NotDelivered(format!("the adapter is gone: {e}")))
                }
            }
        }
    }

    /// Whether a LIVE process is running for this session (see `is_running`).

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

    /// Whether a LIVE process is running for this session. The cached handle's
    /// adapter may have exited (its stdout closed); a dead process is NOT running,
    /// so lifecycle and reconciliation see the truth.
    pub fn is_running(&self, sid: &str) -> bool {
        if !self.running.lock().expect("running").contains_key(sid) {
            return false;
        }
        self.requests
            .lock()
            .expect("requests")
            .get(sid)
            .map(|h| h.is_alive())
            .unwrap_or(false)
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
                {
                    let mut reqs = self.requests.lock().expect("requests");
                    reqs.remove(sid);
                    // Bump while holding the requests lock, so a delivery captured
                    // against the old process cannot pass the generation check.
                    self.bump_generation(sid);
                }
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
    if !spec.additional_dirs.is_empty() {
        env.push((
            "AGENT_HUB_ADDITIONAL_DIRS".into(),
            serde_json::to_string(&spec.additional_dirs).unwrap(),
        ));
    }
    // The hub-managed connections' credentials travel as env vars named by each
    // connection's envName - never in the config payload or the conversation.
    for (name, value) in &spec.connection_env {
        env.push((name.clone(), value.clone()));
    }
    env
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("could not start the adapter: {0}")]
    Spawn(String),
    #[error("adapter protocol: {0}")]
    Protocol(String),
    /// The ADAPTER answered with an error (an RPC-level refusal). Its own answer
    /// is authoritative: the execution it was asked for did not run. Distinct from
    /// a transport failure, where the execution may still be in flight.
    #[error("adapter refused: {message}")]
    Refused { code: String, message: String, data: Value },
}

impl StartError {
    pub fn code(&self) -> &'static str {
        match self {
            StartError::Spawn(_) => "adapter_unreachable",
            StartError::Protocol(_) => "adapter_crash",
            StartError::Refused { data, .. } => {
                // The adapter answers with its OWN hyphenated codes (adapter-v1
                // `errors`). Map them EXPLICITLY at this owning boundary; a code
                // with no contract identity is an honest `adapter_crash`, never an
                // invented one.
                adapter_code_to_contract(data)
            }
        }
    }
}

/// The owning adapter->contract error mapping. The adapter's vocabulary is the
/// hyphenated set in `contract/adapter-v1.json` `errors`; the hub's is the
/// underscored `contract/errors.json`. This is the ONE place they are reconciled
/// (TASK-048 F5/S4).
pub fn adapter_code_to_contract(data: &Value) -> &'static str {
    let code = data.get("code").and_then(|c| c.as_str()).unwrap_or("");
    match code {
        // The adapter's hyphenated codes.
        "revision-conflict" => "revision_conflict",
        "credential-shadowed" => "revision_conflict",
        "unknown-provider" => "provider_not_found",
        "unknown-model" => "model_not_found",
        "validation-failed" => "validation_failed",
        "auth-expired" => "provider_unauthorized",
        "busy-session-active" => "session_busy",
        "unsupported-for-provider" => "unsupported",
        "requires-new-session" => "requires_new_session",
        "agent-preset-locked" => "agent_preset_locked",
        "abort-failed" => "adapter_unreachable",
        // A contract code passed through unchanged (a hub-shaped adapter).
        "provider_unauthorized" => "provider_unauthorized",
        "provider_not_found" => "provider_not_found",
        "model_not_found" => "model_not_found",
        "model_mismatch" => "model_mismatch",
        "model_not_applied" => "model_not_applied",
        "revision_conflict" => "revision_conflict",
        "requires_new_session" => "requires_new_session",
        "unsupported" => "unsupported",
        "validation_failed" => "validation_failed",
        "session_busy" => "session_busy",
        _ => "adapter_crash",
    }
}

#[cfg(test)]
mod env_tests {
    use super::*;
    use std::path::PathBuf;

    fn spec(connection_env: Vec<(String, String)>) -> StartSpec {
        StartSpec {
            sid: "s1".into(),
            harness_id: "pi".into(),
            command: vec!["true".into()],
            plugin_dir: PathBuf::from("."),
            cwd: PathBuf::from("."),
            harness_dir: PathBuf::from("."),
            skills_dir: PathBuf::from("."),
            extensions_dir: PathBuf::from("."),
            presets_dir: None,
            runtime_argv: None,
            resume: None,
            fork_from: None,
            additional_dirs: Vec::new(),
            connection_env,
            config: json!({}),
            grant: None,
            requested_preset_id: None,
            requested_plan: None,
            requested_review: None,
        }
    }

    /// A hub-managed connection's credential reaches the adapter as the env var
    /// named by its envName.
    #[test]
    fn connection_env_lands_in_the_adapter_environment() {
        let env = build_env(&spec(vec![("MY_CONN_TOKEN".into(), "secret".into())]));
        assert!(
            env.iter().any(|(k, v)| k == "MY_CONN_TOKEN" && v == "secret"),
            "the connection credential must be in the adapter env"
        );
    }

    /// No connection env means no stray variable.
    #[test]
    fn no_connections_means_no_variable() {
        let env = build_env(&spec(vec![]));
        assert!(!env.iter().any(|(k, _)| k == "MY_CONN_TOKEN"));
    }

    /// A prompt/abort bound to a process generation that no longer matches the
    /// live one is REFUSED as a definite non-delivery: it can never reach a
    /// process the turn was not claimed against (the prompt-binding remainder).
    #[tokio::test]
    async fn a_stale_generation_is_not_delivered() {
        let sessions = Sessions::new(agent_hub_events::Bus::new(16, 16));
        // A process generation exists (say 7), but we hold generation 6.
        sessions
            .generations
            .lock()
            .expect("generations")
            .insert("s1".into(), 7);
        let err = sessions
            .send_if_generation("s1", 6, "session/prompt", json!({}))
            .await
            .expect_err("a stale generation must not be delivered");
        assert!(
            matches!(err, SendError::NotDelivered(_)),
            "a stale target is a DEFINITE non-delivery, not an unknown result"
        );
    }

    /// Extra roots reach the adapter as AGENT_HUB_ADDITIONAL_DIRS; none means the
    /// variable is absent (not an empty list).
    #[test]
    fn additional_dirs_land_or_are_absent() {
        let mut sp = spec(vec![]);
        sp.additional_dirs = vec!["E:/one".into(), "E:/two".into()];
        let env = build_env(&sp);
        let v = env.iter().find(|(k, _)| k == "AGENT_HUB_ADDITIONAL_DIRS").map(|(_, v)| v.clone());
        assert_eq!(v.as_deref(), Some("[\"E:/one\",\"E:/two\"]"));
        let empty = build_env(&spec(vec![]));
        assert!(!empty.iter().any(|(k, _)| k == "AGENT_HUB_ADDITIONAL_DIRS"));
    }
}

/// How long a control request (`credentials/grant`, `config/set`) may wait for an
/// adapter answer before the hub treats the outcome as UNKNOWN and fails closed. A
/// control request has no business holding a handler (or a start) indefinitely: an
/// adapter that never answers must not block the hub (ADR-0009), and a config that
/// may have applied must not be assumed to have failed.
pub fn control_request_timeout() -> std::time::Duration {
    // Overridable for tests/rehearsal; the default is generous because a real
    // adapter may do work before it answers.
    let secs = std::env::var("AGENT_HUB_CONTROL_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60);
    std::time::Duration::from_secs(secs)
}

/// The outcome of WRITING a request frame. The distinction matters: a caller must
/// not settle a turn on a write error when delivery is UNKNOWN, because the
/// execution may still run (TASK-048 F4).
pub enum SendError {
    /// The frame was NOT written (no live process, a stale generation, a closed
    /// pipe before any byte): the request definitely did not run.
    NotDelivered(String),
    /// The write failed with the process still alive: delivery is UNKNOWN. The
    /// caller must NOT assume the request did not run.
    Unknown(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::NotDelivered(m) => write!(f, "{m}"),
            SendError::Unknown(m) => write!(f, "{m}"),
        }
    }
}

/// Turn a bus error into a StartError, KEEPING the adapter's typed `data.code` when
/// it answered (so a grant/config refusal carries its contract identity, not a
/// stringified `adapter_crash`).
fn bus_error(e: agent_hub_adapter::BusError, what: &str) -> StartError {
    match e {
        agent_hub_adapter::BusError::Rpc { code, message, data } => StartError::Refused {
            code: format!("{what}:{code}"),
            message,
            data,
        },
        other => StartError::Protocol(format!("{what} failed: {other}")),
    }
}

/// Map a bus error into a StartError, KEEPING the adapter's typed `data.code`
/// when it answered, so the identity survives (TASK-048 F5).
pub fn map_bus_err(e: agent_hub_adapter::BusError) -> StartError {
    match e {
        agent_hub_adapter::BusError::Rpc { code, message, data } => {
            StartError::Refused { code: code.to_string(), message, data }
        }
        other => StartError::Protocol(other.to_string()),
    }
}

#[cfg(test)]
mod code_map_tests {
    use super::*;
    use serde_json::json;

    /// The owning adapter->contract mapping is explicit and complete for the
    /// adapter's declared vocabulary (TASK-048 F5/S4).
    #[test]
    fn adapter_codes_map_to_contract_codes() {
        let m = |c: &str| adapter_code_to_contract(&json!({ "code": c }));
        assert_eq!(m("revision-conflict"), "revision_conflict");
        assert_eq!(m("credential-shadowed"), "revision_conflict");
        assert_eq!(m("unknown-provider"), "provider_not_found");
        assert_eq!(m("unknown-model"), "model_not_found");
        assert_eq!(m("auth-expired"), "provider_unauthorized");
        assert_eq!(m("busy-session-active"), "session_busy");
        assert_eq!(m("requires-new-session"), "requires_new_session");
        assert_eq!(m("unsupported-for-provider"), "unsupported");
        assert_eq!(m("validation-failed"), "validation_failed");
        // An unknown code is never invented into a contract code.
        assert_eq!(m("totally-made-up"), "adapter_crash");
        // A hub-shaped adapter passing a contract code through is honored.
        assert_eq!(m("provider_unauthorized"), "provider_unauthorized");
    }
}
