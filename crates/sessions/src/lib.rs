//! The sessions domain (`ARCHITECTURE` §6): the hub's control state over
//! sessions and turns, wired to the frame.

pub mod routes;
pub mod service;

pub use service::{CreateSession, SessionError, SessionView, Sessions, TurnView};
