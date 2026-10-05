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

/// A source is EITHER a git reference (`url` + optional `ref`) OR a release
/// `artifact` (a zip + its digest). Exactly one is honoured; an artifact wins if
/// both are present (the contract names two distinct shapes).
#[derive(Deserialize)]
struct InstallSource {
    url: Option<String>,
    #[serde(rename = "ref")]
    reference: Option<String>,
    artifact: Option<crate::source::ArtifactSpec>,
}

impl InstallSource {
    fn into_source(self) -> Option<crate::source::Source> {
        use crate::source::{ArtifactSpec, Source};
        if let Some(a) = self.artifact {
            // A complete artifact needs every required field.
            let _ = &a;
            return Some(Source::Artifact(ArtifactSpec { ..a }));
        }
        self.url.map(|url| Source::Git { url, reference: self.reference })
    }
}

/// POST /v1/plugins (long): `202 Accepted` + `Location`; the install runs
/// detached and its progress is the `hub.plugins.changed` stream.
async fn install(
    State(s): State<PluginsState>,
    headers: HeaderMap,
    Json(body): Json<InstallBody>,
) -> Response {
    let command_id = command_id(&headers);
    let Some(source) = body.source.into_source() else {
        return s.errors.render(&agent_hub_transport::DomainError::new(
            "validation_failed",
            "the source must name a git url or an artifact",
        ));
    };
    // Resolving the source (clone / download+verify+unpack) is blocking network+fs
    // work, so it runs on the blocking pool; it also registers the command identity
    // before any landing starts. A replay is identified before any work.
    let plugins_begin = s.plugins.clone();
    let cid = command_id.clone();
    let begin = tokio::task::spawn_blocking(move || {
        plugins_begin.begin_install(&cid, &source)
    })
    .await;
    match begin {
        Ok(Ok(crate::service::InstallIntent::Replay(view))) => {
            Json(serde_json::json!({ "plugin": view, "updated": false })).into_response_ok()
        }
        Ok(Ok(crate::service::InstallIntent::Proceed { id, artifact_json })) => {
            let plugins = s.plugins.clone();
            let adapters = s.adapters.clone();
            let location = format!("/v1/plugins/{id}");
            // The landing does synchronous fs + SQLite work; it MUST run on the
            // blocking pool, not on the async runtime (TASK-048 F06).
            Accepted::detached(
                location,
                serde_json::json!({ "pluginId": id, "state": "installing" }),
                async move {
                    let id2 = id.clone();
                    let outcome = tokio::task::spawn_blocking(move || {
                        plugins.finish_install(&id2, artifact_json)
                    })
                    .await;
                    match outcome {
                        Ok(Ok(())) => {
                            // The plugin LANDED: make it USABLE now. The harness
                            // registry is otherwise built once at boot, so a plugin
                            // installed at runtime was invisible until a restart
                            // (docs/issues/20261005-050000). Re-scan so the harness
                            // (and its adapter) are registered immediately.
                            adapters.scan();
                        }
                        Ok(Err(e)) => tracing::error!(error = %e, "detached install failed"),
                        Err(e) => tracing::error!(error = %e, "install task panicked"),
                    }
                },
            )
        }
        Ok(Err(e)) => s.errors.render(&e.to_domain_error()),
        Err(e) => s.errors.render(&agent_hub_transport::DomainError::new(
            "internal_error",
            format!("the install task failed: {e}"),
        )),
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
            let adapters = s.adapters.clone();
            let location = format!("/v1/plugins/{id}");
            let id2 = id.clone();
            Accepted::detached(
                location,
                serde_json::json!({ "id": id, "state": "removing" }),
                async move {
                    // Stop the harness's cached adapter FIRST: a prepared/running
                    // adapter holds files under the plugin dir, so the delete fails
                    // with os error 32 (docs/issues/20261005-060000). Dropping the
                    // handle kills the child (kill_on_drop).
                    adapters.stop_harness(&id2);
                    // Give the OS a moment to release the files the child held.
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    let plugins2 = plugins.clone();
                    let id3 = id2.clone();
                    let outcome = tokio::task::spawn_blocking(move || plugins.finish_remove(&id3)).await;
                    match outcome {
                        Ok(Ok(())) => {
                            // The plugin is GONE: drop it from the live registry
                            // immediately (a removed harness must not stay usable
                            // until a restart).
                            adapters.scan();
                        }
                        Ok(Err(e)) => {
                            // A failed remove must NOT hang at `removing` forever.
                            tracing::error!(error = %e, "detached remove failed");
                            plugins2.fail_remove(&id2, &e.to_string());
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "remove task panicked");
                            plugins2.fail_remove(&id2, &e.to_string());
                        }
                    }
                },
            )
        }
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

