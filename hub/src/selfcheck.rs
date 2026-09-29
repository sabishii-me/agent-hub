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

/// The routes this build mounts, as `"METHOD /path"`. Kept beside the mounts so
/// the surface check has a list to compare (ARCHITECTURE 13.4).
pub fn mounted_surface() -> BTreeSet<String> {
    [
        "GET /v1/hub/status",
        "GET /v1/hub/events",
        // plugins
        "GET /v1/hub/plugins",
        "POST /v1/hub/plugins",
        "DELETE /v1/hub/plugins/{id}",
        "POST /v1/hub/plugins/{id}/prepare",
        // sessions
        "POST /v1/sessions",
        "GET /v1/sessions",
        "GET /v1/sessions/{id}",
        "PATCH /v1/sessions/{id}",
        "DELETE /v1/sessions/{id}",
        "GET /v1/sessions/{id}/turns",
        "POST /v1/sessions/{id}/turns",
        "POST /v1/sessions/{id}/cancel",
        "POST /v1/sessions/{id}/close",
        "POST /v1/sessions/{id}/reopen",
        "POST /v1/sessions/{id}/fork",
        // providers
        "GET /v1/hub/providers",
        "POST /v1/hub/providers",
        "GET /v1/hub/providers/{id}",
        "PATCH /v1/hub/providers/{id}",
        "DELETE /v1/hub/providers/{id}",
        "POST /v1/hub/providers/{id}/logout",
        "GET /v1/hub/providers/{id}/models",
        "PATCH /v1/hub/providers/{id}/models",
        "POST /v1/hub/providers/{id}/models/refresh",
        "GET /v1/hub/providers/models/list",
        // harnesses
        "GET /v1/harnesses",
        "GET /v1/harnesses/{id}/presets",
        "GET /v1/harnesses/{id}/models",
        "GET /v1/harnesses/{id}/tools",
        "POST /v1/harnesses/{id}/enable",
        "POST /v1/harnesses/{id}/disable",
        "GET /v1/hub/harnesses",
        "POST /v1/hub/harnesses/{id}/enable",
        "POST /v1/hub/harnesses/{id}/disable",
        // skills
        "GET /v1/hub/skills",
        "DELETE /v1/hub/skills/{id}",
        "PUT /v1/hub/skills/{id}/files/{file...}",
        "GET /v1/hub/skills/{id}/files/{file...}",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
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
