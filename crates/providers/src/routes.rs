//! The `/v1/model-providers` routes.
//!
//! **NOT WIRED.** The provider data model (endpoint + protocol + declarations +
//! selection + catalog) is the decided target, but the storage here is still the
//! earlier `{id}.json` file store, not the decided data layer (TASK-048 C2). A
//! provider credential is also a secret with no store yet (F02). So every route
//! answers `501 not_implemented` through the shared renderer until the target
//! store is real - a success endpoint over the wrong model is not honest
//! unavailability.

use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;

use agent_hub_transport::{DomainError, ErrorRenderer};

use crate::service::Providers;

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

fn table() -> agent_hub_transport::RouteTable<ProvidersState> {
    let m = get(not_implemented).post(not_implemented);
    let one = get(not_implemented).patch(not_implemented).delete(not_implemented);
    agent_hub_transport::RouteTable::new()
        .mount("/v1/model-providers", &["GET", "POST"], m)
        .mount("/v1/model-providers/{id}", &["GET", "PATCH", "DELETE"], one)
        .mount("/v1/model-providers/{id}/logout", &["POST"], post(not_implemented))
        .mount("/v1/model-providers/{id}/models", &["GET", "PATCH"], get(not_implemented).patch(not_implemented))
        .mount("/v1/model-providers/{id}/models/refresh", &["POST"], post(not_implemented))
        .mount("/v1/models", &["GET"], get(not_implemented))
        .mount("/v1/model-providers/types", &["GET"], get(not_implemented))
        .mount("/v1/model-providers/{id}/auth", &["POST"], post(not_implemented))
        .mount("/v1/model-providers/{id}/auth/{op}", &["GET"], get(not_implemented))
        .mount("/v1/model-providers/{id}/auth/{op}/cancel", &["POST"], post(not_implemented))
}

pub fn routes() -> Router<ProvidersState> {
    table().router()
}

pub fn surface() -> Vec<String> {
    table().surface()
}

async fn not_implemented(State(s): State<ProvidersState>) -> Response {
    s.errors.render(&DomainError::new(
        "not_implemented",
        "the model-provider data layer is not the decided one yet, and a credential has no secret store; the hub does not serve the old JSON store",
    ))
}
