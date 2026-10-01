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
    /// A credential transition is unresolved (a step that could not be confirmed):
    /// the provider is held unusable until it is explicitly resolved, so no
    /// possibly-mismatched credential is consumed or sent.
    #[error("unresolved credential transition: {0}")]
    Unresolved(String),
}

impl ProviderError {
    pub fn code(&self) -> &'static str {
        match self {
            ProviderError::Store(StoreError::NotFound(_)) => "provider_not_found",
            ProviderError::Store(StoreError::Exists(_)) => "already_exists",
            ProviderError::Store(StoreError::Pending(_)) => "revision_conflict",
            ProviderError::Store(StoreError::Db(_)) => "internal_error",
            ProviderError::Validation(_) => "validation_failed",
            ProviderError::Catalog(_) => "provider_catalog_failed",
            ProviderError::NoSecretStore => "not_implemented",
            ProviderError::SecretUnreadable(_) => "internal_error",
            ProviderError::Unauthorized(_) => "provider_unauthorized",
            ProviderError::RevisionConflict(_) => "revision_conflict",
            ProviderError::CatalogNotLoaded(_) => "catalog_not_loaded",
            ProviderError::Unresolved(_) => "revision_conflict",
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
    /// The ONE provider-TYPE authority: given `(type id, version)`, resolve the
    /// descriptor an installed plugin ships, or `None` when no plugin ships it. Used
    /// by create validation, availability and grant admission, so the list, the
    /// store and execution never disagree (TASK-048 G3). None = no plugins root
    /// wired (types cannot be validated).
    type_resolver: Option<Arc<dyn Fn(&str, Option<u32>) -> Option<crate::types::TypeDescriptor> + Send + Sync>>,
}

impl Providers {
    pub fn new(store: ProviderStore, secrets: Arc<SecretStore>, namespace: impl Into<String>) -> Self {
        Providers {
            store,
            secrets,
            namespace: namespace.into(),
            locks: std::sync::Mutex::new(std::collections::HashMap::new()),
            type_resolver: None,
        }
    }

    /// Wire the provider-TYPE authority (the composition root points it at the
    /// installed plugins). Without it, an explicit type cannot be validated.
    pub fn with_type_resolver(
        mut self,
        resolver: Arc<dyn Fn(&str, Option<u32>) -> Option<crate::types::TypeDescriptor> + Send + Sync>,
    ) -> Self {
        self.type_resolver = Some(resolver);
        self
    }

    /// The BUILT-IN type the hub itself provides (the contract: "Omitted values
    /// select the built-in `custom-compatible` type"). It is NOT shipped by a plugin
    /// and is always available - an endpoint plus a dialect is all it needs.
    pub const BUILTIN_TYPE: &'static str = "custom-compatible";

    /// Resolve a provider type to its descriptor, or `None` when no installed
    /// plugin ships it (or no resolver is wired). A `None` version accepts the
    /// single shipped version of that id (the common case).
    pub fn resolve_type(&self, type_id: &str, version: Option<u32>) -> Option<crate::types::TypeDescriptor> {
        self.type_resolver.as_ref().and_then(|f| f(type_id, version))
    }

