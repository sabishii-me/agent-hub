//! The skills domain: the hub's side. A skill is a directory; the hub stores the
//! bytes and installs the effective set into a harness's directory, which the
//! adapter points its harness at (discovery off). Placement is the hub's; it is
//! not an authorization boundary.

pub mod routes;
pub mod service;

pub use service::{SkillError, SkillInfo, Skills};
