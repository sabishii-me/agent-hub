//! The `/v1/hub/skills` routes.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::Deserialize;

use agent_hub_transport::ErrorRenderer;

use crate::service::{SkillError, Skills};

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

pub fn routes() -> Router<SkillsState> {
    Router::new()
        .route("/v1/hub/skills", get(list))
        .route("/v1/hub/skills/{id}", delete(remove))
        .route("/v1/hub/skills/{id}/files/{*file}", get(read_file).put(write_file))
}

fn err(s: &SkillsState, e: SkillError) -> Response {
    s.errors.render(&e.to_domain_error())
}

async fn list(State(s): State<SkillsState>) -> Response {
    match s.skills.list() {
        Ok(skills) => Json(serde_json::json!({ "skills": skills })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn remove(State(s): State<SkillsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.skills.delete(&id) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn read_file(
    State(s): State<SkillsState>,
    AxumPath((id, file)): AxumPath<(String, String)>,
) -> Response {
    match s.skills.read_file(&id, &file) {
        Ok(content) => Json(serde_json::json!({ "skillId": id, "path": file, "content": content }))
            .into_response(),
        Err(e) => err(&s, e),
    }
}

#[derive(Deserialize)]
struct WriteBody {
    content: String,
}

async fn write_file(
    State(s): State<SkillsState>,
    AxumPath((id, file)): AxumPath<(String, String)>,
    Json(body): Json<WriteBody>,
) -> Response {
    match s.skills.write_file(&id, &file, &body.content) {
        Ok(bytes) => {
            Json(serde_json::json!({ "skillId": id, "path": file, "bytes": bytes })).into_response()
        }
        Err(e) => err(&s, e),
    }
}
