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
    #[error("the session has a running turn; this change requires an idle turn")]
    Busy,
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
            SessionError::Busy => "session_busy",
            SessionError::Provider { code, .. } => {
                // A leaked &'static is fine here: the codes are a closed set.
                match code.as_str() {
                    "provider_unauthorized" => "provider_unauthorized",
                    "provider_not_found" => "provider_not_found",
                    "provider_catalog_failed" => "provider_catalog_failed",
                    "revision_conflict" => "revision_conflict",
                    "catalog_not_loaded" => "catalog_not_loaded",
                    "not_implemented" => "not_implemented",
                    // A storage failure is NOT a request-body error.
                    "internal_error" => "internal_error",
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
    #[serde(rename = "presetId", skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<String>,
    #[serde(rename = "appliedPreset", skip_serializing_if = "Option::is_none")]
    pub applied_preset: Option<String>,
    #[serde(rename = "appliedPlan", skip_serializing_if = "Option::is_none")]
    pub applied_plan: Option<bool>,
    #[serde(rename = "appliedReview", skip_serializing_if = "Option::is_none")]
    pub applied_review: Option<bool>,
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
    /// The plugin's declared presets directory (`<plugin>/presets/`), when the
    /// harness declares the `presets` capability. The adapter reads definitions
    /// from it via `AGENT_HUB_PRESETS_DIR`.
    pub presets_dir: Option<PathBuf>,
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
    provider_resolver: Option<
        Box<
            dyn Fn(
                    String,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<crate::runtime::Grant, String>> + Send>,
                > + Send
                + Sync,
        >,
    >,
    /// Resolve the hub-managed connections' credentials for a session's adapter, as
    /// `(envName, value)` for ENABLED connections only. Injected by the composition
    /// root so `sessions` does not depend on `connections`.
    connection_resolver: Option<Box<dyn Fn() -> Result<Vec<(String, String)>, String> + Send + Sync>>,
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
            connection_resolver: None,
            locks: Mutex::new(std::collections::HashMap::new()),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            settled: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Inject the provider resolver (the composition root owns this). It is the
    /// ONLY way a hub-managed provider reaches a session.
    pub fn with_provider_resolver(
        mut self,
        resolver: Box<
            dyn Fn(
                    String,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<crate::runtime::Grant, String>> + Send>,
                > + Send
                + Sync,
        >,
    ) -> Self {
        self.provider_resolver = Some(resolver);
        self
    }

    /// Inject the connection resolver (the composition root owns it). It returns
    /// the ENABLED connections' `(envName, value)` pairs; a disabled one is absent.
    pub fn with_connection_resolver(
        mut self,
        resolver: Box<dyn Fn() -> Result<Vec<(String, String)>, String> + Send + Sync>,
    ) -> Self {
        self.connection_resolver = Some(resolver);
        self
    }

    /// The connection env for a session's adapter (empty when no resolver is
    /// injected, or when a connection is disabled / incomplete).
    fn connection_env(&self) -> Result<Vec<(String, String)>, SessionError> {
        match &self.connection_resolver {
            Some(r) => r().map_err(SessionError::Validation),
            None => Ok(Vec::new()),
        }
    }

    /// Resolve the session's requested provider/model/preset into a grant and the
    /// `config/set` payload. The config selects the adapter's injected provider
    /// (`hub-<id>`), the model within it, and the session preset.
    async fn resolve_grant(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
        preset_id: Option<&str>,
        plan: Option<bool>,
        review: Option<bool>,
    ) -> Result<(Option<crate::runtime::Grant>, serde_json::Value), SessionError> {
        let mut config = serde_json::json!({});
        // plan/review are session-scoped knobs the adapter answers with
        // applied.plan / applied.review. null = do not intervene.
        if let Some(v) = plan {
            config["plan"] = serde_json::json!(v);
        }
        if let Some(v) = review {
            config["review"] = serde_json::json!(v);
        }
        // A preset is a session composition: it is set at the first config and is
        // locked once the session has turns (the adapter enforces that; a locked
        // preset comes back as an error, never silently kept).
        if let Some(p) = preset_id {
            config["presetId"] = serde_json::json!(p);
        }
        let Some(pid) = provider_id else {
            if model_id.is_some() {
                return Err(SessionError::Validation(
                    "modelId requires modelProviderId: a model is chosen within a provider's route".into(),
                ));
            }
            return Ok((None, config));
        };
        let resolver = self.provider_resolver.as_ref().ok_or_else(|| {
            SessionError::Unsupported("this hub has no provider resolver; a managed provider is refused".into())
        })?;
        let mut grant = resolver(pid.to_string()).await.map_err(SessionError::provider)?;
        grant.requested_model_id = model_id.map(str::to_string);
        // The owning contract (`adapter-v1:386`) selects the provider by
        // `config.connectionId`; the model is selected WITHIN that route by
        // `config.model`. The adapter then reports `applied.modelProviderId` (the
        // requested identity) and `applied.connectionId` (its resolved native
        // route).
        config["connectionId"] = serde_json::json!(grant.connection_id);
        config["modelProviderId"] = serde_json::json!(grant.connection_id);
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
    async fn validate_supported(&self, req: &CreateSession) -> Result<(), SessionError> {
        // A managed provider is supported only when a resolver is injected; the
        // resolution (and its failures) happen here, at "accept", so a bad
        // provider is refused before any process is spawned.
        self.resolve_grant(
            req.model_provider_id.as_deref(),
            req.model_id.as_deref(),
            req.preset_id.as_deref(),
            req.plan,
            req.review,
        )
        .await?;
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
    pub async fn accept_create(
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
            preset_id: req.preset_id.clone(),
            applied_preset: None,
            plan: req.plan,
            review: req.review,
            applied_plan: None,
            applied_review: None,
            cwd: Some(cwd),
            additional_dirs: req.additional_directories.clone().unwrap_or_default(),
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
        if let Err(e) = self.validate_supported(&req).await {
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
        // Placement (deleting/creating the shared harness dir) is BLOCKING file
        // work. Run it off the async runtime, so it never stalls other sessions'
        // tasks (TASK-048 N4). The accept path already did a cheap existence check
        // and no side effect.
        let harness = {
            let this = self.clone();
            let harness_id = row.harness_id.clone();
            match tokio::task::spawn_blocking(move || this.harness(&harness_id)).await {
                Ok(Ok(h)) => h,
                Ok(Err(e)) => return self.fail_start(&sid, &e.to_string()).await,
                Err(e) => return self.fail_start(&sid, &format!("placement task failed: {e}")).await,
            }
        };
        // Resolve the credential grant (memory-only in the adapter) and the
        // config that selects the injected provider. A resolution failure here
        // fails the start honestly; it is never silently dropped.
        let (grant, config) =
            match self
                .resolve_grant(
                    row.model_provider_id.as_deref(),
                    row.model_id.as_deref(),
                    row.preset_id.as_deref(),
                    row.plan,
                    row.review,
                )
                .await
            {
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
            presets_dir: harness.presets_dir.clone(),
            runtime_argv: harness.runtime_argv.clone(),
            resume: None,
            fork_from: None,
            additional_dirs: row.additional_dirs.clone(),
            connection_env: self.connection_env().unwrap_or_default(),
            config,
            grant,
            requested_preset_id: row.preset_id.clone(),
            requested_plan: row.plan,
            requested_review: row.review,
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
                row.applied_preset = process.applied_preset.clone();
                row.applied_plan = process.applied_plan;
                row.applied_review = process.applied_review;
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

    /// After a config/set has RUN in the adapter, a failure (an unconfirmed target
    /// or a DB write failure) leaves the adapter in an UNKNOWN configuration. The
    /// session must not keep serving prompts against its old applied identity: stop
    /// the process and mark it `needs-repair`, so the next use reopens and re-grants
    /// (TASK-048 F3).
    async fn quarantine_session(&self, sid: &str, reason: &str) -> SessionError {
        let _ = self.runtime.stop(sid).await;
        if let Ok(Some(mut row)) = self.db.session(sid) {
            row.status = "needs-repair".into();
            row.start_error = Some(reason.to_string());
            row.updated_at = now_utc();
            let _ = self.db.update_session(&row);
            self.bus
                .publish("session.start_failed", serde_json::json!({ "session": self.view(&row) }));
        }
        SessionError::Start(reason.to_string())
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
    /// At boot, no session process is running. Reconcile anything that claims
    /// otherwise (N2):
    /// * a `starting` session whose start was interrupted -> `starting_failed`;
    /// * an `active` session whose process is gone -> `needs-repair` (an orphaned
    ///   tail: the record survives and reopen can restart it);
    /// * any turn still open (`ended IS NULL`) whose session is not running ->
    ///   `interrupted` (we cannot prove it finished, so we do not pretend it did).
    pub fn reconcile_interrupted(&self) -> Result<usize, SessionError> {
        let mut n = 0;
        for row in self.db.list_sessions()? {
            if self.runtime.is_running(&row.id) {
                continue;
            }
            let mut changed = false;
            let mut row = row;
            match row.status.as_str() {
                "starting" => {
                    row.status = "starting_failed".into();
                    row.start_error = Some(
                        "the start was interrupted (the hub restarted or the process died)".into(),
                    );
                    changed = true;
                }
                "active" => {
                    row.status = "needs-repair".into();
                    changed = true;
                }
                _ => {}
            }
            // An open turn in a session we are not running cannot still be
            // running: settle it honestly.
            for t in self.db.list_turns(&row.id)? {
                if t.ended.is_none() {
                    let ended = if t.state == "cancelling" { "interrupted" } else { "failed" };
                    let cause = if t.state == "cancelling" {
                        "the cancel was not confirmed before the hub stopped"
                    } else {
                        "the turn was open when the hub stopped"
                    };
                    let _ = self.db.end_turn(&t.id, ended, &now_utc());
                    self.bus.publish(
                        "turn.ended",
                        serde_json::json!({ "turn": { "state": "ended", "ended": ended, "cause": cause } }),
                    );
                    changed = true;
                }
            }
            if changed {
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

    /// PATCH: change session configuration. Serialized with start/close/reopen and
    /// turn admission on the session lock, so a config change never races a turn.
    ///
    /// The knobs fall into two classes (owning contract):
    /// * **policy** (`plan`/`review`) may apply during a running turn;
    /// * **model/provider/preset/thinking** require an IDLE turn (`409 session_busy`
    ///   while one runs).
    ///
    /// A change is only recorded when the adapter CONFIRMS it (`applied.*`); an
    /// unconfirmed value is never written as if applied (the one exception the
    /// contract names is `thinkingLevel`, reported as null plus a warning).
    pub async fn patch_session(
        self: Arc<Self>,
        id: &str,
        req: PatchSession,
    ) -> Result<PatchOutcome, SessionError> {
        let lock = self.lock_for(id).await;
        let _guard = lock.lock().await;
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        if row.deleted {
            return Err(SessionError::NotFound(id.into()));
        }
        let running = self.db.active_turn(id)?.is_some();
        let wants_model_change = req.model_provider_id.is_some()
            || req.model_id.is_some()
            || req.preset_id.is_some()
            || req.thinking_level.is_some();
        if running && wants_model_change {
            return Err(SessionError::Busy);
        }
        if !self.runtime.is_running(id) && (wants_model_change || req.title.is_some()) {
            return Err(SessionError::Validation(
                "the session is not running; reopen it before changing its configuration".into(),
            ));
        }

        let mut warning: Option<String> = None;

        // policy knobs: allowed during a running turn
        if let Some(v) = req.plan {
            let r = self
                .runtime
                .request(id, "config/set", serde_json::json!({ "sid": id, "config": { "plan": v } }))
                .await
                .map_err(|e| SessionError::Start(e.to_string()))?;
            let applied = r.get("applied").and_then(|a| a.get("plan")).and_then(|p| p.as_bool());
            if applied != Some(v) {
                return Err(SessionError::Validation(format!("the adapter did not confirm plan={v}")));
            }
            row.plan = Some(v);
            row.applied_plan = Some(v);
        }
        if let Some(v) = req.review {
            let r = self
                .runtime
                .request(id, "config/set", serde_json::json!({ "sid": id, "config": { "review": v } }))
                .await
                .map_err(|e| SessionError::Start(e.to_string()))?;
            let applied = r.get("applied").and_then(|a| a.get("review")).and_then(|p| p.as_bool());
            if applied != Some(v) {
                return Err(SessionError::Validation(format!("the adapter did not confirm review={v}")));
            }
            row.review = Some(v);
            row.applied_review = Some(v);
        }

        // title: renamed IN the harness; the harness holds the accepted title
        if let Some(t) = &req.title {
            let r = self
                .runtime
                .request(id, "session/rename", serde_json::json!({ "sid": id, "title": t }))
                .await
                .map_err(|e| SessionError::Start(format!("rename failed: {e}")))?;
            match r.get("title").and_then(|v| v.as_str()) {
                Some(h) => row.title = Some(h.to_string()),
                None => return Err(SessionError::Validation("the harness did not accept the title".into())),
            }
        }

        // model/provider/preset: idle only, confirmed applied
        let model_changing =
            req.model_provider_id.is_some() || req.model_id.is_some() || req.preset_id.is_some();
        if model_changing {
            let new_provider =
                req.model_provider_id.clone().or_else(|| row.model_provider_id.clone());
            let new_model = req.model_id.clone().or_else(|| row.model_id.clone());
            let new_preset = req.preset_id.clone().or_else(|| row.preset_id.clone());
            let (grant, config) = self
                .resolve_grant(
                    new_provider.as_deref(),
                    new_model.as_deref(),
                    new_preset.as_deref(),
                    row.plan,
                    row.review,
                )
                .await?;
            if let Some(g) = &grant {
                self.runtime
                    .grant(id, g)
                    .await
                    .map_err(|e| SessionError::Start(format!("credentials/grant failed: {e}")))?;
            }
            let r = self
                .runtime
                .request(id, "config/set", serde_json::json!({ "sid": id, "config": config }))
                .await
                .map_err(|e| SessionError::Start(format!("config/set failed: {e}")))?;
            let applied = r.get("applied").cloned().unwrap_or(serde_json::Value::Null);
            let ap = applied.get("modelProviderId").and_then(|v| v.as_str()).map(str::to_string);
            let am = applied.get("model").and_then(|v| v.as_str()).map(str::to_string);
            let ar = applied.get("connectionId").and_then(|v| v.as_str()).map(str::to_string);
            let route_ok = ar.is_some();
            let mut mismatch: Option<String> = None;
            if let Some(want) = new_provider.as_deref() {
                if ap.as_deref() != Some(want) {
                    mismatch = Some(format!("the adapter did not confirm provider `{want}`: applied={applied}"));
                }
            }
            if mismatch.is_none() {
                if let Some(want) = new_model.as_deref() {
                    if am.as_deref() != Some(want) {
                        mismatch = Some(format!("the adapter did not confirm model `{want}`: applied={applied}"));
                    }
                }
            }
            if mismatch.is_none() && !route_ok {
                mismatch = Some(format!("the adapter did not confirm a native route: applied={applied}"));
            }
            if mismatch.is_none() {
                if let Some(want) = new_preset.as_deref() {
                    if applied.get("preset").and_then(|v| v.as_str()) != Some(want) {
                        mismatch = Some(format!("the adapter did not confirm preset `{want}`: applied={applied}"));
                    }
                }
            }
            if let Some(msg) = mismatch {
                // The config/set already ran; do not leave the session active with
                // its old applied identity.
                return Err(self.quarantine_session(id, &msg).await);
            }
            row.model_provider_id = new_provider;
            row.model_id = new_model;
            row.preset_id = new_preset;
            row.applied_provider = ap;
            row.applied_model = am;
            row.applied_route = ar;
        }

        // thinkingLevel: unconfirmed -> reported as null + warning, never the
        // requested value (owning contract).
        if let Some(level) = &req.thinking_level {
            let r = self
                .runtime
                .request(id, "config/set", serde_json::json!({ "sid": id, "config": { "thinkingLevel": level } }))
                .await
                .map_err(|e| SessionError::Start(format!("thinking level not applied: {e}")))?;
            let applied =
                r.get("applied").and_then(|a| a.get("thinkingLevel")).and_then(|v| v.as_str());
            if applied != Some(level.as_str()) {
                warning =
                    Some(format!("the thinking level `{level}` was not confirmed by the harness"));
            }
        }

        row.updated_at = now_utc();
        if let Err(e) = self.db.update_session(&row) {
            return Err(self
                .quarantine_session(id, &format!("the patched session could not be persisted: {e}"))
                .await);
        }
        let view = self.view(&row);
        self.bus.publish("session.patched", serde_json::json!({ "session": view }));
        Ok(PatchOutcome { session: view, warning })
    }

    /// Reopen: restart on the stored ref. Refuses a session that is still running
    /// (never overwrite a live instance, R4); serialized on the session lock.
    pub async fn reopen(self: Arc<Self>, id: &str) -> Result<SessionView, SessionError> {
        let lock = self.lock_for(id).await;
        let _guard = lock.lock().await;
        self.ensure_running_locked(id, false).await
    }

    /// Ensure a session has a running adapter process, WITHOUT holding the lock
    /// (the caller holds it). Used by reopen and by the read-through routes
    /// (messages/stats/skills): a read starts the process if it is not running and
    /// caches nothing.
    ///
    /// `already_running_is_ok`: reopen refuses a running session (never overwrite a
    /// live instance); a read-through tolerates one (it just wants the process).
    async fn ensure_running_locked(
        self: &Arc<Self>,
        id: &str,
        already_running_is_ok: bool,
    ) -> Result<SessionView, SessionError> {
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        if self.runtime.is_running(id) {
            if already_running_is_ok {
                return Ok(self.view(&row));
            }
            return Err(SessionError::Validation("the session is already running".into()));
        }
        if row.native_ref.is_none() {
            return Err(SessionError::Validation("the session has no native ref to reopen".into()));
        }
        let harness = {
            let this = self.clone();
            let harness_id = row.harness_id.clone();
            tokio::task::spawn_blocking(move || this.harness(&harness_id))
                .await
                .map_err(|e| SessionError::Start(format!("placement task failed: {e}")))??
        };
        // A restart RE-GRANTS: the adapter's grant is memory-only, so a reopened
        // session must receive the credential again before its config is applied.
        let (grant, config) = self
            .resolve_grant(
                row.model_provider_id.as_deref(),
                row.model_id.as_deref(),
                row.preset_id.as_deref(),
                row.plan,
                row.review,
            )
            .await?;
        let spec = StartSpec {
            sid: row.id.clone(),
            harness_id: harness.id.clone(),
            command: harness.command.clone(),
            plugin_dir: harness.plugin_dir.clone(),
            cwd: PathBuf::from(row.cwd.clone().unwrap_or_default()),
            harness_dir: harness.harness_dir.clone(),
            skills_dir: harness.skills_dir.clone(),
            extensions_dir: harness.extensions_dir.clone(),
            presets_dir: harness.presets_dir.clone(),
            runtime_argv: harness.runtime_argv.clone(),
            resume: row.native_ref.clone(),
            fork_from: None,
            additional_dirs: row.additional_dirs.clone(),
            connection_env: self.connection_env().unwrap_or_default(),
            config,
            grant,
            requested_preset_id: row.preset_id.clone(),
            requested_plan: row.plan,
            requested_review: row.review,
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
        row.applied_preset = process.applied_preset.clone();
        row.applied_plan = process.applied_plan;
        row.applied_review = process.applied_review;
        row.updated_at = now_utc();
        if let Err(e) = self.db.update_session(&row) {
            let _ = self.runtime.stop(id).await;
            row.status = "needs-repair".into();
            row.updated_at = now_utc();
            let _ = self.db.update_session(&row);
            return Err(SessionError::Start(format!(
                "the reopened session could not be persisted: {e}"
            )));
        }
        let view = self.view(&row);
        self.bus.publish("session.reopened", serde_json::json!({ "session": view, "reopened": true }));
        Ok(view)
    }

    /// Fork: start a NEW session whose conversation ends at a completed turn of
    /// this one (or the whole conversation when `afterTurnId` is absent). The
    /// source is NOT changed - same log, same ref, still usable, and it does not
    /// even need a live process. The child is a real session with its own process.
    pub async fn fork(
        self: Arc<Self>,
        source_id: &str,
        req: ForkRequest,
    ) -> Result<(SessionView, serde_json::Value), SessionError> {
        let source = self
            .db
            .session(source_id)?
            .ok_or_else(|| SessionError::NotFound(source_id.into()))?;
        let source_ref = source.native_ref.clone().ok_or_else(|| {
            SessionError::Validation("the source session has no native ref to fork from".into())
        })?;
        let through = match &req.after_turn_id {
            None => None,
            Some(tid) => {
                let turns = self.db.list_turns(source_id)?;
                let mut n = 0u32;
                let mut found = false;
                for t in &turns {
                    if t.ended.as_deref() == Some("completed") {
                        n += 1;
                        if &t.id == tid {
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    return Err(SessionError::Validation(format!(
                        "`{tid}` is not a completed turn of this session"
                    )));
                }
                Some(n)
            }
        };
        let child_id = new_id("s");
        let cwd = self.data_dir.join("sessions").join(&child_id);
        std::fs::create_dir_all(&cwd)?;
        let now = now_utc();
        let mut row = SessionRow {
            id: child_id.clone(),
            harness_id: source.harness_id.clone(),
            model_provider_id: source.model_provider_id.clone(),
            model_id: source.model_id.clone(),
            applied_model: None,
            applied_provider: None,
            applied_route: None,
            preset_id: source.preset_id.clone(),
            applied_preset: None,
            plan: source.plan,
            review: source.review,
            applied_plan: None,
            applied_review: None,
            cwd: Some(cwd.to_string_lossy().to_string()),
            additional_dirs: source.additional_dirs.clone(),
            title: None,
            status: "starting".into(),
            created_at: now.clone(),
            updated_at: now,
            deleted: false,
            forked_from_session: Some(source_id.to_string()),
            forked_from_turn: req.after_turn_id.clone(),
            native_ref: None,
            start_error: None,
        };
        self.db.insert_session(&row)?;
        let lock = self.lock_for(&child_id).await;
        let _guard = lock.lock().await;
        let harness = {
            let this = self.clone();
            let harness_id = row.harness_id.clone();
            tokio::task::spawn_blocking(move || this.harness(&harness_id))
                .await
                .map_err(|e| SessionError::Start(format!("placement task failed: {e}")))??
        };
        let (grant, config) = self
            .resolve_grant(
                row.model_provider_id.as_deref(),
                row.model_id.as_deref(),
                row.preset_id.as_deref(),
                row.plan,
                row.review,
            )
            .await?;
        let spec = StartSpec {
            sid: child_id.clone(),
            harness_id: harness.id.clone(),
            command: harness.command.clone(),
            plugin_dir: harness.plugin_dir.clone(),
            cwd: PathBuf::from(row.cwd.clone().unwrap_or_default()),
            harness_dir: harness.harness_dir.clone(),
            skills_dir: harness.skills_dir.clone(),
            extensions_dir: harness.extensions_dir.clone(),
            presets_dir: harness.presets_dir.clone(),
            runtime_argv: harness.runtime_argv.clone(),
            resume: None,
            fork_from: Some(crate::runtime::ForkFrom { source_ref, through_turn: through }),
            additional_dirs: row.additional_dirs.clone(),
            connection_env: self.connection_env().unwrap_or_default(),
            config,
            grant,
            requested_preset_id: row.preset_id.clone(),
            requested_plan: row.plan,
            requested_review: row.review,
        };
        let process = match self.runtime.start(spec).await {
            Ok(p) => p,
            Err(e) => {
                row.status = "starting_failed".into();
                row.start_error = Some(e.to_string());
                row.updated_at = now_utc();
                let _ = self.db.update_session(&row);
                return Err(SessionError::Start(e.to_string()));
            }
        };
        row.status = "active".into();
        row.native_ref = Some(process.native_ref.clone());
        row.applied_model = process.applied_model.clone();
        row.applied_provider = process.applied_provider.clone();
        row.applied_route = process.applied_route.clone();
        row.applied_preset = process.applied_preset.clone();
        row.applied_plan = process.applied_plan;
        row.applied_review = process.applied_review;
        row.updated_at = now_utc();
        if let Err(e) = self.db.update_session(&row) {
            let _ = self.runtime.stop(&child_id).await;
            row.status = "needs-repair".into();
            let _ = self.db.update_session(&row);
            return Err(SessionError::Start(format!("the fork could not be persisted: {e}")));
        }
        let view = self.view(&row);
        self.bus.publish("session.forked", serde_json::json!({ "session": view }));
        let forked_from = serde_json::json!({
            "sessionId": source_id,
            "afterTurnId": req.after_turn_id,
        });
        Ok((view, forked_from))
    }
    /// Compact a session's own conversation. The hub does not compact anything and
    /// computes no numbers: it asks the harness (starting the process if needed) and
    /// reports what the harness says, tagged `source: harness`. A field the harness
    /// does not report is ABSENT.
    pub async fn compact(
        self: Arc<Self>,
        id: &str,
        req: CompactRequest,
    ) -> Result<serde_json::Value, SessionError> {
        let mut params = serde_json::json!({});
        if let Some(i) = &req.instructions {
            params["instructions"] = serde_json::json!(i);
        }
        let v = self.clone().read_through(id, "session/compact", params).await?;
        let mut body = v.as_object().cloned().unwrap_or_default();
        body.insert("sessionId".into(), serde_json::json!(id));
        body.insert("source".into(), serde_json::json!("harness"));
        Ok(serde_json::Value::Object(body))
    }

    /// A READ-THROUGH capability call on a session: start its process if it is not
    /// running (caching nothing), then ask the adapter. `method`/`params` are the
    /// session-scoped adapter method.
    pub async fn read_through(
        self: Arc<Self>,
        id: &str,
        method: &str,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value, SessionError> {
        {
            let lock = self.lock_for(id).await;
            let _guard = lock.lock().await;
            self.ensure_running_locked(id, true).await?;
        }
        if let Some(obj) = params.as_object_mut() {
            obj.insert("sid".into(), serde_json::json!(id));
        }
        self.runtime
            .request(id, method, params)
            .await
            .map_err(|e| SessionError::Start(e.to_string()))
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
            preset_id: row.preset_id.clone(),
            applied_preset: row.applied_preset.clone(),
            applied_plan: row.applied_plan,
            applied_review: row.applied_review,
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

/// The PATCH /v1/sessions/{id} body. Every field is optional; an absent field is
/// unchanged.
#[derive(Debug, Default, serde::Deserialize)]
pub struct PatchSession {
    #[serde(rename = "modelProviderId")]
    pub model_provider_id: Option<String>,
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
    #[serde(rename = "presetId")]
    pub preset_id: Option<String>,
    pub plan: Option<bool>,
    pub review: Option<bool>,
    #[serde(rename = "thinkingLevel")]
    pub thinking_level: Option<String>,
    pub title: Option<String>,
}

/// The fork body: an optional 1-based completed-turn anchor. Absent = the whole
/// conversation.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ForkRequest {
    #[serde(rename = "afterTurnId")]
    pub after_turn_id: Option<String>,
}

/// The compact body: optional instructions for the harness's own compaction.
#[derive(Debug, Default, serde::Deserialize)]
pub struct CompactRequest {
    pub instructions: Option<String>,
}

/// What a PATCH returns: the session and an optional warning (an unconfirmed
/// thinking level is reported as null plus a warning).
pub struct PatchOutcome {
    pub session: SessionView,
    pub warning: Option<String>,
}

#[derive(serde::Deserialize)]
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
    pub async fn accept_turn(
        self: Arc<Self>,
        session_id: &str,
        req: TurnRequest,
    ) -> Result<TurnOutcome, SessionError> {
        // Admission takes the SAME session lock a PATCH holds for its whole
        // grant/config transition, so a turn cannot be admitted while a config
        // switch is in flight (TASK-048 F3).
        let lock = self.lock_for(session_id).await;
        let _guard = lock.lock().await;
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
        // The DISPATCH gate: right before sending the prompt, re-read BOTH the
        // durable turn state (a cancel writes `cancelling`) and the in-memory cancel
        // set. If a cancel landed between the `running` commit and here, do NOT
        // send - the turn is cancelled, not dispatched (TASK-048 F4).
        {
            let cur = self.db.turn(&turn_id).ok().flatten().map(|t| t.state).unwrap_or_default();
            let cancelled = self.cancelled.lock().await.contains(&turn_id);
            if cancelled || cur == "cancelling" {
                self.settle_turn(&turn_id, "cancelled", None).await;
                return;
            }
        }
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
        // (a) A cancelling turn in a session with NO live process: the cancel can
        // never be confirmed now; settle `interrupted`.
        for row in self.db.sessions_with_cancelling_turns()? {
            if self.runtime.is_running(&row) {
                continue;
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

    /// The core-side CANCEL TIMEOUT (adapter-v1:385): an abort that was delivered
    /// but never confirmed must not hold the execution occupancy forever. After
    /// `timeout_secs`, the hub STOPS the adapter (a deliberate, performed stop, not
    /// a send failure) and settles the turn `interrupted`. This releases occupancy
    /// on a real action rather than a hope.
    pub async fn timeout_unconfirmed_cancels(&self, timeout_secs: i64) -> Result<usize, SessionError> {
        let mut n = 0;
        for (turn_id, session_id) in self.db.cancelling_turns_older_than(timeout_secs)? {
            // Stop the adapter so no late events from the old turn can arrive; the
            // stop is best-effort, the settle is what releases occupancy.
            let _ = self.runtime.stop(&session_id).await;
            self.settle_turn(&turn_id, "interrupted", Some("the cancel was not confirmed within the timeout"))
                .await;
            n += 1;
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
        // Serialize the cancel INTENT with admission and dispatch on the session
        // lock, so the `running` commit and the `cancelling` write cannot interleave
        // (TASK-048 F4).
        let lock = self.lock_for(session_id).await;
        let _guard = lock.lock().await;
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
        // The intent MUST be durable: if the durable write fails, the cancel is NOT
        // recorded and the caller must not believe it was.
        if !self.db.set_turn_state(&active.id, "cancelling")? {
            // The turn is already terminal (or gone): nothing to cancel.
            let t = self
                .db
                .turn(&active.id)?
                .ok_or_else(|| SessionError::NotFound(active.id.clone()))?;
            return Ok(turn_view(&t));
        }
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

#[cfg(test)]
mod reconcile_tests {
    use super::*;
    use agent_hub_db::{Db, SessionRow};

    fn sessions(db: Db) -> Sessions {
        Sessions::new(
            db,
            agent_hub_events::Bus::new(8, 8),
            std::env::temp_dir(),
            Box::new(|_| true),
            Box::new(|id| {
                Ok(HarnessSpec {
                    id: id.into(),
                    command: vec!["true".into()],
                    plugin_dir: std::path::PathBuf::from("."),
                    runtime_argv: None,
                    harness_dir: std::path::PathBuf::from("."),
                    skills_dir: std::path::PathBuf::from("."),
                    extensions_dir: std::path::PathBuf::from("."),
                    presets_dir: None,
                })
            }),
        )
    }

    fn row(id: &str, status: &str) -> SessionRow {
        SessionRow {
            id: id.into(),
            harness_id: "pi".into(),
            model_provider_id: None,
            model_id: None,
            applied_model: None,
            applied_provider: None,
            applied_route: None,
            preset_id: None,
            applied_preset: None,
            plan: None,
            review: None,
            applied_plan: None,
            applied_review: None,
            cwd: None,
            additional_dirs: Vec::new(),
            title: None,
            status: status.into(),
            created_at: now_utc(),
            updated_at: now_utc(),
            deleted: false,
            forked_from_session: None,
            forked_from_turn: None,
            native_ref: Some("ref".into()),
            start_error: None,
        }
    }

    /// N2: at boot an `active` session with no process becomes `needs-repair`, an
    /// open turn becomes terminal, and a `starting` session becomes `starting_failed`.
    #[test]
    fn boot_reconciles_orphaned_sessions_and_open_turns() {
        let db = Db::open_in_memory().unwrap();
        db.insert_session(&row("a", "active")).unwrap();
        db.insert_session(&row("b", "starting")).unwrap();
        db.insert_session(&row("c", "readonly")).unwrap();
        db.admit_turn("a", "k", "turn:x", "t1").unwrap();
        db.set_turn_state("t1", "running").unwrap();

        let s = sessions(db);
        let n = s.reconcile_interrupted().unwrap();
        assert!(n >= 2, "active + starting are reconciled");

        assert_eq!(s.db.session("a").unwrap().unwrap().status, "needs-repair");
        assert_eq!(s.db.session("b").unwrap().unwrap().status, "starting_failed");
        assert_eq!(s.db.session("c").unwrap().unwrap().status, "readonly", "closed stays closed");
        let t = s.db.turn("t1").unwrap().unwrap();
        assert_eq!(t.state, "ended");
        assert_eq!(t.ended.as_deref(), Some("failed"), "an open turn is settled, not left busy");
    }
}
