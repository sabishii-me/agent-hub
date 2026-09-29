//! An adapter plugin's `manifest.json` (`contract/adapter-v1.json`, `manifest`).
//!
//! A plugin is a directory; the directory name is its id. The manifest's
//! `protocol` must equal the adapter contract's version, or the hub refuses to
//! start the adapter (`harness_invalid`).

use serde::Deserialize;

/// The adapter protocol version this hub speaks.
pub const ADAPTER_PROTOCOL_VERSION: u64 = 0;

#[derive(Debug, Clone, Deserialize)]
pub struct AdapterManifest {
    pub id: Option<String>,
    #[serde(rename = "pluginType")]
    pub plugin_type: Option<String>,
    pub protocol: Option<u64>,
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Harness-side extensions this plugin ships (registry defaults).
    #[serde(default)]
    pub extensions: Vec<String>,
    pub name: Option<String>,
    pub base: Option<String>,
    pub version: Option<String>,
    pub runtime: Option<RuntimeSpec>,
    pub permission_model: Option<String>,
    pub plan_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeSpec {
    pub package: Option<String>,
    pub version: Option<String>,
    /// The harness argv (`["node","runtime/dist/cli.js"]`). The hub resolves the
    /// parts against the plugin directory and hands the absolute argv to the
    /// adapter as `AGENT_HUB_RUNTIME_COMMAND` (`contract/adapter-v1.json`).
    pub command: Option<Vec<String>>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("no manifest.json")]
    Missing,
    #[error("manifest is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("the manifest claims id `{claimed}` but the directory is `{directory}`")]
    IdMismatch { claimed: String, directory: String },
    #[error("protocol {claimed} is not the hub's adapter protocol {expected}")]
    ProtocolMismatch { claimed: u64, expected: u64 },
    #[error("the manifest declares no command")]
    NoCommand,
}

impl AdapterManifest {
    /// Validate against a plugin directory named `directory`.
    pub fn load(dir: &std::path::Path, directory: &str) -> Result<Self, ManifestError> {
        let raw = std::fs::read_to_string(dir.join("manifest.json"))
            .map_err(|_| ManifestError::Missing)?;
        let m: AdapterManifest =
            serde_json::from_str(&raw).map_err(|e| ManifestError::InvalidJson(e.to_string()))?;
        if let Some(claimed) = &m.id {
            if claimed != directory {
                return Err(ManifestError::IdMismatch {
                    claimed: claimed.clone(),
                    directory: directory.to_string(),
                });
            }
        }
        if let Some(claimed) = m.protocol {
            if claimed != ADAPTER_PROTOCOL_VERSION {
                return Err(ManifestError::ProtocolMismatch {
                    claimed,
                    expected: ADAPTER_PROTOCOL_VERSION,
                });
            }
        }
        if m.command.as_ref().map(|c| c.is_empty()).unwrap_or(true) {
            return Err(ManifestError::NoCommand);
        }
        Ok(m)
    }

    pub fn declares(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// The absolute argv the hub hands to the adapter as
    /// `AGENT_HUB_RUNTIME_COMMAND`: the first part is kept, the rest are resolved
    /// against the plugin directory (the old hub did exactly this,
    /// `server.mjs:782`).
    pub fn runtime_argv(&self, dir: &std::path::Path) -> Option<Vec<String>> {
        let command = self.runtime.as_ref()?.command.as_ref()?;
        if command.is_empty() {
            return None;
        }
        Some(
            command
                .iter()
                .enumerate()
                .map(|(i, part)| {
                    if i == 0 {
                        part.clone()
                    } else {
                        dir.join(part).to_string_lossy().to_string()
                    }
                })
                .collect(),
        )
    }
}

/// An extension a plugin ships: an id (its `extensions/<id>` directory name).
#[derive(Debug, Clone)]
pub struct ExtensionEntry {
    pub id: String,
    pub dir: std::path::PathBuf,
}

impl AdapterManifest {
    /// The extensions this plugin ships: the directory names under its
    /// `extensions/`, or the `extensions` array the manifest declares. The union,
    /// because a manifest may declare ids that also have directories.
    pub fn shipped_extensions(&self, dir: &std::path::Path) -> Vec<ExtensionEntry> {
        use std::collections::BTreeMap;
        let mut by_id: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
        let ext_root = dir.join("extensions");
        if let Ok(entries) = std::fs::read_dir(&ext_root) {
            for e in entries.flatten() {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let id = e.file_name().to_string_lossy().to_string();
                    if !id.starts_with('.') {
                        by_id.insert(id, e.path());
                    }
                }
            }
        }
        for id in &self.extensions {
            by_id.entry(id.clone()).or_insert_with(|| ext_root.join(id));
        }
        by_id
            .into_iter()
            .map(|(id, dir)| ExtensionEntry { id, dir })
            .collect()
    }
}
