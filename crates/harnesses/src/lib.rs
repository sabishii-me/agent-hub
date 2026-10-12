//! The harnesses domain: a thin top-level projection of the adapter registry,
//! and the capability-gated runtime answers routed to the adapter.
//! `known: false` means the harness declares no such capability - never a faked
//! empty list.

pub mod routes;
pub mod service;

pub use service::{HarnessError, Harnesses};
