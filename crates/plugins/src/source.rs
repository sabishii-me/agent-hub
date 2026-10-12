//! Resolving a `POST /v1/plugins` **source** into a staging directory that holds a
//! plugin tree (a directory with a `manifest.json`).
//!
//! The contract declares TWO sources with ONE landing rule:
//!
//! - `{url, ref?}` clones a git repository (a URL or a local path) at `ref`;
//! - `{artifact:{url, sha256, id, pluginType, version, size?}}` downloads a release
//!   zip and verifies it -- **size first, then sha256** -- BEFORE anything is unpacked.
//!
//! Nothing here trusts the source: the artifact id/pluginType are checked against the
//! downloaded archive's own manifest, and the digest is checked before a single byte
//! is written to a plugin directory.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::manifest::Manifest;

/// What `POST /v1/plugins` asked to install.
#[derive(Debug, Clone)]
pub enum Source {
    /// A git repository (a URL or a local path), optionally at `ref`.
    Git { url: String, reference: Option<String> },
    /// A release artifact: a zip plus the digest that makes it verifiable.
    Artifact(ArtifactSpec),
}

/// The artifact the install names (`defs.artifact`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ArtifactSpec {
    pub url: String,
    pub sha256: String,
    pub id: String,
    pub version: String,
    #[serde(rename = "pluginType")]
    pub plugin_type: Option<String>,
    pub size: Option<u64>,
}

/// Why a source could not be turned into a plugin tree. Each maps to a contract code.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("artifact download failed: {0}")]
    Download(String),
    #[error("the artifact did not match its declared size: expected {expected}, got {got}")]
    SizeMismatch { expected: u64, got: u64 },
    #[error("the artifact did not match its declared sha256: expected {expected}, got {got}")]
    DigestMismatch { expected: String, got: String },
    #[error("the archive is not a usable plugin: {0}")]
    Archive(String),
    #[error("the archive manifest declares id `{got}`, not `{expected}`")]
    IdMismatch { expected: String, got: String },
    #[error("the archive manifest declares pluginType `{got}`, not `{expected}`")]
    TypeMismatch { expected: String, got: String },
    #[error("git clone failed: {0}")]
    Git(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl SourceError {
    pub fn code(&self) -> &'static str {
        use SourceError::*;
        match self {
            Download(_) => "artifact_download_failed",
            SizeMismatch { .. } | DigestMismatch { .. } => "artifact_digest_mismatch",
            Archive(_) | IdMismatch { .. } | TypeMismatch { .. } => "plugin_archive_invalid",
            Git(_) => "plugin_install_failed",
            Io(_) => "internal_error",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// Resolve a source into `staging` (an existing, EMPTY directory the caller owns):
/// place the plugin tree there and return the id its own manifest declares.
pub fn resolve(source: &Source, staging: &Path) -> Result<String, SourceError> {
    match source {
        Source::Git { url, reference } => clone_git(url, reference.as_deref(), staging),
        Source::Artifact(spec) => fetch_artifact(spec, staging),
    }
}

/// Clone a git repository at `reference` and return its manifest id. A local path is
/// cloned too (git is the installer), so both a URL and a path work.
fn clone_git(url: &str, reference: Option<&str>, staging: &Path) -> Result<String, SourceError> {
    let out = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .args(reference.map(|r| format!("--branch={r}")).into_iter())
        .arg(url)
        .arg(staging)
        .output()
        .map_err(|e| SourceError::Git(format!("could not run git: {e}")))?;
    if !out.status.success() {
        // A tag/commit that is not a branch needs a fetch + checkout: retry the full
        // clone, then check out the exact ref (a commit is a valid `ref`).
        let full = Command::new("git")
            .arg("clone")
            .arg(url)
            .arg(staging)
            .output()
            .map_err(|e| SourceError::Git(format!("could not run git: {e}")))?;
        if !full.status.success() {
            return Err(SourceError::Git(
                String::from_utf8_lossy(&full.stderr).trim().to_string(),
            ));
        }
        if let Some(r) = reference {
            let co = Command::new("git")
                .arg("-C")
                .arg(staging)
                .arg("checkout")
                .arg(r)
                .output()
                .map_err(|e| SourceError::Git(format!("could not run git: {e}")))?;
            if !co.status.success() {
                return Err(SourceError::Git(
                    String::from_utf8_lossy(&co.stderr).trim().to_string(),
                ));
            }
        }
    }
    manifest_id(staging)
}

/// Download the release zip, verify **size first then sha256**, unpack it into
/// `staging`, and confirm the archive manifest declares the named id and pluginType.
fn fetch_artifact(spec: &ArtifactSpec, staging: &Path) -> Result<String, SourceError> {
    let want_sha = spec.sha256.trim().to_ascii_lowercase();
    if want_sha.len() != 64 || !want_sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(SourceError::Archive("artifact.sha256 must be 64 hex characters".into()));
    }
    let bytes = download(&spec.url)?;
    // Size FIRST (a truncated or 0-byte download is caught even before hashing).
    if let Some(expected) = spec.size {
        if bytes.len() as u64 != expected {
            return Err(SourceError::SizeMismatch { expected, got: bytes.len() as u64 });
        }
    }
    let got_sha = hex::encode(Sha256::digest(&bytes));
    if got_sha != want_sha {
        return Err(SourceError::DigestMismatch { expected: want_sha, got: got_sha });
    }
    // Only now is anything unpacked.
    unpack_zip(&bytes, staging)?;
    let (id, plugin_type) = archive_manifest(staging)?;
    if id != spec.id {
        return Err(SourceError::IdMismatch { expected: spec.id.clone(), got: id });
    }
    if let Some(want) = &spec.plugin_type {
        if plugin_type.as_deref() != Some(want.as_str()) {
            return Err(SourceError::TypeMismatch {
                expected: want.clone(),
                got: plugin_type.unwrap_or_default(),
            });
        }
    }
    Ok(id)
}

/// A blocking download of the whole artifact (the mature HTTP client).
///
/// `reqwest::blocking` refuses to build inside a tokio runtime, and the resolve
/// runs on a blocking-pool thread (which IS a tokio thread). So the download owns
/// a plain OS thread: it cannot be blocked by anyone else's runtime and cannot
/// perturb this one.
fn download(url: &str) -> Result<Vec<u8>, SourceError> {
    let url = url.to_string();
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(300))
                    .build()
                    .map_err(|e| SourceError::Download(e.to_string()))?;
                let resp = client
                    .get(&url)
                    .send()
                    .map_err(|e| SourceError::Download(e.to_string()))?;
                if !resp.status().is_success() {
                    return Err(SourceError::Download(format!("HTTP {}", resp.status())));
                }
                resp.bytes()
                    .map(|b| b.to_vec())
                    .map_err(|e| SourceError::Download(e.to_string()))
            })
            .join()
            .unwrap_or_else(|_| Err(SourceError::Download("download thread panicked".into())))
    })
}

