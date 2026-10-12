//! A helper for the T3 kill test: record an install boundary, then abort without
//! a graceful close (simulating a crash). Usage:
//!   t3-kill-writer <db> <plugin-root> <step>
//! It exits by `std::process::abort()` so SQLite never checkpoints/closes.

use agent_hub_db::{Db, InstallStep, Layout, PluginRow, PluginState};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db_path = &args[1];
    let root = &args[2];
    let step = match args[3].as_str() {
        "staged" => InstallStep::Staged,
        "old_moved_aside" => InstallStep::OldMovedAside,
        "new_in_place" => InstallStep::NewInPlace,
        "committed" => InstallStep::Committed,
        other => panic!("unknown step {other}"),
    };

    let db = Db::open(db_path).expect("open db");
    let layout = Layout::for_plugin(root, "p");
    db.upsert_plugin(&PluginRow {
        id: "p".into(),
        name: Some("v1".into()),
        summary: None,
        plugin_type: Some("harness-adapter".into()),
        source: None,
        reference: None,
        commit: None,
        state: PluginState::Ready,
        detail: None,
        installed_at: None,
        artifact: None,
    })
    .expect("upsert");
    db.set_step("p", step).expect("record step");
    if step == InstallStep::OldMovedAside {
        let _ = std::fs::rename(&layout.target, &layout.outgoing);
    }

    // Crash: no Drop, no close, no checkpoint.
    std::process::abort();
}
