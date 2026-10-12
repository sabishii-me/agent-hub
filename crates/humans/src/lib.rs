//! The humans domain: approvals and questions - a harness asking a person to
//! decide. The hub holds the pending decisions; the adapter initiates and
//! receives the reply. An unanswered approval is denied by its deadline (fail
//! closed), never allowed by default.

pub mod routes;
pub mod service;

pub use service::{approval_view, question_view, HumanError, Humans};

/// Shared with the data layer (one implementation).
pub use agent_hub_db::now_utc;
