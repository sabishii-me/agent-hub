//! The sessions domain (`ARCHITECTURE` §6): the hub's control state over sessions
//! and turns, kept distinct from the harness's native conversation.
//!
//! A session now owns a **real adapter process** (`runtime.rs`): `create` starts
//! it with `session/start` + `config/set`, and reports `active` only when that
//! really succeeded. `close` stops that session's process (the record stays,
//! status `readonly`); `reopen` restarts on the stored native ref. TURN, FORK and
//! COMPACT are **not wired** and answer `not_implemented`.

use std::path::PathBuf;

use agent_hub_db::{now_utc, Db, SessionRow};
use agent_hub_events::Bus;
use serde::{Deserialize, Serialize};

use crate::runtime::{Sessions as Runtime, StartSpec};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session `{0}` not found")]
    NotFound(String),
    #[error("session `{0}` is not active")]
    NotActive(String),
    #[error("no adapter for harness `{0}`")]
    NoHarness(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("the session could not be started: {0}")]
    Start(String),
    #[error(transparent)]
    Db(#[from] agent_hub_db::DbError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl SessionError {
    pub fn code(&self) -> &'static str {
        match self {
            SessionError::NotFound(_) => "unknown_session",
            SessionError::NotActive(_) => "session_closed",
            SessionError::NoHarness(_) => "harness_not_found",
            SessionError::Validation(_) => "validation_failed",
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
    pub plan: Option<bool>,
    pub review: Option<bool>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub status: String,
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
}

/// A harness the sessions domain may start: its adapter argv + plugin dir + the
/// hub-side dirs it needs. Supplied by the hub (which reads the plugin manifest);
/// the sessions domain does not scan plugins itself.
#[derive(Debug, Clone)]
pub struct HarnessSpec {
    pub id: String,
    pub command: Vec<String>,
    pub plugin_dir: PathBuf,
    pub runtime_argv: Option<Vec<String>>,
}

/// The sessions domain: the durable rows plus the running adapter processes.
pub struct Sessions {
    pub db: Db,
    bus: Bus,
    pub data_dir: PathBuf,
    runtime: Runtime,
    /// Resolves a harness id to its adapter spec (from the plugin registry).
    harnesses: Box<dyn Fn(&str) -> Option<HarnessSpec> + Send + Sync>,
}

impl Sessions {
    pub fn new(
        db: Db,
        bus: Bus,
        data_dir: impl Into<PathBuf>,
        harnesses: Box<dyn Fn(&str) -> Option<HarnessSpec> + Send + Sync>,
    ) -> Self {
        Sessions {
            db,
            bus,
            data_dir: data_dir.into(),
            runtime: Runtime::new(),
            harnesses,
        }
    }

    fn harness(&self, id: &str) -> Result<HarnessSpec, SessionError> {
        (self.harnesses)(id).ok_or_else(|| SessionError::NoHarness(id.into()))
    }

    /// Create a session AND start its real adapter. `active` is returned only
    /// when `session/start` + `config/set` both succeeded; a failure leaves no
    /// `active` row.
    pub async fn create(&self, req: CreateSession) -> Result<SessionView, SessionError> {
        if req.harness_id.trim().is_empty() {
            return Err(SessionError::Validation("harnessId is required".into()));
        }
        let harness = self.harness(&req.harness_id)?;
        let id = new_id("s");
        let cwd = match req.cwd.filter(|c| !c.is_empty()) {
            Some(c) => c,
            None => {
                let dir = self.data_dir.join("sessions").join(&id);
                std::fs::create_dir_all(&dir)?;
                dir.to_string_lossy().to_string()
            }
        };

        // Start the real adapter BEFORE writing an active row.
        let spec = StartSpec {
            sid: id.clone(),
            harness_id: harness.id.clone(),
            command: harness.command.clone(),
            plugin_dir: harness.plugin_dir.clone(),
            cwd: PathBuf::from(&cwd),
            harness_dir: self.data_dir.join("agents").join(&req.harness_id),
            skills_dir: self.data_dir.join("agents").join(&req.harness_id).join("skills"),
            extensions_dir: self.data_dir.join("agents").join(&req.harness_id).join("extensions"),
            runtime_argv: harness.runtime_argv.clone(),
            resume: None,
            config: self.config_payload(&id),
        };
        let process = self
            .runtime
            .start(spec)
            .await
            .map_err(|e| SessionError::Start(e.to_string()))?;

        let now = now_utc();
        let row = SessionRow {
            id: id.clone(),
            harness_id: req.harness_id.clone(),
            model_provider_id: req.model_provider_id.clone(),
            model_id: req.model_id.clone(),
            applied_model: process
                .applied
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            plan: req.plan,
            review: req.review,
            cwd: Some(cwd),
            title: req.title.clone(),
            status: "active".into(),
            created_at: now.clone(),
            updated_at: now,
            deleted: false,
            forked_from_session: None,
            forked_from_turn: None,
            native_ref: Some(process.native_ref.clone()),
        };
        self.db.insert_session(&row)?;
        let view = self.view(&row);
        self.bus.publish("session.created", serde_json::json!({ "session": view }));
        Ok(view)
    }

    /// The `config/set` payload the hub materializes. Empty for now (no managed
    /// provider, no preset); the handshake shape is what matters here.
    fn config_payload(&self, _sid: &str) -> serde_json::Value {
        serde_json::json!({})
    }

    pub fn get(&self, id: &str) -> Result<SessionView, SessionError> {
        let row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        Ok(self.view(&row))
    }

    pub fn list(&self) -> Result<Vec<SessionView>, SessionError> {
        Ok(self.db.list_sessions()?.iter().map(|r| self.view(r)).collect())
    }

    /// Close: stop the session's process, keep the record, status -> readonly.
    /// Idempotent. ACP: cancel work and free resources; the session stays.
    pub async fn close(&self, id: &str) -> Result<SessionView, SessionError> {
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        self.runtime.stop(id).await;
        if row.status != "readonly" {
            row.status = "readonly".into();
            row.updated_at = now_utc();
            self.db.update_session(&row)?;
            self.bus.publish("session.closed", serde_json::json!({ "session": self.view(&row) }));
        }
        Ok(self.view(&row))
    }

    /// Reopen: clear the status; the next start re-attaches on the stored ref.
    pub async fn reopen(&self, id: &str) -> Result<SessionView, SessionError> {
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        if row.native_ref.is_none() {
            return Err(SessionError::Validation("the session has no native ref to reopen".into()));
        }
        let harness = self.harness(&row.harness_id)?;
        let spec = StartSpec {
            sid: row.id.clone(),
            harness_id: harness.id.clone(),
            command: harness.command.clone(),
            plugin_dir: harness.plugin_dir.clone(),
            cwd: PathBuf::from(row.cwd.clone().unwrap_or_default()),
            harness_dir: self.data_dir.join("agents").join(&row.harness_id),
            skills_dir: self.data_dir.join("agents").join(&row.harness_id).join("skills"),
            extensions_dir: self.data_dir.join("agents").join(&row.harness_id).join("extensions"),
            runtime_argv: harness.runtime_argv.clone(),
            resume: row.native_ref.clone(),
            config: self.config_payload(&row.id),
        };
        let process = self
            .runtime
            .start(spec)
            .await
            .map_err(|e| SessionError::Start(e.to_string()))?;
        row.status = "active".into();
        row.native_ref = Some(process.native_ref.clone());
        row.updated_at = now_utc();
        self.db.update_session(&row)?;
        let view = self.view(&row);
        self.bus
            .publish("session.reopened", serde_json::json!({ "session": view, "reopened": true }));
        Ok(view)
    }

    /// Delete (soft): stop the process, mark the row deleted.
    pub async fn delete(&self, id: &str) -> Result<(), SessionError> {
        if self.db.session(id)?.is_none() {
            return Err(SessionError::NotFound(id.into()));
        }
        self.runtime.stop(id).await;
        self.db.mark_session_deleted(id)?;
        Ok(())
    }

    /// Whether the session's adapter process is currently running.
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
            plan: row.plan,
            review: row.review,
            cwd: row.cwd.clone(),
            title: row.title.clone(),
            status: row.status.clone(),
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
