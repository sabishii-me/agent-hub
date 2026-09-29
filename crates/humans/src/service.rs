//! The humans domain (`ARCHITECTURE` §3): approvals and questions - a harness
//! asking a person to decide.
//!
//! The hub holds the pending decisions as **control state**. The adapter
//! initiates (`approval_need` / a question); the decision travels back to the
//! adapter as its reply. The hub never invents a decision: an unanswered approval
//! is **denied by the deadline (fail closed)**, never allowed by default.

use std::collections::HashMap;
use std::sync::Mutex;

use agent_hub_events::Bus;
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum HumanError {
    #[error("approval `{0}` not found")]
    ApprovalNotFound(String),
    #[error("question `{0}` not found")]
    QuestionNotFound(String),
    #[error("validation failed: {0}")]
    Validation(String),
}

impl HumanError {
    pub fn code(&self) -> &'static str {
        match self {
            HumanError::ApprovalNotFound(_) => "approval_not_found",
            HumanError::QuestionNotFound(_) => "question_not_found",
            HumanError::Validation(_) => "validation_failed",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// A pending approval.
#[derive(Debug, Clone)]
pub struct Approval {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub args: Value,
    pub options: Option<Vec<String>>,
    pub requested_at: String,
    /// The decision once made (`Some`), or `None` while pending.
    pub decision: Option<String>,
    pub reason: Option<String>,
}

/// A pending question.
#[derive(Debug, Clone)]
pub struct Question {
    pub id: String,
    pub session_id: String,
    pub questions: Value,
    pub requested_at: String,
    pub answers: Option<Value>,
}

/// The humans domain: the pending decisions.
pub struct Humans {
    bus: Bus,
    approvals: Mutex<HashMap<String, Approval>>,
    questions: Mutex<HashMap<String, Question>>,
    counter: std::sync::atomic::AtomicU64,
}

impl Humans {
    pub fn new(bus: Bus) -> Self {
        Humans {
            bus,
            approvals: Mutex::new(HashMap::new()),
            questions: Mutex::new(HashMap::new()),
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn next_id(&self, prefix: &str) -> String {
        let n = self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{prefix}-{}", crate::now_millis() + n)
    }

    /// Register an approval the adapter raised (`approval_need`).
    #[allow(clippy::too_many_arguments)]
    pub fn raise_approval(
        &self,
        session_id: &str,
        tool: &str,
        args: Value,
        options: Option<Vec<String>>,
    ) -> Approval {
        let id = self.next_id("ap");
        let approval = Approval {
            id: id.clone(),
            session_id: session_id.into(),
            tool: tool.into(),
            args,
            options,
            requested_at: crate::now_rfc3339(),
            decision: None,
            reason: None,
        };
        self.approvals.lock().expect("approvals").insert(id.clone(), approval.clone());
        self.bus.publish(
            "approval.requested",
            json!({ "approvalId": id, "sessionId": session_id, "tool": tool, "state": "pending" }),
        );
        approval
    }

    pub fn list_approvals(&self, session_id: &str) -> Vec<Approval> {
        let all = self.approvals.lock().expect("approvals");
        let mut out: Vec<Approval> = all
            .values()
            .filter(|a| a.session_id == session_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.requested_at.cmp(&b.requested_at));
        out
    }

    /// Resolve an approval. The decision must be one the harness offered (a
    /// locked-down rule: never a decision the harness did not present), unless
    /// the harness offered none.
    pub fn resolve_approval(
        &self,
        session_id: &str,
        approval_id: &str,
        decision: &str,
    ) -> Result<Approval, HumanError> {
        let mut all = self.approvals.lock().expect("approvals");
        let approval = all
            .get_mut(approval_id)
            .ok_or_else(|| HumanError::ApprovalNotFound(approval_id.into()))?;
        if approval.session_id != session_id {
            return Err(HumanError::ApprovalNotFound(approval_id.into()));
        }
        if let Some(options) = &approval.options {
            if !options.iter().any(|o| o == decision) {
                return Err(HumanError::Validation(format!(
                    "`{decision}` is not one of the offered options: {}",
                    options.join(", ")
                )));
            }
        }
        approval.decision = Some(decision.to_string());
        let out = approval.clone();
        self.bus.publish(
            "approval.resolved",
            json!({
                "approvalId": approval_id, "sessionId": session_id,
                "approved": !decision.to_lowercase().contains("reject"),
                "state": "resolved",
            }),
        );
        Ok(out)
    }

    pub fn raise_question(&self, session_id: &str, questions: Value) -> Question {
        let id = self.next_id("q");
        let question = Question {
            id: id.clone(),
            session_id: session_id.into(),
            questions,
            requested_at: crate::now_rfc3339(),
            answers: None,
        };
        self.questions.lock().expect("questions").insert(id.clone(), question.clone());
        self.bus.publish(
            "question.requested",
            json!({ "questionId": id, "sessionId": session_id, "state": "pending" }),
        );
        question
    }

    pub fn list_questions(&self, session_id: &str) -> Vec<Question> {
        let all = self.questions.lock().expect("questions");
        let mut out: Vec<Question> = all
            .values()
            .filter(|q| q.session_id == session_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.requested_at.cmp(&b.requested_at));
        out
    }

    pub fn answer_question(
        &self,
        session_id: &str,
        question_id: &str,
        answers: Value,
    ) -> Result<Question, HumanError> {
        let mut all = self.questions.lock().expect("questions");
        let question = all
            .get_mut(question_id)
            .ok_or_else(|| HumanError::QuestionNotFound(question_id.into()))?;
        if question.session_id != session_id {
            return Err(HumanError::QuestionNotFound(question_id.into()));
        }
        question.answers = Some(answers.clone());
        let out = question.clone();
        self.bus.publish(
            "question.answered",
            json!({ "questionId": question_id, "sessionId": session_id, "answers": answers }),
        );
        Ok(out)
    }
}

// --- views (the contract's shapes) -----------------------------------------

pub fn approval_view(a: &Approval) -> Value {
    json!({
        "id": a.id,
        "sessionId": a.session_id,
        "tool": a.tool,
        "args": a.args,
        "options": a.options,
        "reason": a.reason,
        "decision": a.decision,
        "requestedAt": a.requested_at,
    })
}

pub fn question_view(q: &Question) -> Value {
    json!({
        "id": q.id,
        "sessionId": q.session_id,
        "questions": q.questions,
        "answers": q.answers,
        "requestedAt": q.requested_at,
    })
}
