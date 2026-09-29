//! The sessions domain (`ARCHITECTURE` §6): the hub's control state over
//! sessions and turns, kept distinct from the harness's native conversation.
//!
//! The hub owns: session identity, the requested/applied model and modes, the
//! working directory, the status, and the **admission** of a turn. The harness
//! owns the conversation itself. A turn carries a logical command identity
//! (`idempotencyKey`, contract-mandated), so a retried `202` never starts a
//! second turn (ARCHITECTURE §11, R1).

use agent_hub_db::{now_utc, Db, SessionRow, TurnRow};
use agent_hub_events::Bus;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session `{0}` not found")]
    NotFound(String),
    #[error("session `{0}` is closed")]
    Closed(String),
    #[error("session `{0}` is busy with a turn")]
    Busy(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error(transparent)]
    Db(#[from] agent_hub_db::DbError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
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
    #[serde(rename = "activeTurn")]
    pub active_turn: Option<TurnView>,
    #[serde(rename = "turnRunning")]
    pub turn_running: bool,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    pub deleted: bool,
}

/// A turn as the contract's `turnState`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnView {
    pub id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
    #[serde(rename = "partialPersisted")]
    pub partial_persisted: bool,
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

/// The sessions domain.
pub struct Sessions {
    pub db: Db,
    bus: Bus,
    /// Where an omitted `cwd` allocates a directory.
    pub data_dir: std::path::PathBuf,
}

impl Sessions {
    pub fn new(db: Db, bus: Bus, data_dir: impl Into<std::path::PathBuf>) -> Self {
        Sessions { db, bus, data_dir: data_dir.into() }
    }

    fn announce(&self, name: &str, payload: serde_json::Value) {
        self.bus.publish(name, payload);
    }

    /// Create a session. `cwd` omitted -> a fresh directory under the data root.
    pub fn create(&self, req: CreateSession) -> Result<SessionView, SessionError> {
        if req.harness_id.trim().is_empty() {
            return Err(SessionError::Validation("harnessId is required".into()));
        }
        let id = new_id("s");
        let cwd = match req.cwd.filter(|c| !c.is_empty()) {
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
            harness_id: req.harness_id,
            model_provider_id: req.model_provider_id,
            model_id: req.model_id,
            applied_model: None,
            plan: req.plan,
            review: req.review,
            cwd: Some(cwd),
            title: req.title,
            status: "active".into(),
            created_at: now.clone(),
            updated_at: now,
            deleted: false,
            forked_from_session: None,
            forked_from_turn: None,
        };
        self.db.insert_session(&row)?;
        let view = self.view(&row);
        self.announce("session.created", serde_json::json!({ "session": view }));
        Ok(view)
    }

    pub fn get(&self, id: &str) -> Result<SessionView, SessionError> {
        let row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        Ok(self.view(&row))
    }

    pub fn list(&self) -> Result<Vec<SessionView>, SessionError> {
        Ok(self.db.list_sessions()?.iter().map(|r| self.view(r)).collect())
    }

    fn view(&self, row: &SessionRow) -> SessionView {
        let active = self.db.active_turn(&row.id).ok().flatten();
        let active_turn = active.as_ref().map(|t| TurnView {
            id: t.id.clone(),
            state: t.state.clone(),
            ended: t.ended.clone(),
            partial_persisted: false,
        });
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
            turn_running: active.is_some(),
            active_turn,
            created_at: row.created_at.clone(),
            updated_at: row.updated_at.clone(),
            deleted: row.deleted,
        }
    }
}

/// A fresh, sortable id: `prefix-<millis>-<counter>`.
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

impl Sessions {
    /// Admit a turn. The `idempotencyKey` is the logical command identity: the
    /// same key returns the same turn (a retried `202` does not start a second
    /// turn), while a different key is a new turn even in the same session.
    pub fn admit_turn(
        &self,
        session_id: &str,
        idempotency_key: &str,
        _content: serde_json::Value,
    ) -> Result<TurnView, SessionError> {
        if idempotency_key.trim().is_empty() {
            return Err(SessionError::Validation("idempotencyKey is required".into()));
        }
        let session = self
            .db
            .session(session_id)?
            .ok_or_else(|| SessionError::NotFound(session_id.into()))?;
        if session.status != "active" {
            return Err(SessionError::Closed(session_id.into()));
        }

        // Same key -> the same turn (replay), never a second one.
        if let Some(existing) = self.db.turn_by_key(session_id, idempotency_key)? {
            return Ok(TurnView {
                id: existing.id,
                state: existing.state,
                ended: existing.ended,
                partial_persisted: false,
            });
        }
        // A different key while a turn runs is refused (session_busy), not queued.
        if let Some(active) = self.db.active_turn(session_id)? {
            let _ = active;
            return Err(SessionError::Busy(session_id.into()));
        }

        let now = agent_hub_db::now_utc();
        let turn = TurnRow {
            id: new_id("t"),
            session_id: session_id.into(),
            state: "admitted".into(),
            ended: None,
            native_turn_id: None,
            idempotency_key: idempotency_key.into(),
            created_at: now.clone(),
            ended_at: None,
        };
        self.db.insert_turn(&turn)?;

        let view = TurnView {
            id: turn.id.clone(),
            state: turn.state.clone(),
            ended: None,
            partial_persisted: false,
        };
        self.announce("turn.admitted", serde_json::json!({ "turn": view }));
        // The adapter drives the turn; without one, it is admitted and awaits it.
        let _ = self.db.set_turn_state(&turn.id, "running");
        let running = TurnView { state: "running".into(), ..view };
        self.announce("turn.running", serde_json::json!({ "turn": running }));
        Ok(running)
    }

