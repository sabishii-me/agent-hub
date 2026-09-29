//! The sessions domain (`ARCHITECTURE` §6, §11): the hub's control state over
//! sessions, with a REAL adapter process per session.
//!
//! `create` is a **long command** (§11, R1): it durably reserves the command ->
//! session association BEFORE any side effect, answers `202 + Location`, and
//! starts the adapter in the background. The session resource reports `starting`
//! -> `active` / `starting_failed`, readable through GET. Close stops that
//! session's process; reopen restarts on the stored ref. Only the config the
//! slice supports is accepted; anything else is refused, never silently ignored.

use std::path::PathBuf;
use std::sync::Arc;

use agent_hub_db::{now_utc, Db, SessionRow};
use agent_hub_events::Bus;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::runtime::{Sessions as Runtime, StartSpec};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session `{0}` not found")]
    NotFound(String),
    #[error("no adapter for harness `{0}`")]
    NoHarness(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("idempotency conflict for `{0}`")]
    Conflict(String),
    #[error("the session could not be started: {0}")]
    Start(String),
    #[error("the abort could not be delivered: {0}")]
    AbortFailed(String),
    /// A provider resolution failure with its OWN contract code (e.g.
    /// `provider_unauthorized`), so the identity survives to the response
    /// (TASK-048 F5).
    #[error("{message}")]
    Provider { code: String, message: String },
    #[error(transparent)]
    Db(#[from] agent_hub_db::DbError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl SessionError {
    /// Build a provider error from the resolver's typed string. The resolver
    /// returns `<code>|<message>` so the contract identity is preserved.
    pub fn provider(raw: String) -> Self {
        match raw.split_once('|') {
            Some((code, message)) => SessionError::Provider {
                code: code.to_string(),
                message: message.to_string(),
            },
            None => SessionError::Validation(raw),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            SessionError::NotFound(_) => "unknown_session",
            SessionError::NoHarness(_) => "harness_not_found",
            SessionError::Validation(_) => "validation_failed",
            SessionError::Unsupported(_) => "unsupported",
            SessionError::AbortFailed(_) => "abort_failed",
            SessionError::Provider { code, .. } => {
                // A leaked &'static is fine here: the codes are a closed set.
                match code.as_str() {
                    "provider_unauthorized" => "provider_unauthorized",
                    "provider_not_found" => "provider_not_found",
                    "provider_catalog_failed" => "provider_catalog_failed",
                    "revision_conflict" => "revision_conflict",
                    "catalog_not_loaded" => "catalog_not_loaded",
                    "not_implemented" => "not_implemented",
                    _ => "validation_failed",
                }
            }
            SessionError::Conflict(_) => "idempotency_conflict",
            SessionError::Start(_) => "adapter_crash",
            SessionError::Db(_) | SessionError::Io(_) => "internal_error",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// A session as the contract's `session` object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    pub id: String,
    #[serde(rename = "harnessId")]
    pub harness_id: String,
    #[serde(rename = "modelProviderId")]
    pub model_provider_id: Option<String>,
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
    #[serde(rename = "appliedModel")]
    pub applied_model: Option<String>,
    /// The provider the adapter CONFIRMED applied (`applied.modelProviderId`).
    #[serde(rename = "appliedProvider", skip_serializing_if = "Option::is_none")]
    pub applied_provider: Option<String>,
    /// The native route the adapter resolved (`applied.connectionId`).
    #[serde(rename = "appliedRoute", skip_serializing_if = "Option::is_none")]
    pub applied_route: Option<String>,
    pub plan: Option<bool>,
    pub review: Option<bool>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub status: String,
    #[serde(rename = "startError")]
    pub start_error: Option<String>,
    #[serde(rename = "nativeRef")]
    pub native_ref: Option<String>,
    #[serde(rename = "turnRunning")]
    pub turn_running: bool,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    pub deleted: bool,
}

/// What POST /v1/sessions accepts. This slice supports only the default config
/// (`harnessId`, optional `cwd`); any other non-default field is REFUSED, not
/// silently ignored (R3).
#[derive(Debug, Deserialize)]
pub struct CreateSession {
    #[serde(rename = "harnessId")]
    pub harness_id: String,
    #[serde(rename = "modelProviderId")]
    pub model_provider_id: Option<String>,
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub plan: Option<bool>,
    pub review: Option<bool>,
    #[serde(rename = "presetId")]
    pub preset_id: Option<String>,
    #[serde(rename = "additionalDirectories")]
    pub additional_directories: Option<Vec<String>>,
}

/// A harness the sessions domain may start.
#[derive(Debug, Clone)]
pub struct HarnessSpec {
    pub id: String,
    pub command: Vec<String>,
    pub plugin_dir: PathBuf,
    pub runtime_argv: Option<Vec<String>>,
    /// The declared, hub-side asset dirs for this harness, ALREADY placed by the
    /// adapter's own rules (the sessions domain must not re-implement placement).
    pub harness_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub extensions_dir: PathBuf,
}

/// What `create` returns: the reserved session (to be answered 202) or a replay.
pub enum CreateOutcome {
    /// First time: the session row exists as `starting`; the caller runs the
    /// start in the background and answers 202 + Location.
    Accepted(SessionView),
    /// A retry: the original session (already started or starting).
    Replay(SessionView),
}

pub struct Sessions {
    pub db: Db,
    bus: Bus,
    pub data_dir: PathBuf,
    runtime: Runtime,
    /// Cheap, SIDE-EFFECT-FREE: does an adapter plugin exist for this id? Called
    /// at accept time (before any reservation), so it must do no file work.
    harness_exists: Box<dyn Fn(&str) -> bool + Send + Sync>,
    /// The full spec (does placement). Called only in the DETACHED start (N4).
    harness_spec: Box<dyn Fn(&str) -> Result<HarnessSpec, String> + Send + Sync>,
    /// Resolve a hub-managed provider id to a credential grant. This is the
    /// providers-domain boundary: `sessions` does not depend on `providers`; the
    /// composition root injects the resolver. `None` = no provider injection
    /// configured, so a session with `modelProviderId` is refused (honest).
    provider_resolver: Option<Box<dyn Fn(&str) -> Result<crate::runtime::Grant, String> + Send + Sync>>,
    /// One lock per session: start/close/reopen on the same session never race
    /// (R4). A lock held across the lifecycle of one session.
    locks: Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>>,
    /// Turns a cancel was requested for (the run settles them as cancelled).
    cancelled: Mutex<std::collections::HashSet<String>>,
    /// Turns already settled (a late result never overwrites a decided one).
    settled: Mutex<std::collections::HashSet<String>>,
}

/// A canonical, unambiguous fingerprint of the semantic request (R2). Lengths are
/// prefixed so no field can be confused with another by concatenation.
fn fingerprint(req: &CreateSession) -> String {
    fn part(s: &str) -> String {
        format!("{}:{}", s.len(), s)
    }
    let mut out = String::new();
    out.push_str(&part("create"));
    out.push_str(&part(&req.harness_id));
    out.push_str(&part(req.model_provider_id.as_deref().unwrap_or("")));
    out.push_str(&part(req.model_id.as_deref().unwrap_or("")));
    out.push_str(&part(req.title.as_deref().unwrap_or("")));
    out.push_str(&part(req.cwd.as_deref().unwrap_or("")));
    out.push_str(&part(req.preset_id.as_deref().unwrap_or("")));
    out.push_str(&part(match req.plan { Some(true) => "plan=1", Some(false) => "plan=0", None => "" }));
    out.push_str(&part(match req.review { Some(true) => "review=1", Some(false) => "review=0", None => "" }));
    out.push_str(&part(&req.additional_directories.clone().unwrap_or_default().join("\u{1f}")));
    out
}

impl Sessions {
    pub fn new(
        db: Db,
        bus: Bus,
        data_dir: impl Into<PathBuf>,
        harness_exists: Box<dyn Fn(&str) -> bool + Send + Sync>,
        harness_spec: Box<dyn Fn(&str) -> Result<HarnessSpec, String> + Send + Sync>,
    ) -> Self {
        let runtime = Runtime::new(bus.clone());
        Sessions {
            db,
            bus,
            data_dir: data_dir.into(),
            runtime,
            harness_exists,
            harness_spec,
            provider_resolver: None,
            locks: Mutex::new(std::collections::HashMap::new()),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            settled: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Inject the provider resolver (the composition root owns this). It is the
    /// ONLY way a hub-managed provider reaches a session.
    pub fn with_provider_resolver(
        mut self,
        resolver: Box<dyn Fn(&str) -> Result<crate::runtime::Grant, String> + Send + Sync>,
    ) -> Self {
        self.provider_resolver = Some(resolver);
        self
    }

    /// Resolve the session's requested provider/model into a grant and the
    /// `config/set` payload. The config selects the adapter's injected provider
    /// (`hub-<id>`) and the model within it.
    fn resolve_grant(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> Result<(Option<crate::runtime::Grant>, serde_json::Value), SessionError> {
        let Some(pid) = provider_id else {
            if model_id.is_some() {
                return Err(SessionError::Validation(
                    "modelId requires modelProviderId: a model is chosen within a provider's route".into(),
                ));
            }
            return Ok((None, serde_json::json!({})));
        };
        let resolver = self.provider_resolver.as_ref().ok_or_else(|| {
            SessionError::Unsupported("this hub has no provider resolver; a managed provider is refused".into())
        })?;
        let mut grant = resolver(pid).map_err(SessionError::provider)?;
        grant.requested_model_id = model_id.map(str::to_string);
        // The owning contract (`adapter-v1:386`) selects the provider by
        // `config.connectionId`; the model is selected WITHIN that route by
        // `config.model`. The adapter then reports `applied.modelProviderId` (the
        // requested identity) and `applied.connectionId` (its resolved native
        // route).
        let mut config = serde_json::json!({
            "connectionId": grant.connection_id,
            "modelProviderId": grant.connection_id,
        });
        if let Some(m) = model_id {
            config["model"] = serde_json::json!(m);
        }
        Ok((Some(grant), config))
    }

    async fn lock_for(&self, sid: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(sid.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// The body validation for what this slice supports. Called AFTER the
    /// reservation, so a conflict is decided first (R1/R2).
    fn validate_supported(&self, req: &CreateSession) -> Result<(), SessionError> {
        // A managed provider is supported only when a resolver is injected; the
        // resolution (and its failures) happen here, at "accept", so a bad
        // provider is refused before any process is spawned.
        self.resolve_grant(req.model_provider_id.as_deref(), req.model_id.as_deref())?;
        if req.preset_id.is_some() {
            return Err(SessionError::Unsupported("presetId is not supported yet".into()));
        }
        if req.plan.is_some() || req.review.is_some() {
            return Err(SessionError::Unsupported("plan/review are not supported yet".into()));
        }
        if req.additional_directories.as_ref().map(|d| !d.is_empty()).unwrap_or(false) {
            return Err(SessionError::Unsupported("additionalDirectories are not supported yet".into()));
        }
        Ok(())
    }

    /// The full spec (does placement). Called in the detached start only.
    fn harness(&self, id: &str) -> Result<HarnessSpec, SessionError> {
        (self.harness_spec)(id).map_err(SessionError::NoHarness)
    }

    fn harness_exists(&self, id: &str) -> bool {
        (self.harness_exists)(id)
    }

    /// Accept a create command. This slice supports only the default config:
    /// **any non-default field is refused** (R3), never silently ignored.
    /// The command -> session association is reserved durably BEFORE any side
    /// effect (R1/R2): a retry returns the original; a different request with the
    /// same key is a conflict; a new key is a new command.
    pub fn accept_create(
        &self,
        command_id: &str,
        req: CreateSession,
    ) -> Result<CreateOutcome, SessionError> {
        if req.harness_id.trim().is_empty() {
            return Err(SessionError::Validation("harnessId is required".into()));
        }

        let fp = fingerprint(&req);
        let id = new_id("s");
        let cwd = match req.cwd.clone().filter(|c| !c.is_empty()) {
            Some(c) => c,
            None => {
                let dir = self.data_dir.join("sessions").join(&id);
                std::fs::create_dir_all(&dir)?;
                dir.to_string_lossy().to_string()
            }
        };
        let now = now_utc();
        let row = SessionRow {
            id: id.clone(),
            harness_id: req.harness_id.clone(),
            model_provider_id: req.model_provider_id.clone(),
            model_id: req.model_id.clone(),
            applied_model: None,
            applied_provider: None,
            applied_route: None,
            plan: None,
            review: None,
            cwd: Some(cwd),
            title: req.title.clone(),
            status: "starting".into(),
            created_at: now.clone(),
            updated_at: now,
            deleted: false,
            forked_from_session: None,
            forked_from_turn: None,
            native_ref: None,
            start_error: None,
        };

        // R1/R2: ONE atomic decision - reserve the key and insert the `starting`
        // row in a single transaction, BEFORE any body validation. So the same
        // key with ANY different body is a conflict, whatever that body is.
        match self.db.reserve_session_command(command_id, &fp, &row)? {
            agent_hub_db::ReserveOutcome::Reserved => {}
            agent_hub_db::ReserveOutcome::Replay(sid) => {
                let row = self
                    .db
                    .session(&sid)?
                    .ok_or_else(|| SessionError::NotFound(sid.clone()))?;
                return Ok(CreateOutcome::Replay(self.view(&row)));
            }
            agent_hub_db::ReserveOutcome::Conflict => {
                return Err(SessionError::Conflict(command_id.into()));
            }
        }

        // The reservation is committed. Now validate the body; if it is not
        // supported, UNDO the reservation (drop the row and the command key) and
        // return the refusal - never leave a `starting` row for a refused command.
        if let Err(e) = self.validate_supported(&req) {
            let _ = self.db.delete_session_row(&id);
            let _ = self.db.remove_session_command(command_id);
            return Err(e);
        }
        if !self.harness_exists(&req.harness_id) {
            let _ = self.db.delete_session_row(&id);
            let _ = self.db.remove_session_command(command_id);
            return Err(SessionError::NoHarness(req.harness_id.clone()));
        }

        let view = self.view(&row);
        self.bus.publish("session.created", serde_json::json!({ "session": view }));
        Ok(CreateOutcome::Accepted(view))
    }

    /// The background half of create: start the adapter and move the session from
    /// `starting` to `active`/`starting_failed`. Runs DETACHED (the caller already
    /// answered 202). On any failure the started process is stopped (R4).
    pub async fn run_start(self: Arc<Self>, sid: String) {
        let lock = self.lock_for(&sid).await;
        let _guard = lock.lock().await;
        let row = match self.db.session(&sid) {
            Ok(Some(r)) => r,
            _ => return,
        };
        // Guard: only a `starting` session starts. A close/delete that raced ahead
        // (holding the same lock) leaves a non-starting status; do not revive it.
        if row.status != "starting" || row.deleted {
            return;
        }
        let harness = match self.harness(&row.harness_id) {
            Ok(h) => h,
            Err(e) => return self.fail_start(&sid, &e.to_string()).await,
        };
        // Resolve the credential grant (memory-only in the adapter) and the
        // config that selects the injected provider. A resolution failure here
        // fails the start honestly; it is never silently dropped.
        let (grant, config) =
            match self.resolve_grant(row.model_provider_id.as_deref(), row.model_id.as_deref()) {
                Ok(v) => v,
                Err(e) => return self.fail_start(&sid, &e.to_string()).await,
            };
        let spec = StartSpec {
            sid: sid.clone(),
            harness_id: harness.id.clone(),
            command: harness.command.clone(),
            plugin_dir: harness.plugin_dir.clone(),
            cwd: PathBuf::from(row.cwd.clone().unwrap_or_default()),
            harness_dir: harness.harness_dir.clone(),
            skills_dir: harness.skills_dir.clone(),
            extensions_dir: harness.extensions_dir.clone(),
            runtime_argv: harness.runtime_argv.clone(),
            resume: None,
            config,
            grant,
        };
        match self.runtime.start(spec).await {
            Ok(process) => {
                let mut row = row;
                row.status = "active".into();
                row.native_ref = Some(process.native_ref.clone());
                // The applied identity is the adapter's PROOF (what it confirmed),
                // not the request. Recorded so a turn's model is read from
                // reality, never assumed.
                row.applied_model = process.applied_model.clone();
                row.applied_provider = process.applied_provider.clone();
                row.applied_route = process.applied_route.clone();
                row.start_error = None;
                row.updated_at = now_utc();
                if let Err(e) = self.db.update_session(&row) {
                    // DB write failed AFTER the process started: stop it and mark
                    // the failure, never leave a live process behind a broken row.
                    let _ = self.runtime.stop(&sid).await;
                    let _ = e;
                    return self.fail_start(&sid, "the session row could not be persisted").await;
                }
                self.bus.publish("session.started", serde_json::json!({ "session": self.view(&row) }));
            }
            Err(e) => self.fail_start(&sid, &e.to_string()).await,
        }
    }

    async fn fail_start(&self, sid: &str, error: &str) {
        if let Ok(Some(mut row)) = self.db.session(sid) {
            row.status = "starting_failed".into();
            row.start_error = Some(error.to_string());
            row.updated_at = now_utc();
            let _ = self.db.update_session(&row);
            self.bus.publish("session.start_failed", serde_json::json!({ "session": self.view(&row) }));
        }
    }

    pub fn get(&self, id: &str) -> Result<SessionView, SessionError> {
        let row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        Ok(self.view(&row))
    }

    /// Boot reconciliation (N2.2): a `starting` session with NO running process is
    /// a start that was interrupted (the process died, or the hub restarted). It
    /// must not keep claiming it is starting. Mark it `starting_failed` so a
    /// client reads an honest terminal/unknown state, never a lie. No replay of
    /// unknown side effects.
    pub fn reconcile_interrupted(&self) -> Result<usize, SessionError> {
        let mut n = 0;
        for row in self.db.list_sessions()? {
            if row.status == "starting" && !self.runtime.is_running(&row.id) {
                let mut row = row;
                row.status = "starting_failed".into();
                row.start_error = Some("the start was interrupted (the hub restarted or the process died)".into());
                row.updated_at = now_utc();
                self.db.update_session(&row)?;
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn list(&self) -> Result<Vec<SessionView>, SessionError> {
        Ok(self.db.list_sessions()?.iter().map(|r| self.view(r)).collect())
    }

    /// Close: stop the session's process and free its resources; the record stays
    /// and the status becomes `readonly`. Serialized with start/reopen on the same
    /// session (R4). If the process cannot be confirmed stopped, the error is
    /// propagated and `readonly` is NOT written (R4: no claiming release).
    pub async fn close(self: Arc<Self>, id: &str) -> Result<SessionView, SessionError> {
        let lock = self.lock_for(id).await;
        let _guard = lock.lock().await;
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        // Stop the process; a failure is surfaced, not swallowed.
        self.runtime
            .stop(id)
            .await
            .map_err(|e| SessionError::Start(format!("the session process could not be stopped: {e}")))?;
        if row.status != "readonly" {
            row.status = "readonly".into();
            row.updated_at = now_utc();
            self.db.update_session(&row)?;
            self.bus.publish("session.closed", serde_json::json!({ "session": self.view(&row) }));
        }
        Ok(self.view(&row))
    }

    /// Reopen: restart on the stored ref. Refuses a session that is still running
    /// (never overwrite a live instance, R4); serialized on the session lock.
    pub async fn reopen(self: Arc<Self>, id: &str) -> Result<SessionView, SessionError> {
        let lock = self.lock_for(id).await;
        let _guard = lock.lock().await;
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        if self.runtime.is_running(id) {
            return Err(SessionError::Validation("the session is already running".into()));
        }
        if row.native_ref.is_none() {
            return Err(SessionError::Validation("the session has no native ref to reopen".into()));
        }
        let harness = self.harness(&row.harness_id)?;
        // A restart RE-GRANTS: the adapter's grant is memory-only, so a reopened
        // session must receive the credential again before its config is applied.
        let (grant, config) =
            self.resolve_grant(row.model_provider_id.as_deref(), row.model_id.as_deref())?;
        let spec = StartSpec {
            sid: row.id.clone(),
            harness_id: harness.id.clone(),
            command: harness.command.clone(),
            plugin_dir: harness.plugin_dir.clone(),
            cwd: PathBuf::from(row.cwd.clone().unwrap_or_default()),
            harness_dir: harness.harness_dir.clone(),
            skills_dir: harness.skills_dir.clone(),
            extensions_dir: harness.extensions_dir.clone(),
            runtime_argv: harness.runtime_argv.clone(),
            resume: row.native_ref.clone(),
            config,
            grant,
        };
        let process = self
            .runtime
            .start(spec)
            .await
            .map_err(|e| SessionError::Start(e.to_string()))?;
        row.status = "active".into();
        row.native_ref = Some(process.native_ref.clone());
        row.applied_model = process.applied_model.clone();
        row.applied_provider = process.applied_provider.clone();
        row.applied_route = process.applied_route.clone();
        row.updated_at = now_utc();
        self.db.update_session(&row)?;
        let view = self.view(&row);
        self.bus.publish("session.reopened", serde_json::json!({ "session": view, "reopened": true }));
        Ok(view)
    }

    /// Delete (soft): stop the process, mark the row deleted.
    pub async fn delete(self: Arc<Self>, id: &str) -> Result<(), SessionError> {
        let lock = self.lock_for(id).await;
        let _guard = lock.lock().await;
        if self.db.session(id)?.is_none() {
            return Err(SessionError::NotFound(id.into()));
        }
        self.runtime
            .stop(id)
            .await
            .map_err(|e| SessionError::Start(e.to_string()))?;
        self.db.mark_session_deleted(id)?;
        Ok(())
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.runtime.is_running(id)
    }

    fn view(&self, row: &SessionRow) -> SessionView {
        SessionView {
            id: row.id.clone(),
            harness_id: row.harness_id.clone(),
            model_provider_id: row.model_provider_id.clone(),
            model_id: row.model_id.clone(),
            applied_model: row.applied_model.clone(),
            applied_provider: row.applied_provider.clone(),
            applied_route: row.applied_route.clone(),
            plan: row.plan,
            review: row.review,
            cwd: row.cwd.clone(),
            title: row.title.clone(),
            status: row.status.clone(),
            start_error: row.start_error.clone(),
            native_ref: row.native_ref.clone(),
            turn_running: false,
            created_at: row.created_at.clone(),
            updated_at: row.updated_at.clone(),
            deleted: row.deleted,
        }
    }
}

/// A fresh, sortable id.
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{millis:x}-{n:x}")
}

/// A turn as the contract's `turnState`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnView {
    pub id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(rename = "partialPersisted")]
    pub partial_persisted: bool,
    #[serde(rename = "partialItems")]
    pub partial_items: i64,
}

/// What a turn admission returned.
pub enum TurnOutcome {
    /// A new turn, reserved durable; run the prompt detached.
    Accepted(TurnView),
    /// The same idempotency key and content: the original turn.
    Replay(TurnView),
}

#[derive(Debug, Deserialize)]
pub struct TurnRequest {
    pub content: serde_json::Value,
    #[serde(rename = "idempotencyKey")]
    pub idempotency_key: String,
    #[serde(default)]
    pub source: Option<String>,
}

impl Sessions {
    /// Accept a turn: reserve its command identity DURABLY (the UNIQUE
    /// (session, idempotencyKey) is the decision point), refuse a second turn
    /// while one runs (`session_busy`, never queued), and reject an empty body.
    /// An **unknown** turn is never replayed.
    pub fn accept_turn(
        &self,
        session_id: &str,
        req: TurnRequest,
    ) -> Result<TurnOutcome, SessionError> {
        if req.idempotency_key.trim().is_empty() {
            return Err(SessionError::Validation("idempotencyKey is required".into()));
        }
        let session = self
            .db
            .session(session_id)?
            .ok_or_else(|| SessionError::NotFound(session_id.into()))?;
        if session.status != "active" {
            return Err(SessionError::Validation(format!(
                "the session is {}; it must be active to send a turn",
                session.status
            )));
        }
        let text = text_of(&req.content);
        if text.is_empty() {
            return Err(SessionError::Validation("content must contain text".into()));
        }
        let intent = format!("turn:{}", text);

        // Admit the turn: identity AND busy are ONE atomic decision. A refused
        // admission leaves no `admitted` row (TASK-048 P1).
        let turn_id = new_id("t");
        match self.db.admit_turn(session_id, &req.idempotency_key, &intent, &turn_id)? {
            agent_hub_db::TurnAdmission::Reserved => {}
            agent_hub_db::TurnAdmission::Replay(id) => {
                let t = self
                    .db
                    .turn(&id)?
                    .ok_or_else(|| SessionError::NotFound(id.clone()))?;
                return Ok(TurnOutcome::Replay(turn_view(&t)));
            }
            agent_hub_db::TurnAdmission::Conflict => {
                return Err(SessionError::Conflict(req.idempotency_key));
            }
            agent_hub_db::TurnAdmission::Busy(_) => {
                return Err(SessionError::Validation(
                    "the session has a running turn; a new turn is refused, not queued".into(),
                ));
            }
        }

        let t = self.db.turn(&turn_id)?.ok_or_else(|| SessionError::NotFound(turn_id.clone()))?;
        self.bus
            .publish("turn.admitted", serde_json::json!({ "turn": turn_view(&t), "sessionId": session_id }));
        Ok(TurnOutcome::Accepted(turn_view(&t)))
    }

    /// The detached half of a turn: send `session/prompt` on the session's own
    /// process and move the turn to a terminal state when it ends. The adapter's
    /// notifications (message.delta etc.) are pumped to the event bus separately.
    pub async fn run_turn(self: Arc<Self>, session_id: String, turn_id: String, text: String) {
        if !self.runtime.is_running(&session_id) {
            self.settle_turn(&turn_id, "failed", Some("the session has no running process")).await;
            return;
        }
        // Dispatch is a single decision with the cancel intent: if the turn is
        // already terminal (a cancel settled it) or was cancelled before the
        // prompt was sent, do NOT dispatch - a prompt sent for a cancelled turn is
        // exactly the race that used to leave an unconfirmed stop (TASK-048 F4).
        let already_cancelled = self.cancelled.lock().await.contains(&turn_id);
        if already_cancelled {
            self.settle_turn(&turn_id, "cancelled", None).await;
            return;
        }
        // `running` is a NON-terminal move: if a terminal was already committed
        // (a cancel raced us) the guarded update is a no-op, and `false` means the
        // turn was settled - do not dispatch.
        if !self.db.set_turn_state(&turn_id, "running").unwrap_or(false) {
            return;
        }
        self.bus.publish(
            "turn.running",
            serde_json::json!({ "turn": { "state": "running" }, "sessionId": session_id, "turnId": turn_id }),
        );
        let params = serde_json::json!({
            "sid": session_id,
            "message": text,
            "clientMessageId": turn_id,
        });
        // The prompt's OWN result is the authoritative terminal (adapter-v1:96-103
        // returns `turn-ended`; `turn_end.state: ok|aborted|failed` is the proof).
        // No pre-prompt snapshot, and never "any RPC success = completed".
        match self.runtime.request(&session_id, "session/prompt", params).await {
            Ok(result) => {
                let (ended, cause) = run_end_of(&result);
                self.settle_turn(&turn_id, ended, cause.as_deref()).await;
            }
            Err(e) => {
                // A transport error is NOT a confirmed stop: if a cancel was
                // requested the honest terminal is `interrupted` (the turn did not
                // reach a clean end), otherwise `failed`.
                let cancelled = self.cancelled.lock().await.contains(&turn_id);
                let ended = if cancelled { "interrupted" } else { "failed" };
                self.settle_turn(&turn_id, ended, Some(&e.to_string())).await;
            }
        }
    }

    /// Settle a turn's terminal state **once**, in the DATABASE (the guard is the
    /// SQL `WHERE ended IS NULL`). Only the call that performed the settle
    /// publishes, so the streamed fact and the stored row cannot disagree.
    async fn settle_turn(&self, turn_id: &str, ended: &str, cause: Option<&str>) {
        // Commit the terminal in the DATABASE first, retrying a transient failure
        // so a known terminal is not left un-persisted (TASK-048 F4). We publish
        // ONLY after the database confirms, so the stream never announces a fact
        // the store does not hold.
        let mut last_err = None;
        for attempt in 0..5u32 {
            match self.db.end_turn(turn_id, ended, &now_utc()) {
                Ok(true) => {
                    self.settled.lock().await.insert(turn_id.to_string());
                    self.bus.publish(
                        "turn.ended",
                        serde_json::json!({ "turn": { "state": "ended", "ended": ended, "cause": cause } }),
                    );
                    return;
                }
                // Already settled by a racing winner: nothing to publish.
                Ok(false) => return,
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(50 * (attempt as u64 + 1))).await;
                }
            }
        }
        // The database never accepted the terminal. Do NOT publish a lie; leave
        // the turn visibly un-settled so reconciliation/boot can finish it.
        if let Some(e) = last_err {
            tracing::error!(turn = %turn_id, ended, error = %e, "the turn terminal could not be persisted; left for reconciliation");
        }
    }

    /// Reconcile a turn whose cancel was requested but whose prompt never
    /// returned a terminal (a delivered-but-unconfirmed abort, or a crash). The
    /// turn is settled `interrupted` - the honest answer when we cannot prove it
    /// stopped - so `busy` is released deliberately, not by a send failure
    /// (TASK-048 F4).
    pub async fn reconcile_stalled_cancels(&self) -> Result<usize, SessionError> {
        let mut n = 0;
        for row in self.db.sessions_with_cancelling_turns()? {
            if self.runtime.is_running(&row) {
                continue; // still alive: let run_turn settle it
            }
            if let Some(t) = self.db.active_turn(&row)? {
                if t.state == "cancelling" {
                    self.settle_turn(&t.id, "interrupted", Some("the cancel was not confirmed")).await;
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    pub fn list_turns(&self, session_id: &str) -> Result<Vec<TurnView>, SessionError> {
        if self.db.session(session_id)?.is_none() {
            return Err(SessionError::NotFound(session_id.into()));
        }
        Ok(self.db.list_turns(session_id)?.iter().map(turn_view).collect())
    }

    /// Cancel: idempotent. Sends `session/abort`; the turn ends when the adapter
    /// confirms (an adapter ACK is NOT "stopped").
    pub async fn cancel_turn(self: Arc<Self>, session_id: &str) -> Result<TurnView, SessionError> {
        // Idempotent: no running turn -> return the current (terminal) state.
        let active = match self.db.active_turn(session_id)? {
            Some(t) => t,
            None => {
                let turns = self.db.list_turns(session_id)?;
                return turns
                    .last()
                    .map(turn_view)
                    .ok_or_else(|| SessionError::Validation("the session has no turns".into()));
            }
        };
        // Record the cancel INTENT durably (before the abort), so a prompt that has
        // not yet been dispatched sees it (run_turn) and a restart can reconcile.
        // `cancelling` is a NON-terminal move: the busy state is NOT released.
        self.cancelled.lock().await.insert(active.id.clone());
        let _ = self.db.set_turn_state(&active.id, "cancelling");
        // Deliver the abort. A send failure does NOT settle the turn: the prompt
        // may still be running, so releasing busy would be a lie. The turn stays
        // held; the caller sees `abort-failed` and can retry. The terminal is
        // decided by the prompt's own return, or by reconciliation (TASK-048 F4).
        if let Err(e) = self
            .runtime
            .request(session_id, "session/abort", serde_json::json!({ "sid": session_id }))
            .await
        {
            let t = self
                .db
                .turn(&active.id)?
                .ok_or_else(|| SessionError::NotFound(active.id.clone()))?;
            return Err(SessionError::AbortFailed(format!(
                "{e}; the turn is still held (state {}), retry cancel",
                t.state
            )));
        }
        let t = self
            .db
            .turn(&active.id)?
            .ok_or_else(|| SessionError::NotFound(active.id.clone()))?;
        Ok(turn_view(&t))
    }

}

/// Map the adapter's returned `turn-ended` to the contract's terminal. `ok` ->
/// `completed`, `aborted` -> `cancelled`, `failed` -> `failed`; anything
/// unreadable is `failed` (never silently `completed`).
fn run_end_of(result: &serde_json::Value) -> (&'static str, Option<String>) {
    let state = result
        .get("state")
        .or_else(|| result.get("turn").and_then(|t| t.get("state")))
        .and_then(|s| s.as_str());
    match state {
        Some("ok") => ("completed", None),
        Some("aborted") => ("cancelled", None),
        Some("failed") => (
            "failed",
            result
                .get("error")
                .map(|e| e.to_string())
                .or_else(|| Some("the adapter reported a failed turn".into())),
        ),
        _ => ("failed", Some("the adapter did not report a turn state".into())),
    }
}

fn turn_view(t: &agent_hub_db::TurnRow) -> TurnView {
    TurnView {
        id: t.id.clone(),
        state: t.state.clone(),
        ended: t.ended.clone(),
        cause: None,
        partial_persisted: false,
        partial_items: 0,
    }
}

/// The plain text of a content array (public so the route can carry it to the
/// detached task).
pub fn text_of_request(content: &serde_json::Value) -> String {
    text_of(content)
}

/// The plain text of a content array (this slice supports text parts).
fn text_of(content: &serde_json::Value) -> String {
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod run_end_tests {
    use super::run_end_of;
    use serde_json::json;

    #[test]
    fn the_adapter_state_is_the_terminal() {
        assert_eq!(run_end_of(&json!({"state":"ok"})).0, "completed");
        assert_eq!(run_end_of(&json!({"state":"aborted"})).0, "cancelled");
        assert_eq!(run_end_of(&json!({"state":"failed"})).0, "failed");
    }

    /// A success WITHOUT a state is NOT completed: it is failed (never silently
    /// "the model finished").
    #[test]
    fn an_unreadable_state_is_failed_not_completed() {
        assert_eq!(run_end_of(&json!({})).0, "failed");
        assert_eq!(run_end_of(&json!({"unexpected":true})).0, "failed");
    }
}
