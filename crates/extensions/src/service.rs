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
    let target = base.join("extensions");
    // Build the NEW set in a temp sibling, then RENAME it into place: a reader (a
    // starting adapter) sees either the old complete tree or the new complete tree,
    // never a half-copied one (TASK-048 N4). The rename is atomic on one volume.
    let staging = base.join(format!(".extensions.staging-{}", std::process::id()));
    std::fs::remove_dir_all(&staging).ok();
    std::fs::create_dir_all(&staging)?;
    for want in selected {
        let src = &shipped.iter().find(|(id, _)| id == want).unwrap().1;
        if !src.exists() {
            // The source directory does not exist: this is a MISSING trust
            // component, not an installed one. An empty marker (an earlier
            // version) would make a missing approval extension look installed
            // (TASK-048 F07). Fail loudly; never fabricate a placement.
            std::fs::remove_dir_all(&staging).ok();
            return Err(ExtensionError::SourceMissing(want.clone(), src.display().to_string()));
        }
        copy_tree(src, &staging.join(want))?;
    }
    // Swap: move the current tree aside, move the new one in, then drop the aside.
    let previous = base.join(format!(".extensions.old-{}", std::process::id()));
    std::fs::remove_dir_all(&previous).ok();
    if target.exists() {
        std::fs::rename(&target, &previous)?;
    }
    match std::fs::rename(&staging, &target) {
        Ok(()) => {
            std::fs::remove_dir_all(&previous).ok();
            Ok(target)
        }
        Err(e) => {
            // Put the old tree back so the harness keeps a usable set.
            let _ = std::fs::rename(&previous, &target);
            std::fs::remove_dir_all(&staging).ok();
            Err(ExtensionError::Io(e))
        }
    }
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
