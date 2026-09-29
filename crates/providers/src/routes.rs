//! The `/v1/hub/providers` routes, merged onto the frame's transport.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};

use crate::record::ProviderRecord;
use crate::service::{CreateProvider, PatchProvider, ProviderError, Providers};
use crate::store::StoreError;

#[derive(Clone)]
pub struct ProvidersState {
    pub providers: Arc<Providers>,
}

impl ProvidersState {
    pub fn new(providers: Providers) -> Self {
        ProvidersState { providers: Arc::new(providers) }
    }
}

pub fn routes() -> Router<ProvidersState> {
    Router::new()
        .route("/v1/hub/providers", get(list).post(create))
        .route(
            "/v1/hub/providers/{id}",
            get(get_one).patch(patch_one).delete(remove),
        )
        .route("/v1/hub/providers/{id}/logout", post(logout))
        .route("/v1/hub/providers/{id}/models", get(models).patch(set_models))
        .route("/v1/hub/providers/{id}/models/refresh", post(refresh))
        .route("/v1/hub/providers/models/list", get(list_models))
}

fn err(e: ProviderError) -> Response {
    let (status, code) = match &e {
        ProviderError::Store(StoreError::NotFound(_)) => (StatusCode::NOT_FOUND, "provider_not_found"),
        ProviderError::Store(StoreError::Exists(_)) => (StatusCode::CONFLICT, "provider_exists"),
        ProviderError::Store(StoreError::Corrupt(_, _)) => (StatusCode::INTERNAL_SERVER_ERROR, "provider_corrupt"),
        ProviderError::Validation(_) => (StatusCode::BAD_REQUEST, "validation_failed"),
        ProviderError::Catalog(_) => (StatusCode::BAD_GATEWAY, "catalog_failed"),
        ProviderError::Store(StoreError::Io(_)) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    (status, Json(serde_json::json!({ "error": code, "detail": e.to_string() }))).into_response()
}

/// The provider object the routes return: **the token never leaves**.
fn view(rec: &ProviderRecord) -> serde_json::Value {
    let mut v = serde_json::to_value(rec).unwrap_or(serde_json::json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.remove("token");
        obj.insert("tokenConfigured".into(), serde_json::json!(rec.token.is_some()));
        obj.insert(
            "providerTypeAvailable".into(),
            serde_json::json!(rec.provider_type.is_some()),
        );
    }
    v
}

async fn list(State(s): State<ProvidersState>) -> Response {
    match s.providers.list() {
        Ok((records, broken)) => Json(serde_json::json!({
            "providers": records.iter().map(view).collect::<Vec<_>>(),
            "broken": broken,
        }))
        .into_response(),
        Err(e) => err(e),
    }
}

async fn create(State(s): State<ProvidersState>, Json(req): Json<CreateProvider>) -> Response {
    match s.providers.create(req) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec) })).into_response(),
        Err(e) => err(e),
    }
}

async fn get_one(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.get(&id) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec) })).into_response(),
        Err(e) => err(e),
    }
}

async fn patch_one(
    State(s): State<ProvidersState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<PatchProvider>,
) -> Response {
    match s.providers.patch(&id, req) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec) })).into_response(),
        Err(e) => err(e),
    }
}

async fn remove(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.delete(&id) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => err(e),
    }
}

async fn logout(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.logout(&id) {
        Ok(rec) => Json(serde_json::json!({ "provider": view(&rec) })).into_response(),
        Err(e) => err(e),
    }
}

async fn models(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.get(&id) {
        Ok(rec) => Json(models_body(&rec)).into_response(),
        Err(e) => err(e),
    }
}

fn models_body(rec: &ProviderRecord) -> serde_json::Value {
    serde_json::json!({
        "providerId": rec.id,
        "fetchedAt": rec.catalog.as_ref().and_then(|c| c.fetched_at.clone()),
        "stale": rec.catalog_stale(),
        "models": rec.merged_models(),
    })
}

#[derive(serde::Deserialize)]
struct SelectionBody {
    #[serde(rename = "enabledModelIds")]
    enabled_model_ids: Vec<String>,
}

async fn set_models(
    State(s): State<ProvidersState>,
    AxumPath(id): AxumPath<String>,
    Json(b): Json<SelectionBody>,
) -> Response {
    match s.providers.set_selection(&id, b.enabled_model_ids) {
        Ok(rec) => Json(models_body(&rec)).into_response(),
        Err(e) => err(e),
    }
}

async fn refresh(State(s): State<ProvidersState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.providers.refresh(&id).await {
        Ok(rec) => Json(models_body(&rec)).into_response(),
        Err(e) => err(e),
    }
}

/// API 1: the core-managed catalogs, independent of any harness.
async fn list_models(State(s): State<ProvidersState>) -> Response {
    let (records, _) = match s.providers.list() {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let mut models = Vec::new();
    let mut catalogs = Vec::new();
    for rec in &records {
        for m in rec.merged_models() {
            models.push(serde_json::json!({
                "id": m["id"],
                "name": m["name"],
                "providerId": rec.id,
                "provider": rec.label,
                "enabled": m["enabled"],
                "available": true,
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
