//! A plugin's `manifest.json` and the contract's `plugin` view.

use serde::{Deserialize, Serialize};

/// What a plugin's `manifest.json` declares. Only the fields the hub reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Manifest {
    pub id: Option<String>,
    pub name: Option<String>,
    pub summary: Option<String>,
    /// The plugin's own version (a release names it in the artifact).
    pub version: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(rename = "pluginType")]
    pub plugin_type: Option<String>,
    pub runtime: Option<RuntimeSpec>,
    /// The minimum HUB version this plugin needs (ADR-0008); below it the hub does
    /// NOT accept/activate the plugin.
    #[serde(rename = "minHubVersion")]
    pub min_hub_version: Option<String>,
}

/// The runtime a manifest pins (`runtime/package` + `.version` + `.target`).
#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeSpec {
    pub package: Option<String>,
    pub version: Option<String>,
    pub target: Option<String>,
}

impl Manifest {
    /// Parse a manifest, returning an error string when it cannot be honoured.
    pub fn parse(raw: &str) -> Result<Self, String> {
        serde_json::from_str(raw).map_err(|e| format!("manifest is not valid JSON: {e}"))
    }

    /// Why the manifest cannot be honoured, or `None`.
    pub fn invalid_reason(&self) -> Option<String> {
        match self.plugin_type.as_deref() {
            Some("harness-adapter") | Some("model-provider") => None,
            Some(other) => Some(format!("unknown pluginType `{other}`")),
            None => Some("manifest declares no pluginType".into()),
        }
    }
}

/// The contract's `plugin` object (GET /v1/plugins items).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginView {
    pub id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub capabilities: Vec<String>,
    #[serde(rename = "pluginType")]
    pub plugin_type: String,
    pub origin: String,
    pub path: String,
    pub source: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub commit: Option<String>,
    #[serde(rename = "installedAt")]
    pub installed_at: Option<String>,
    pub runtime: Option<serde_json::Value>,
    #[serde(rename = "runtimeReady")]
    pub runtime_ready: bool,
    pub state: String,
    pub prepare: Option<PrepareInfo>,
    pub invalid: Option<String>,
    pub artifact: Option<serde_json::Value>,
}

/// `{detail, startedAt, finishedAt}` of the last runtime prepare.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrepareInfo {
    pub detail: Option<String>,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(rename = "finishedAt")]
    pub finished_at: Option<String>,
}
