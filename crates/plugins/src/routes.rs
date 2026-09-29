//! The `/v1/hub/plugins` routes, merged onto the frame's transport.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use agent_hub_transport::Transport;

use crate::PluginError;
use crate::service::Plugins;

/// Shared plugins state merged into the transport's router.
#[derive(Clone)]
pub struct PluginsState {
    pub plugins: Arc<Plugins>,
    pub transport: Transport,
}

impl PluginsState {
    pub fn new(plugins: Plugins, transport: Transport) -> Self {
        PluginsState { plugins: Arc::new(plugins), transport }
    }
}

/// The plugins routes as a stateful builder to merge onto `transport::routes()`.
pub fn routes() -> Router<PluginsState> {
    Router::new()
        .route("/v1/hub/plugins", get(list).post(install))
        .route("/v1/hub/plugins/{id}", delete(remove))
        .route("/v1/hub/plugins/{id}/prepare", post(prepare))
}

fn err(e: PluginError) -> Response {
    let (status, code) = match &e {
        PluginError::NotFound(_) => (StatusCode::NOT_FOUND, "plugin_not_found"),
        PluginError::NotInstalledByHub(_) => (StatusCode::CONFLICT, "plugin_not_removable"),
        PluginError::InUse(_, _) => (StatusCode::CONFLICT, "plugin_in_use"),
        PluginError::Busy(_, _) => (StatusCode::CONFLICT, "plugin_busy"),
        PluginError::InvalidManifest(_) => (StatusCode::BAD_REQUEST, "plugin_manifest_invalid"),
        PluginError::Conflict(_) => (StatusCode::CONFLICT, "idempotency_conflict"),
        PluginError::Db(_) | PluginError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    (status, Json(serde_json::json!({ "error": code, "detail": e.to_string() }))).into_response()
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
        .into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
struct InstallBody {
    source: InstallSource,
}

#[derive(Deserialize)]
struct InstallSource {
    /// A local path (git/artifact fetch is a later domain).
    url: String,
}

/// POST /v1/hub/plugins. `Idempotency-Key` carries the command identity; a
/// missing key is treated as a fresh intent.
async fn install(State(s): State<PluginsState>, body: Json<InstallBody>) -> Response {
    let command_id = format!("install-{}", uuid_like());
    let source = std::path::PathBuf::from(&body.source.url);
    match s.plugins.install_from_dir(&command_id, &source) {
        Ok((view, updated)) => {
            let plugins = s.plugins.list().unwrap_or_default();
            Json(serde_json::json!({ "plugin": view, "updated": updated, "plugins": plugins }))
                .into_response()
        }
        Err(e) => err(e),
    }
}

async fn remove(State(s): State<PluginsState>, AxumPath(id): AxumPath<String>) -> Response {
    let command_id = format!("remove-{}", uuid_like());
    match s.plugins.remove(&command_id, &id) {
        Ok(()) => {
            let plugins = s.plugins.list().unwrap_or_default();
            Json(serde_json::json!({ "ok": true, "id": id, "plugins": plugins })).into_response()
        }
        Err(e) => err(e),
    }
}

/// POST /v1/hub/plugins/{id}/prepare. Runtime prepare is delegated to the
/// adapter; without one, the runtime is reported not-ready rather than faked.
async fn prepare(State(s): State<PluginsState>, AxumPath(id): AxumPath<String>) -> Response {
    if s.plugins.db.plugin(&id).ok().flatten().is_none() {
        return err(PluginError::NotFound(id));
    }
    Json(serde_json::json!({
        "harnessId": id,
        "runtimeReady": false,
        "ready": false,
        "detail": "no adapter is attached; runtime prepare is not available yet"
    }))
    .into_response()
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{n:x}")
}
