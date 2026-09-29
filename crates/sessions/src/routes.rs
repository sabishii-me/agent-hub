//! The `/v1/sessions` routes.
//!
//! **NOT WIRED.** A session is not handed to an adapter yet: there is no
//! execution boundary behind `active` / `running` / `fork`. Per a cross-review
//! (TASK-048 F01) an earlier version answered those successes with no harness
//! behind them; that is a fake success, not an unfinished handler. Until the
//! adapter boundary is real, every session route answers `501 not_implemented`
//! with a detail, so a caller can never mistake a fabricated state for a running
//! session. (ARCHITECTURE §18.)

use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use axum::Router;

use agent_hub_transport::{DomainError, ErrorRenderer};

use crate::service::Sessions;

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

fn table() -> agent_hub_transport::RouteTable<SessionsState> {
    let n = not_implemented;
    agent_hub_transport::RouteTable::new()
        .get("/v1/sessions", n)
        .post("/v1/sessions", n)
        .get("/v1/sessions/{id}", n)
        .patch("/v1/sessions/{id}", n)
        .delete("/v1/sessions/{id}", n)
        .get("/v1/sessions/{id}/turns", n)
        .post("/v1/sessions/{id}/turns", n)
        .post("/v1/sessions/{id}/cancel", n)
        .post("/v1/sessions/{id}/close", n)
        .post("/v1/sessions/{id}/reopen", n)
        .post("/v1/sessions/{id}/fork", n)
}

pub fn routes() -> Router<SessionsState> {
    table().router()
}

pub fn surface() -> Vec<String> {
    table().surface()
}

/// `not_implemented` goes through the SAME error renderer as every domain, so
/// there is one mapping (invariant 13.7, TASK-048 C5).
async fn not_implemented(State(s): State<SessionsState>) -> Response {
    s.errors.render(&DomainError::new(
        "not_implemented",
        "sessions are not wired to an adapter yet; a session cannot be started, and no state is faked",
    ))
}
