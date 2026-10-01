//! The error master table (`contract/errors.json`): the single source of truth
//! for every route failure (ARCHITECTURE §13.7).
//!
//! A domain returns a **code**, never an HTTP status. The transport looks the
//! code up here and writes the status, so the mapping lives in one place and the
//! contract is what a consumer reads.

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEntry {
    pub http: u16,
    pub retryable: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct RawTable {
    errors: BTreeMap<String, ErrorEntry>,
}

/// The loaded error table.
#[derive(Debug, Clone)]
pub struct ErrorTable {
    errors: BTreeMap<String, ErrorEntry>,
}

impl ErrorTable {
    /// Load and validate `contract/errors.json`. Every code must carry an
    /// `http` status, or the table is unusable.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let table: RawTable =
            serde_json::from_str(raw).map_err(|e| format!("errors.json: {e}"))?;
        if table.errors.is_empty() {
            return Err("errors.json declares no codes".into());
        }
        Ok(ErrorTable { errors: table.errors })
    }

    /// The entry for a code, or `None` when the code is not in the contract.
    pub fn get(&self, code: &str) -> Option<&ErrorEntry> {
        self.errors.get(code)
    }

    /// The HTTP status for a code. An unknown code is a programming error (a
    /// domain invented a code the contract does not declare) and maps to 500.
    pub fn http_for(&self, code: &str) -> u16 {
        self.errors.get(code).map(|e| e.http).unwrap_or(500)
    }

    pub fn is_retryable(&self, code: &str) -> bool {
        self.errors.get(code).map(|e| e.retryable).unwrap_or(false)
    }

    /// Whether a code is declared. A domain returning an undeclared code is a
    /// bug the boot self-check can catch.
    pub fn declares(&self, code: &str) -> bool {
        self.errors.contains_key(code)
    }

    pub fn codes(&self) -> impl Iterator<Item = &str> {
        self.errors.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_real_table() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contract/errors.json"
        ))
        .unwrap();
        let table = ErrorTable::parse(&raw).unwrap();
        assert_eq!(table.http_for("unauthorized"), 401);
        assert_eq!(table.http_for("unknown_session"), 404);
        assert_eq!(table.http_for("session_busy"), 409);
        assert_eq!(table.http_for("validation_failed"), 400);
        assert!(table.is_retryable("adapter_unreachable"));
        assert!(!table.is_retryable("session_busy"));
        // A domain inventing a code is a 500, and detectable.
        assert_eq!(table.http_for("not_a_real_code"), 500);
        assert!(!table.declares("not_a_real_code"));
    }
}
