//! The harnesses domain (`ARCHITECTURE` §3): a thin top-level projection of the
//! adapter registry, plus capability-gated runtime answers routed to the adapter.
//!
//! The rule the contract states twice: `known: false` when the harness declares
//! no such capability - **never a faked empty list**. A harness is registered on
//! first sight (a present adapter plugin).

use std::sync::Arc;

use agent_hub_adapter::{AdapterError, Adapters, HarnessStatus};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("harness `{0}` not found")]
    NotFound(String),
    #[error("this harness does not support `{0}`")]
    Unsupported(String),
    #[error("adapter: {0}")]
    Adapter(#[from] AdapterError),
}

impl HarnessError {
    /// The **contract** error code (`contract/errors.json`).
    pub fn code(&self) -> &'static str {
        match self {
            HarnessError::NotFound(_) => "harness_not_found",
            HarnessError::Unsupported(_) => "unsupported",
            HarnessError::Adapter(AdapterError::NotFound(_)) => "harness_not_found",
            HarnessError::Adapter(AdapterError::Unsupported(_)) => "unsupported",
            HarnessError::Adapter(_) => "adapter_unreachable",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// The harnesses domain over the adapter registry.
pub struct Harnesses {
    pub adapters: Arc<Adapters>,
}

impl Harnesses {
    pub fn new(adapters: Arc<Adapters>) -> Self {
        Harnesses { adapters }
    }

    /// The `harness` projection for one registered harness.
    pub fn view(&self, id: &str) -> Result<Value, HarnessError> {
        let h = self.adapters.get(id).map_err(HarnessError::Adapter)?;
        let m = &h.manifest;
        Ok(json!({
            "id": h.id,
            "name": m.name.clone().unwrap_or_else(|| h.id.clone()),
            "base": m.base.clone().unwrap_or_else(|| "native".into()),
            "adapterVersion": m.version,
            "runtimeVersion": m.runtime.as_ref().and_then(|r| r.version.clone()),
            "runtimePackage": m.runtime.as_ref().and_then(|r| r.package.clone()),
            "capabilities": m.capabilities,
            "status": h.status.as_str(),
            "presetId": null,
            "appliedPreset": null,
        }))
    }

    pub fn list(&self) -> Vec<Value> {
        self.adapters
            .list()
            .iter()
            .map(|h| self.view(&h.id).unwrap_or_else(|_| json!({ "id": h.id })))
            .collect()
    }

    /// The management view (`GET /v1/harnesses`): the same registry plus the
    /// installed extensions a harness ships.
    pub fn management(&self) -> Value {
        let harnesses: Vec<Value> = self
            .adapters
            .list()
            .iter()
            .map(|h| {
                json!({
                    "id": h.id,
                    "status": h.status.as_str(),
                    "capabilities": h.manifest.capabilities,
                    "runtimeVersion": h.manifest.runtime.as_ref().and_then(|r| r.version.clone()),
                    "runtimePackage": h.manifest.runtime.as_ref().and_then(|r| r.package.clone()),
                })
            })
            .collect();
        // The union of extension ids across installed plugins: the only
        // extensions a harness may be given (the contract's rule).
        let mut extensions: Vec<String> = Vec::new();
        for h in self.adapters.list() {
            for e in h.manifest.shipped_extensions(&h.directory) {
                extensions.push(e.id);
            }
        }
        extensions.sort();
        extensions.dedup();
        json!({ "harnesses": harnesses, "availableExtensions": extensions })
    }

    pub fn set_status(&self, id: &str, status: HarnessStatus) -> Result<Value, HarnessError> {
        self.adapters
            .set_status(id, status)
            .map_err(HarnessError::Adapter)?;
        self.view(id)
    }

    /// The capability-gated answer shape: `known`, the list, and the extras.
    async fn gated(
        &self,
        id: &str,
        capability: &str,
        method: &str,
        list_key: &str,
    ) -> Result<Value, HarnessError> {
        let harness = self.adapters.get(id).map_err(HarnessError::Adapter)?;
        if !harness.manifest.declares(capability) {
            return Ok(json!({ "harnessId": id, "known": false, list_key: [] }));
        }
        let reply = self
            .adapters
            .call(id, capability, method, json!({}))
            .await
            .map_err(HarnessError::Adapter)?;
        // The adapter's own answer already carries harnessId/known/list.
        Ok(reply)
    }

    /// The extension ids this harness may be given (the plugin's shipped set),
    /// harness-scoped (a bare id is meaningless across harnesses).
    pub fn extensions(&self, id: &str) -> Result<Value, HarnessError> {
        let h = self.adapters.get(id).map_err(HarnessError::Adapter)?;
        let ids: Vec<String> = h
            .manifest
            .shipped_extensions(&h.directory)
            .into_iter()
            .map(|e| e.id)
            .collect();
        Ok(json!({ "harnessId": id, "available": ids }))
    }

    pub async fn presets(&self, id: &str) -> Result<Value, HarnessError> {
        self.gated(id, "presets", "presets/list", "presets").await
    }

    pub async fn models(&self, id: &str) -> Result<Value, HarnessError> {
        self.gated(id, "models", "models/list", "models").await
    }

    pub async fn tools(&self, id: &str) -> Result<Value, HarnessError> {
        self.gated(id, "tools", "tools/list", "tools").await
    }
}
