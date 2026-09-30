//! The `/v1/plugins` routes, merged onto the frame's transport.
//!
//! A long operation (install, remove) is **detached**: the route answers
//! `202 Accepted` + `Location` at once and the work runs on a task
//! (ARCHITECTURE §13.2, ADR-0009). Errors are a contract **code**; the transport
//! maps the code to a status (`ErrorRenderer`), never this module.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Deserialize;

use agent_hub_transport::{Accepted, DomainError, ErrorRenderer, Transport};

use crate::service::Plugins;

#[derive(Clone)]
pub struct PluginsState {
    pub plugins: Arc<Plugins>,
    pub transport: Transport,
    pub errors: ErrorRenderer,
    /// The adapter registry: enable/disable acts on a harness (a plugin is a
    /// harness too), so the lifecycle routes reach it here.
    pub adapters: Arc<agent_hub_adapter::Adapters>,
}

impl PluginsState {
    pub fn new(
        plugins: Plugins,
        transport: Transport,
        errors: ErrorRenderer,
        adapters: Arc<agent_hub_adapter::Adapters>,
    ) -> Self {
        PluginsState { plugins: Arc::new(plugins), transport, errors, adapters }
    }
}

/// Build the router AND its surface from one declaration (TASK-048 C5): the
/// `RouteTable` records each `"METHOD /path"` as it mounts, so the boot
/// self-check's list cannot drift from the actual routes.
fn table() -> agent_hub_transport::RouteTable<PluginsState> {
    agent_hub_transport::RouteTable::new()
        .get("/v1/plugins", list)
        .post("/v1/plugins", install)
        .get("/v1/plugins/{id}", get_one)
        .delete("/v1/plugins/{id}", remove)
        .post("/v1/plugins/{id}/prepare", prepare)
        .post("/v1/plugins/{id}/enable", enable)
        .post("/v1/plugins/{id}/disable", disable)
        .get("/v1/plugins/catalog", catalog)
        .post("/v1/plugins/registry/refresh", refresh_registry)
        .get("/v1/plugins/{id}/icon/{variant}", icon)
}

/// POST /v1/plugins/{id}/enable - enable a plugin's lifecycle. A disabled harness
/// refuses session create/turns; a rescan does not silently re-enable it (the status
/// is durable).
async fn enable(State(s): State<PluginsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s
        .adapters
        .set_status(&id, agent_hub_adapter::HarnessStatus::Enabled)
    {
        Ok(_) => Json(serde_json::json!({ "ok": true, "id": id, "status": "enabled" })).into_response(),
        Err(_) => {
            // Not a harness, or unknown: a plugin that is not a harness has no
            // lifecycle of its own here.
            s.errors.render(&DomainError::new("not_found", format!("no plugin `{id}`")))
        }
    }
}

/// POST /v1/plugins/{id}/disable - disable a plugin's lifecycle (a disabled
/// harness refuses session create/turns).
async fn disable(State(s): State<PluginsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s
        .adapters
        .set_status(&id, agent_hub_adapter::HarnessStatus::Disabled)
    {
        Ok(_) => Json(serde_json::json!({ "ok": true, "id": id, "status": "disabled" })).into_response(),
        Err(_) => s
            .errors
            .render(&DomainError::new("not_found", format!("no plugin `{id}`"))),
    }
}

/// GET /v1/plugins/catalog - the plugin catalog the hub was shipped with: the
/// registry file restated verbatim (the hub does not resolve, rank or rewrite it).
/// A missing or invalid file is a `fault`, never a 500.
async fn catalog(State(s): State<PluginsState>) -> Response {
    Json(s.plugins.catalog()).into_response()
}

/// POST /v1/plugins/registry/refresh - read AGENT_HUB_REGISTRY_URL and write it
/// where the hub reads the catalog. The URL is contacted ONLY here.
async fn refresh_registry(State(s): State<PluginsState>) -> Response {
    match s.plugins.refresh_registry().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

/// GET /v1/plugins/{id}/icon/{variant} - serve one variant (light|dark) of a
/// plugin's own icon, from the file its manifest declares. 404 when the plugin or
/// the variant is absent. The hub never inlines the bytes.
async fn icon(
    State(s): State<PluginsState>,
    AxumPath((id, variant)): AxumPath<(String, String)>,
) -> Response {
    if variant != "light" && variant != "dark" {
        return s.errors.render(&DomainError::new("not_found", "variant must be light|dark"));
    }
    let Ok(h) = s.adapters.get(&id) else {
        return s.errors.render(&DomainError::new("not_found", format!("no plugin `{id}`")));
    };
    let Some(rel) = h.manifest.icons.as_ref().and_then(|m| m.get(&variant)) else {
        return s.errors.render(&DomainError::new("not_found", format!("`{id}` has no {variant} icon")));
    };
    // The icon path is relative to the plugin directory; refuse a traversal.
    let path = h.directory.join(rel);
    if !path.starts_with(&h.directory) {
        return s.errors.render(&DomainError::new("not_found", "icon path escapes the plugin"));
    }
    match std::fs::read(&path) {
        Ok(bytes) => {
            let ct = match path.extension().and_then(|e| e.to_str()) {
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                _ => "application/octet-stream",
            };
            (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, ct)],
                bytes,
            )
                .into_response()
        }
        Err(_) => s.errors.render(&DomainError::new("not_found", format!("no icon file for `{id}` ({variant})"))),
    }
}

pub fn routes() -> Router<PluginsState> {
    table().router()
}

/// The mounted surface, from the same table the router is built from.
pub fn surface() -> Vec<String> {
    table().surface()
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

/// POST /v1/plugins (long): `202 Accepted` + `Location`; the install runs
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
            let location = format!("/v1/plugins/{id}");
            // The install does synchronous fs + SQLite work; it MUST run on the
            // blocking pool, not on the async runtime (TASK-048 F06).
            Accepted::detached(
                location,
                serde_json::json!({ "pluginId": id, "state": "installing" }),
                async move {
                    let id2 = id.clone();
                    let outcome = tokio::task::spawn_blocking(move || {
                        plugins.finish_install(&id2, &source)
                    })
                    .await;
                    match outcome {
                        Ok(Err(e)) => tracing::error!(error = %e, "detached install failed"),
                        Err(e) => tracing::error!(error = %e, "install task panicked"),
                        Ok(Ok(())) => {}
                    }
                },
            )
        }
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

async fn get_one(State(s): State<PluginsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.plugins.get(&id) {
        Ok(view) => Json(serde_json::json!({ "plugin": view })).into_response_ok(),
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
            let location = format!("/v1/plugins/{id}");
            let id2 = id.clone();
            Accepted::detached(
                location,
                serde_json::json!({ "id": id, "state": "removing" }),
                async move {
                    let outcome = tokio::task::spawn_blocking(move || plugins.finish_remove(&id2)).await;
                    match outcome {
                        Ok(Err(e)) => tracing::error!(error = %e, "detached remove failed"),
                        Err(e) => tracing::error!(error = %e, "remove task panicked"),
                        Ok(Ok(())) => {}
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
