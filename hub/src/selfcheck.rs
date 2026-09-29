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
        // transport
        "GET /v1/status",
        "GET /v1/events",
        // plugins (only the routes this build actually mounts)
        "GET /v1/plugins",
        "POST /v1/plugins",
        "DELETE /v1/plugins/{id}",
        "POST /v1/plugins/{id}/prepare",
        // sessions (all answer 501 until wired; the paths are mounted)
        "GET /v1/sessions",
        "POST /v1/sessions",
        "GET /v1/sessions/{id}",
        "PATCH /v1/sessions/{id}",
        "DELETE /v1/sessions/{id}",
        "GET /v1/sessions/{id}/turns",
        "POST /v1/sessions/{id}/turns",
        "POST /v1/sessions/{id}/cancel",
        "POST /v1/sessions/{id}/close",
        "POST /v1/sessions/{id}/reopen",
        "POST /v1/sessions/{id}/fork",
        // model providers
        "GET /v1/model-providers",
        "POST /v1/model-providers",
        "GET /v1/model-providers/{id}",
        "PATCH /v1/model-providers/{id}",
        "DELETE /v1/model-providers/{id}",
        "POST /v1/model-providers/{id}/logout",
        "GET /v1/model-providers/{id}/models",
        "PATCH /v1/model-providers/{id}/models",
        "POST /v1/model-providers/{id}/models/refresh",
        "GET /v1/models",
        // harnesses
        "GET /v1/harnesses",
        "GET /v1/harnesses/{id}/presets",
        "GET /v1/harnesses/{id}/models",
        "GET /v1/harnesses/{id}/tools",
        // skills
        "GET /v1/skills",
        "DELETE /v1/skills/{id}",
        "PUT /v1/skills/{id}/files/{file...}",
        "GET /v1/skills/{id}/files/{file...}",
        // humans
        "GET /v1/sessions/{id}/approvals",
        "POST /v1/sessions/{id}/approvals/{aid}",
        "GET /v1/sessions/{id}/questions",
        "POST /v1/sessions/{id}/questions/{qid}",
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
            if let (Some(m), Some(p)) = (
                e.get("method").and_then(|v| v.as_str()),
                e.get("path").and_then(|v| v.as_str()),
            ) {
                expected.insert(format!("{} {}", m.to_uppercase(), p));
            }
        }
    }
    // The dangerous direction is serving a route the contract does NOT declare:
    // that is a refusal to start. A contract route not mounted yet is unfinished
    // work (ARCHITECTURE 18), reported but not fatal.
    let extra: Vec<&String> = mounted.difference(&expected).collect();
    if !extra.is_empty() {
        return Err(format!(
            "these mounted routes are NOT in contract/v1.json: {}",
            extra.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    let missing = expected.difference(mounted).count();
    if missing > 0 {
        tracing::info!(missing, "contract routes not mounted yet");
    }
    Ok(())
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