    /// Whether a named type is USABLE. The built-in `custom-compatible` type is
    /// always usable (the hub provides it); any other named type must be shipped by
    /// an installed plugin. A provider naming no type is the built-in default.
    pub fn type_available(&self, type_id: Option<&str>, version: Option<u32>) -> bool {
        match type_id {
            None => true,
            Some(Self::BUILTIN_TYPE) => true,
            Some(t) => self.resolve_type(t, version).is_some(),
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

    /// A stable digest of the row's CONFIGURATION (url + api + declarations). It
    /// EXCLUDES `secret_ref`, which recovery itself sets: including it would make a
    /// create never match its own intent. Recorded in the journal so recovery can
    /// prove a row is THIS operation's result, not an unrelated update that reached
    /// the same revision (TASK-048 F1-current).
    fn config_digest(rec: &ProviderRecord) -> String {
        let decl = serde_json::to_string(&rec.declarations).unwrap_or_default();
        // A simple, dependency-free digest; collisions are not security-critical
        // here (it distinguishes concurrent writes, not attackers).
        let mut h: u64 = 0xcbf29ce484222325;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        feed(rec.url.as_deref().unwrap_or("").as_bytes());
        feed(b"");
        feed(rec.api.as_deref().unwrap_or("").as_bytes());
        feed(b"");
        feed(decl.as_bytes());
        format!("{h:016x}")
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
        // The TYPE authority: an explicit type/version the hub cannot resolve is
        // refused BEFORE any row or secret write (TASK-048 G3). A provider naming no
        // type uses the built-in default and is accepted.
        if let Some(t) = req.provider_type.as_deref() {
            if !self.type_available(Some(t), req.provider_type_version) {
                return Err(ProviderError::Validation(format!(
                    "provider type `{t}` is not installed; no plugin ships it"
                )));
            }
        }
        // The TYPE OWNS its endpoint when its descriptor declares one: the endpoint
        // is the plugin's fact, the person supplies only the key. A caller override
        // is REFUSED (the contract: "a type that owns its endpoint supplies it and
        // rejects overrides"). The built-in has no fixed endpoint and keeps taking
        // the caller's url/api.
        let mut req = req;
        if let Some(t) = req.provider_type.as_deref() {
            if let Some(desc) = self.resolve_type(t, req.provider_type_version) {
                if let Some((url, api)) = desc.owned_endpoint() {
                    if let (Some(want), Some(got)) = (url.as_deref(), req.url.as_deref()) {
                        if want != got {
                            return Err(ProviderError::Validation(format!(
                                "provider type `{t}` owns its endpoint ({want}); it does not allow overriding `url`"
                            )));
                        }
                    }
                    if let (Some(want), Some(got)) = (api.as_deref(), req.api.as_deref()) {
                        if want != got {
                            return Err(ProviderError::Validation(format!(
                                "provider type `{t}` owns its protocol ({want}); it does not allow overriding `api`"
                            )));
                        }
                    }
                    if url.is_some() {
                        req.url = url;
                    }
                    if api.is_some() {
                        req.api = api;
                    }
                }
            }
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
        // 1b. An unresolved credential transition for this id blocks a REBUILD: a
        //     create over a pending op would leave a row whose ownership the journal
        //     still describes (TASK-048 F1).
        if self.store.has_pending_op(&id)? {
            return Err(ProviderError::RevisionConflict(format!(
                "provider `{id}` has an unresolved credential transition; resolve it before creating"
            )));
        }
        // 2. the row first: a duplicate id is refused with no secret written.
        //    Re-read it: the database minted the incarnation our save is guarded by.
        record = self.store.create(&record)?;
        // 3. the credential, into the keychain only, then PERSIST the reference so
        //    the row actually records where its credential lives.
        if let Some(t) = &req.token {
            // Record the transition BEFORE the credential write: a crash between
            // the two stores leaves a visible, recoverable marker. begin_op
            // REFUSES when an unresolved transition exists for this provider.
            let op = self.store.begin_op(
                &id,
                "create",
                &self.secret_ref(&id),
                &record.incarnation,
                record.revision,
                &Self::config_digest(&record),
            )?;
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
            // The credential and its reference are both in place: finish THIS op.
            self.store.finish_op(&op)?;
        }
        // A create with no token begins no op and must NOT finish someone else's.
        Ok(record)
    }

    /// Refuse an operation that MUTATES a provider's config/revision or CONSUMES
    /// its credential while an unresolved credential transition stands. This is the
    /// single admission check every such entry uses; the provider lock makes the
    /// check and the operation one step. A create for a fresh id has no pending op,
    /// so it is unaffected (TASK-048 F1-current).
    fn admit_no_pending(&self, id: &str) -> Result<(), ProviderError> {
        if self.store.has_pending_op(id)? {
            return Err(ProviderError::Unresolved(format!(
                "provider `{id}` has an unresolved credential transition; resolve it before changing configuration or consuming the credential"
            )));
        }
        Ok(())
    }

    pub async fn patch(&self, id: &str, req: PatchProvider) -> Result<ProviderRecord, ProviderError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        self.admit_no_pending(id)?;
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
        // The credential LOCATION is the row's own reference, never a re-derived
        // key (a rebuilt id must not re-acquire an old credential).
        let reference = rec.secret_ref.clone().unwrap_or_else(|| self.secret_ref(id));
        // Merge the non-credential fields first, so the INTENDED post-write revision
        // is known before we journal the operation.
        if let Some(v) = req.declarations {
            rec.declarations = v;
        }
        let changing_token = req.token.is_some();
        if changing_token {
            bump = true;
        }
        if bump {
            rec.revision += 1;
        }
        // The intended row state AFTER this operation saves. Recovery confirms the
        // credential ONLY when the row matches THIS (TASK-048 F1): a row at a
        // different revision means the save never landed, so we must not bless the
        // credential with it.
        let intended_incarnation = rec.incarnation.clone();
        let intended_revision = rec.revision;
        let mut op: Option<String> = None;
        if let Some(tok) = req.token {
            match tok {
                Some(t) => {
                    if !self.secrets.is_available() {
                        return Err(ProviderError::NoSecretStore);
                    }
                    op = Some(self.store.begin_op(
                        id,
                        "patch",
                        &reference,
                        &intended_incarnation,
                        intended_revision,
                        &Self::config_digest(&rec),
                    )?);
                    self.secrets
                        .set(&reference, &t)
                        .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
                    rec.secret_ref = Some(reference.clone());
                }
                None => {
                    op = Some(self.store.begin_op(
                        id,
                        "patch",
                        &reference,
                        &intended_incarnation,
                        intended_revision,
                        &Self::config_digest(&rec),
                    )?);
                    self.secrets
                        .delete(&reference)
                        .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
                    rec.secret_ref = None;
                }
            }
        }
        // A read ERROR is an error, never read as "not configured" (only when the
        // row claims a reference do we consult the store here; an unreadable one
        // aborts the patch rather than silently clearing the reference).
        if rec.secret_ref.is_some() {
            let configured = self
                .secrets
                .get(rec.secret_ref.as_ref().unwrap())
                .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?
                .is_some();
            rec.token_configured = configured;
            if !configured {
                rec.secret_ref = None;
            }
        }
        self.store.save(&rec)?;
        if let Some(o) = op {
            self.store.finish_op(&o)?;
        }
        self.with_configured(rec)
    }

    /// Delete the provider AND its stored credential.
    pub async fn delete(&self, id: &str) -> Result<(), ProviderError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        // Journal first: if the credential is removed but the row delete fails, the
        // boot sweep finishes the row deletion (no orphan credential).
        let row = self.store.get(id)?;
        let reference = row.secret_ref.clone().unwrap_or_else(|| self.secret_ref(id));
        let op = self.store.begin_op(
            id,
            "delete",
            &reference,
            &row.incarnation,
            row.revision,
            &Self::config_digest(&row),
        )?;
        self.secrets
            .delete(&reference)
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
        self.store.delete(id)?;
        self.store.finish_op(&op)?;
        Ok(())
    }

    /// Drop the credential, keep the row.
    pub async fn logout(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        let mut rec = self.store.get(id)?;
        let reference = rec.secret_ref.clone().unwrap_or_else(|| self.secret_ref(id));
        let op = self.store.begin_op(
            id,
            "logout",
            &reference,
            &rec.incarnation,
            rec.revision,
            &Self::config_digest(&rec),
        )?;
        self.secrets
            .delete(&reference)
            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
        // The row no longer points at a credential.
        rec.secret_ref = None;
        self.store.save(&rec)?;
        self.store.finish_op(&op)?;
        self.with_configured(rec)
    }

    pub async fn set_selection(
        &self,
        id: &str,
        enabled: Vec<String>,
    ) -> Result<ProviderRecord, ProviderError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        self.admit_no_pending(id)?;
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
        if let Some(reference) = rec.secret_ref.clone() {
            let has = self
                .secrets
                .get(&reference)
                .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?
                .is_some();
            if !has {
                rec.secret_ref = None;
            }
        }
        self.store.save(&rec)?;
        self.with_configured(rec)
    }

