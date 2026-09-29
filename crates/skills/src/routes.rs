//! The `/v1/skills` routes.
//!
//! **NOT WIRED.** Skills are a top-level mechanism whose **content comes from a
//! plugin**, delivered the way extensions are (workspace/session layering), per
//! the decided direction. The earlier version was a hub-authored store with real
//! `PUT`/`DELETE` mutators - the excluded model (TASK-048 C2). Those mutators are
//! stopped: every route answers `501 not_implemented` until the plugin-source,
//! layered model is real. A success endpoint over the wrong model is not honest
//! unavailability.

use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use axum::routing::{delete, get};
use axum::Router;

use agent_hub_transport::{DomainError, ErrorRenderer};

use crate::service::Skills;

#[derive(Clone)]
pub struct SkillsState {
    pub skills: Arc<Skills>,
    pub errors: ErrorRenderer,
}

impl SkillsState {
    pub fn new(skills: Skills, errors: ErrorRenderer) -> Self {
        SkillsState { skills: Arc::new(skills), errors }
    }
}

pub fn surface() -> &'static [&'static str] {
    &["GET /v1/skills", "DELETE /v1/skills/{id}", "GET /v1/skills/{id}/files/{file...}", "PUT /v1/skills/{id}/files/{file...}"]
}

pub fn routes() -> Router<SkillsState> {
    Router::new()
        .route("/v1/skills", get(not_implemented))
        .route("/v1/skills/{id}", delete(not_implemented))
        .route(
            "/v1/skills/{id}/files/{*file}",
            get(not_implemented).put(not_implemented),
        )
}

async fn not_implemented(State(s): State<SkillsState>) -> Response {
    // Through the SAME error renderer as every domain (invariant 13.7, C5).
    s.errors.render(&DomainError::new(
        "not_implemented",
        "skills are a plugin-sourced, layered mechanism that is not wired yet; the hub does not serve a self-authored store",
    ))
}