/// Unpack a zip into `staging`, refusing any entry that would escape it, and
/// stripping a single common top-level directory (a release zip usually wraps
/// everything in one dir; the landing rule wants a dir with a manifest).
fn unpack_zip(bytes: &[u8], staging: &Path) -> Result<(), SourceError> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| SourceError::Archive(format!("zip: {e}")))?;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        if let Ok(e) = archive.by_index(i) {
            names.push(e.name().to_string());
        }
    }
    let root = common_root(&names);
    for i in 0..archive.len() {
        let mut entry =
            archive.by_index(i).map_err(|e| SourceError::Archive(format!("zip entry: {e}")))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(SourceError::Archive(format!(
                "archive entry escapes the archive"
            )));
        };
        let rel = match &root {
            Some(r) => match rel.strip_prefix(r) {
                Ok(p) => p.to_path_buf(),
                Err(_) => continue,
            },
            None => rel.to_path_buf(),
        };
        if rel.as_os_str().is_empty() {
            continue; // the wrapper dir itself
        }
        let out = staging.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
        }
    }
    Ok(())
}

/// The single top-level directory every entry shares, if there is one.
fn common_root(names: &[String]) -> Option<PathBuf> {
    let mut it = names
        .iter()
        .filter_map(|n| n.replace('\\', "/").split('/').next().map(str::to_string))
        .filter(|s| !s.is_empty());
    let first = it.next()?;
    if it.all(|s| s == first) {
        Some(PathBuf::from(first))
    } else {
        None
    }
}

fn manifest_id(dir: &Path) -> Result<String, SourceError> {
    let (id, _) = archive_manifest(dir)?;
    Ok(id)
}

/// Read the plugin tree own manifest (id, pluginType).
fn archive_manifest(dir: &Path) -> Result<(String, Option<String>), SourceError> {
    let raw = find_manifest(dir)
        .ok_or_else(|| SourceError::Archive("the tree declares no manifest.json".into()))?;
    let manifest = Manifest::parse(&raw).map_err(SourceError::Archive)?;
    let id = manifest
        .id
        .clone()
        .ok_or_else(|| SourceError::Archive("the manifest declares no id".into()))?;
    Ok((id, manifest.plugin_type.clone()))
}

/// Find `manifest.json` at the root, or one level down (a single wrapper dir).
fn find_manifest(dir: &Path) -> Option<String> {
    let root = dir.join("manifest.json");
    if root.is_file() {
        return std::fs::read_to_string(root).ok();
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let nested = e.path().join("manifest.json");
            if nested.is_file() {
                return std::fs::read_to_string(nested).ok();
            }
        }
    }
    None
}
