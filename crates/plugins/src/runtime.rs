//! Installing a harness runtime FROM THE HUB.
//!
//! The hub owns this. The plugin only DECLARES its runtime: the manifest pins
//! `runtime.{package,version,command}` and the artifact ships `runtime.sources.json`, the closure
//! resolved ONCE on a release machine (by a package manager, where a resolver belongs) and recorded
//! as the official tarballs it is: a URL the vendor publishes, plus the `integrity` value npm
//! itself would verify.
//!
//! Installing is therefore mechanical, and needs NO package manager and NO system node toolchain:
//!
//!   download -> verify integrity -> unpack -> place -> swap in
//!
//! A plugin cannot reach outside this: it cannot run a package manager, cannot touch the system,
//! and cannot make the install depend on what the machine happens to have. The hub supervises it.
//!
//! Platform-specific sources are the vendors' own metadata (`os`, `cpu`), filtered here exactly as
//! npm filters them, so one recorded closure serves every platform.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// A plugin's `runtime.sources.json`: the recorded closure.
#[derive(Debug, Deserialize)]
pub struct RuntimeClosure {
    pub package: String,
    pub version: String,
    /// The directory (relative to the plugin root) the closure unpacks into, e.g. `runtime`.
    #[serde(default)]
    pub target: Option<String>,
    /// How many leading path components to strip from each tarball (npm's `strip`, usually 1).
    #[serde(default)]
    pub strip: Option<usize>,
    pub sources: Vec<RuntimeSource>,
}

#[derive(Debug, Deserialize)]
pub struct RuntimeSource {
    pub url: String,
    /// npm's `integrity`: `sha512-<base64>`.
    pub integrity: String,
    /// Where inside the target this source lands (empty = the target root).
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub os: Option<Vec<String>>,
    #[serde(default)]
    pub cpu: Option<Vec<String>>,
    #[serde(default)]
    pub optional: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("the plugin ships no runtime.sources.json; the hub cannot install a runtime it does not know")]
    NoClosure,
    #[error("runtime.sources.json is invalid: {0}")]
    BadClosure(String),
    #[error("download {url}: {detail}")]
    Download { url: String, detail: String },
    #[error("integrity mismatch for {url}: expected {expected}, got {got}")]
    Integrity { url: String, expected: String, got: String },
    #[error("could not unpack {url}: {detail}")]
    Unpack { url: String, detail: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl RuntimeError {
    /// A contract error code (see `contract/errors.json`).
    pub fn code(&self) -> &'static str {
        "plugin_install_failed"
    }
}

/// The platform key npm would use for `os`/`cpu` filtering, e.g. `win32`, `x64`.
fn host_os() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other, // linux
    }
}
fn host_cpu() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    }
}

/// npm's platform filter: an entry with `os`/`cpu` that do not list this host is skipped.
fn applies(src: &RuntimeSource) -> bool {
    if let Some(os) = &src.os {
        if !os.iter().any(|o| o == host_os()) {
            return false;
        }
    }
    if let Some(cpu) = &src.cpu {
        if !cpu.iter().any(|c| c == host_cpu()) {
            return false;
        }
    }
    true
}

/// Read and parse `<plugin>/runtime.sources.json`.
pub fn read_closure(plugin_dir: &Path) -> Result<RuntimeClosure, RuntimeError> {
    let p = plugin_dir.join("runtime.sources.json");
    let raw = std::fs::read_to_string(&p).map_err(|_| RuntimeError::NoClosure)?;
    serde_json::from_str(&raw).map_err(|e| RuntimeError::BadClosure(e.to_string()))
}

