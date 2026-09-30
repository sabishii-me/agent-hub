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

    // Build a COMPLETE, IMMUTABLE SNAPSHOT of the selected set, then hand that
    // snapshot's path to the adapter for this start (TASK-048 S5/N4). No reader
    // ever observes a partially-updated tree: the snapshot is written under a
    // private name and only named as the returned path once it is complete. The
    // caller points the adapter at THIS path, so concurrent starts each get a
    // stable, complete set. Snapshots are swept by age, never while current.
    let rev = unique();
    let snapshots = base.join("extensions.snapshots");
    std::fs::create_dir_all(&snapshots)?;
    let snapshot = snapshots.join(format!(".snap-{rev}"));
    std::fs::remove_dir_all(&snapshot).ok();
    std::fs::create_dir_all(&snapshot)?;

    let result: Result<(), ExtensionError> = (|| {
        for want in selected {
            let src = &shipped.iter().find(|(id, _)| id == want).unwrap().1;
            if !src.exists() {
                // A declared source with no directory is a MISSING trust component,
                // not an installed one. Fail loudly; never fabricate a placement.
                return Err(ExtensionError::SourceMissing(
                    want.clone(),
                    src.display().to_string(),
                ));
            }
            copy_tree(src, &snapshot.join(want))?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&snapshot);
        return Err(e);
    }

    // Hand the adapter THIS snapshot's path directly: it is COMPLETE and
    // IMMUTABLE, so the reader never races a replacement and never sees a mixed or
    // missing set. There is no shared mutable name to swap, hence no window
    // (TASK-048 S5). Older snapshots are swept by COUNT: keep the newest few so a
    // still-running adapter's snapshot is not deleted under it, drop the rest.
    sweep_old_snapshots(&snapshots, &snapshot, KEEP_SNAPSHOTS);

    Ok(snapshot)
}

/// How many recent snapshots to keep (a running adapter reads its own; the newest
/// few cover overlapping starts). Sweeping is best-effort and never removes the
/// just-published snapshot.
const KEEP_SNAPSHOTS: usize = 8;

/// Sweep old extension snapshots, keeping the newest `keep`. A failure to sweep is
/// a leak, not a correctness failure.
fn sweep_old_snapshots(dir: &Path, keep: &Path, keep_count: usize) {
    let mut snaps: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().map(|n| n.to_string_lossy().starts_with(".snap-")).unwrap_or(false))
            .collect(),
        Err(_) => return,
    };
    // Newest first by the millis prefix in the name.
    snaps.sort();
    snaps.reverse();
    for (i, p) in snaps.iter().enumerate() {
        if p == keep {
            continue;
        }
        if i < keep_count {
            continue;
        }
        let _ = std::fs::remove_dir_all(p);
    }
}

/// A unique-enough name suffix (millis + a counter) for snapshot dirs.
fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{millis:x}-{:x}", C.fetch_add(1, Ordering::Relaxed))
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

    /// A returned snapshot is COMPLETE and contains exactly the selected set; two
    /// calls produce two independent snapshots (S5).
    #[test]
    fn a_snapshot_is_complete_and_independent() {
        let dir = std::env::temp_dir().join(format!("ext-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Two shipped extensions, each with a file.
        for (id, f) in [("e1", "a"), ("e2", "b")] {
            let d = dir.join(format!("src-{id}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(f), id).unwrap();
        }
        let shipped = vec![
            ("e1".to_string(), dir.join("src-e1")),
            ("e2".to_string(), dir.join("src-e2")),
        ];
        let s1 = install_for_harness(&dir, "pi", &shipped, &["e1".into(), "e2".into()]).unwrap();
        assert!(s1.join("e1/a").is_file(), "the snapshot holds e1");
        assert!(s1.join("e2/b").is_file(), "the snapshot holds e2");
        let s2 = install_for_harness(&dir, "pi", &shipped, &["e1".into()]).unwrap();
        assert!(s2.join("e1/a").is_file(), "the second snapshot holds e1");
        assert!(!s2.join("e2").exists(), "a de-selected extension is absent from the new snapshot");
        // The FIRST snapshot is untouched (independent, immutable).
        assert!(s1.join("e2/b").is_file(), "the first snapshot is unchanged");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
