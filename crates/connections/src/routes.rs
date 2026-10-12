//! The `/v1/connections` routes: the hub-managed connections. A connection is
//! enable/disable/delete only; `disabled` means cut-off, which means zero
//! materialization (the credential is not handed to any session).

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};

use agent_hub_transport::{ErrorRenderer, RouteTable};

use crate::service::{ConnectionError, Connections, CreateConnection, PatchConnection};

#[derive(Clone)]
pub struct ConnectionsState {
    pub connections: Arc<Connections>,
    pub errors: ErrorRenderer,
}

impl ConnectionsState {
    pub fn new(connections: Connections, errors: ErrorRenderer) -> Self {
        ConnectionsState { connections: Arc::new(connections), errors }
    }
    pub fn new_shared(connections: Arc<Connections>, errors: ErrorRenderer) -> Self {
        ConnectionsState { connections, errors }
    }
}

fn table() -> RouteTable<ConnectionsState> {
    RouteTable::new()
        .get("/v1/connections", list)
        .post("/v1/connections", create)
        .patch("/v1/connections/{id}", patch_one)
        .delete("/v1/connections/{id}", remove)
}

pub fn routes() -> Router<ConnectionsState> {
    table().router()
}

pub fn surface() -> Vec<String> {
    table().surface()
}

fn err(s: &ConnectionsState, e: ConnectionError) -> Response {
    s.errors.render(&e.to_domain_error())
}

async fn list(State(s): State<ConnectionsState>) -> Response {
    match s.connections.list() {
        Ok(conns) => Json(serde_json::json!({ "connections": conns })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn create(State(s): State<ConnectionsState>, Json(req): Json<CreateConnection>) -> Response {
    match s.connections.create(req).await {
        Ok(c) => Json(serde_json::json!({ "connection": c })).into_response(),
        Err(e) => err(&s, e),
    }
}


async fn patch_one(
    State(s): State<ConnectionsState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<PatchConnection>,
) -> Response {
    match s.connections.patch(&id, req).await {
        Ok(c) => Json(serde_json::json!({ "connection": c })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn remove(State(s): State<ConnectionsState>, AxumPath(id): AxumPath<String>) -> Response {
    match s.connections.delete(&id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => err(&s, e),
    }
}
