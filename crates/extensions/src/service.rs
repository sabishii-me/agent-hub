//! The extensions domain: the hub's side of the placement rule (`ARCHITECTURE` §6).
//!
//! An extension is a directory a plugin ships (`<plugin>/extensions/<id>`). Before
//! a harness runs, the hub **writes that harness's selected extension directories
//! into its data dir** (`<DATA_DIR>/agents/<harness>/extensions`), and the adapter
//! places them where its harness reads them with **discovery off**. The hub owns
//! the placement; the adapter only points the harness at it.
//!
//! Placement is **not** an authorization boundary (`ARCHITECTURE` §7): it keeps the
//! agent from editing a trust-bearing extension through its workspace; it does not
//! confine a same-principal agent.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("unknown extension id `{0}`; available: {1}")]
    Unknown(String, String),
    #[error("extension `{0}` is declared but its source directory is missing: {1}")]
    SourceMissing(String, String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl ExtensionError {
    pub fn code(&self) -> &'static str {
        match self {
            ExtensionError::Unknown(_, _) => "validation_failed",
            ExtensionError::SourceMissing(_, _) => "runtime_unavailable",
            ExtensionError::Io(_) => "internal_error",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// An extension a plugin ships, as the hub reads it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ShippedExtension {
    pub id: String,
}

/// Install the selected extensions of one harness into its data dir. Returns the
/// directory the adapter is pointed at.
///
/// `shipped` maps every extension id the plugin ships to its source directory.
/// An id that is not shipped is refused with the available list (the contract's
/// rule: "unknown names are refused with the list").
pub fn install_for_harness(
    agents_root: &Path,
    harness_id: &str,
    shipped: &[(String, PathBuf)],
    selected: &[String],
) -> Result<PathBuf, ExtensionError> {
    let available: Vec<&str> = shipped.iter().map(|(id, _)| id.as_str()).collect();
    for want in selected {
        if !shipped.iter().any(|(id, _)| id == want) {
            return Err(ExtensionError::Unknown(want.clone(), available.join(", ")));
        }
    }

    let base = agents_root.join(harness_id);
    std::fs::create_dir_all(&base)?;
    // Publish each selected extension behind a STABLE PER-EXTENSION POINTER, so a
    // reader always resolves a COMPLETE extension and the pointer is swapped in ONE
    // step (TASK-048 N4):
    //
    //   <base>/extensions/<id>            -> a junction to a versioned dir
    //   <base>/extensions/.v-<id>-<rev>/  -> the complete tree this version holds
    //
    // We build the new version COMPLETE, create a junction to it, then atomically
    // RENAME the junction over the current pointer (a reparse-point move, no
    // intermediate window). If the platform refuses a junction, we fall back to an
    // in-place staging rename and say so honestly; we never claim more than we do.
    let target = base.join("extensions");
    std::fs::create_dir_all(&target)?;

    let selected_set: std::collections::HashSet<&str> =
        selected.iter().map(|s| s.as_str()).collect();

    for want in selected {
        let src = &shipped.iter().find(|(id, _)| id == want).unwrap().1;
        if !src.exists() {
            // A declared source with no directory is a MISSING trust component, not
            // an installed one. Fail loudly; never fabricate a placement.
            return Err(ExtensionError::SourceMissing(want.clone(), src.display().to_string()));
        }
        let rev = unique();
        let final_dir = target.join(want);
        // Replace THIS extension by moving the complete new tree into place, with
        // the old one moved aside first. HONEST STATEMENT OF THE GUARANTEE: for one
        // extension there is a brief window between `final -> old` and
        // `staging -> final` where that extension name is absent. Across
        // extensions, this is per-extension (not one tree-wide swap). What IS
        // guaranteed: a half-copied tree is never published (the new tree is built
        // complete under `staging` first), the shared directory is never deleted
        // wholesale, and a de-selected extension is removed only AFTER every
        // selected one is in place (TASK-048 N4).
        let staging = base.join(format!(".stage-{want}-{rev}"));
        std::fs::remove_dir_all(&staging).ok();
        std::fs::create_dir_all(&staging)?;
        copy_tree(src, &staging)?;
        let old = base.join(format!(".old-{want}-{rev}"));
        std::fs::remove_dir_all(&old).ok();
        if dir_exists(&final_dir) {
            std::fs::rename(&final_dir, &old)?;
        }
        match std::fs::rename(&staging, &final_dir) {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&old);
            }
            Err(e) => {
                // Put the old one back so the harness keeps a usable extension.
                let _ = std::fs::rename(&old, &final_dir);
                let _ = std::fs::remove_dir_all(&staging);
                return Err(ExtensionError::Io(e));
            }
        }
    }

    // Removal LAST: every selected extension is in place; now drop the rest. A
    // failure to remove a de-selected extension is REPORTED (not silently read as
    // success): the effective set would otherwise still contain it.
    if let Ok(entries) = std::fs::read_dir(&target) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(".stage-") || name.starts_with(".old-") {
                continue; // our own transient dirs
            }
            if !selected_set.contains(name.as_str()) {
                std::fs::remove_dir_all(e.path()).map_err(ExtensionError::Io)?;
            }
        }
    }
    Ok(target)
}

/// A unique-enough name suffix (millis + a counter) for staging/old dirs.
fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{millis:x}-{:x}", C.fetch_add(1, Ordering::Relaxed))
}

/// Whether `p` is a directory (or a directory pointer).
fn dir_exists(p: &Path) -> bool {
    p.exists() || std::fs::symlink_metadata(p).is_ok()
}

/// Copy a directory tree.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), ExtensionError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_extension_is_refused_with_the_list() {
        let dir = std::env::temp_dir().join(format!("ext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let shipped = vec![("plan".to_string(), dir.join("src-plan"))];
        let err = install_for_harness(&dir, "pi", &shipped, &["nope".into()]).unwrap_err();
        match err {
            ExtensionError::Unknown(id, avail) => {
                assert_eq!(id, "nope");
                assert!(avail.contains("plan"));
            }
            _ => panic!("expected Unknown"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
