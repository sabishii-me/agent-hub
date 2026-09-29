//! The `/v1/hub/plugins` routes, merged onto the frame's transport.
//!
//! A long operation (install, remove) is **detached**: the route answers
//! `202 Accepted` + `Location` at once and the work runs on a task
//! (ARCHITECTURE §13.2, ADR-0009). Errors are a contract **code**; the transport
//! maps the code to a status (`ErrorRenderer`), never this module.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use agent_hub_transport::{Accepted, DomainError, ErrorRenderer, Transport};

use crate::service::Plugins;

#[derive(Clone)]
pub struct PluginsState {
    pub plugins: Arc<Plugins>,
    pub transport: Transport,
    pub errors: ErrorRenderer,
}

impl PluginsState {
    pub fn new(plugins: Plugins, transport: Transport, errors: ErrorRenderer) -> Self {
        PluginsState { plugins: Arc::new(plugins), transport, errors }
    }
}

pub fn routes() -> Router<PluginsState> {
    Router::new()
        .route("/v1/hub/plugins", get(list).post(install))
        .route("/v1/hub/plugins/{id}", delete(remove))
        .route("/v1/hub/plugins/{id}/prepare", post(prepare))
}

/// The client's logical command identity (ARCHITECTURE §11, R1). A retry carries
/// the same `Idempotency-Key`, so the hub must recognize it and not run twice.
/// An absent key is a fresh intent.
fn command_id(headers: &HeaderMap) -> String {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(new_command_id)
}

fn new_command_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cmd-{n:x}-{:x}", C.fetch_add(1, Ordering::Relaxed))
}

async fn list(State(s): State<PluginsState>) -> Response {
    match s.plugins.list() {
        Ok(plugins) => Json(serde_json::json!({
            "plugins": plugins,
            "roots": {
                "given": std::env::var("AGENT_HUB_PLUGINS_DIR").ok(),
                "hub": s.plugins.root.to_string_lossy(),
                "searched": [s.plugins.root.to_string_lossy()],
            }
        }))
        .into_response_ok(),
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

/// A tiny helper so `Json(...)` reads as a response without importing
/// `IntoResponse` everywhere.
trait IntoResponseOk {
    fn into_response_ok(self) -> Response;
}
impl IntoResponseOk for Json<serde_json::Value> {
    fn into_response_ok(self) -> Response {
        use axum::response::IntoResponse;
        self.into_response()
    }
}

#[derive(Deserialize)]
struct InstallBody {
    source: InstallSource,
}

#[derive(Deserialize)]
struct InstallSource {
    url: String,
}

/// POST /v1/hub/plugins (long): `202 Accepted` + `Location`; the install runs
/// detached and its progress is the `hub.plugins.changed` stream.
async fn install(
    State(s): State<PluginsState>,
    headers: HeaderMap,
    Json(body): Json<InstallBody>,
) -> Response {
    let command_id = command_id(&headers);
    let source = std::path::PathBuf::from(&body.source.url);
    // Validate + register the command identity synchronously (cheap), then run
    // the move detached. A replay is identified before any work starts.
    match s.plugins.begin_install(&command_id, &source) {
        Ok(crate::service::InstallIntent::Replay(view)) => {
            Json(serde_json::json!({ "plugin": view, "updated": false })).into_response_ok()
        }
        Ok(crate::service::InstallIntent::Proceed { id }) => {
            let plugins = s.plugins.clone();
            let location = format!("/v1/hub/plugins/{id}");
            let errors = s.errors.clone();
            Accepted::detached(
                location,
                serde_json::json!({ "pluginId": id, "state": "installing" }),
                async move {
                    if let Err(e) = plugins.finish_install(&id, &source) {
                        tracing::error!(error = %e, "detached install failed");
                    }
                    let _ = errors; // the failure is reported on the event stream
                },
            )
        }
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

async fn remove(
    State(s): State<PluginsState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    let command_id = command_id(&headers);
    match s.plugins.begin_remove(&command_id, &id) {
        Ok(crate::service::RemoveIntent::Replay) => {
            Json(serde_json::json!({ "ok": true, "id": id })).into_response_ok()
        }
        Ok(crate::service::RemoveIntent::Proceed) => {
            let plugins = s.plugins.clone();
            let location = format!("/v1/hub/plugins/{id}");
            let id2 = id.clone();
            Accepted::detached(
                location,
                serde_json::json!({ "id": id, "state": "removing" }),
                async move {
                    if let Err(e) = plugins.finish_remove(&id2) {
                        tracing::error!(error = %e, "detached remove failed");
                    }
                },
            )
        }
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

async fn prepare(State(s): State<PluginsState>, AxumPath(id): AxumPath<String>) -> Response {
    if s.plugins.db.plugin(&id).ok().flatten().is_none() {
        return s.errors.render(&DomainError::new("not_found", format!("plugin `{id}`")));
    }
    Json(serde_json::json!({
        "harnessId": id,
        "runtimeReady": false,
        "ready": false,
        "detail": "no adapter is attached; runtime prepare is not available yet"
    }))
    .into_response_ok()
}
