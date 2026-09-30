//! The `/v1/harnesses` routes (the one roster; enable/disable is plugin lifecycle).

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::Value;

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
        // Harness-private connections/auth are forwarded VERBATIM to the adapter;
        // the hub owns the route, the adapter owns the harness mechanism (ARCH 6).
        .get("/v1/harnesses/{id}/connections/schema", conn_schema)
        .get("/v1/harnesses/{id}/connections", conn_list)
        .post("/v1/harnesses/{id}/connections", conn_create)
        .post("/v1/harnesses/{id}/connections/validate", conn_validate)
        .delete("/v1/harnesses/{id}/connections/{cid}", conn_delete)
        .post("/v1/harnesses/{id}/auth", auth_start)
        .get("/v1/harnesses/{id}/auth/{op}", auth_status)
        .post("/v1/harnesses/{id}/auth/{op}/cancel", auth_cancel)
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

/// Forward a harness-private request to the adapter's `connections/*`/`auth/*`
/// (the adapter implements the harness's own mechanism). The hub adds no behaviour.
async fn conn_schema(State(s): State<HarnessesState>, AxumPath(id): AxumPath<String>) -> Response {
    forward(&s, &id, "providers", "connections/schema", serde_json::json!({})).await
}

async fn conn_list(State(s): State<HarnessesState>, AxumPath(id): AxumPath<String>) -> Response {
    forward(&s, &id, "providers", "connections/list", serde_json::json!({})).await
}

async fn conn_create(
    State(s): State<HarnessesState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    forward(&s, &id, "providers", "connections/save", body).await
}

async fn conn_validate(
    State(s): State<HarnessesState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    forward(&s, &id, "providers", "connections/validate", body).await
}

async fn conn_delete(
    State(s): State<HarnessesState>,
    AxumPath((id, cid)): AxumPath<(String, String)>,
) -> Response {
    forward(&s, &id, "providers", "connections/delete", serde_json::json!({ "connectionId": cid })).await
}

async fn auth_start(
    State(s): State<HarnessesState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<Value>,
) -> Response {
    forward(&s, &id, "providers", "auth/start", body).await
}

async fn auth_status(
    State(s): State<HarnessesState>,
    AxumPath((id, op)): AxumPath<(String, String)>,
) -> Response {
    forward(&s, &id, "providers", "auth/status", serde_json::json!({ "operationId": op })).await
}

async fn auth_cancel(
    State(s): State<HarnessesState>,
    AxumPath((id, op)): AxumPath<(String, String)>,
) -> Response {
    forward(&s, &id, "providers", "auth/cancel", serde_json::json!({ "operationId": op })).await
}

/// The shared forwarding body: call the adapter, render its answer or its typed
/// error (an `unsupported` is the honest "this harness has no such mechanism").
async fn forward(
    s: &HarnessesState,
    id: &str,
    capability: &str,
    method: &str,
    params: Value,
) -> Response {
    match s.harnesses.forward(id, capability, method, params).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}
