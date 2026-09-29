//! The adapter domain (`ARCHITECTURE` §4, `contract/adapter-v1.json`).
//!
//! The hub owns the adapter's **process** and speaks the agent-bus protocol
//! (JSON-RPC 2.0 over stdin/stdout, one LF-terminated line per message). A
//! harness is a plugin that declares the adapter protocol; the hub registers it,
//! starts its adapter when needed, routes capability calls, and turns the
//! adapter's notifications into events. No adapter code runs in-process.

pub mod bus;
pub mod manager;
pub mod manifest;

pub use bus::{AgentBus, BusError, Notification};
pub use manager::{AdapterError, Adapters, Harness, HarnessStatus};
pub use manifest::{AdapterManifest, ManifestError, ADAPTER_PROTOCOL_VERSION};
