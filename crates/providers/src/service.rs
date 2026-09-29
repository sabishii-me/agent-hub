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
    /// The per-instance keychain namespace (a persistent hub instance id).
    pub namespace: String,
    /// One lock per provider id: operations that touch BOTH the row and the
    /// secret store are serialised, so a create cannot interleave with a delete
    /// and leave an orphan credential (TASK-048 P1). The DB mutex only protects a
    /// single SQL call; this covers the whole two-store operation.
    locks: std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Providers {
    pub fn new(store: ProviderStore, secrets: Arc<SecretStore>, namespace: impl Into<String>) -> Self {
        Providers {
            store,
            secrets,
            namespace: namespace.into(),
            locks: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// The operation lock for one provider id (a short lock to take it; the
    /// returned mutex is held across the whole two-store operation).
    fn lock_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .expect("provider locks")
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
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
        // The stored reference is the row's own record of where its credential
        // lives; the derived key is the same string (kept in sync above).
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
    pub async fn create(&self, req: CreateProvider) -> Result<ProviderRecord, ProviderError> {
        if req.token.is_some() && !self.secrets.is_available() {
            return Err(ProviderError::NoSecretStore);
        }
        let id = req.id.clone().unwrap_or_else(|| new_id("prov"));
        let lock = self.lock_for(&id);
        let _guard = lock.lock().await;
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
        // 3. the credential, into the keychain only, then PERSIST the reference so
        //    the row actually records where its credential lives.
        if let Some(t) = &req.token {
            // Record the transition BEFORE the credential write: a crash between
            // the two stores leaves a visible, recoverable marker.
            self.store.begin_op(&id, "create", &self.secret_ref(&id))?;
            if let Err(e) = self.secrets.set(&self.secret_ref(&id), t) {
                // Roll back the row so a failed credential does not leave a
                // half-created provider. If the rollback ITSELF fails, the error
                // says so - we do not claim a clean recovery we did not achieve.
                if let Err(rb) = self.store.delete(&id) {
                    return Err(ProviderError::SecretUnreadable(format!(
                        "the credential could not be stored ({e}); the half-created provider could not be rolled back ({rb})"
                    )));
                }
                return Err(ProviderError::SecretUnreadable(e.to_string()));
            }
            record.secret_ref = Some(self.secret_ref(&id));
            record.token_configured = true;
            // Persist the reference (so the row owns it, not just the return value).
            if let Err(e) = self.store.save(&record) {
                // The credential exists but the row does not point at it: remove
                // the credential rather than leave an unowned secret.
                let _ = self.secrets.delete(&self.secret_ref(&id));
                let _ = self.store.delete(&id);
                return Err(ProviderError::Store(e));
            }
        }
        self.store.finish_op(&id)?;
        Ok(record)
    }

    pub async fn patch(&self, id: &str, req: PatchProvider) -> Result<ProviderRecord, ProviderError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
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
                    self.store.begin_op(id, "patch", &self.secret_ref(id))?;
                    self.secrets
                        .set(&self.secret_ref(id), &t)
                        .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
                    bump = true;
                }
                None => {
                    self.store.begin_op(id, "patch", &self.secret_ref(id))?;
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
        // The reference is owned by the row and persisted with it (do NOT clear:
        // that is what made the "reference" column a lie).
        rec.token_configured = self.secrets.get(&self.secret_ref(id)).map(|v| v.is_some()).unwrap_or(false);
        rec.secret_ref = if rec.token_configured { Some(self.secret_ref(id)) } else { None };
        self.store.save(&rec)?;
        self.store.finish_op(id)?;
        self.with_configured(rec)
    }

    /// Delete the provider AND its stored credential.
    pub async fn delete(&self, id: &str) -> Result<(), ProviderError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        // Journal first: if the credential is removed but the row delete fails, the
        // boot sweep finishes the row deletion (no orphan credential).
        self.store.begin_op(id, "delete", &self.secret_ref(id))?;
        self.secrets
            .delete(&self.secret_ref(id))
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
        self.store.delete(id)?;
        self.store.finish_op(id)?;
        Ok(())
    }

    /// Drop the credential, keep the row.
    pub async fn logout(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        let mut rec = self.store.get(id)?;
        self.store.begin_op(id, "logout", &self.secret_ref(id))?;
        self.secrets
            .delete(&self.secret_ref(id))
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
        // The row no longer points at a credential.
        rec.secret_ref = None;
        self.store.save(&rec)?;
        self.store.finish_op(id)?;
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
        let has = self.secrets.get(&self.secret_ref(id)).map(|v| v.is_some()).unwrap_or(false);
        rec.secret_ref = if has { Some(self.secret_ref(id)) } else { None };
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
        let has = self.secrets.get(&self.secret_ref(id)).map(|v| v.is_some()).unwrap_or(false);
        rec.secret_ref = if has { Some(self.secret_ref(id)) } else { None };
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

impl Providers {
    /// The boot sweep: resolve every credential transition that was in flight
    /// when the process stopped. This is the "recoverable state" half of the
    /// cross-store story - an interrupted create/delete/patch is finished here
    /// rather than left as a silent orphan.
    ///
    /// * `create`/`patch`/`logout` never completed: the row is the source of truth.
    ///   If a credential exists but the row does not point at it, drop the
    ///   credential; if the row points at a credential that does not exist, clear
    ///   the reference. Either way the journal entry is removed.
    /// * `delete`: the credential was (probably) removed; finish the row delete.
    pub fn recover_pending(&self) {
        let pending = match self.store.pending_ops() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "could not read pending provider ops");
                return;
            }
        };
        for (provider, op, secret_ref) in pending {
            let _ = match op.as_str() {
                "delete" => {
                    let _ = self.secrets.delete(&secret_ref);
                    self.store.delete(&provider).map(|_| ()).or_else(|e| match e {
                        crate::store::StoreError::NotFound(_) => Ok(()),
                        other => Err(other),
                    })
                }
                _ => {
                    // A create/patch/logout that did not finish: reconcile the
                    // credential with the row.
                    let has = self.secrets.get(&secret_ref).map(|v| v.is_some()).unwrap_or(false);
                    let row = match self.store.get(&provider) {
                        Ok(r) => Some(r),
                        Err(_) => None,
                    };
                    match row {
                        Some(mut r) if has => {
                            r.secret_ref = Some(secret_ref.clone());
                            self.store.save(&r).map(|_| ())
                        }
                        Some(mut r) => {
                            r.secret_ref = None;
                            self.store.save(&r).map(|_| ())
                        }
                        // No row, but a credential exists: it is unowned -> remove.
                        None => self.secrets.delete(&secret_ref).map(|_| ()).map_err(|e| {
                            crate::store::StoreError::Db(agent_hub_db::DbError::Conflict(e.to_string()))
                        }),
                    }
                }
            };
            let _ = self.store.finish_op(&provider);
        }
    }
}

