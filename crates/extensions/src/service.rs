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
    // Publish IN PLACE and ADDITIVELY: never delete or swap the shared dir a running
    // adapter may be reading. For each SELECTED extension, copy it into a NEW
    // versioned subdir and move that subdir into its final name with ONE rename (an
    // atomic replace of just that extension). Only AFTER every selected extension is
    // in place do we remove the de-selected ones. A reader therefore sees either the
    // old version of an extension or the new one - never a half-copied tree and never
    // a missing one (TASK-048 N4).
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
        // Build THIS extension completely in a temp dir, then rename it over its
        // final name (a single-step replace of one extension's subtree).
        let staging = base.join(format!(".stage-{want}-{}", unique()));
        std::fs::remove_dir_all(&staging).ok();
        std::fs::create_dir_all(&staging)?;
        copy_tree(src, &staging)?;
        let final_dir = target.join(want);
        let old = base.join(format!(".old-{want}-{}", unique()));
        std::fs::remove_dir_all(&old).ok();
        if dir_exists(&final_dir) {
            std::fs::rename(&final_dir, &old).ok();
        }
        match std::fs::rename(&staging, &final_dir) {
            Ok(()) => {
                std::fs::remove_dir_all(&old).ok();
            }
            Err(e) => {
                // Put the old one back so the harness keeps a usable extension.
                let _ = std::fs::rename(&old, &final_dir);
                std::fs::remove_dir_all(&staging).ok();
                return Err(ExtensionError::Io(e));
            }
        }
    }

    // Removal LAST: now that every selected extension is present, drop the rest.
    if let Ok(entries) = std::fs::read_dir(&target) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !selected_set.contains(name.as_str()) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    Ok(target)
}

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

fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{millis:x}-{:x}", C.fetch_add(1, Ordering::Relaxed))
}

/// Whether `p` is a directory OR a directory pointer (junction/symlink).
fn dir_exists(p: &Path) -> bool {
    p.exists() || std::fs::symlink_metadata(p).is_ok()
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
