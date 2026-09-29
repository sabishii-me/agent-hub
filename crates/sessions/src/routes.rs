//! The `/v1/sessions` routes, merged onto the frame's transport.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::service::{CreateSession, SessionError, Sessions};

/// Shared sessions state merged into the transport's router.
#[derive(Clone)]
pub struct SessionsState {
    pub sessions: Arc<Sessions>,
}

impl SessionsState {
    pub fn new(sessions: Sessions) -> Self {
        SessionsState { sessions: Arc::new(sessions) }
    }
}

pub fn routes() -> Router<SessionsState> {
    Router::new()
        .route("/v1/sessions", get(list).post(create))
        .route("/v1/sessions/{id}", get(get_one).patch(patch).delete(remove))
        .route("/v1/sessions/{id}/turns", get(list_turns).post(admit))
        .route("/v1/sessions/{id}/cancel", post(cancel))
        .route("/v1/sessions/{id}/close", post(close))
        .route("/v1/sessions/{id}/reopen", post(reopen))
        .route("/v1/sessions/{id}/fork", post(fork))
}

fn err(e: SessionError) -> Response {
    let (status, code) = match &e {
        SessionError::NotFound(_) => (StatusCode::NOT_FOUND, "session_not_found"),
        SessionError::Closed(_) => (StatusCode::CONFLICT, "session_closed"),
        SessionError::Busy(_) => (StatusCode::CONFLICT, "session_busy"),
        SessionError::Validation(_) => (StatusCode::BAD_REQUEST, "validation_failed"),
        SessionError::Db(_) | SessionError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    (status, Json(serde_json::json!({ "error": code, "detail": e.to_string() }))).into_response()
}

async fn create(State(s): State<SessionsState>, Json(req): Json<CreateSession>) -> Response {
    match s.sessions.create(req) {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => err(e),
    }
}

async fn list(State(s): State<SessionsState>) -> Response {
    match s.sessions.list() {
        Ok(sessions) => {
            Json(serde_json::json!({ "sessions": sessions, "next_cursor": null })).into_response()
        }
        Err(e) => err(e),
    }
}

async fn get_one(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.get(&id) {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
struct PatchBody {
    #[serde(rename = "modelProviderId")]
    model_provider_id: Option<Option<String>>,
    #[serde(rename = "modelId")]
    model_id: Option<Option<String>>,
    title: Option<Option<String>>,
    plan: Option<Option<bool>>,
    review: Option<Option<bool>>,
}

async fn patch(
    State(s): State<SessionsState>,
    AxumPath(id): AxumPath<String>,
    Json(b): Json<PatchBody>,
) -> Response {
    match s.sessions.patch(&id, b.model_provider_id, b.model_id, b.title, b.plan, b.review) {
        Ok(session) => {
            Json(serde_json::json!({ "session": session, "warning": null })).into_response()
        }
        Err(e) => err(e),
    }
}

async fn remove(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.delete(&id) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id, "exported": false })).into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
struct TurnBody {
    #[serde(rename = "idempotencyKey")]
    idempotency_key: String,
    #[serde(default)]
    content: serde_json::Value,
}

async fn admit(
    State(s): State<SessionsState>,
    AxumPath(id): AxumPath<String>,
    Json(b): Json<TurnBody>,
) -> Response {
    match s.sessions.admit_turn(&id, &b.idempotency_key, b.content) {
        Ok(turn) => Json(serde_json::json!({ "turn": turn })).into_response(),
        Err(e) => err(e),
    }
}

async fn list_turns(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.list_turns(&id) {
        Ok(turns) => Json(serde_json::json!({ "turns": turns, "next_cursor": null })).into_response(),
        Err(e) => err(e),
    }
}

async fn cancel(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.cancel_turn(&id) {
        Ok(turn) => Json(serde_json::json!({ "turn": turn })).into_response(),
        Err(e) => err(e),
    }
}

async fn close(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.close(&id) {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => err(e),
    }
}

async fn reopen(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.sessions.reopen(&id) {
        Ok(session) => {
            Json(serde_json::json!({ "session": session, "reopened": true })).into_response()
        }
        Err(e) => err(e),
    }
}

#[derive(Deserialize, Default)]
struct ForkBody {
    #[serde(rename = "afterTurnId")]
    after_turn_id: Option<String>,
}

async fn fork(
    State(s): State<SessionsState>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<ForkBody>>,
) -> Response {
    let after = body.and_then(|b| b.0.after_turn_id);
    match s.sessions.fork(&id, after.clone()) {
        Ok(session) => Json(serde_json::json!({
            "session": session,
            "forkedFrom": { "sessionId": id, "afterTurnId": after }
        }))
        .into_response(),
        Err(e) => err(e),
    }
}
