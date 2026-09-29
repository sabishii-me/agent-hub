//! The `/v1/sessions` routes.
//!
//! Wired: create (a real adapter start), get, list, close (stop the process),
//! reopen (restart on the stored ref), delete. NOT wired: turn, fork, compact,
//! patch - they answer `501 not_implemented`, because they need the turn
//! lifecycle that is not built yet.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};

use agent_hub_transport::{DomainError, ErrorRenderer, RouteTable};

use crate::service::{CreateSession, SessionError, Sessions};

#[derive(Clone)]
pub struct SessionsState {
    pub sessions: Arc<Sessions>,
    pub errors: ErrorRenderer,
}

impl SessionsState {
    pub fn new(sessions: Sessions, errors: ErrorRenderer) -> Self {
        SessionsState { sessions: Arc::new(sessions), errors }
    }
}

fn table() -> RouteTable<SessionsState> {
    RouteTable::new()
        .get("/v1/sessions", list)
        .post("/v1/sessions", create)
        .get("/v1/sessions/{id}", get_one)
        .patch("/v1/sessions/{id}", not_implemented)
        .delete("/v1/sessions/{id}", remove)
        .get("/v1/sessions/{id}/turns", not_implemented)
        .post("/v1/sessions/{id}/turns", not_implemented)
        .post("/v1/sessions/{id}/cancel", not_implemented)
        .post("/v1/sessions/{id}/close", close)
        .post("/v1/sessions/{id}/reopen", reopen)
        .post("/v1/sessions/{id}/fork", not_implemented)
        .post("/v1/sessions/{id}/compact", not_implemented)
        .get("/v1/sessions/{id}/messages", not_implemented)
        .get("/v1/sessions/{id}/stats", not_implemented)
        .get("/v1/sessions/{id}/skills", not_implemented)
        .get("/v1/sessions/{id}/artifacts", not_implemented)
        .post("/v1/sessions/{id}/repair", not_implemented)
        .get("/v1/sessions/{id}/resources", not_implemented)
        .post("/v1/sessions/{id}/resources/read", not_implemented)
}

pub fn routes() -> Router<SessionsState> {
    table().router()
}

pub fn surface() -> Vec<String> {
    table().surface()
}

fn err(s: &SessionsState, e: SessionError) -> Response {
    s.errors.render(&e.to_domain_error())
}

async fn list(State(s): State<SessionsState>) -> Response {
    match s.sessions.list() {
        Ok(sessions) => {
            Json(serde_json::json!({ "sessions": sessions, "next_cursor": null })).into_response()
        }
        Err(e) => err(&s, e),
    }
}

async fn create(
    State(s): State<SessionsState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateSession>,
) -> Response {
    // The client's logical command identity (ARCHITECTURE §11, R1): a retry with
    // the same key returns the same session; an absent key is a fresh intent.
    let command_id = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::service::new_id("create"));
    match s.sessions.create(&command_id, req).await {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn get_one(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.get(&id) {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn remove(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.delete(&id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id, "exported": false })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn close(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.close(&id).await {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn reopen(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.reopen(&id).await {
        Ok(session) => {
            Json(serde_json::json!({ "session": session, "reopened": true })).into_response()
        }
        Err(e) => err(&s, e),
    }
}

/// A route that needs the turn lifecycle, which is not built yet.
async fn not_implemented(State(s): State<SessionsState>) -> Response {
    s.errors.render(&DomainError::new(
        "not_implemented",
        "this session route needs the turn lifecycle, which is not wired yet",
    ))
}
