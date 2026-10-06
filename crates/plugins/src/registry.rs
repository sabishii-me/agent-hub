//! WHERE the plugin registry comes from — the trust boundary of the plugin system.
//!
//! A registry is the deployer's allow-list: the set of plugins the hub may install, each id
//! pinned to one release (url + sha256). For that to mean anything, the hub must NOT let a
//! caller — or an environment variable — point the hub at a DIFFERENT registry. Otherwise
//! anyone can serve a fake registry and inject any plugin (docs/issues/20261005-130000).
//!
//! So the registry ADDRESS is fixed at build time:
//!
//! - **release build** (`cargo build --release`): the ONE official address below. There is no
//!   code path that reads an environment variable to change it — the override does not exist.
//! - **dev build** (`cargo build`, `#[cfg(debug_assertions)]`): `AGENT_HUB_REGISTRY_URL` may
//!   override the address, so tests can point at a loopback registry and exercise the error
//!   paths. This override is COMPILED OUT of a release binary.
//!
//! The registry CONTENT is not pinned: the official registry may publish new versions, and the
//! hub pulls them (POST /v1/plugins/registry/refresh). Only the ADDRESS is fixed.

/// The one official registry the shipped hub will ever contact. Fixed at build time.
///
/// It is the `registry` release asset of the hub's OWN repository - a fixed artifact name on
/// a fixed tag (`registry`), so the address never moves when the catalog changes. The release
/// is published only by the maintainer; the hub never takes a registry address from a caller or
/// the environment (docs/issues/20261005-130000).
pub const OFFICIAL_REGISTRY_URL: &str =
    "https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json";

/// The address the hub fetches the registry from.
///
/// In a **release** build this is always [`OFFICIAL_REGISTRY_URL`] — no environment variable is
/// consulted, so nothing at runtime can redirect the hub to a foreign registry.
///
/// In a **dev** build an explicit `AGENT_HUB_REGISTRY_URL` overrides it (tests need to point at a
/// local registry). An empty/unset variable falls back to the official address.
pub fn source_url() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(v) = std::env::var("AGENT_HUB_REGISTRY_URL") {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    OFFICIAL_REGISTRY_URL.to_string()
}

/// Whether an environment override is even possible in THIS build. A release binary answers
/// `false`: its registry address cannot be changed at runtime. (Tests assert this.)
pub const fn env_override_allowed() -> bool {
    cfg!(debug_assertions)
}
