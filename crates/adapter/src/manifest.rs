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
    pub command: Option<String>,
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
}