    pub fn list_turns(&self, session_id: &str) -> Result<Vec<TurnView>, SessionError> {
        if self.db.session(session_id)?.is_none() {
            return Err(SessionError::NotFound(session_id.into()));
        }
        Ok(self
            .db
            .list_turns(session_id)?
            .iter()
            .map(|t| TurnView {
                id: t.id.clone(),
                state: t.state.clone(),
                ended: t.ended.clone(),
                partial_persisted: false,
            })
            .collect())
    }

    /// Cancel the running turn. The adapter's cancel ACK is not "stopped": the
    /// hub records `cancelling` and the turn ends when the adapter says so.
    pub fn cancel_turn(&self, session_id: &str) -> Result<TurnView, SessionError> {
        let active = self
            .db
            .active_turn(session_id)?
            .ok_or_else(|| SessionError::NotFound(format!("{session_id}: no active turn")))?;
        self.db.set_turn_state(&active.id, "cancelling")?;
        let view = TurnView {
            id: active.id.clone(),
            state: "cancelling".into(),
            ended: None,
            partial_persisted: false,
        };
        self.announce("turn.cancelling", serde_json::json!({ "turn": view }));
        Ok(view)
    }

    /// Patch a session's control state.
    #[allow(clippy::too_many_arguments)]
    pub fn patch(
        &self,
        id: &str,
        model_provider_id: Option<Option<String>>,
        model_id: Option<Option<String>>,
        title: Option<Option<String>>,
        plan: Option<Option<bool>>,
        review: Option<Option<bool>>,
    ) -> Result<SessionView, SessionError> {
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        if let Some(v) = model_provider_id {
            row.model_provider_id = v;
        }
        if let Some(v) = model_id {
            row.model_id = v;
        }
        if let Some(v) = title {
            row.title = v;
        }
        if let Some(v) = plan {
            row.plan = v;
        }
        if let Some(v) = review {
            row.review = v;
        }
        row.updated_at = agent_hub_db::now_utc();
        self.db.update_session(&row)?;
        let view = self.view(&row);
        self.announce("session.updated", serde_json::json!({ "session": view }));
        Ok(view)
    }

    /// Close (stop driving) or reopen a session. Closing is not deleting.
    pub fn close(&self, id: &str) -> Result<SessionView, SessionError> {
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        row.status = "readonly".into();
        row.updated_at = agent_hub_db::now_utc();
        self.db.update_session(&row)?;
        let view = self.view(&row);
        self.announce("session.closed", serde_json::json!({ "session": view }));
        Ok(view)
    }

    pub fn reopen(&self, id: &str) -> Result<SessionView, SessionError> {
        let mut row = self.db.session(id)?.ok_or_else(|| SessionError::NotFound(id.into()))?;
        row.status = "active".into();
        row.updated_at = agent_hub_db::now_utc();
        self.db.update_session(&row)?;
        let view = self.view(&row);
        self.announce("session.reopened", serde_json::json!({ "session": view }));
        Ok(view)
    }

    /// Fork: a new session that points at the source and the turn it forked
    /// after (a hub control-state relation; the harness conversation is copied
    /// by the adapter).
    pub fn fork(
        &self,
        source_id: &str,
        after_turn_id: Option<String>,
    ) -> Result<SessionView, SessionError> {
        let src = self.db.session(source_id)?.ok_or_else(|| SessionError::NotFound(source_id.into()))?;
        let id = new_id("s");
        let now = agent_hub_db::now_utc();
        let row = SessionRow {
            id: id.clone(),
            harness_id: src.harness_id.clone(),
            model_provider_id: src.model_provider_id.clone(),
            model_id: src.model_id.clone(),
            applied_model: src.applied_model.clone(),
            plan: src.plan,
            review: src.review,
            cwd: src.cwd.clone(),
            title: src.title.clone(),
            status: "active".into(),
            created_at: now.clone(),
            updated_at: now,
            deleted: false,
            forked_from_session: Some(source_id.into()),
            forked_from_turn: after_turn_id.clone(),
        };
        self.db.insert_session(&row)?;
        let view = self.view(&row);
        self.announce(
            "session.forked",
            serde_json::json!({ "session": view, "forkedFrom": { "sessionId": source_id, "afterTurnId": after_turn_id } }),
        );
        Ok(view)
    }

    /// Delete (soft): the row stays for export, `deleted` is set.
    pub fn delete(&self, id: &str) -> Result<(), SessionError> {
        if self.db.session(id)?.is_none() {
            return Err(SessionError::NotFound(id.into()));
        }
        self.db.mark_session_deleted(id)?;
        Ok(())
    }
}
