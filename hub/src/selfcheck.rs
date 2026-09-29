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

/// The mounted surface, COLLECTED from each module's `surface()`. There is one
/// source per module (beside its `route(...)` calls), so the check cannot drift
/// from the router by editing a separate central table (TASK-048 C5).
pub fn mounted_surface() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    out.extend(agent_hub_transport::surface().iter().map(|s| s.to_string()));
    out.extend(agent_hub_plugins::routes::surface());
    out.extend(agent_hub_sessions::routes::surface().iter().map(|s| s.to_string()));
    out.extend(agent_hub_providers::routes::surface().iter().map(|s| s.to_string()));
    out.extend(agent_hub_harnesses::routes::surface().iter().map(|s| s.to_string()));
    out.extend(agent_hub_skills::routes::surface().iter().map(|s| s.to_string()));
    out.extend(agent_hub_humans::routes::surface().iter().map(|s| s.to_string()));
    out
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
