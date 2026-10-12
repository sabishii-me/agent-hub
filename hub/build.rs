//! The build marker: the git revision this binary was built from.
//!
//! `GET /v1/status.buildId` and the discovery record report it so a client that
//! spawned the hub can prove WHICH build it is talking to. It is captured at
//! compile time; a build outside a git checkout reports `unknown`.

use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=HUB_BUILD_ID={sha}");
    // Rebuild the marker when the revision moves.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
}
