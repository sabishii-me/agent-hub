//! The single place an error becomes an HTTP response (ARCHITECTURE §13.7).
//!
//! A domain returns a [`DomainError`] - a contract error **code** plus a detail
//! string - and never an HTTP status. The transport looks the code up in
//! `contract/errors.json` and writes the status and body. One mapping, driven by
//! the contract, so a code is only ever defined in one place.

use agent_hub_contract::ErrorTable;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// A domain failure: a contract error code plus a human detail.
///
/// The `code` MUST be one declared in `contract/errors.json`; the transport
/// resolves its status. An undeclared code maps to 500 and is a bug.
#[derive(Debug, Clone)]
pub struct DomainError {
    pub code: String,
    pub detail: String,
}

impl DomainError {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        DomainError { code: code.into(), detail: detail.into() }
    }

    /// A code with no extra detail (the contract carries the message).
    pub fn code(code: impl Into<String>) -> Self {
        DomainError { code: code.into(), detail: String::new() }
    }
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.detail.is_empty() {
            write!(f, "{}", self.code)
        } else {
            write!(f, "{}: {}", self.code, self.detail)
        }
    }
}

impl std::error::Error for DomainError {}

/// A response body carries the code and, when present, the detail.
pub fn render(err: &DomainError, table: &ErrorTable) -> Response {
    let status = StatusCode::from_u16(table.http_for(&err.code)).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut body = json!({ "error": err.code });
    if !err.detail.is_empty() {
        body["detail"] = json!(err.detail);
    }
    (status, Json(body)).into_response()
}

/// The transport hands every domain handler this renderer, so no domain imports
/// an HTTP status type.
#[derive(Clone)]
pub struct ErrorRenderer {
    table: std::sync::Arc<ErrorTable>,
}

impl ErrorRenderer {
    pub fn new(table: ErrorTable) -> Self {
        ErrorRenderer { table: std::sync::Arc::new(table) }
    }

    pub fn render(&self, err: &DomainError) -> Response {
        render(err, &self.table)
    }

    /// The status a code resolves to (for tests and the boot self-check).
    pub fn status_for(&self, code: &str) -> u16 {
        self.table.http_for(code)
    }

    pub fn table(&self) -> &ErrorTable {
        &self.table
    }
}
