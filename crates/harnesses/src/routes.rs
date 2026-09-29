//! The `/v1/harnesses` and `/v1/hub/harnesses` routes.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use agent_hub_adapter::HarnessStatus;
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
        .route("/v1/harnesses", get(list))
        .route("/v1/harnesses/{id}/enable", post(enable))
        .route("/v1/harnesses/{id}/disable", post(disable))
        .route("/v1/harnesses/{id}/presets", get(presets))
        .route("/v1/harnesses/{id}/models", get(models))
        .route("/v1/harnesses/{id}/tools", get(tools))
        .route("/v1/hub/harnesses", get(management))
        .route("/v1/hub/harnesses/{id}/enable", post(enable))
        .route("/v1/hub/harnesses/{id}/disable", post(disable))
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
            "note": "no harness plugins are present; put one under AGENT_HUB_PLUGINS_DIR or install it with POST /v1/hub/plugins"
        }))
        .into_response()
    } else {
        Json(serde_json::json!({ "harnesses": harnesses, "next_cursor": null })).into_response()
    }
}

async fn management(State(s): State<HarnessesState>) -> Response {
    Json(s.harnesses.management()).into_response()
}

async fn enable(State(s): State<HarnessesState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.harnesses.set_status(&id, HarnessStatus::Enabled) {
        Ok(_) => Json(serde_json::json!({ "ok": true, "id": id, "status": "enabled" })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn disable(State(s): State<HarnessesState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.harnesses.set_status(&id, HarnessStatus::Disabled) {
        Ok(_) => Json(serde_json::json!({ "ok": true, "id": id, "status": "disabled" })).into_response(),
        Err(e) => err(&s, e),
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
