//! The `/v1/harnesses` routes (the one roster; enable/disable is plugin lifecycle).

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
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

fn table() -> agent_hub_transport::RouteTable<HarnessesState> {
    agent_hub_transport::RouteTable::new()
        .get("/v1/harnesses", list)
        .get("/v1/harnesses/{id}/presets", presets)
        .get("/v1/harnesses/{id}/models", models)
        .get("/v1/harnesses/{id}/tools", tools)
        .get("/v1/harnesses/{id}/extensions", extensions)
        .patch("/v1/harnesses/{id}/extensions", patch_extensions)
}

#[derive(serde::Deserialize)]
struct ExtensionsBody {
    #[serde(default)]
    extensions: Vec<String>,
}

/// PATCH /v1/harnesses/{id}/extensions - select the extensions the hub installs for
/// this harness. An unknown id is refused with the available list.
async fn patch_extensions(
    State(s): State<HarnessesState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ExtensionsBody>,
) -> Response {
    match s.harnesses.set_extensions(&id, &body.extensions) {
        Ok(v) => Json(v).into_response(),
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

pub fn routes() -> Router<HarnessesState> {
    table().router()
}

pub fn surface() -> Vec<String> {
    table().surface()
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
