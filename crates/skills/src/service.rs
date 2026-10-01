//! The skills domain: the hub's side (`ARCHITECTURE` §6).
//!
//! A skill is a **directory**, not a record: its id is the directory name. The
//! hub stores the bytes; it does not interpret them. Before a harness's process
//! starts the hub **installs** the effective skills into
//! `<DATA_DIR>/agents/<harness>/skills` and hands that directory over as
//! `AGENT_HUB_INSTALLED_SKILLS_DIR` (`contract/adapter-v1.json`). The placement
//! is the hub's; the harness is pointed at it with discovery off.
//!
//! Placement is **not** an authorization boundary (`ARCHITECTURE` §7): it keeps
//! the user's own skill directories out of a managed session; it does not
//! confine a same-principal agent.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("skill `{0}` not found")]
    NotFound(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl SkillError {
    pub fn code(&self) -> &'static str {
        match self {
            SkillError::NotFound(_) => "not_found",
            SkillError::InvalidPath(_) => "validation_failed",
            SkillError::Io(_) => "internal_error",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// The skills the hub holds (`<DATA_DIR>/skills`).
pub struct Skills {
    pub root: PathBuf,
    /// `<DATA_DIR>/agents` - where per-harness installs live.
    pub agents_root: PathBuf,
}

/// A skill directory listing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillInfo {
    pub id: String,
    #[serde(rename = "hasManifest")]
    pub has_manifest: bool,
    pub files: u64,
}

impl Skills {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Skills {
            root: data_dir.join("skills"),
            agents_root: data_dir.join("agents"),
        }
    }

    fn skill_dir(&self, id: &str) -> Result<PathBuf, SkillError> {
        validate_segment(id)?;
        Ok(self.root.join(id))
    }

    /// List skill directories under the hub's skills root.
    pub fn list(&self) -> Result<Vec<SkillInfo>, SkillError> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            if id.starts_with('.') {
                continue;
            }
            let dir = entry.path();
            out.push(SkillInfo {
                id,
                has_manifest: dir.join("SKILL.md").exists() || dir.join("manifest.json").exists(),
                files: count_files(&dir),
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Read one file of a skill.
    pub fn read_file(&self, id: &str, path: &str) -> Result<String, SkillError> {
        let full = self.resolve(id, path)?;
        std::fs::read_to_string(&full).map_err(|_| SkillError::NotFound(format!("{id}/{path}")))
    }

    /// Write one file of a skill, creating directories as needed. A courier:
    /// bytes are stored, never interpreted.
    pub fn write_file(&self, id: &str, path: &str, content: &str) -> Result<u64, SkillError> {
        let full = self.resolve(id, path)?;
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full, content)?;
        Ok(content.len() as u64)
    }

    pub fn delete(&self, id: &str) -> Result<(), SkillError> {
        let dir = self.skill_dir(id)?;
        if !dir.exists() {
            return Err(SkillError::NotFound(id.into()));
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    /// Resolve `id/path` inside the skills root, refusing anything that escapes
    /// it (a `..` segment or an absolute path is an invalid input, not a read).
    fn resolve(&self, id: &str, path: &str) -> Result<PathBuf, SkillError> {
        validate_segment(id)?;
        for seg in path.split('/') {
            validate_segment(seg)?;
        }
        Ok(self.root.join(id).join(path))
    }

    /// Install a skill into a harness's directory, returning the directory the
    /// adapter is pointed at (`AGENT_HUB_INSTALLED_SKILLS_DIR`).
    pub fn install_for_harness(&self, harness_id: &str, skill_ids: &[String]) -> Result<PathBuf, SkillError> {
        validate_segment(harness_id)?;
        let target = self.agents_root.join(harness_id).join("skills");
        std::fs::remove_dir_all(&target).ok();
        std::fs::create_dir_all(&target)?;
        for id in skill_ids {
            let src = self.skill_dir(id)?;
            if !src.exists() {
                return Err(SkillError::NotFound(id.clone()));
            }
            copy_tree(&src, &target.join(id))?;
        }
        Ok(target)
    }
}

/// A single path segment must not be empty, `.`, `..`, or absolute.
fn validate_segment(seg: &str) -> Result<(), SkillError> {
    if seg.is_empty() || seg == "." || seg == ".." || seg.contains('\\') || seg.contains(':') {
        return Err(SkillError::InvalidPath(seg.into()));
    }
    Ok(())
}

fn count_files(dir: &Path) -> u64 {
    let mut n = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => n += 1,
                Err(_) => {}
            }
        }
    }
    n
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), SkillError> {
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
    fn refuses_path_traversal() {
        let s = Skills::new(std::env::temp_dir());
        assert!(s.read_file("ok", "../escape").is_err());
        assert!(s.read_file("..", "x").is_err());
        assert!(s.read_file("a", "/etc/passwd").is_err());
    }

    #[test]
    fn write_then_list_and_install() {
        let dir = std::env::temp_dir().join(format!("sk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let s = Skills::new(&dir);
        s.write_file("deploy", "SKILL.md", "# Deploy\n").unwrap();
        s.write_file("deploy", "scripts/run.sh", "echo hi\n").unwrap();
        let list = s.list().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].has_manifest);
        assert_eq!(list[0].files, 2);

        let target = s.install_for_harness("pi", &["deploy".into()]).unwrap();
        assert!(target.join("deploy/SKILL.md").exists());
        assert!(target.join("deploy/scripts/run.sh").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
