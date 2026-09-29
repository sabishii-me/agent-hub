//! The `/v1/harnesses` routes (the one roster; enable/disable is plugin lifecycle).

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};

use agent_hub_transport::ErrorRenderer;

use crate::service::{HarnessError, Harnesses};

#[derive(Clone)]
pub struct HarnessesState {
    pub harnesses: Arc<Harnesses>,
    pub errors: ErrorRenderer,
}

impl HarnessesState {
    pub fn new(harnesses: Harnesses, errors: ErrorRenderer) -> Self {
        HarnessesState { harnesses: Arc::new(harnesses), errors }
    }
}

pub fn routes() -> Router<HarnessesState> {
    Router::new()
        // One roster: the thin projection (which is also the management view).
        .route("/v1/harnesses", get(list))
        .route("/v1/harnesses/{id}/presets", get(presets))
        .route("/v1/harnesses/{id}/models", get(models))
        .route("/v1/harnesses/{id}/tools", get(tools))
        .route("/v1/harnesses/{id}/extensions", get(extensions))
}

fn err(s: &HarnessesState, e: HarnessError) -> Response {
    s.errors.render(&e.to_domain_error())
}

async fn list(State(s): State<HarnessesState>) -> Response {
    let harnesses = s.harnesses.list();
    if harnesses.is_empty() {
        Json(serde_json::json!({
            "harnesses": [],
            "next_cursor": null,
            "note": "no harness plugins are present; put one under AGENT_HUB_PLUGINS_DIR or install it with POST /v1/plugins"
        }))
        .into_response()
    } else {
        Json(serde_json::json!({ "harnesses": harnesses, "next_cursor": null })).into_response()
    }
}




async fn presets(State(s): State<HarnessesState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.harnesses.presets(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn models(State(s): State<HarnessesState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.harnesses.models(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn tools(State(s): State<HarnessesState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.harnesses.tools(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(&s, e),
    }
}

/// GET /v1/harnesses/{id}/extensions - the extension ids this harness may be
/// given (the plugin's shipped set), harness-scoped because a bare id is
/// meaningless across harnesses.
async fn extensions(State(s): State<HarnessesState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.harnesses.extensions(&id) {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(&s, e),
    }
}
