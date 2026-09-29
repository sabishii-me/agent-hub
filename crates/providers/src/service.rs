//! The providers domain (`ARCHITECTURE` §5): provider relationship state in the
//! database, the credential in the OS secret store (a reference, never a value),
//! the model selection and the catalog fetch.

use std::sync::Arc;

use agent_hub_secrets::SecretStore;

use crate::catalog;
use crate::record::ProviderRecord;
use crate::store::{ProviderStore, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("catalog: {0}")]
    Catalog(#[from] catalog::CatalogError),
    #[error("this hub has no secret store; a credential is refused rather than stored in plaintext")]
    NoSecretStore,
    #[error("the stored credential could not be read: {0}")]
    SecretUnreadable(String),
}

impl ProviderError {
    pub fn code(&self) -> &'static str {
        match self {
            ProviderError::Store(StoreError::NotFound(_)) => "provider_not_found",
            ProviderError::Store(StoreError::Exists(_)) => "already_exists",
            ProviderError::Store(StoreError::Db(_)) => "internal_error",
            ProviderError::Validation(_) => "validation_failed",
            ProviderError::Catalog(_) => "provider_catalog_failed",
            ProviderError::NoSecretStore => "not_implemented",
            ProviderError::SecretUnreadable(_) => "internal_error",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

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
    pub secrets: Arc<SecretStore>,
    /// The per-instance keychain namespace (a unique hub instance is isolated).
    pub namespace: String,
}

impl Providers {
    pub fn new(store: ProviderStore, secrets: Arc<SecretStore>, namespace: impl Into<String>) -> Self {
        Providers { store, secrets, namespace: namespace.into() }
    }

    /// The keychain reference for a provider's credential, scoped to this hub
    /// instance so instances never share a key.
    fn secret_ref(&self, id: &str) -> String {
        format!("{}:provider-{id}", self.namespace)
    }

    /// Set the derived `token_configured` from the authoritative secret store.
    /// A read ERROR (not "absent") is surfaced, never read as "not configured"
    /// (P1/P2: an unreadable credential is not a missing one).
    fn with_configured(&self, mut r: ProviderRecord) -> Result<ProviderRecord, ProviderError> {
        let has = match self.secrets.get(&self.secret_ref(&r.id)) {
            Ok(v) => v.is_some(),
            Err(e) => return Err(ProviderError::SecretUnreadable(e.to_string())),
        };
        r.token_configured = has;
        r.secret_ref = if has { Some(self.secret_ref(&r.id)) } else { None };
        Ok(r)
    }

    pub fn list(&self) -> Result<Vec<ProviderRecord>, ProviderError> {
        self.store
            .list()?
            .into_iter()
            .map(|r| self.with_configured(r))
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        self.with_configured(self.store.get(id)?)
    }

    /// Create a provider. Order (no side effect on a refusal):
    /// 1. refuse a credential when the store is unavailable;
    /// 2. insert the row (a duplicate id is refused HERE, before any secret write);
    /// 3. only then store the credential; a later failure rolls the row back.
    pub fn create(&self, req: CreateProvider) -> Result<ProviderRecord, ProviderError> {
        if req.token.is_some() && !self.secrets.is_available() {
            return Err(ProviderError::NoSecretStore);
        }
        let id = req.id.unwrap_or_else(|| new_id("prov"));
        let mut record = ProviderRecord {
            id: id.clone(),
            label: req.label,
            url: req.url,
            api: req.api,
            secret_ref: None,
            provider_type: req.provider_type,
            provider_type_version: req.provider_type_version,
            declarations: req.declarations.unwrap_or_default(),
            enabled_model_ids: Vec::new(),
            revision: 1,
            catalog: None,
            token_configured: false,
        }
        .with_defaults();
        // 2. the row first: a duplicate id is refused with no secret written.
        self.store.create(&record)?;
        // 3. the credential, into the keychain only.
        if let Some(t) = &req.token {
            if let Err(e) = self.secrets.set(&self.secret_ref(&id), t) {
                // Roll back the row so a failed credential does not leave a
                // half-created provider.
                let _ = self.store.delete(&id);
                return Err(ProviderError::SecretUnreadable(e.to_string()));
            }
            record.token_configured = true;
            record.secret_ref = Some(self.secret_ref(&id));
        }
        Ok(record)
    }

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
                    if !self.secrets.is_available() {
                        return Err(ProviderError::NoSecretStore);
                    }
                    self.secrets
                        .set(&self.secret_ref(id), &t)
                        .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
                    bump = true;
                }
                None => {
                    self.secrets
                        .delete(&self.secret_ref(id))
                        .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
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
        rec.secret_ref = None; // derived, not persisted
        self.store.save(&rec)?;
        self.with_configured(rec)
    }

    /// Delete the provider AND its stored credential.
    pub fn delete(&self, id: &str) -> Result<(), ProviderError> {
        self.secrets
            .delete(&self.secret_ref(id))
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
        self.store.delete(id)?;
        Ok(())
    }

    /// Drop the credential, keep the row.
    pub fn logout(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        let rec = self.store.get(id)?;
        self.secrets
            .delete(&self.secret_ref(id))
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
        self.with_configured(rec)
    }

    pub fn set_selection(
        &self,
        id: &str,
        enabled: Vec<String>,
    ) -> Result<ProviderRecord, ProviderError> {
        let mut rec = self.store.get(id)?;
        if let Some(cat) = &rec.catalog {
            for want in &enabled {
                if !cat.models.iter().any(|m| &m.id == want) {
                    return Err(ProviderError::Validation(format!("unknown model id `{want}`")));
                }
            }
        }
        rec.enabled_model_ids = enabled;
        rec.secret_ref = None;
        self.store.save(&rec)?;
        self.with_configured(rec)
    }

    pub async fn refresh(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        let mut rec = self.store.get(id)?;
        let url = rec
            .url
            .clone()
            .ok_or_else(|| ProviderError::Validation("this provider has no url".into()))?;
        let token = self
            .secrets
            .get(&self.secret_ref(id))
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
        let models = catalog::fetch(rec.api.as_deref(), &url, token.as_deref()).await?;
        let previous_enabled = rec.enabled_model_ids.clone();
        let previous_models = rec.catalog.as_ref().map(|c| c.models.clone()).unwrap_or_default();
        let mut merged = models.clone();
        for old in &previous_models {
            if !merged.iter().any(|m| m.id == old.id) {
                merged.push(old.clone());
            }
        }
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
        rec.secret_ref = None;
        self.store.save(&rec)?;
        self.with_configured(rec)
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