/// Materialise the closure into `<plugin>/<target>` (default `runtime`).
///
/// Every source is downloaded, its `integrity` verified, and its tarball unpacked into a fresh
/// staging dir under the target with `strip` leading components removed and `path` prepended. Only
/// after EVERY source succeeds is the staging swapped into place, so a failed install leaves the
/// previous runtime untouched.
pub fn install(plugin_dir: &Path, closure: &RuntimeClosure) -> Result<PathBuf, RuntimeError> {
    let target_rel = closure.target.clone().unwrap_or_else(|| "runtime".into());
    let target = plugin_dir.join(&target_rel);
    let strip = closure.strip.unwrap_or(1);

    let parent = target.parent().unwrap_or(plugin_dir);
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".{}-staging-{}", target_rel, std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    let result = (|| -> Result<(), RuntimeError> {
        // Downloads are IO-bound and independent: fetch them BOUNDED-PARALLEL so 146 tarballs do not
        // go one-at-a-time. Unpacking then runs SERIALLY into the shared staging dir (two tarballs
        // may create the same parent directories, so unpack order must not race). Verification
        // happens as each download completes, so a bad byte never reaches the disk.
        let sources: Vec<&RuntimeSource> = closure.sources.iter().filter(|s| applies(s)).collect();
        let limit = download_concurrency().min(sources.len().max(1));
        let fetched = parallel_download(&sources, limit)?;
        for (src, bytes) in sources.iter().zip(fetched) {
            let dest = match src.path.as_deref() {
                Some(p) if !p.is_empty() => staging.join(p),
                _ => staging.clone(),
            };
            std::fs::create_dir_all(&dest)?;
            unpack_tgz(&src.url, &bytes, &dest, strip)?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            // Swap in: replace the whole target with the verified staging.
            let old = parent.join(format!(".{}-old-{}", target_rel, std::process::id()));
            let _ = std::fs::remove_dir_all(&old);
            if target.exists() {
                std::fs::rename(&target, &old)?;
            }
            std::fs::rename(&staging, &target)?;
            let _ = std::fs::remove_dir_all(&old);
            Ok(target)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// How many tarballs to fetch at once. The hub's own scheduling (IO-bound; the number is not a user
/// setting). `AGENT_HUB_FETCH_CONCURRENCY` overrides; default 8.
fn download_concurrency() -> usize {
    std::env::var("AGENT_HUB_FETCH_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(8)
}

/// Download every source with at most `limit` in flight, verifying each. The result is in SOURCE
/// order, so the caller can unpack serially without a lookaside. The first failure stops the pool.
fn parallel_download(sources: &[&RuntimeSource], limit: usize) -> Result<Vec<Vec<u8>>, RuntimeError> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    let n = sources.len();
    let out_shared: Mutex<Vec<Option<Vec<u8>>>> = Mutex::new((0..n).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    let error: Mutex<Option<RuntimeError>> = Mutex::new(None);

    std::thread::scope(|scope| {
        for _ in 0..limit {
            scope.spawn(|| loop {
                // If an earlier worker failed, stop taking new work.
                if error.lock().map(|e| e.is_some()).unwrap_or(true) {
                    return;
                }
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= n {
                    return;
                }
                let src = sources[i];
                let got = download(&src.url).and_then(|b| {
                    verify_integrity(&src.url, &src.integrity, &b)?;
                    Ok(b)
                });
                match got {
                    Ok(b) => {
                        if let Ok(mut slot) = out_shared.lock() {
                            slot[i] = Some(b);
                        }
                    }
                    Err(e) => {
                        if let Ok(mut err) = error.lock() {
                            if err.is_none() {
                                *err = Some(e);
                            }
                        }
                        return;
                    }
                }
            });
        }
    });

    if let Ok(mut err) = error.lock() {
        if let Some(e) = err.take() {
            return Err(e);
        }
    }
    Ok(out_shared
        .into_inner()
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.unwrap_or_default())
        .collect())
}

/// A blocking download on its own OS thread (the same shape `source.rs` uses: it cannot be blocked
/// by, or perturb, the tokio runtime).
fn download(url: &str) -> Result<Vec<u8>, RuntimeError> {
    let url = url.to_string();
    let fallback = url.clone();
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(900))
                    .build()
                    .map_err(|e| RuntimeError::Download { url: url.clone(), detail: e.to_string() })?;
                let resp = client
                    .get(&url)
                    .send()
                    .map_err(|e| RuntimeError::Download { url: url.clone(), detail: e.to_string() })?;
                if !resp.status().is_success() {
                    return Err(RuntimeError::Download { url: url.clone(), detail: format!("HTTP {}", resp.status()) });
                }
                resp.bytes()
                    .map(|b| b.to_vec())
                    .map_err(|e| RuntimeError::Download { url: url.clone(), detail: e.to_string() })
            })
            .join()
            .unwrap_or_else(|_| Err(RuntimeError::Download { url: fallback.clone(), detail: "download thread panicked".into() }))
    })
}

/// Verify npm's `integrity` (`sha512-<base64>`, or `sha256-...`). The algorithm is taken from the
/// string, so a future closure is not assumed to be sha512.
fn verify_integrity(url: &str, integrity: &str, bytes: &[u8]) -> Result<(), RuntimeError> {
    let (alg, want_b64) = integrity
        .split_once('-')
        .ok_or_else(|| RuntimeError::BadClosure(format!("integrity `{integrity}` has no algorithm")))?;
    use base64::Engine;
    use sha2::Digest;
    let got_b64 = match alg {
        "sha512" => base64::engine::general_purpose::STANDARD.encode(sha2::Sha512::digest(bytes)),
        "sha256" => base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(bytes)),
        other => return Err(RuntimeError::BadClosure(format!("unsupported integrity algorithm `{other}`"))),
    };
    // npm's base64 may omit padding; normalise BOTH sides (strip padding) before comparing.
    let norm = |s: &str| s.trim_end_matches('=').to_string();
    if norm(&got_b64) != norm(want_b64) {
        return Err(RuntimeError::Integrity {
            url: url.to_string(),
            expected: want_b64.to_string(),
            got: got_b64,
        });
    }
    Ok(())
}

/// Unpack a `.tgz` (npm tarball) into `dest`, stripping `strip` leading path components.
fn unpack_tgz(url: &str, bytes: &[u8], dest: &Path, strip: usize) -> Result<(), RuntimeError> {
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
    let mut ar = tar::Archive::new(gz);
    ar.set_preserve_permissions(true);
    ar.set_preserve_mtime(false);
    let entries = ar
        .entries()
        .map_err(|e| RuntimeError::Unpack { url: url.into(), detail: e.to_string() })?;
    for entry in entries {
        let mut entry = entry.map_err(|e| RuntimeError::Unpack { url: url.into(), detail: e.to_string() })?;
        let raw = entry
            .path()
            .map_err(|e| RuntimeError::Unpack { url: url.into(), detail: e.to_string() })?
            .to_path_buf();
        // Strip the leading components (npm tarballs wrap everything in `package/`).
        let comps: Vec<_> = raw.components().collect();
        if comps.len() <= strip {
            continue;
        }
        let rel: PathBuf = comps[strip..].iter().collect();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        // Refuse a traversal (`..`) or an absolute path.
        if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(RuntimeError::Unpack { url: url.into(), detail: format!("refused a path outside the target: {rel:?}") });
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry
            .unpack(&out)
            .map_err(|e| RuntimeError::Unpack { url: url.into(), detail: e.to_string() })?;
    }
    Ok(())
}
