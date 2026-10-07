//! T3: durability across a **real process crash** (ARCHITECTURE §10).
//!
//! The `t3-kill-writer` helper records an install boundary and then
//! `std::process::abort()`s - no Drop, no SQLite close, no WAL checkpoint. The
//! parent reopens the same files and recovers, proving the recorded step and the
//! on-disk copies survived a crash rather than a graceful shutdown.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-t3k-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn kill_writer(root: &std::path::Path, db_path: &std::path::Path, step: &str) {
    // `CARGO_BIN_EXE_<name>` points at the built helper.
    let exe = env!("CARGO_BIN_EXE_t3-kill-writer");
    let status = Command::new(exe)
        .arg(db_path)
        .arg(root)
        .arg(step)
        .status()
        .expect("spawn writer");
    // abort() -> a non-zero, non-success exit; that is the crash.
    assert!(!status.success(), "writer was expected to abort");
}

#[test]
fn step_survives_a_real_abort_and_recovers() {
    let dir = tmp("oldmoved");
    let db_path = dir.join("hub.sqlite");
    let root = dir.join("plugins");
    fs::create_dir_all(&root).unwrap();

    // An old plugin is in place.
    let target = root.join("p");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("manifest.json"), r#"{"id":"p"}"#).unwrap();

    // Crash after the old copy was moved aside.
    kill_writer(&root, &db_path, "old_moved_aside");

    // The step and the moved copy survived (no graceful close).
    assert!(!target.exists(), "old copy was moved aside before the crash");
    assert!(root.join(".p.outgoing").exists(), "outgoing survived");

    let db = agent_hub_db::Db::open(&db_path).expect("reopen after crash");
    assert_eq!(
        agent_hub_db::recovery::recorded_step(&db, "p"),
        Some("old_moved_aside"),
        "the recorded step did not survive the crash"
    );

    // Boot sweep restores the last committed copy.
    let outcomes = agent_hub_db::recover(&db, &root).expect("recover");
    assert_eq!(outcomes[0].1, agent_hub_db::RecoveryOutcome::RolledBackToOutgoing);
    assert!(target.join("manifest.json").exists(), "old plugin restored");
    assert!(!root.join(".p.outgoing").exists());
}

#[test]
fn commit_boundary_survives_and_sweeps_after_crash() {
    let dir = tmp("committed");
    let db_path = dir.join("hub.sqlite");
    let root = dir.join("plugins");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("p");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("manifest.json"), r#"{"id":"p"}"#).unwrap();

    kill_writer(&root, &db_path, "committed");

    let db = agent_hub_db::Db::open(&db_path).unwrap();
    assert_eq!(
        agent_hub_db::recovery::recorded_step(&db, "p"),
        Some("committed")
    );
    let outcomes = agent_hub_db::recover(&db, &root).unwrap();
    assert_eq!(outcomes[0].1, agent_hub_db::RecoveryOutcome::SweptOutgoing);
    assert!(target.join("manifest.json").exists());
}
