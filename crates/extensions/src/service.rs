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
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl ExtensionError {
    pub fn code(&self) -> &'static str {
        match self {
            ExtensionError::Unknown(_, _) => "validation_failed",
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

    let target = agents_root.join(harness_id).join("extensions");
    // Replace the installed set so a removed extension is gone from the next run.
    std::fs::remove_dir_all(&target).ok();
    std::fs::create_dir_all(&target)?;
    for want in selected {
        let src = &shipped.iter().find(|(id, _)| id == want).unwrap().1;
        if src.exists() {
            copy_tree(src, &target.join(want))?;
        } else {
            // A declared id with no directory: install an empty marker so the
            // harness's placement is honest rather than silently absent.
            std::fs::create_dir_all(target.join(want))?;
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