/// What the sessions domain needs to grant a provider to an adapter. Kept as a
/// plain struct here so `sessions` does not depend on `providers`.
pub struct ResolvedGrant {
    pub connection_id: String,
    pub value: String,
    pub url: Option<String>,
    pub api: Option<String>,
    pub declarations: Option<serde_json::Value>,
    pub requested_provider_id: String,
}

impl Providers {
    /// Resolve a hub-managed provider id into a grant: the row (endpoint,
    /// protocol, declarations) plus the credential READ FROM the secret store.
    /// This is the single boundary a managed provider crosses into a session.
    ///
    /// Refused (never silently defaulted) when:
    /// * the provider does not exist;
    /// * it has no url (an adapter cannot materialise an endpointless provider);
    /// * no credential is stored (a session would fail at the first turn anyway -
    ///   fail now, with a clear reason).
    pub fn resolve_grant(&self, id: &str) -> Result<ResolvedGrant, ProviderError> {
        let rec = self.store.get(id)?;
        let url = rec
            .url
            .clone()
            .ok_or_else(|| ProviderError::Validation(format!("provider `{id}` has no url")))?;
        let value = self
            .secrets
            .get(&self.secret_ref(id))
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?
            .ok_or_else(|| {
                ProviderError::Validation(format!(
                    "provider `{id}` has no stored credential; log in first"
                ))
            })?;
        let declarations = if rec.declarations.is_empty() {
            None
        } else {
            serde_json::to_value(&rec.declarations).ok()
        };
        Ok(ResolvedGrant {
            connection_id: id.to_string(),
            value,
            url: Some(url),
            api: rec.api.clone(),
            declarations,
            requested_provider_id: id.to_string(),
        })
    }
}
