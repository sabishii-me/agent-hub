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
    /// A credential was supplied but there is no OS secret store to keep it, so
    /// it is refused rather than written in plaintext (TASK-048 F02).
    #[error("this hub has no secret store; a credential is not accepted and must not be stored in plaintext")]
    NoSecretStore,
}

impl ProviderError {
    /// The **contract** error code (`contract/errors.json`).
    pub fn code(&self) -> &'static str {
        match self {
            ProviderError::Store(StoreError::NotFound(_)) => "provider_not_found",
            ProviderError::Store(StoreError::Exists(_)) => "already_exists",
            ProviderError::Store(StoreError::Corrupt(_, _)) => "internal_error",
            ProviderError::Store(StoreError::Io(_)) => "internal_error",
            ProviderError::Validation(_) => "validation_failed",
            ProviderError::Catalog(_) => "provider_catalog_failed",
            ProviderError::NoSecretStore => "not_implemented",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// What POST /v1/model-providers accepts.
#[derive(Debug, Default, serde::Deserialize)]
pub struct CreateProvider {
    pub id: Option<String>,
    pub label: Option<String>,
    pub url: Option<String>,
    pub api: Option<String>,
    /// A token is NOT accepted until an OS secret store exists (TASK-048 F02):
    /// accepting one would write a plaintext secret to a file. Refused, not
    /// silently dropped.
    pub token: Option<String>,
    pub declarations: Option<std::collections::BTreeMap<String, crate::record::Declaration>>,
    #[serde(rename = "providerType")]
    pub provider_type: Option<String>,
    #[serde(rename = "providerTypeVersion")]
    pub provider_type_version: Option<u32>,
}

/// What PATCH /v1/model-providers/{id} accepts (all optional).
#[derive(Debug, Default, serde::Deserialize)]
pub struct PatchProvider {
    pub label: Option<Option<String>>,
    pub url: Option<Option<String>>,
    pub api: Option<Option<String>>,
    /// Refused until a secret store exists (see `CreateProvider::token`).
    pub token: Option<Option<String>>,
    pub declarations: Option<std::collections::BTreeMap<String, crate::record::Declaration>>,
}

pub struct Providers {
    pub store: ProviderStore,
    /// The OS secret store. A credential is kept here, never in the record file.
    pub secrets: std::sync::Arc<agent_hub_secrets::SecretStore>,
}

impl Providers {
    pub fn new(store: ProviderStore, secrets: std::sync::Arc<agent_hub_secrets::SecretStore>) -> Self {
        Providers { store, secrets }
    }

    /// The keychain key for a provider's credential.
    fn secret_key(id: &str) -> String {
        format!("provider-{id}-token")
    }

    pub fn list(&self) -> Result<(Vec<ProviderRecord>, Vec<Broken>), ProviderError> {
        Ok(self.store.list()?)
    }

    pub fn get(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        Ok(self.store.get(id)?)
    }

    pub fn create(&self, req: CreateProvider) -> Result<ProviderRecord, ProviderError> {
        if req.token.is_some() && !self.secrets.is_available() {
            return Err(ProviderError::NoSecretStore);
        }
        let id = req.id.unwrap_or_else(|| new_id("prov"));
        let token_configured = match &req.token {
            Some(t) => {
                self.secrets
                    .set(&Self::secret_key(&id), t)
                    .map_err(|_| ProviderError::NoSecretStore)?;
                true
            }
            None => false,
        };
        let record = ProviderRecord {
            id,
            label: req.label,
            url: req.url,
            api: req.api,
            token_configured,
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
        if let Some(tok) = req.token {
            match tok {
                Some(t) => {
                    self.secrets
                        .set(&Self::secret_key(id), &t)
                        .map_err(|_| ProviderError::NoSecretStore)?;
                    if !rec.token_configured {
                        rec.token_configured = true;
                    }
                    bump = true;
                }
                None => {
                    // Clearing the credential.
                    let _ = self.secrets.delete(&Self::secret_key(id));
                    rec.token_configured = false;
                    bump = true;
                }
            }
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
    /// Drop the stored credential and keep the row. There is no plaintext
    /// credential here (no secret store yet), so this is a no-op on the record;
    /// it exists for the contract shape. It never invents a token change.
    pub fn logout(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        let mut rec = self.store.get(id)?;
        self.secrets
            .delete(&Self::secret_key(id))
            .map_err(|_| ProviderError::NoSecretStore)?;
        rec.token_configured = false;
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
        // Authenticate with the stored credential when one is present.
        let token = self.secrets.get(&Self::secret_key(id)).ok().flatten();
        let models = catalog::fetch(rec.api.as_deref(), &url, token.as_deref()).await?;

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
