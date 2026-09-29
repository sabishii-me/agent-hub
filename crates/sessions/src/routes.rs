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

use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};

use agent_hub_transport::ErrorRenderer;

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

pub fn routes() -> Router<SessionsState> {
    Router::new()
        .route("/v1/sessions", get(not_implemented).post(not_implemented))
        .route(
            "/v1/sessions/{id}",
            get(not_implemented).patch(not_implemented).delete(not_implemented),
        )
        .route("/v1/sessions/{id}/turns", get(not_implemented).post(not_implemented))
        .route("/v1/sessions/{id}/cancel", post(not_implemented))
        .route("/v1/sessions/{id}/close", post(not_implemented))
        .route("/v1/sessions/{id}/reopen", post(not_implemented))
        .route("/v1/sessions/{id}/fork", post(not_implemented))
}

async fn not_implemented() -> Response {
    // `not_implemented` (501) is the contract's own code for "endpoint not
    // implemented; never faked". Rendered without a renderer because it is a
    // fixed, contract-declared answer.
    Json(serde_json::json!({
        "error": "not_implemented",
        "detail": "sessions are not wired to an adapter yet; a session cannot be started, and no state is faked"
    }))
    .into_response_with_status()
}

trait WithStatus {
    fn into_response_with_status(self) -> Response;
}
impl WithStatus for Json<serde_json::Value> {
    fn into_response_with_status(self) -> Response {
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        (StatusCode::NOT_IMPLEMENTED, self).into_response()
    }
}
