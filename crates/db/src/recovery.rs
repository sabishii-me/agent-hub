//! Plugin install/replace **recovery** (`ARCHITECTURE` §10, task T3).
//!
//! An install is two renames plus a DB commit, over a directory tree and a row.
//! Each move is preceded by a durable step, so a boot sweep can finish or roll
//! back what a crash left. This module encodes the §10 boundary table exactly:
//!
//! | boundary (crash between these) | on restart |
//! |---|---|
//! | before any move (`Staged`) | nothing; the old plugin is intact |
//! | after `target->outgoing`, before `staging->target` (`OldMovedAside`) | restore `outgoing` -> `target`, drop `staging` |
//! | after `staging->target`, before the commit (`NewInPlace`) | the new copy is in place but uncommitted: finish the commit, or roll back to `outgoing`; the state says which |
//! | after the commit, before deleting `outgoing` (`Committed`) | done; sweep `outgoing` |
//! | rollback failed | keep both copies + a record: "unusable, recovery retained" |
//! | recovery interrupted again | resume from the recorded step, not from scratch |
//!
//! Directory names are **not** the contract: `outgoing` is the last committed
//! copy and a `staging` tree may itself be complete. The step decides.

use std::path::{Path, PathBuf};

use crate::{step_name, Db, InstallStep, PluginState};

/// The outcome of sweeping one plugin's unfinished operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Nothing was recorded; nothing to do.
    Nothing,
    /// A staged tree was dropped; the old plugin was intact.
    DroppedStaging,
    /// The old copy was restored; the new one was never committed.
    RolledBackToOutgoing,
    /// The new copy was in place and the commit was finished.
    FinishedCommit,
    /// The commit was already done; the transient `outgoing` was swept.
    SweptOutgoing,
    /// Rollback failed; both copies are kept and the state is "not usable,
    /// recovery copy retained".
    RetainedRecoveryCopy,
}

/// Paths a plugin install uses, relative to a plugin's root directory.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The plugin's live directory (what the hub serves).
    pub target: PathBuf,
    /// The previous committed copy, moved aside during a replace.
    pub outgoing: PathBuf,
    /// The validated new tree, before it is moved into place.
    pub staging: PathBuf,
}

impl Layout {
    pub fn for_plugin(root: impl AsRef<Path>, id: &str) -> Self {
        let root = root.as_ref();
        Layout {
            target: root.join(id),
            outgoing: root.join(format!(".{id}.outgoing")),
            staging: root.join(format!(".{id}.staging")),
        }
    }
}

/// Sweep every unfinished operation. Runs at boot, **before** any GC.
pub fn recover(db: &Db, plugin_root: &Path) -> Result<Vec<(String, RecoveryOutcome)>, crate::DbError> {
    let mut out = Vec::new();
    for (id, step, retained) in db.unfinished_ops()? {
        let layout = Layout::for_plugin(plugin_root, &id);
        let outcome = recover_one(db, &id, step, retained, &layout)?;
        out.push((id, outcome));
    }
    Ok(out)
}

fn recover_one(
    db: &Db,
    id: &str,
    step: InstallStep,
    retained: bool,
    layout: &Layout,
) -> Result<RecoveryOutcome, crate::DbError> {
    // A retained copy is terminal until a human acts: never silently delete it.
    if retained || step == InstallStep::RecoveryRetained {
        set_state(db, id, PluginState::Failed, "recovery copy retained")?;
        return Ok(RecoveryOutcome::RetainedRecoveryCopy);
    }

    match step {
        InstallStep::Staged => {
            // Nothing moved; the old plugin is intact. Drop the staged tree.
            remove_dir(&layout.staging);
            clear(db, id, PluginState::Ready)?;
            Ok(RecoveryOutcome::DroppedStaging)
        }
        InstallStep::OldMovedAside => {
            // Old copy is the last committed version. Restore it; drop staging.
            if layout.outgoing.exists() {
                remove_dir(&layout.target);
                if std::fs::rename(&layout.outgoing, &layout.target).is_err() {
                    return fail_retain(db, id);
                }
            }
            remove_dir(&layout.staging);
            clear(db, id, PluginState::Ready)?;
            Ok(RecoveryOutcome::RolledBackToOutgoing)
        }
        InstallStep::NewInPlace => {
            // The new copy is in place and complete; finish the commit.
            if layout.target.exists() {
                set_state(db, id, PluginState::Ready, "recovered after install")?;
                db.clear_step(id)?;
                remove_dir(&layout.outgoing);
                Ok(RecoveryOutcome::FinishedCommit)
            } else {
                // The new copy did not survive; roll back to outgoing.
                recover_one(db, id, InstallStep::OldMovedAside, false, layout)
            }
        }
        InstallStep::Committed => {
            // Done; only the transient outgoing remains.
            remove_dir(&layout.outgoing);
            clear(db, id, PluginState::Ready)?;
            Ok(RecoveryOutcome::SweptOutgoing)
        }
        InstallStep::RecoveryRetained => unreachable!("handled above"),
    }
}

