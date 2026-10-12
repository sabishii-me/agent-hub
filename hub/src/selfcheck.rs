//! The runtime's surface ROWS (`/v1/surface`), and - as TESTS ONLY - the two checks that the
//! code agrees with the contract.
//!
//! These checks are **build-time**, not runtime: the contract and the route tables are both
//! compile-time constants, so proving "the mounted surface matches the contract" once, in a
//! test, is the whole job. The serving binary does not re-prove a fixed fact on every start
//! (that would be a no-op that can only turn a static truth into an artificial startup
//! failure). `cargo test` is the gate.

/// The mounted surface as STRUCTURED rows (`/v1/surface` needs method/path/auth), from
/// the same per-module tables the contract test unions. One source per module, beside its
/// `route(...)` calls, so it cannot drift from the router by editing a separate table.
pub fn surface_rows() -> Vec<agent_hub_transport::SurfaceRow> {
    let mut out: Vec<(String, String)> = Vec::new();
    let collect = |v: Vec<String>, out: &mut Vec<(String, String)>| {
        for s in v {
            if let Some((m, p)) = s.split_once(' ') {
                out.push((m.to_string(), p.to_string()));
            }
        }
    };
    collect(agent_hub_transport::surface(), &mut out);
    collect(agent_hub_plugins::routes::surface(), &mut out);
    collect(agent_hub_sessions::routes::surface(), &mut out);
    collect(agent_hub_providers::routes::surface(), &mut out);
    collect(agent_hub_connections::routes::surface(), &mut out);
    collect(agent_hub_harnesses::routes::surface(), &mut out);
    collect(agent_hub_skills::routes::surface(), &mut out);
    collect(agent_hub_humans::routes::surface(), &mut out);
    out.sort();
    out.dedup();
    out.into_iter()
        .map(|(method, path)| agent_hub_transport::SurfaceRow { method, path, auth: true })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The mounted surface, COLLECTED from each module's `surface()`.
    fn mounted_surface() -> BTreeSet<String> {
        surface_rows()
            .into_iter()
            .map(|r| format!("{} {}", r.method.to_uppercase(), r.path))
            .collect()
    }

    fn contract() -> serde_json::Value {
        serde_json::from_str(include_str!("../../contract/v1.json")).unwrap()
    }

    fn error_table() -> agent_hub_contract::ErrorTable {
        agent_hub_contract::ErrorTable::parse(include_str!("../../contract/errors.json")).unwrap()
    }

    /// Serving a route the contract does NOT declare is a bug; a contract route not mounted yet
    /// is unfinished work (allowed). The dangerous direction is the first one.
    #[test]
    fn the_mounted_surface_is_declared_in_the_contract() {
        let contract = contract();
        let mounted = mounted_surface();
        let mut expected: BTreeSet<String> = BTreeSet::new();
        for e in contract.get("endpoints").and_then(|v| v.as_array()).into_iter().flatten() {
            if let (Some(m), Some(p)) = (
                e.get("method").and_then(|v| v.as_str()),
                e.get("path").and_then(|v| v.as_str()),
            ) {
                expected.insert(format!("{} {}", m.to_uppercase(), p));
            }
        }
        let extra: Vec<&String> = mounted.difference(&expected).collect();
        assert!(
            extra.is_empty(),
            "these mounted routes are NOT in contract/v1.json: {extra:?}"
        );
    }

    /// Every error code the code can return must be declared in `contract/errors.json`.
    #[test]
    fn the_error_codes_are_declared_in_the_contract() {
        let table = error_table();
        // The declared set, as written by the domains (an invented code is a bug here).
        let declared = [
            "not_found", "plugin_remove_failed", "plugin_in_use", "plugin_dir_busy",
            "plugin_archive_invalid", "idempotency_conflict", "plugin_install_failed",
            "internal_error", "unknown_session", "session_closed", "session_busy",
            "validation_failed", "provider_not_found", "already_exists", "provider_catalog_failed",
        ];
        let unknown: Vec<&str> =
            declared.iter().copied().filter(|c| !table.declares(c)).collect();
        assert!(unknown.is_empty(), "these error codes are not declared: {unknown:?}");
        assert!(table.declares("unknown_session") && table.declares("session_busy"));
    }

    /// A contract file must parse - a malformed contract is a build failure, not a runtime one.
    #[test]
    fn the_contract_parses() {
        assert!(contract().get("endpoints").and_then(|v| v.as_array()).is_some());
        let _ = error_table();
    }
}
