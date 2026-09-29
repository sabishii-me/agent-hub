//! The plugins domain (`ARCHITECTURE` §10, task T3): install, remove, prepare
//! and the listing, wired to the frame.
//!
//! The lifecycle's recovery is `agent-hub-db`'s; this crate owns the routes, the
//! one-verdict state (`state.rs`), the logical command identity (`identity.rs`),
//! and the `hub.plugins.changed` announcements.

pub mod identity;
pub mod manifest;
pub mod routes;
pub mod service;
pub mod state;

pub use identity::{CommandIds, Idempotency};
pub use manifest::{Manifest, PluginView};
pub use service::{now_rfc3339, PluginError, Plugins};
pub use state::{Op, Ops};