fn fail_retain(db: &Db, id: &str) -> Result<RecoveryOutcome, crate::DbError> {
    db.mark_backup_retained(id)?;
    db.set_step(id, InstallStep::RecoveryRetained)?;
    set_state(db, id, PluginState::Failed, "rollback failed; recovery copy retained")?;
    Ok(RecoveryOutcome::RetainedRecoveryCopy)
}

fn clear(db: &Db, id: &str, state: PluginState) -> Result<(), crate::DbError> {
    set_state(db, id, state, "")?;
    db.clear_step(id)
}

fn set_state(
    db: &Db,
    id: &str,
    state: PluginState,
    detail: &str,
) -> Result<(), crate::DbError> {
    if let Some(mut row) = db.plugin(id)? {
        row.state = state;
        row.detail = if detail.is_empty() { None } else { Some(detail.to_string()) };
        db.upsert_plugin(&row)?;
    } else {
        db.upsert_plugin(&crate::PluginRow {
            id: id.into(),
            name: None,
            summary: None,
            plugin_type: None,
            source: None,
            reference: None,
            commit: None,
            state,
            detail: if detail.is_empty() { None } else { Some(detail.to_string()) },
            installed_at: None,
            artifact: None,
        })?;
    }
    Ok(())
}

fn remove_dir(p: &Path) {
    if p.exists() {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// The **forward** install algorithm, so the recovery rules have a matching
/// writer and a test can crash it at each boundary. Returns the outcome if it
/// ran to completion.
pub fn install(
    db: &Db,
    id: &str,
    layout: &Layout,
    staging_src: &Path,
    crash_after: Option<InstallStep>,
) -> Result<RecoveryOutcome, crate::DbError> {
    // NOTE: the caller records the artifact (a plugin-domain fact) on the row
    // BEFORE calling install; install only owns the tree + step machine.
    // 0. A validated tree is staged.
    db.set_step(id, InstallStep::Staged)?;
    if crash_after == Some(InstallStep::Staged) {
        return Ok(RecoveryOutcome::Nothing);
    }
    let _ = std::fs::remove_dir_all(&layout.staging);
    std::fs::rename(staging_src, &layout.staging).map_err(io_to_db)?;

    // 1. Move the old committed copy aside.
    if layout.target.exists() {
        let _ = std::fs::remove_dir_all(&layout.outgoing);
        std::fs::rename(&layout.target, &layout.outgoing).map_err(io_to_db)?;
    }
    db.set_step(id, InstallStep::OldMovedAside)?;
    if crash_after == Some(InstallStep::OldMovedAside) {
        return Ok(RecoveryOutcome::Nothing);
    }

    // 2. Move the new tree into place.
    std::fs::rename(&layout.staging, &layout.target).map_err(io_to_db)?;
    db.set_step(id, InstallStep::NewInPlace)?;
    if crash_after == Some(InstallStep::NewInPlace) {
        return Ok(RecoveryOutcome::Nothing);
    }

    // 3. Commit the row, then sweep the transient copy.
    db.set_step(id, InstallStep::Committed)?;
    if crash_after == Some(InstallStep::Committed) {
        return Ok(RecoveryOutcome::Nothing);
    }
    set_state(db, id, PluginState::Ready, "installed")?;
    db.clear_step(id)?;
    Ok(RecoveryOutcome::FinishedCommit)
}

fn io_to_db(e: std::io::Error) -> crate::DbError {
    crate::DbError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

/// The step a crash left recorded (used by the tests to name the boundary).
pub fn recorded_step(db: &Db, id: &str) -> Option<&'static str> {
    db.step(id).ok().flatten().map(step_name)
}
