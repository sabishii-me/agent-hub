//! The `/v1/sessions` routes.
//!
//! Wired: create (a **long command**: 202 + Location, command identity,
//! background start), get, list, close, reopen, delete. Not wired: turn, fork,
//! compact, patch, messages, stats, skills, repair, resources - `501`.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};

use agent_hub_transport::{Accepted, DomainError, ErrorRenderer, RouteTable};

use crate::service::{CreateOutcome, CreateSession, SessionError, Sessions, TurnOutcome, TurnRequest};

#[derive(Clone)]
pub struct SessionsState {
    pub sessions: Arc<Sessions>,
    pub errors: ErrorRenderer,
}

impl SessionsState {
    pub fn new(sessions: Sessions, errors: ErrorRenderer) -> Self {
        SessionsState { sessions: Arc::new(sessions), errors }
    }

    pub fn new_shared(sessions: Arc<Sessions>, errors: ErrorRenderer) -> Self {
        SessionsState { sessions, errors }
    }
}

fn table() -> RouteTable<SessionsState> {
    RouteTable::new()
        .get("/v1/sessions", list)
        .post("/v1/sessions", create)
        .get("/v1/sessions/{id}", get_one)
        .patch("/v1/sessions/{id}", not_implemented)
        .delete("/v1/sessions/{id}", remove)
        .get("/v1/sessions/{id}/turns", list_turns)
        .post("/v1/sessions/{id}/turns", send_turn)
        .post("/v1/sessions/{id}/cancel", cancel_turn)
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

/// POST /v1/sessions - a LONG command. Accept (reserve the command and the
/// `starting` session durably), answer `202 Accepted` + `Location`, and start the
/// adapter in the background. A retry returns the ORIGINAL session.
async fn create(
    State(s): State<SessionsState>,
    headers: HeaderMap,
    Json(req): Json<CreateSession>,
) -> Response {
    let command_id = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| crate::service::new_id("create"));

    let view = match s.sessions.accept_create(&command_id, req) {
        Ok(CreateOutcome::Accepted(view)) => {
            // Detached start: the caller is answered 202 at once.
            let sessions = s.sessions.clone();
            let sid = view.id.clone();
            tokio::spawn(async move { sessions.run_start(sid).await; });
            view
        }
        Ok(CreateOutcome::Replay(view)) => {
            // A retry: the resource already exists; answer 202 pointing at it.
            view
        }
        Err(e) => return err(&s, e),
    };

    let location = format!("/v1/sessions/{}", view.id);
    Accepted::new(location, serde_json::json!({ "session": view })).into_response()
}

async fn get_one(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.get(&id) {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn remove(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    let sessions = s.sessions.clone();
    match sessions.delete(&id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id, "exported": false })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn close(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    let sessions = s.sessions.clone();
    match sessions.close(&id).await {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn reopen(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    let sessions = s.sessions.clone();
    match sessions.reopen(&id).await {
        Ok(session) => {
            Json(serde_json::json!({ "session": session, "reopened": true })).into_response()
        }
        Err(e) => err(&s, e),
    }
}

/// POST /v1/sessions/{id}/turns - a LONG command: reserve the turn identity,
/// answer `202 + Location`, and run the prompt detached. An unknown turn is never
/// replayed; a second turn while one runs is refused.
async fn send_turn(
    State(s): State<SessionsState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<TurnRequest>,
) -> Response {
    let text = crate::service::text_of_request(&req.content);
    match s.sessions.accept_turn(&id, req) {
        Ok(TurnOutcome::Accepted(turn)) => {
            let sessions = s.sessions.clone();
            let sid = id.clone();
            let tid = turn.id.clone();
            tokio::spawn(async move {
                sessions.run_turn(sid, tid, text).await;
            });
            let location = format!("/v1/sessions/{id}/turns");
            Accepted::new(location, serde_json::json!({ "turn": turn })).into_response()
        }
        Ok(TurnOutcome::Replay(turn)) => Json(serde_json::json!({ "turn": turn })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn list_turns(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.list_turns(&id) {
        Ok(turns) => Json(serde_json::json!({ "turns": turns, "next_cursor": null })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn cancel_turn(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    let sessions = s.sessions.clone();
    match sessions.cancel_turn(&id).await {
        Ok(turn) => Json(serde_json::json!({ "turn": turn })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn not_implemented(State(s): State<SessionsState>) -> Response {
    s.errors.render(&DomainError::new(
        "not_implemented",
        "this session route needs the turn lifecycle, which is not wired yet",
    ))
}
