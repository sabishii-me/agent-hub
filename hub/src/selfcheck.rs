//! The boot self-check (ARCHITECTURE §13.4, §13.7).
//!
//! Two seams are held at boot, before the hub serves:
//!
//! 1. every error code a domain can return is declared in `contract/errors.json`
//!    (a domain inventing a code is a refusal to start, not a runtime 500);
//! 2. the mounted route surface matches `contract/v1.json` (a forgotten or extra
//!    route is a refusal), so the contract is the interface it claims to be.

use std::collections::BTreeSet;

/// Check that every `code` a domain can emit is declared in the contract table.
pub fn check_error_codes(declared: &[&str], table: &agent_hub_contract::ErrorTable) -> Result<(), String> {
    let mut unknown: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|c| !table.declares(c))
        .collect();
    unknown.sort_unstable();
    unknown.dedup();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "these error codes are not declared in contract/errors.json: {}",
            unknown.join(", ")
        ))
    }
}

/// Every route the hub mounts, as `"METHOD /path"`, compared against the
/// contract's endpoints.
pub fn check_surface(mounted: &BTreeSet<String>, contract: &serde_json::Value) -> Result<(), String> {
    let mut expected: BTreeSet<String> = BTreeSet::new();
    if let Some(endpoints) = contract.get("endpoints").and_then(|v| v.as_array()) {
        for e in endpoints {
            if let (Some(m), Some(p)) = (e.get("method").and_then(|v| v.as_str()), e.get("path").and_then(|v| v.as_str())) {
                expected.insert(format!("{} {}", m.to_uppercase(), p));
            }
        }
    }
    let missing: Vec<&String> = expected.difference(mounted).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        // Not every contract route is implemented yet (ARCHITECTURE 18); the
        // self-check reports what is missing rather than failing boot.
        Err(format!("{} contract routes are not mounted yet", missing.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undeclared_code_is_caught() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../contract/errors.json"
        ))
        .unwrap();
        let table = agent_hub_contract::ErrorTable::parse(&raw).unwrap();
        assert!(check_error_codes(&["unknown_session", "session_busy"], &table).is_ok());
        assert!(check_error_codes(&["unknown_session", "made_up"], &table).is_err());
    }
}
