//! The extensions domain: the hub's side. An extension is a directory a plugin
//! ships; before a harness runs the hub writes that harness's selected extension
//! directories into its data dir, and the adapter places them with discovery off.
//! Placement is the hub's; it is not an authorization boundary.

pub mod service;

pub use service::{install_for_harness, ExtensionError, ShippedExtension};
