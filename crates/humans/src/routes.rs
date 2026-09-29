//! The `/v1/sessions/{id}/approvals` and `/questions` routes.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use agent_hub_transport::ErrorRenderer;

use crate::service::{approval_view, question_view, HumanError, Humans};

#[derive(Clone)]
pub struct HumansState {
    pub humans: Arc<Humans>,
    pub errors: ErrorRenderer,
}

impl HumansState {
    pub fn new(humans: Arc<Humans>, errors: ErrorRenderer) -> Self {
        HumansState { humans, errors }
    }

    /// Convenience for a caller that owns the domain outright.
    pub fn shared(humans: Humans, errors: ErrorRenderer) -> Self {
        HumansState { humans: Arc::new(humans), errors }
    }
}

pub fn surface() -> &'static [&'static str] {
    &["GET /v1/sessions/{id}/approvals", "POST /v1/sessions/{id}/approvals/{aid}", "GET /v1/sessions/{id}/questions", "POST /v1/sessions/{id}/questions/{qid}"]
}

pub fn routes() -> Router<HumansState> {
    Router::new()
        .route("/v1/sessions/{id}/approvals", get(list_approvals))
        .route("/v1/sessions/{id}/approvals/{aid}", post(decide))
        .route("/v1/sessions/{id}/questions", get(list_questions))
        .route("/v1/sessions/{id}/questions/{qid}", post(answer))
}

fn err(s: &HumansState, e: HumanError) -> Response {
    s.errors.render(&e.to_domain_error())
}

async fn list_approvals(State(s): State<HumansState>, AxumPath(id): AxumPath<String>) -> Response {
    let approvals: Vec<serde_json::Value> =
        s.humans.list_approvals(&id).iter().map(approval_view).collect();
    Json(serde_json::json!({ "approvals": approvals })).into_response()
}

#[derive(Deserialize)]
struct DecideBody {
    decision: String,
}

async fn decide(
    State(s): State<HumansState>,
    AxumPath((id, aid)): AxumPath<(String, String)>,
    Json(body): Json<DecideBody>,
) -> Response {
    match s.humans.resolve_approval(&id, &aid, &body.decision) {
        Ok(a) => Json(serde_json::json!({ "approval": approval_view(&a) })).into_response(),
        Err(e) => err(&s, e),
    }
}

async fn list_questions(State(s): State<HumansState>, AxumPath(id): AxumPath<String>) -> Response {
    let questions: Vec<serde_json::Value> =
        s.humans.list_questions(&id).iter().map(question_view).collect();
    Json(serde_json::json!({ "questions": questions })).into_response()
}

#[derive(Deserialize)]
struct AnswerBody {
    answers: serde_json::Value,
}

async fn answer(
    State(s): State<HumansState>,
    AxumPath((id, qid)): AxumPath<(String, String)>,
    Json(body): Json<AnswerBody>,
) -> Response {
    match s.humans.answer_question(&id, &qid, body.answers) {
        Ok(q) => Json(serde_json::json!({ "question": question_view(&q) })).into_response(),
        Err(e) => err(&s, e),
    }
}
