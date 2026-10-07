//! The `/v1/skills` routes.
//!
//! A skill is a DIRECTORY the hub holds under `<DATA_DIR>/skills` (contract:
//! `GET /v1/skills`). The hub is a COURIER: it stores bytes and never interprets
//! them (`PUT .../files`). Placement is the hub's and is NOT an authorization
//! boundary (ARCHITECTURE §7).
//!
//! The earlier hub-authored store with its own install state is the excluded model
//! (TASK-048 C2). This file serves the contract's directory model and nothing more:
//! list / delete / read a file / write a file. A path that escapes the skills dir is
//! REFUSED, not sanitised (contract).

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Deserialize;

use agent_hub_transport::ErrorRenderer;

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

fn table() -> agent_hub_transport::RouteTable<SkillsState> {
    agent_hub_transport::RouteTable::new()
        .get("/v1/skills", list)
        .delete("/v1/skills/{id}", delete)
        // axum spells a catch-all `{*file}`; the contract spells it `{file...}`.
        .get_as("/v1/skills/{id}/files/{*file}", "/v1/skills/{id}/files/{file...}", read_file)
        .put_as("/v1/skills/{id}/files/{*file}", "/v1/skills/{id}/files/{file...}", write_file)
}

pub fn routes() -> Router<SkillsState> {
    table().router()
}

pub fn surface() -> Vec<String> {
    table().surface()
}

/// `GET /v1/skills` - one entry per skill directory.
async fn list(State(s): State<SkillsState>) -> Response {
    match s.skills.list() {
        Ok(skills) => Json(serde_json::json!({ "skills": skills })).into_response(),
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

/// `DELETE /v1/skills/{id}` - remove a skill directory, whole.
async fn delete(State(s): State<SkillsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.skills.delete(&id) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

/// `GET /v1/skills/{id}/files/{file...}` - read one file, byte for byte.
async fn read_file(
    State(s): State<SkillsState>,
    AxumPath((id, file)): AxumPath<(String, String)>,
) -> Response {
    match s.skills.read_file(&id, &file) {
        Ok(content) => Json(serde_json::json!({ "skillId": id, "path": file, "content": content }))
            .into_response(),
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}

#[derive(Deserialize)]
struct WriteBody {
    content: String,
}

/// `PUT /v1/skills/{id}/files/{file...}` - write one file, creating directories.
async fn write_file(
    State(s): State<SkillsState>,
    AxumPath((id, file)): AxumPath<(String, String)>,
    Json(body): Json<WriteBody>,
) -> Response {
    match s.skills.write_file(&id, &file, &body.content) {
        Ok(bytes) => Json(serde_json::json!({ "skillId": id, "path": file, "bytes": bytes }))
            .into_response(),
        Err(e) => s.errors.render(&e.to_domain_error()),
    }
}
