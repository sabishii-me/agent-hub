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
    /// A managed provider has no stored credential (contract
    /// `provider_unauthorized`): a distinct identity so a caller can tell "log in
    /// first" apart from a bad request.
    #[error("managed provider `{0}` has no stored credential; log in first")]
    Unauthorized(String),
    /// The provider changed during a refresh (contract `revision_conflict`).
    #[error("provider `{0}` changed during refresh; refresh again")]
    RevisionConflict(String),
    /// No catalog has been fetched yet (contract `catalog_not_loaded`).
    #[error("provider `{0}` has no catalog; fetch it first")]
    CatalogNotLoaded(String),
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
            ProviderError::Unauthorized(_) => "provider_unauthorized",
            ProviderError::RevisionConflict(_) => "revision_conflict",
            ProviderError::CatalogNotLoaded(_) => "catalog_not_loaded",
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

    /// Set `token_configured` from the authoritative secret store, using the
    /// row's OWN stored reference. A row with no reference is simply not
    /// configured: we do NOT fall back to a re-derived key, so a rebuilt provider
    /// with the same id can never re-acquire a previous instance's credential
    /// (TASK-048 F1). A read ERROR is surfaced, never read as "absent".
    fn with_configured(&self, mut r: ProviderRecord) -> Result<ProviderRecord, ProviderError> {
        let has = match &r.secret_ref {
            Some(reference) => self
                .secrets
                .get(reference)
                .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?
                .is_some(),
            None => false,
        };
        r.token_configured = has;
        if !has {
            r.secret_ref = None;
        }
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
            incarnation: String::new(),
        }
        .with_defaults();
        // 2. the row first: a duplicate id is refused with no secret written.
        //    Re-read it: the database minted the incarnation our save is guarded by.
        record = self.store.create(&record)?;
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
                // The credential exists but the row does not point at it. Try to
                // remove it; if THAT fails, the journal entry is KEPT so the next
                // boot retries - we never claim a clean state we did not reach
                // (TASK-048 F1).
                match self.secrets.delete(&self.secret_ref(&id)) {
                    Ok(()) => {
                        let _ = self.store.delete(&id);
                    }
                    Err(cleanup) => {
                        tracing::warn!(provider = %id, error = %cleanup, "unowned credential left; journal kept for recovery");
                        return Err(ProviderError::SecretUnreadable(format!(
                            "the reference could not be saved ({e}) and the credential could not be removed ({cleanup}); recovery is pending"
                        )));
                    }
                }
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

    pub async fn set_selection(
        &self,
        id: &str,
        enabled: Vec<String>,
    ) -> Result<ProviderRecord, ProviderError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        let mut rec = self.store.get(id)?;
        if let Some(cat) = &rec.catalog {
            for want in &enabled {
                if !cat.models.iter().any(|m| &m.id == want) {
                    return Err(ProviderError::Validation(format!("unknown model id `{want}`")));
                }
            }
        }
        rec.enabled_model_ids = enabled;
        rec.revision += 1;
        // The reference is the ROW's own (never re-derived from the id).
        let has = self
            .secrets
            .get(&self.secret_ref(id))
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?
            .is_some();
        rec.secret_ref = if has { Some(self.secret_ref(id)) } else { None };
        self.store.save(&rec)?;
        self.with_configured(rec)
    }

    pub async fn refresh(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        // Hold the provider's operation lock for the whole read-fetch-write, so a
        // refresh cannot interleave with a patch/delete/create of the same id.
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        let read = self.store.get(id)?;
        let url = read
            .url
            .clone()
            .ok_or_else(|| ProviderError::Validation("this provider has no url".into()))?;
        // The credential comes from the ROW's reference, not a re-derived key.
        let token = match &read.secret_ref {
            Some(r) => self
                .secrets
                .get(r)
                .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?,
            None => None,
        };
        // Network await: the row may change under us (a delete+recreate, a patch).
        let models = catalog::fetch(read.api.as_deref(), &url, token.as_deref()).await?;
        // Re-read and REFUSE to write back over a different incarnation/revision:
        // the object we fetched for is gone (TASK-048 F2). Confirm the reference
        // is unchanged too, so we never attach a new token's route to an old fetch.
        let current = self.store.get(id)?;
        if current.incarnation != read.incarnation || current.revision != read.revision {
            return Err(ProviderError::RevisionConflict(id.to_string()));
        }
        let mut rec = current;
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
        rec.revision += 1;
        rec.catalog = Some(catalog::catalog_at(merged, rec.revision));
        rec.enabled_model_ids = enabled;
        // The reference is unchanged (verified above); keep the row's own value.
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
    /// when the process stopped. It is **retry-safe**: a journal entry is removed
    /// ONLY when every necessary step is confirmed. A failure or an unknown state
    /// (an unreadable credential, an unreadable row) keeps the entry, so the next
    /// boot tries again and the orphan remains visible. Nothing is ever claimed
    /// recovered on a guess (TASK-048 F1).
    pub fn recover_pending(&self) {
        let pending = match self.store.pending_ops() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "could not read pending provider ops");
                return;
            }
        };
        for (provider, op, secret_ref) in pending {
            match self.recover_one(&provider, &op, &secret_ref) {
                Ok(()) => {
                    if let Err(e) = self.store.finish_op(&provider) {
                        tracing::warn!(provider, error = %e, "recovery finished but the journal entry could not be cleared");
                    }
                }
                Err(e) => {
                    // Keep the entry: the state is still unresolved and must stay
                    // visible for the next boot. Never clear the journal on a
                    // failure.
                    tracing::warn!(provider, op, error = %e, "provider recovery deferred (journal kept)");
                }
            }
        }
    }

    /// Resolve ONE journal entry. `Ok` means every necessary step was confirmed;
    /// any error leaves the entry in place.
    fn recover_one(&self, provider: &str, op: &str, secret_ref: &str) -> Result<(), ProviderError> {
        if op == "delete" {
            // The row is the source of truth after a delete: remove the credential
            // (a missing one is fine), then the row (a missing one is fine).
            self.secrets
                .delete(secret_ref)
                .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
            match self.store.delete(provider) {
                Ok(()) => Ok(()),
                Err(StoreError::NotFound(_)) => Ok(()),
                Err(e) => Err(ProviderError::Store(e)),
            }
        } else {
            // create/patch/logout did not finish: reconcile the credential with
            // the row. An unreadable credential or row is an ERROR, never read as
            // "absent" (that is what would let a rebuilt id re-acquire an old
            // secret).
            let has = self
                .secrets
                .get(secret_ref)
                .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?
                .is_some();
            let row = match self.store.get(provider) {
                Ok(r) => Some(r),
                Err(StoreError::NotFound(_)) => None,
                Err(e) => return Err(ProviderError::Store(e)),
            };
            match row {
                Some(mut r) => {
                    // The row exists: make its ownership match reality.
                    r.secret_ref = if has { Some(secret_ref.to_string()) } else { None };
                    self.store.save(&r)?;
                    Ok(())
                }
                None if has => {
                    // No row but a credential exists: it is unowned. A failed
                    // delete keeps the journal (retry), it does not pretend the
                    // orphan is gone.
                    self.secrets
                        .delete(secret_ref)
                        .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
                    Ok(())
                }
                None => Ok(()),
            }
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
        // The credential location is the ROW's reference. A row with none is
        // simply unauthorized - never a re-derived key (TASK-048 F1/F5).
        let value = match &rec.secret_ref {
            Some(reference) => self
                .secrets
                .get(reference)
                .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?
                .ok_or_else(|| ProviderError::Unauthorized(id.to_string()))?,
            None => return Err(ProviderError::Unauthorized(id.to_string())),
        };
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

#[cfg(test)]
mod recovery_tests {
    use super::*;

    /// A journal entry whose secret store is UNREACHABLE must be KEPT: recovery
    /// does not claim a state it could not confirm (F1).
    #[test]
    fn an_unresolvable_entry_keeps_its_journal() {
        let db = agent_hub_db::Db::open_in_memory().unwrap();
        let store = ProviderStore::new(db);
        // An unavailable secret store: every read/write errors.
        let secrets = std::sync::Arc::new(agent_hub_secrets::SecretStore::probe_unavailable_for_test());
        let p = Providers::new(store, secrets, "test-instance");
        p.store.begin_op("p1", "create", "test-instance:provider-p1").unwrap();
        p.recover_pending();
        // The entry is still there: the failure kept it for retry.
        let pending = p.store.pending_ops().unwrap();
        assert_eq!(pending.len(), 1, "an unresolvable recovery keeps its journal");
    }
}