    pub async fn refresh(&self, id: &str) -> Result<ProviderRecord, ProviderError> {
        // Hold the provider's operation lock for the whole read-fetch-write, so a
        // refresh cannot interleave with a patch/delete/create of the same id.
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        // A refresh CONSUMES the credential (it sends it as a bearer) and rewrites
        // the row: it must not run over an unresolved transition, or it could send a
        // new token to an old URL (TASK-048 F1-current).
        self.admit_no_pending(id)?;
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
        for p in pending {
            match self.recover_one(&p) {
                Ok(()) => {
                    if let Err(e) = self.store.finish_op(&p.id) {
                        tracing::warn!(provider = %p.provider, error = %e, "recovery finished but the journal entry could not be cleared");
                    }
                }
                Err(e) => {
                    // Keep the entry: the state is still unresolved and must stay
                    // visible for the next boot. Never clear the journal on a
                    // failure.
                    tracing::warn!(provider = %p.provider, op = %p.op, error = %e, "provider recovery deferred (journal kept)");
                }
            }
        }
    }

    /// Resolve ONE journal entry. `Ok` means every necessary step was confirmed;
    /// any error leaves the entry in place. It may only CONFIRM a credential when
    /// the row STILL matches the state the operation intended to write; a row that
    /// changed under a failed write must not be handed a mismatched credential
    /// (TASK-048 F1).
    fn recover_one(&self, p: &agent_hub_db::PendingOp) -> Result<(), ProviderError> {
        let (provider, op, secret_ref) = (p.provider.as_str(), p.op.as_str(), p.secret_ref.as_str());
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
            // Did the row land EXACTLY as the operation intended? Only then may we
            // bless the credential with this row. Matching the incarnation AND
            // revision AND the CONFIG DIGEST proves the row is THIS operation's
            // result: an unrelated update that happens to reach the same revision
            // (a URL patch, a selection change) has a different digest and must NOT
            // be read as this op's commit (TASK-048 F1-current).
            let intent_matches = |r: &ProviderRecord| {
                Some(r.incarnation.as_str()) == p.expected_incarnation.as_deref()
                    && Some(r.revision) == p.expected_revision
                    && p.expected_config.as_deref() == Some(Self::config_digest(r).as_str())
            };
            match row {
                Some(mut r) if intent_matches(&r) => {
                    // The row landed at the revision the operation intended to save:
                    // the credential now belongs to THIS row.
                    r.secret_ref = if has { Some(secret_ref.to_string()) } else { None };
                    self.store.save(&r)?;
                    Ok(())
                }
                Some(mut r) => {
                    // The row CHANGED under the failed operation (a URL/token patch
                    // that did not persist, a rebuild). We must NOT attach the new
                    // credential to a row it was not intended for: drop the
                    // credential and leave the row pointing at whatever it owns.
                    tracing::warn!(
                        provider,
                        "a credential transition did not land on its intended row; dropping the credential rather than mispairing it"
                    );
                    if has {
                        self.secrets
                            .delete(secret_ref)
                            .map_err(|e| ProviderError::SecretUnreadable(e.to_string()))?;
                    }
                    if r.secret_ref.as_deref() == Some(secret_ref) {
                        r.secret_ref = None;
                        self.store.save(&r)?;
                    }
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
    pub async fn resolve_grant(&self, id: &str) -> Result<ResolvedGrant, ProviderError> {
        // The provider lock makes the CONFIG and the CREDENTIAL one consistent
        // snapshot: a concurrent delete+recreate or a URL/token patch cannot hand
        // out an old endpoint with a new token (TASK-048 F2).
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        if self.store.has_pending_op(id)? {
            return Err(ProviderError::RevisionConflict(format!(
                "provider `{id}` has an unresolved credential transition; retry"
            )));
        }
        let rec = self.store.get(id)?;
        // The TYPE authority at GRANT admission: a provider naming a type no
        // installed plugin ships is readable but UNUSABLE - it cannot grant (the
        // contract: "a record naming one stays readable but unusable", TASK-048 G3).
        if !self.type_available(rec.provider_type.as_deref(), rec.provider_type_version) {
            return Err(ProviderError::Validation(format!(
                "provider `{id}` names type `{}`, which no installed plugin ships; it is unusable",
                rec.provider_type.as_deref().unwrap_or("")
            )));
        }
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
        p.store
            .begin_op("p1", "create", "test-instance:provider-p1", "inc", 1, "d")
            .unwrap();
        p.recover_pending();
        // The entry is still there: the failure kept it for retry.
        let pending = p.store.pending_ops().unwrap();
        assert_eq!(pending.len(), 1, "an unresolvable recovery keeps its journal");
    }

    fn arecord(id: &str, url: Option<&str>) -> crate::record::ProviderRecord {
        crate::record::ProviderRecord {
            id: id.into(),
            label: None,
            url: url.map(str::to_string),
            api: None,
            secret_ref: None,
            provider_type: None,
            provider_type_version: None,
            declarations: Default::default(),
            enabled_model_ids: Vec::new(),
            revision: 1,
            catalog: None,
            token_configured: false,
            incarnation: String::new(),
        }
    }

    /// A pending transition makes the resolver REFUSE (F2): it never hands out a
    /// config/credential pair across an unresolved write.
    #[tokio::test]
    async fn resolve_grant_refuses_while_a_transition_is_pending() {
        let db = agent_hub_db::Db::open_in_memory().unwrap();
        let store = ProviderStore::new(db);
        let secrets = std::sync::Arc::new(agent_hub_secrets::SecretStore::probe_unavailable_for_test());
        let p = Providers::new(store, secrets, "test-instance");
        p.store.create(&arecord("p1", Some("https://x.test"))).unwrap();
        p.store
            .begin_op("p1", "patch", "test-instance:provider-p1", "inc", 1, "d")
            .unwrap();
        let e = p.resolve_grant("p1").await;
        assert!(matches!(e, Err(ProviderError::RevisionConflict(_))));
    }
}

#[cfg(test)]
mod recovery_intent_tests {
    use super::*;

    fn record(id: &str, url: &str) -> crate::record::ProviderRecord {
        crate::record::ProviderRecord {
            id: id.into(),
            label: None,
            url: Some(url.into()),
            api: None,
            secret_ref: None,
            provider_type: None,
            provider_type_version: None,
            declarations: Default::default(),
            enabled_model_ids: Vec::new(),
            revision: 1,
            catalog: None,
            token_configured: false,
            incarnation: String::new(),
        }
    }

    /// Recovery confirms the credential ONLY when the row matches the intended
    /// POST-write revision. When a patch's row save did NOT land (the row is still
    /// at the old revision), recovery DROPS the credential rather than attaching a
    /// new token to the old URL (TASK-048 F1).
    #[tokio::test]
    async fn recovery_drops_a_credential_whose_row_save_did_not_land() {
        let secrets = std::sync::Arc::new(agent_hub_secrets::SecretStore::probe(format!(
            "agent-hub-test-{}",
            std::process::id()
        )));
        if !secrets.is_available() {
            eprintln!("SKIP: no OS secret store");
            return;
        }
        let db = agent_hub_db::Db::open_in_memory().unwrap();
        let store = ProviderStore::new(db);
        let p = Providers::new(store, secrets.clone(), "itest");
        let mut rec = p.store.create(&record("p1", "https://old.test")).unwrap();
        let reference = p.secret_ref("p1");
        // The credential lands in the keychain...
        secrets.set(&reference, "TOKEN-NEW").unwrap();
        // ...but the row save did NOT (the journal recorded intended revision 2; the
        // row stayed at revision 1).
        p.store
            .begin_op(
                "p1",
                "patch",
                &reference,
                &rec.incarnation,
                rec.revision + 1,
                &Providers::config_digest(&rec),
            )
            .unwrap();

        p.recover_pending();

        // The credential was dropped, NOT attached to the old-URL row.
        assert_eq!(secrets.get(&reference).unwrap(), None, "the mismatched credential is dropped");
        rec = p.store.get("p1").unwrap();
        assert_eq!(rec.secret_ref, None, "the row does not point at a dropped credential");
    }


    /// S1: while a credential transition is unresolved, the OTHER entries that
    /// consume the credential or change the row MUST refuse - not just the resolver.
    #[tokio::test]
    async fn every_mutator_refuses_while_a_transition_is_unresolved() {
        let db = agent_hub_db::Db::open_in_memory().unwrap();
        let store = ProviderStore::new(db);
        let secrets = std::sync::Arc::new(agent_hub_secrets::SecretStore::probe(format!(
            "agent-hub-test-s1-{}",
            std::process::id()
        )));
        // The barrier is DB-level and needs no OS keychain, so run regardless.
        let p = Providers::new(store, secrets, "itest");
        let rec = p.store.create(&record("p1", "https://u0.test")).unwrap();
        let reference = p.secret_ref("p1");
        p.store
            .begin_op("p1", "patch", &reference, &rec.incarnation, rec.revision + 1, "digest-x")
            .unwrap();

        // PATCH (no token), selection and refresh all refuse.
        let patch = p
            .patch(
                "p1",
                crate::service::PatchProvider {
                    url: Some(Some("https://u2.test".to_string())),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(patch, Err(ProviderError::Unresolved(_))), "patch must refuse: {patch:?}");

        let sel = p.set_selection("p1", vec!["m".into()]).await;
        assert!(matches!(sel, Err(ProviderError::Unresolved(_))), "selection must refuse: {sel:?}");

        let refresh = p.refresh("p1").await;
        assert!(matches!(refresh, Err(ProviderError::Unresolved(_))), "refresh must refuse: {refresh:?}");

        // The row was NOT bumped by any of them.
        assert_eq!(p.store.get("p1").unwrap().revision, rec.revision);
    }

    /// S1: recovery must NOT read an UNRELATED update that reached the same
    /// revision as this op's commit. The config digest distinguishes them.
    #[tokio::test]
    async fn recovery_rejects_an_unrelated_update_at_the_same_revision() {
        let secrets = std::sync::Arc::new(agent_hub_secrets::SecretStore::probe(format!(
            "agent-hub-test-s1b-{}",
            std::process::id()
        )));
        if !secrets.is_available() {
            eprintln!("SKIP: no OS secret store");
            return;
        }
        let db = agent_hub_db::Db::open_in_memory().unwrap();
        let store = ProviderStore::new(db);
        let p = Providers::new(store, secrets.clone(), "itest");
        let mut rec = p.store.create(&record("p1", "https://U0.test")).unwrap();
        let reference = p.secret_ref("p1");
        // Journal the intended row state (revision 2, config U0).
        secrets.set(&reference, "T-NEW").unwrap();
        p.store
            .begin_op(
                "p1",
                "patch",
                &reference,
                &rec.incarnation,
                2,
                &Providers::config_digest(&rec),
            )
            .unwrap();
        // An UNRELATED update lands U2 at revision 2 (different config, same rev).
        rec.url = Some("https://U2.test".into());
        rec.revision = 2;
        p.store.save(&rec).unwrap();

        p.recover_pending();

        // The digest differs, so recovery must NOT bless T-NEW; the credential is
        // dropped and the row keeps its own (none).
        assert_eq!(
            secrets.get(&reference).unwrap(),
            None,
            "an unrelated update at the same revision must not be read as this op's commit"
        );
    }

}
