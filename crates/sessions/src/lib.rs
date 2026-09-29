//! The sessions domain: the hub's control state over sessions and turns. A
//! session owns a real adapter process (`runtime`); `create`/`close`/`reopen` are
//! wired, while turn/fork/compact are not and answer `not_implemented`.

pub mod routes;
pub mod runtime;
pub mod service;

pub use runtime::{SessionProcess, Sessions as Runtime, StartError, StartSpec};
pub use service::{CreateSession, HarnessSpec, SessionError, SessionView, Sessions};
