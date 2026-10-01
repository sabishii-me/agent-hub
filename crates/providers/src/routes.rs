//! The `/v1/model-providers` routes.
//!
//! Wired: the credential path is real - a provider's `token` is stored in the OS
//! secret store (`agent-hub-secrets`), never in the record file; `logout` deletes
//! it; the catalog fetch authenticates with it. The catalog/model listing and the
//! provider-type/auth surfaces that need the fuller provider data layer stay
//! `501 not_implemented`.
//!
//! A provider is DATA (`ARCHITECTURE` §5); the hub owns the request, the auth, the
//! catalog fetch and the field mapping.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};

use agent_hub_transport::{DomainError, ErrorRenderer, RouteTable};

use crate::service::{CreateProvider, PatchProvider, ProviderError, Providers};

#[derive(Clone)]
pub struct ProvidersState {
    pub providers: Arc<Providers>,
    pub errors: ErrorRenderer,
    /// Where installed plugins live: a model-provider plugin ships its type
    /// descriptor here (see `crate::types`).
    pub plugins_root: std::path::PathBuf,
    /// Hub-level authorization operations (see `crate::auth`).
    pub auth: Arc<crate::auth::AuthStore>,
}

impl ProvidersState {
    pub fn new(providers: Providers, errors: ErrorRenderer, plugins_root: impl Into<std::path::PathBuf>) -> Self {
        ProvidersState {
            providers: Arc::new(providers),
            errors,
            plugins_root: plugins_root.into(),
            auth: Arc::new(crate::auth::AuthStore::new()),
        }
    }

    /// From an existing shared handle (the composition root shares one instance
    /// with the session provider resolver).
    pub fn new_shared(
        providers: Arc<Providers>,
        errors: ErrorRenderer,
        plugins_root: impl Into<std::path::PathBuf>,
    ) -> Self {
        ProvidersState {
            providers,
            errors,
            plugins_root: plugins_root.into(),
            auth: Arc::new(crate::auth::AuthStore::new()),
        }
    }
}

fn table() -> RouteTable<ProvidersState> {
    RouteTable::new()
        .get("/v1/model-providers", list)
        .post("/v1/model-providers", create)
        .get("/v1/model-providers/{id}", get_one)
        .patch("/v1/model-providers/{id}", patch_one)
        .delete("/v1/model-providers/{id}", remove)
        .post("/v1/model-providers/{id}/logout", logout)
        .get("/v1/model-providers/{id}/models", models)
        .patch("/v1/model-providers/{id}/models", patch_models)
        .post("/v1/model-providers/{id}/models/refresh", refresh)
        .get("/v1/models", list_models)
        .get("/v1/model-providers/types", list_types)
        .post("/v1/model-providers/{id}/auth", start_auth)
        .get("/v1/model-providers/{id}/auth/{op}", auth_status)
        .post("/v1/model-providers/{id}/auth/{op}/cancel", auth_cancel)
}

pub fn routes() -> Router<ProvidersState> {
    table().router()
}

pub fn surface() -> Vec<String> {
    table().surface()
}

fn err(s: &ProvidersState, e: ProviderError) -> Response {
    s.errors.render(&e.to_domain_error())
}

/// The provider object returned: **no credential, only whether one is stored**.
/// The provider object returned: **no credential, only whether one is stored**.
/// `type_available` comes from the ONE type authority, so availability is not
/// inferred from a field being present (TASK-048 G3).
fn view(rec: &crate::record::ProviderRecord, type_available: bool) -> serde_json::Value {
    let mut v = serde_json::to_value(rec).unwrap_or(serde_json::json!({}));
    if let Some(obj) = v.as_object_mut() {
        // The secret REFERENCE is an internal keychain id: never exposed.
        obj.remove("secret_ref");
        obj.remove("incarnation"); // internal: the version guard is not a wire field
        obj.insert("tokenConfigured".into(), serde_json::json!(rec.token_configured));
        obj.insert("providerTypeAvailable".into(), serde_json::json!(type_available));
    }
    v
}

/// The availability of a record's named type, from the type authority.
fn available(s: &ProvidersState, rec: &crate::record::ProviderRecord) -> bool {
    s.providers
        .type_available(rec.provider_type.as_deref(), rec.provider_type_version)
}

async fn list(State(s): State<ProvidersState>) -> Response {
    match s.providers.list() {
        Ok(records) => Json(serde_json::json!({
            "providers": records.iter().map(|r| view(r, available(&s, r))).collect::<Vec<_>>(),
            "broken": [],
        }))
        .into_response(),
        Err(e) => err(&s, e),
    }
}

