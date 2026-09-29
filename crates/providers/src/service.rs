//! The providers domain (`ARCHITECTURE` §5): CRUD over the file store, the
//! model selection, and the catalog fetch. A provider is **data**; the hub owns
//! the request and the mapping.

use crate::catalog;
use crate::record::ProviderRecord;
use crate::store::{Broken, ProviderStore, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("catalog: {0}")]
    Catalog(#[from] catalog::CatalogError),
}

/// What POST /v1/hub/providers accepts.
#[derive(Debug, Default, serde::Deserialize)]
pub struct CreateProvider {
    pub id: Option<String>,
    pub label: Option<String>,
    pub url: Option<String>,
    pub api: Option<String>,
    pub token: Option<String>,
    pub declarations: Option<std::collections::BTreeMap<String, crate::record::Declaration>>,
    #[serde(rename = "providerType")]
    pub provider_type: Option<String>,
    #[serde(rename = "providerTypeVersion")]
    pub provider_type_version: Option<u32>,
}

/// What PATCH /v1/hub/providers/{id} accepts (all optional).
#[derive(Debug, Default, serde::Deserialize)]
pub struct PatchProvider {
    pub label: Option<Option<String>>,
    pub url: Option<Option<String>>,
    pub api: Option<Option<String>>,
    pub token: Option<Option<String>>,
    pub declarations: Option<std::collections::BTreeMap<String, crate::record::Declaration>>,
}

pub struct Providers {
    pub store: ProviderStore,
}

impl Providers {
    pub fn new(store: ProviderStore) -> Self {
        Providers { store }
    }

    pub fn list(&self) -> Result<(Vec<ProviderRecord>, Vec<Broken>), ProviderError> {
        Ok(self.store.list()?)
    }

    pub fn get(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        Ok(self.store.get(id)?)
    }

    pub fn create(&self, req: CreateProvider) -> Result<ProviderRecord, ProviderError> {
        let id = req.id.unwrap_or_else(|| new_id("prov"));
        let record = ProviderRecord {
            id,
            label: req.label,
            url: req.url,
            api: req.api,
            token: req.token,
            provider_type: req.provider_type,
            provider_type_version: req.provider_type_version,
            declarations: req.declarations.unwrap_or_default(),
            enabled_model_ids: Vec::new(),
            revision: 1,
            catalog: None,
        }
        .with_defaults();
        self.store.create(&record)?;
        Ok(record)
    }

    /// Patch a provider. Changing url, api or the credential **bumps the
    /// revision**, so the cached catalog reports stale until refreshed.
    pub fn patch(&self, id: &str, req: PatchProvider) -> Result<ProviderRecord, ProviderError> {
        let mut rec = self.store.get(id)?;
        let mut bump = false;
        if let Some(v) = req.label {
            rec.label = v;
        }
        if let Some(v) = req.url {
            if rec.url != v {
                bump = true;
            }
            rec.url = v;
        }
        if let Some(v) = req.api {
            if rec.api != v {
                bump = true;
            }
            rec.api = v;
        }
        if let Some(v) = req.token {
            if rec.token != v {
                bump = true;
            }
            rec.token = v;
        }
        if let Some(v) = req.declarations {
            rec.declarations = v;
        }
        if bump {
            rec.revision += 1;
        }
        self.store.save(&rec)?;
        Ok(rec)
    }

    pub fn delete(&self, id: &str) -> Result<(), ProviderError> {
        self.store.delete(id)?;
        Ok(())
    }

    /// Drop the stored credential and keep the row.
    pub fn logout(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        let mut rec = self.store.get(id)?;
        if rec.token.is_some() {
            rec.token = None;
            rec.revision += 1;
        }
        self.store.save(&rec)?;
        Ok(rec)
    }

    /// Replace the enabled selection. Unknown ids are refused; `[]` disables all.
    pub fn set_selection(
        &self,
        id: &str,
        enabled: Vec<String>,
    ) -> Result<ProviderRecord, ProviderError> {
        let mut rec = self.store.get(id)?;
        if let Some(catalog) = &rec.catalog {
            for want in &enabled {
                if !catalog.models.iter().any(|m| &m.id == want) {
                    return Err(ProviderError::Validation(format!(
                        "unknown model id `{want}`"
                    )));
                }
            }
        }
        rec.enabled_model_ids = enabled;
        self.store.save(&rec)?;
        Ok(rec)
    }

    /// Refresh the catalog. First/new models are enabled; previous choices are
    /// preserved; removed models are retained as unavailable; a failure keeps
    /// the previous catalog.
    pub async fn refresh(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        let mut rec = self.store.get(id)?;
        let url = rec
            .url
            .clone()
            .ok_or_else(|| ProviderError::Validation("this provider has no url".into()))?;
        let models = catalog::fetch(rec.api.as_deref(), &url, rec.token.as_deref()).await?;

        let previous_enabled = rec.enabled_model_ids.clone();
        let previous_models = rec.catalog.as_ref().map(|c| c.models.clone()).unwrap_or_default();

        // Merge: new models first, then removed ones kept as unavailable.
        let mut merged = models.clone();
        for old in &previous_models {
            if !merged.iter().any(|m| m.id == old.id) {
                merged.push(old.clone());
            }
        }

        // First refresh enables everything; later refreshes keep the choice and
        // enable genuinely new models.
        let mut enabled = previous_enabled.clone();
        if previous_enabled.is_empty() && rec.catalog.is_none() {
            enabled = merged.iter().map(|m| m.id.clone()).collect();
        } else {
            for m in &models {
                if !previous_models.iter().any(|o| o.id == m.id) && !enabled.contains(&m.id) {
                    enabled.push(m.id.clone());
                }
            }
        }

        rec.catalog = Some(catalog::catalog_at(merged, rec.revision));
        rec.enabled_model_ids = enabled;
        self.store.save(&rec)?;
        Ok(rec)
    }
}

fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{prefix}-{millis:x}-{:x}", C.fetch_add(1, Ordering::Relaxed))
}
