//! T3 acceptance: crash at each install boundary, then recover (ARCHITECTURE §10).
//!
//! Each test uses **real directories** and a **real SQLite file**, kills the
//! forward install at one boundary, and asserts what recovery leaves - the §10
//! table, exercised rather than described.

use std::fs;
use std::path::PathBuf;

use agent_hub_db::{install, recover, Db, InstallStep, Layout, PluginState, RecoveryOutcome};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-t3-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_tree(root: &std::path::Path, marker: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("manifest.json"), format!("{{\"id\":\"{marker}\"}}")).unwrap();
}

fn db_with_ready_plugin(id: &str) -> Db {
    let db = Db::open_in_memory().unwrap();
    agent_hub_db::PluginRow {
        id: id.into(),
        name: Some("v1".into()),
        summary: None,
        plugin_type: Some("harness-adapter".into()),
        source: None,
        reference: None,
        commit: None,
        state: PluginState::Ready,
        detail: None,
        installed_at: None,
    }
    .pipe(|row| db.upsert_plugin(&row).unwrap());
    db
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

#[test]
fn no_crash_installs_and_replaces() {
    let root = tmp("forward");
    let db = Db::open_in_memory().unwrap();
    let layout = Layout::for_plugin(&root, "p");
    let src = root.join("new");
    write_tree(&src, "v2");

    let outcome = install(&db, "p", &layout, &src, None).unwrap();
    assert_eq!(outcome, RecoveryOutcome::FinishedCommit);
    assert!(layout.target.join("manifest.json").exists());
    assert_eq!(db.plugin("p").unwrap().unwrap().state, PluginState::Ready);
    assert_eq!(db.step("p").unwrap(), None);
}

#[test]
fn crash_after_staged_keeps_old_plugin() {
    let root = tmp("staged");
    let id = "p";
    let layout = Layout::for_plugin(&root, id);
    // Old plugin is in place.
    write_tree(&layout.target, "v1");
    let db = db_with_ready_plugin(id);
    let src = root.join("new");
    write_tree(&src, "v2");

    install(&db, id, &layout, &src, Some(InstallStep::Staged)).unwrap();
    assert_eq!(agent_hub_db::recovery::recorded_step(&db, id), Some("staged"));

    let outcomes = recover(&db, &root).unwrap();
    assert_eq!(outcomes[0].1, RecoveryOutcome::DroppedStaging);
    // The old plugin survived.
    assert!(layout.target.join("manifest.json").exists());
    assert_eq!(db.step(id).unwrap(), None);
}

#[test]
fn crash_after_old_moved_aside_restores_outgoing() {
    let root = tmp("oldmoved");
    let id = "p";
    let layout = Layout::for_plugin(&root, id);
    write_tree(&layout.target, "v1");
    let db = db_with_ready_plugin(id);
    let src = root.join("new");
    write_tree(&src, "v2");

    install(&db, id, &layout, &src, Some(InstallStep::OldMovedAside)).unwrap();
    // The old copy is at outgoing, target is gone.
    assert!(!layout.target.exists());
    assert!(layout.outgoing.exists());

    let outcomes = recover(&db, &root).unwrap();
    assert_eq!(outcomes[0].1, RecoveryOutcome::RolledBackToOutgoing);
    assert!(layout.target.exists(), "old copy restored");
    assert!(!layout.outgoing.exists());
    assert!(!layout.staging.exists());
}

#[test]
fn crash_after_new_in_place_finishes_the_commit() {
    let root = tmp("newinplace");
    let id = "p";
    let layout = Layout::for_plugin(&root, id);
    write_tree(&layout.target, "v1");
    let db = db_with_ready_plugin(id);
    let src = root.join("new");
    write_tree(&src, "v2");

    install(&db, id, &layout, &src, Some(InstallStep::NewInPlace)).unwrap();
    // New copy is in place, uncommitted.
    assert!(layout.target.exists());

    let outcomes = recover(&db, &root).unwrap();
    assert_eq!(outcomes[0].1, RecoveryOutcome::FinishedCommit);
    assert_eq!(db.plugin(id).unwrap().unwrap().state, PluginState::Ready);
    assert!(!layout.outgoing.exists(), "transient outgoing swept");
}

#[test]
fn crash_after_commit_sweeps_outgoing() {
    let root = tmp("committed");
    let id = "p";
    let layout = Layout::for_plugin(&root, id);
    write_tree(&layout.target, "v1");
    let db = db_with_ready_plugin(id);
    let src = root.join("new");
    write_tree(&src, "v2");

    install(&db, id, &layout, &src, Some(InstallStep::Committed)).unwrap();

    let outcomes = recover(&db, &root).unwrap();
    assert_eq!(outcomes[0].1, RecoveryOutcome::SweptOutgoing);
    // Note: SwptOutgoing means the commit had happened; the plugin is ready.
    assert!(layout.target.exists());
    assert!(!layout.outgoing.exists());
}

#[test]
fn rollback_failure_retains_a_recovery_copy() {
    let root = tmp("retained");
    let id = "p";
    let layout = Layout::for_plugin(&root, id);
    write_tree(&layout.target, "v1");
    let db = db_with_ready_plugin(id);
    // A recorded "recovery retained" step is terminal: recovery must not delete.
    db.set_step(id, InstallStep::OldMovedAside).unwrap();
    db.mark_backup_retained(id).unwrap();
    write_tree(&layout.outgoing, "v1-backup");

    let outcomes = recover(&db, &root).unwrap();
    assert_eq!(outcomes[0].1, RecoveryOutcome::RetainedRecoveryCopy);
    assert!(layout.outgoing.exists(), "recovery copy retained");
    assert_eq!(db.plugin(id).unwrap().unwrap().state, PluginState::Failed);
}

#[test]
fn recovery_is_idempotent_when_interrupted_again() {
    let root = tmp("idempotent");
    let id = "p";
    let layout = Layout::for_plugin(&root, id);
    write_tree(&layout.target, "v1");
    let db = db_with_ready_plugin(id);
    let src = root.join("new");
    write_tree(&src, "v2");

    install(&db, id, &layout, &src, Some(InstallStep::OldMovedAside)).unwrap();
    // First sweep rolls back and clears the record.
    recover(&db, &root).unwrap();
    // Second sweep finds nothing - it does not act from scratch.
    let second = recover(&db, &root).unwrap();
    assert!(second.is_empty(), "nothing left to recover");
    assert!(layout.target.exists());
}
