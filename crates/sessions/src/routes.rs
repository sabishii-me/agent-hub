//! The `/v1/sessions` routes.
//!
//! Wired: create (a **long command**: 202 + Location, command identity,
//! background start), get, list, close, reopen, delete. Not wired: turn, fork,
//! compact, patch, messages, stats, skills, repair, resources - `501`.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};

use agent_hub_transport::{Accepted, DomainError, ErrorRenderer, RouteTable};

use crate::service::{
    CompactRequest, CreateOutcome, CreateSession, ForkRequest, PatchSession, SessionError, Sessions,
    TurnOutcome, TurnRequest,
};

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
        .patch("/v1/sessions/{id}", patch_one)
        .delete("/v1/sessions/{id}", remove)
        .get("/v1/sessions/{id}/turns", list_turns)
        .post("/v1/sessions/{id}/turns", send_turn)
        .post("/v1/sessions/{id}/cancel", cancel_turn)
        .post("/v1/sessions/{id}/close", close)
        .post("/v1/sessions/{id}/reopen", reopen)
        .post("/v1/sessions/{id}/fork", fork)
        .post("/v1/sessions/{id}/compact", compact)
        .get("/v1/sessions/{id}/messages", messages)
        .get("/v1/sessions/{id}/stats", stats)
        .get("/v1/sessions/{id}/skills", session_skills)
        .get("/v1/sessions/{id}/artifacts", not_implemented)
        .post("/v1/sessions/{id}/repair", not_implemented)
        .get("/v1/sessions/{id}/resources", not_implemented)
        .post("/v1/sessions/{id}/resources/read", not_implemented)
}

#[derive(serde::Deserialize)]
struct MessagesQuery {
    #[serde(rename = "beforeId")]
    before_id: Option<String>,
    limit: Option<u32>,
}

/// GET /v1/sessions/{id}/messages - read-through native history. The hub asks the
/// adapter's `history/page` (starting the session's process first if needed) and
/// translates its `{messages, hasMore}` into the contract's
/// `{messages, next_cursor}`: the cursor is the OLDEST id on the page while more
/// remain, else null.
async fn messages(
    State(s): State<SessionsState>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<MessagesQuery>,
) -> Response {
    let mut params = serde_json::json!({});
    if let Some(b) = &q.before_id {
        params["beforeId"] = serde_json::json!(b);
    }
    if let Some(l) = q.limit {
        params["limit"] = serde_json::json!(l);
    }
    let sessions = s.sessions.clone();
    match sessions.read_through(&id, "history/page", params).await {
        Ok(v) => {
            let messages = v.get("messages").cloned().unwrap_or(serde_json::json!([]));
            let has_more = v.get("hasMore").and_then(|h| h.as_bool()).unwrap_or(false);
            let next_cursor = if has_more {
                messages
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|m| m.get("id"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            };
            Json(serde_json::json!({ "messages": messages, "next_cursor": next_cursor })).into_response()
        }
        Err(e) => err(&s, e),
    }
}

/// GET /v1/sessions/{id}/stats - read-through session statistics from the harness.
/// Nothing is cached and nothing is invented: a field the harness does not report is
/// absent.
async fn stats(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    let sessions = s.sessions.clone();
    match sessions.read_through(&id, "session/stats", serde_json::json!({})).await {
        Ok(v) => {
            let mut body = v.as_object().cloned().unwrap_or_default();
            body.insert("sessionId".into(), serde_json::json!(id));
            body.insert("source".into(), serde_json::json!("harness"));
            Json(serde_json::Value::Object(body)).into_response()
        }
        Err(e) => err(&s, e),
    }
}

/// GET /v1/sessions/{id}/skills - the skills THIS session's harness actually has,
/// read from the harness (not the hub's installed set).
async fn session_skills(State(s): State<SessionsState>, AxumPath(id): AxumPath<String>) -> Response {
    let sessions = s.sessions.clone();
    match sessions.read_through(&id, "skills/list", serde_json::json!({})).await {
        Ok(v) => {
            let skills = v.get("skills").cloned().unwrap_or(serde_json::json!([]));
            Json(serde_json::json!({
                "sessionId": id,
                "source": "harness",
                "known": true,
                "skills": skills
            }))
            .into_response()
        }
        Err(e) => err(&s, e),
    }
}

/// POST /v1/sessions/{id}/compact - ask the harness to compact its own
/// conversation. The hub reports the harness's own result (nothing is computed
/// here).
async fn compact(
    State(s): State<SessionsState>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<CompactRequest>>,
) -> Response {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let sessions = s.sessions.clone();
    match sessions.compact(&id, req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(&s, e),
    }
}

/// POST /v1/sessions/{id}/fork - start a NEW session whose conversation ends at a
/// completed turn of this one (the source is untouched).
async fn fork(
    State(s): State<SessionsState>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<ForkRequest>>,
) -> Response {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let sessions = s.sessions.clone();
    match sessions.fork(&id, req).await {
        Ok((session, forked_from)) => {
            Json(serde_json::json!({ "session": session, "forkedFrom": forked_from })).into_response()
        }
        Err(e) => err(&s, e),
    }
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

async fn patch_one(
    State(s): State<SessionsState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<PatchSession>,
) -> Response {
    let sessions = s.sessions.clone();
    match sessions.patch_session(&id, req).await {
        Ok(out) => {
            let mut body = serde_json::json!({ "session": out.session });
            body["warning"] = serde_json::json!(out.warning);
            Json(body).into_response()
        }
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