async fn create(State(s): State<ProvidersState>, Json(req): Json<CreateProvider>) -> Response {
    match s.providers.create(req).await {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec, available(&s, &rec)) })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn get_one(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.get(&id) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec, available(&s, &rec)) })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn patch_one(
    State(s): State<ProvidersState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<PatchProvider>,
) -> Response {
    match s.providers.patch(&id, req).await {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec, available(&s, &rec)) })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn remove(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.delete(&id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn logout(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.logout(&id).await {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec, available(&s, &rec)) })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn models(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.get(&id) {
        Ok(rec) => Json(models_body(&rec)).into_response(),
        Err(e) => err(&s, e),
    }
}

fn models_body(rec: &crate::record::ProviderRecord) -> serde_json::Value {
    serde_json::json!({
        "providerId": rec.id,
        "fetchedAt": rec.catalog.as_ref().and_then(|c| c.fetched_at.clone()),
        "stale": rec.catalog_stale(),
        "models": rec.merged_models(),
    })
}

#[derive(serde::Deserialize)]
struct SelectionBody {
    #[serde(rename = "enabledModelIds", default)]
    enabled_model_ids: Vec<String>,
}

/// PATCH /v1/model-providers/{id}/models - replace the enabled selection. `[]`
/// disables all; an unknown id is refused. The selection lives on the provider, not
/// in the fetched catalog, so a refresh never changes it.
async fn patch_models(
    State(s): State<ProvidersState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<SelectionBody>,
) -> Response {
    match s.providers.set_selection(&id, body.enabled_model_ids).await {
        Ok(rec) => Json(models_body(&rec)).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn refresh(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.refresh(&id).await {
        Ok(rec) => Json(models_body(&rec)).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn list_models(State(s): State<ProvidersState>) -> Response {
    let records = match s.providers.list() {
        Ok(v) => v,
        Err(e) => return err(&s, e),
    };
    let mut models = Vec::new();
    let mut catalogs = Vec::new();
    for rec in &records {
        for m in rec.merged_models() {
            models.push(serde_json::json!({
                "id": m["id"], "name": m["name"], "providerId": rec.id,
                "provider": rec.label, "enabled": m["enabled"], "available": true,
            }));
        }
        catalogs.push(serde_json::json!({
            "providerId": rec.id,
            "fetchedAt": rec.catalog.as_ref().and_then(|c| c.fetched_at.clone()),
            "stale": rec.catalog_stale(),
            "models": rec.merged_models(),
        }));
    }
    Json(serde_json::json!({ "models": models, "failures": [], "catalogs": catalogs })).into_response()
}

/// `GET /v1/model-providers/types`: the provider types installed plugins ship, as
/// DATA. A type no plugin ships stays absent; a descriptor that cannot be honoured
/// is `broken[]` (never a 500). The hub reads the descriptor; it never runs plugin
/// code (ARCHITECTURE 5).
async fn list_types(State(s): State<ProvidersState>) -> Response {
    let catalog = crate::types::TypeCatalog::scan(&s.plugins_root);
    Json(catalog.to_json()).into_response()
}

/// `POST /v1/model-providers/{id}/auth`: start the authorization the provider TYPE
/// declares. A type that declares no INTERACTIVE method is refused `501` rather than
/// a fabricated flow (the step schema of a device-code/browser flow is not specified,
/// so the hub does not invent it). No plugin code runs in-process.
async fn start_auth(
    State(s): State<ProvidersState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let provider = match s.providers.get(&id) {
        Ok(p) => p,
        Err(e) => return s.errors.render(&e.to_domain_error()),
    };
    let kind = provider.provider_type.clone();
    let Some(kind) = kind else {
        return s.errors.render(&DomainError::new(
            "unsupported",
            "this provider names no type; there is no declared auth method to run",
        ));
    };
    let catalog = crate::types::TypeCatalog::scan(&s.plugins_root);
    let desc = catalog.types.into_iter().find(|t| t.id == kind);
    let Some(desc) = desc else {
        return s.errors.render(&DomainError::new(
            "unsupported",
            format!("no installed plugin ships the provider type `{kind}`"),
        ));
    };
    // NO SIDE EFFECT on a refusal: the interactive-method check runs FIRST and,
    // when there is no executor, the hub refuses WITHOUT creating an operation. A
    // fabricated pending operation (one no executor can ever resolve) would be a
    // placeholder disguised as a resource (TASK-048 G4). The operation is created
    // ONLY when a real execution is about to start.
    match crate::auth::AuthStore::interactive_method(&desc) {
        None => s.errors.render(&DomainError::new(
            "unsupported",
            format!("the provider type `{kind}` declares no interactive auth method"),
        )),
        Some(_method) => {
            // The step schema of a device-code/browser flow is not defined in the
            // owning contract yet; refusing here (with no operation created) is the
            // honest state until the flow is implemented. Do NOT create a pending op.
            s.errors.render(&DomainError::new(
                "unsupported",
                format!(
                    "the provider type `{kind}` declares an interactive auth method, but its step flow is not implemented yet"
                ),
            ))
        }
    }
}

/// `GET /v1/model-providers/{id}/auth/{op}`: the current state of a hub-level auth
/// operation.
async fn auth_status(
    State(s): State<ProvidersState>,
    AxumPath((id, op)): AxumPath<(String, String)>,
) -> Response {
    match s.auth.get(&op) {
        Some(operation) if operation.provider == id => {
            Json(operation.to_json()).into_response()
        }
        _ => s.errors.render(&DomainError::new(
            "not_found",
            format!("no authorization operation `{op}` for provider `{id}`"),
        )),
    }
}

/// `POST /v1/model-providers/{id}/auth/{op}/cancel`: cancel a pending operation.
/// Idempotent.
async fn auth_cancel(
    State(s): State<ProvidersState>,
    AxumPath((id, op)): AxumPath<(String, String)>,
) -> Response {
    match s.auth.get(&op) {
        Some(operation) if operation.provider == id => {
            let _ = s.auth.cancel(&op);
            Json(serde_json::json!({ "ok": true, "id": op })).into_response()
        }
        _ => s.errors.render(&DomainError::new(
            "not_found",
            format!("no authorization operation `{op}` for provider `{id}`"),
        )),
    }
}

