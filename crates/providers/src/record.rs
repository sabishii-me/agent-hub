//! A **model provider is data** (`ARCHITECTURE` §5): one JSON file in the hub's
//! data dir (`providers/<id>.json`). The hub owns HTTP, auth, the catalog fetch
//! and the field mapping; **no plugin runs code inside the hub's process**.

use serde::{Deserialize, Serialize};

/// One declaration for a model id: what the model accepts. Sent to a harness at
/// session start; declarations override the catalog per field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Declaration {
    #[serde(rename = "name", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "thinkingLevels", skip_serializing_if = "Option::is_none")]
    pub thinking_levels: Option<Vec<String>>,
    #[serde(rename = "contextWindow", skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(rename = "maxTokens", skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<serde_json::Value>,
}

/// A stored provider record. The on-disk shape is exactly what the contract's
/// routes read and write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The endpoint. A type may own its endpoint; then this is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The wire protocol in the harness vocabulary (openai-completions, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// The credential. Stored separately in production; here the record is the
    /// source and the keyring is a later concern, so it is **never** returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "providerType")]
    pub provider_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "providerTypeVersion")]
    pub provider_type_version: Option<u32>,
    /// Per-model declarations, keyed by model id.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub declarations: std::collections::BTreeMap<String, Declaration>,
    /// The enabled selection. Stored here, **not** in the cached catalog, so a
    /// refresh never changes it.
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "enabledModelIds")]
    pub enabled_model_ids: Vec<String>,
    /// A revision bumped when url/api/credential change; the catalog reports
    /// `stale` until refreshed.
    #[serde(default)]
    pub revision: u64,
    /// The cached catalog, when one was fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<Catalog>,
}

/// The cached catalog for a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(rename = "fetchedAt")]
    pub fetched_at: Option<String>,
    /// The provider revision the catalog was fetched at.
    #[serde(default)]
    pub revision: u64,
    pub models: Vec<CatalogModel>,
}

/// One catalog entry: the vendor's facts, before declarations are applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_levels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<serde_json::Value>,
}

impl ProviderRecord {
    /// New records name `custom-compatible` v1 when nothing is given.
    pub fn with_defaults(mut self) -> Self {
        if self.provider_type.is_none() {
            self.provider_type = Some("custom-compatible".into());
        }
        if self.provider_type_version.is_none() {
            self.provider_type_version = Some(1);
        }
        self
    }

    /// The catalog is stale when it was fetched at a different revision.
    pub fn catalog_stale(&self) -> bool {
        match &self.catalog {
            None => true,
            Some(c) => c.revision != self.revision,
        }
    }

    /// The merged model list the harness receives: catalog facts, overridden by
    /// declarations, with `enabled` from the stored selection.
    pub fn merged_models(&self) -> Vec<serde_json::Value> {
        let models = self.catalog.as_ref().map(|c| c.models.as_slice()).unwrap_or(&[]);
        let mut out: Vec<serde_json::Value> = Vec::new();
        for m in models {
            let d = self.declarations.get(&m.id);
            let enabled = self.enabled_model_ids.iter().any(|e| e == &m.id);
            out.push(serde_json::json!({
                "id": m.id,
                "name": d.and_then(|d| d.name.clone()).or_else(|| m.name.clone()),
                "enabled": enabled,
                "available": true,
                "thinkingLevels": d.and_then(|d| d.thinking_levels.clone()).or_else(|| m.thinking_levels.clone()),
                "contextWindow": d.and_then(|d| d.context_window).or(m.context_window),
                "maxTokens": d.and_then(|d| d.max_tokens).or(m.max_tokens),
                "input": d.and_then(|d| d.input.clone()).or_else(|| m.input.clone()),
                "cost": d.and_then(|d| d.cost.clone()).or_else(|| m.cost.clone()),
            }));
        }
        out
    }
}