/// POST /v1/plugins/{id}/prepare - ask the plugin's ADAPTER to materialise the
/// runtime its manifest pins (`runtime/prepare`), then VERIFY the declared command
/// now exists. The hub installs no harness and knows no package names: it asks,
/// waits, and verifies (contract `adapter-v1.json` capability `runtime`). A plugin
/// that brings its own runtime does not declare the capability; it is not an error
/// to ask, but there is nothing to do.
async fn prepare(State(s): State<PluginsState>, AxumPath(id): AxumPath<String>) -> Response {
    let server_known = s.plugins.db.plugin(&id).ok().flatten().is_some();
    let harness = match s.adapters.get(&id) {
        Ok(h) => h,
        Err(_) => {
            // Not a harness: a plugin the hub can see whose directory carries no
            // adapter manifest has nothing to prepare.
            if server_known {
                return Json(serde_json::json!({
                    "harnessId": id,
                    "runtimeReady": false,
                    "ready": false,
                    "detail": "this plugin declares no adapter; there is no runtime to prepare"
                }))
                .into_response_ok();
            }
            return s.errors.render(&DomainError::new("not_found", format!("plugin `{id}`")));
        }
    };
    if !harness.manifest.capabilities.iter().any(|c| c == "runtime") {
        // The plugin does not declare the runtime capability: it brings its own
        // runtime, so `runtime/prepare` is not part of its contract. That is a
        // valid, ready state - not a failure.
        return Json(serde_json::json!({
            "harnessId": id,
            "runtimeReady": false,
            "ready": true,
            "detail": "this plugin declares no runtime capability; it brings its own runtime"
        }))
        .into_response_ok();
    }
    match s.adapters.call(&id, "runtime", "runtime/prepare", serde_json::json!({})).await {
        Ok(v) => {
            let adapter_ready = v.get("ready").and_then(serde_json::Value::as_bool).unwrap_or(false);
            let detail = v
                .get("detail")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            // The hub VERIFIES the declared command now exists: the adapter's own
            // `ready` is not taken on faith, because "which version runs" is only
            // true if the manifest's command is on disk.
            let runtime_ready = runtime_target_present(&harness);
            Json(serde_json::json!({
                "harnessId": id,
                "runtimeReady": runtime_ready,
                "ready": adapter_ready && runtime_ready,
                "package": v.get("package").cloned().unwrap_or(serde_json::Value::Null),
                "version": v.get("version").cloned().unwrap_or(serde_json::Value::Null),
                "target": v.get("target").cloned().unwrap_or(serde_json::Value::Null),
                "detail": if detail.is_empty() {
                    if runtime_ready { "runtime present".to_string() } else { "the declared runtime command is not on disk".to_string() }
                } else { detail }
            }))
            .into_response_ok()
        }
        Err(agent_hub_adapter::AdapterError::Unsupported(_)) => s.errors.render(&DomainError::new(
            "unsupported",
            format!("the `{id}` adapter does not implement `runtime/prepare`"),
        )),
        Err(e) => s.errors.render(&DomainError::new(
            "plugin_install_failed",
            format!("runtime prepare failed: {e}"),
        )),
    }
}

/// Whether the runtime command the manifest declares is on disk. The command is
/// relative to the plugin directory (`["node","runtime/dist/cli.js"]`); a plugin
/// whose runtime command is just `node` (brings its own) is trivially present.
fn runtime_target_present(harness: &agent_hub_adapter::Harness) -> bool {
    let Some(cmd) = harness.manifest.command.as_ref() else {
        return false;
    };
    let Some(program) = cmd.first() else {
        return false;
    };
    // The runtime argv is the FIRST non-flag argument after the interpreter, when
    // the manifest declares one (e.g. `node runtime/dist/cli.js`).
    let rel = cmd.iter().skip(1).find(|a| !a.starts_with('-'));
    match rel {
        Some(rel) => harness.directory.join(rel).exists(),
        None => std::path::Path::new(program).exists(),
    }
}
