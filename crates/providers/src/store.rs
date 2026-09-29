//! The provider store: **the database** owns a provider's relationship state
//! (`ARCHITECTURE` §5, §8). A provider's endpoint, protocol, declarations,
//! selection and cached catalog are rows; its **credential** is only a keychain
//! reference, never a value in a row or a file.

use agent_hub_db::{Db, ProviderRow};

use crate::record::ProviderRecord;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("provider `{0}` not found")]
    NotFound(String),
    #[error("provider `{0}` already exists")]
    Exists(String),
    #[error(transparent)]
    Db(#[from] agent_hub_db::DbError),
}

/// The store over the `providers` table.
pub struct ProviderStore {
    db: Db,
}

impl ProviderStore {
    pub fn new(db: Db) -> Self {
        ProviderStore { db }
    }

    pub fn list(&self) -> Result<Vec<ProviderRecord>, StoreError> {
        Ok(self.db.list_providers()?.iter().map(to_record).collect())
    }

    pub fn get(&self, id: &str) -> Result<ProviderRecord, StoreError> {
        self.db
            .provider(id)?
            .map(|r| to_record(&r))
            .ok_or_else(|| StoreError::NotFound(id.into()))
    }

    /// Insert a new provider; a duplicate id is refused with no side effect.
    pub fn create(&self, record: &ProviderRecord) -> Result<(), StoreError> {
        match self.db.insert_provider(&to_row(record)) {
            Ok(()) => Ok(()),
            Err(agent_hub_db::DbError::Conflict(_)) => Err(StoreError::Exists(record.id.clone())),
            Err(e) => Err(StoreError::Db(e)),
        }
    }

    pub fn save(&self, record: &ProviderRecord) -> Result<(), StoreError> {
        self.db.save_provider(&to_row(record))?;
        Ok(())
    }

    /// Record an in-flight credential transition (see `db::providers`).
    pub fn begin_op(&self, provider: &str, op: &str, secret_ref: &str) -> Result<(), StoreError> {
        self.db.begin_provider_op(provider, op, secret_ref)?;
        Ok(())
    }

    pub fn finish_op(&self, provider: &str) -> Result<(), StoreError> {
        self.db.finish_provider_op(provider)?;
        Ok(())
    }

    /// In-flight credential transitions (for the boot sweep).
    pub fn pending_ops(&self) -> Result<Vec<(String, String, String)>, StoreError> {
        Ok(self.db.pending_provider_ops()?)
    }

    /// Delete the row. The caller owns the credential deletion (the secret store
    /// is separate); this never touches the keychain.
    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        match self.db.delete_provider(id) {
            Ok(()) => Ok(()),
            Err(agent_hub_db::DbError::NotFound(_)) => Err(StoreError::NotFound(id.into())),
            Err(e) => Err(StoreError::Db(e)),
        }
    }
}

fn to_row(r: &ProviderRecord) -> ProviderRow {
    ProviderRow {
        id: r.id.clone(),
        label: r.label.clone(),
        url: r.url.clone(),
        api: r.api.clone(),
        provider_type: r.provider_type.clone(),
        provider_type_version: r.provider_type_version,
        declarations: serde_json::to_value(&r.declarations).unwrap_or_else(|_| serde_json::json!({})),
        enabled_model_ids: r.enabled_model_ids.clone(),
        revision: r.revision,
        secret_ref: r.secret_ref.clone(),
        catalog: r
            .catalog
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
    }
}

fn to_record(r: &ProviderRow) -> ProviderRecord {
    ProviderRecord {
        id: r.id.clone(),
        label: r.label.clone(),
        url: r.url.clone(),
        api: r.api.clone(),
        provider_type: r.provider_type.clone(),
        provider_type_version: r.provider_type_version,
        declarations: serde_json::from_value(r.declarations.clone()).unwrap_or_default(),
        enabled_model_ids: r.enabled_model_ids.clone(),
        revision: r.revision,
        secret_ref: r.secret_ref.clone(),
        catalog: r.catalog.as_ref().and_then(|c| serde_json::from_value(c.clone()).ok()),
        token_configured: false, // set by the service from the secret store
    }
}
