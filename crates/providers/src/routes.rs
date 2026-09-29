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
}

impl ProvidersState {
    pub fn new(providers: Providers, errors: ErrorRenderer) -> Self {
        ProvidersState { providers: Arc::new(providers), errors }
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
        .patch("/v1/model-providers/{id}/models", not_implemented)
        .post("/v1/model-providers/{id}/models/refresh", refresh)
        .get("/v1/models", list_models)
        .get("/v1/model-providers/types", not_implemented)
        .post("/v1/model-providers/{id}/auth", not_implemented)
        .get("/v1/model-providers/{id}/auth/{op}", not_implemented)
        .post("/v1/model-providers/{id}/auth/{op}/cancel", not_implemented)
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
fn view(rec: &crate::record::ProviderRecord) -> serde_json::Value {
    let mut v = serde_json::to_value(rec).unwrap_or(serde_json::json!({}));
    if let Some(obj) = v.as_object_mut() {
        // The secret REFERENCE is an internal keychain id: never exposed.
        obj.remove("secret_ref");
        obj.insert("tokenConfigured".into(), serde_json::json!(rec.token_configured));
        obj.insert(
            "providerTypeAvailable".into(),
            serde_json::json!(rec.provider_type.is_some()),
        );
    }
    v
}

async fn list(State(s): State<ProvidersState>) -> Response {
    match s.providers.list() {
        Ok(records) => Json(serde_json::json!({
            "providers": records.iter().map(view).collect::<Vec<_>>(),
            "broken": [],
        }))
        .into_response(),
        Err(e) => err(&s, e),
    }
}

async fn create(State(s): State<ProvidersState>, Json(req): Json<CreateProvider>) -> Response {
    match s.providers.create(req) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec) })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn get_one(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.get(&id) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec) })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn patch_one(
    State(s): State<ProvidersState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<PatchProvider>,
) -> Response {
    match s.providers.patch(&id, req) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec) })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn remove(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.delete(&id) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn logout(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.logout(&id) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec) })).into_response(),
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

async fn not_implemented(State(s): State<ProvidersState>) -> Response {
    s.errors.render(&DomainError::new(
        "not_implemented",
        "this model-provider route needs the fuller provider data layer, which is not wired yet",
    ))
}
