//! The file-backed provider store (`ARCHITECTURE` §5): one JSON file per
//! provider under the hub's data dir. The hub reads the root it was given; a
//! malformed file is reported, never silently dropped.

use std::path::{Path, PathBuf};

use crate::record::ProviderRecord;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("provider `{0}` not found")]
    NotFound(String),
    #[error("provider `{0}` already exists")]
    Exists(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("provider file `{0}` is not valid JSON: {1}")]
    Corrupt(String, String),
}

/// A file the store could not load, with its reason.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Broken {
    pub file: String,
    pub error: String,
}

/// The store over `providers/`.
pub struct ProviderStore {
    root: PathBuf,
}

impl ProviderStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ProviderStore { root: root.into() }
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    pub fn list(&self) -> Result<(Vec<ProviderRecord>, Vec<Broken>), StoreError> {
        let mut records = Vec::new();
        let mut broken = Vec::new();
        if !self.root.exists() {
            return Ok((records, broken));
        }
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            match std::fs::read_to_string(&path) {
                Ok(raw) => match serde_json::from_str::<ProviderRecord>(&raw) {
                    Ok(rec) => records.push(rec),
                    Err(e) => broken.push(Broken { file, error: e.to_string() }),
                },
                Err(e) => broken.push(Broken { file, error: e.to_string() }),
            }
        }
        records.sort_by(|a, b| a.id.cmp(&b.id));
        broken.sort_by(|a, b| a.file.cmp(&b.file));
        Ok((records, broken))
    }

    pub fn get(&self, id: &str) -> Result<ProviderRecord, StoreError> {
        let path = self.path_for(id);
        let raw = std::fs::read_to_string(&path).map_err(|_| StoreError::NotFound(id.into()))?;
        serde_json::from_str(&raw).map_err(|e| StoreError::Corrupt(id.into(), e.to_string()))
    }

    /// Insert a new record; a duplicate id is refused.
    pub fn create(&self, record: &ProviderRecord) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.root)?;
        let path = self.path_for(&record.id);
        if path.exists() {
            return Err(StoreError::Exists(record.id.clone()));
        }
        self.write(record)
    }

    pub fn save(&self, record: &ProviderRecord) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.root)?;
        self.write(record)
    }

    fn write(&self, record: &ProviderRecord) -> Result<(), StoreError> {
        let path = self.path_for(&record.id);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(record).expect("serialize provider");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        let path = self.path_for(id);
        if !path.exists() {
            return Err(StoreError::NotFound(id.into()));
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Shared with the data layer (one implementation).
pub use agent_hub_db::now_utc;
