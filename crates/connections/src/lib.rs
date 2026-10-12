//! The connections domain: the hub-managed connections as
//! relationship state in the database, plus the credential in the OS secret store
//! (a per-instance reference, never a value). A connection is enable/disable/delete
//! only; `disabled` means cut-off, which means zero materialization.

pub mod routes;
pub mod service;

pub use service::{ConnectionError, ConnectionView, Connections, CreateConnection, PatchConnection};
