//! Provider rows: the hub-managed model providers as **relationship state in the
//! database** (`ARCHITECTURE` §5, §8). A provider's endpoint, protocol, model
//! declarations, selection and cached catalog live here; its **credential** does
//! NOT - only a **secret reference** (a keychain key), never a value.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::DbError;

/// A stored provider. The credential is a reference, never a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRow {
    pub id: String,
    pub label: Option<String>,
    pub url: Option<String>,
    pub api: Option<String>,
    pub provider_type: Option<String>,
    pub provider_type_version: Option<u32>,
    pub declarations: serde_json::Value,
    pub enabled_model_ids: Vec<String>,
    pub revision: u64,
    /// The keychain reference for this provider's credential, when one is stored.
    pub secret_ref: Option<String>,
    /// The cached catalog: `{fetchedAt, revision, models}` or null.
    pub catalog: Option<serde_json::Value>,
}

pub const SCHEMA_PROVIDERS: &str = r#"
CREATE TABLE IF NOT EXISTS providers (
  id                    TEXT PRIMARY KEY,
  label                 TEXT,
  url                   TEXT,
  api                   TEXT,
  provider_type         TEXT,
  provider_type_version INTEGER,
  declarations          TEXT NOT NULL DEFAULT '{}',
  enabled_model_ids     TEXT NOT NULL DEFAULT '[]',
  revision              INTEGER NOT NULL DEFAULT 1,
  secret_ref            TEXT,
  catalog               TEXT
);
"#;

impl crate::Db {
    pub fn list_providers(&self) -> Result<Vec<ProviderRow>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, label, url, api, provider_type, provider_type_version, declarations, enabled_model_ids, revision, secret_ref, catalog FROM providers ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| Ok(read_row(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn provider(&self, id: &str) -> Result<Option<ProviderRow>, DbError> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT id, label, url, api, provider_type, provider_type_version, declarations, enabled_model_ids, revision, secret_ref, catalog FROM providers WHERE id = ?1",
                params![id],
                |r| Ok(read_row(r)),
            )
            .optional()?)
    }

    /// Insert a provider; a duplicate id is refused (no side effect).
    pub fn insert_provider(&self, p: &ProviderRow) -> Result<(), DbError> {
        let conn = self.lock();
        let exists: bool = conn
            .query_row("SELECT 1 FROM providers WHERE id = ?1", params![p.id], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if exists {
            return Err(DbError::Conflict(format!("provider `{}` already exists", p.id)));
        }
        write_provider(&conn, p)?;
        Ok(())
    }

    pub fn save_provider(&self, p: &ProviderRow) -> Result<(), DbError> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE providers SET label=?2, url=?3, api=?4, provider_type=?5, provider_type_version=?6, declarations=?7, enabled_model_ids=?8, revision=?9, secret_ref=?10, catalog=?11 WHERE id=?1",
            params![
                p.id, p.label, p.url, p.api, p.provider_type, p.provider_type_version,
                serde_json::to_string(&p.declarations).unwrap_or_else(|_| "{}".into()),
                serde_json::to_string(&p.enabled_model_ids).unwrap_or_else(|_| "[]".into()),
                p.revision as i64, p.secret_ref,
                p.catalog.as_ref().map(|c| serde_json::to_string(c).unwrap_or_default()),
            ],
        )?;
        if n == 0 {
            return Err(DbError::NotFound(p.id.clone()));
        }
        Ok(())
    }

    pub fn delete_provider(&self, id: &str) -> Result<(), DbError> {
        let conn = self.lock();
        let n = conn.execute("DELETE FROM providers WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(DbError::NotFound(id.into()));
        }
        Ok(())
    }
}

fn read_row(r: &rusqlite::Row<'_>) -> ProviderRow {
    let decl: String = r.get(6).unwrap_or_else(|_| "{}".into());
    let enabled: String = r.get(7).unwrap_or_else(|_| "[]".into());
    let catalog: Option<String> = r.get(10).unwrap_or(None);
    ProviderRow {
        id: r.get(0).unwrap_or_default(),
        label: r.get(1).unwrap_or(None),
        url: r.get(2).unwrap_or(None),
        api: r.get(3).unwrap_or(None),
        provider_type: r.get(4).unwrap_or(None),
        provider_type_version: r.get(5).unwrap_or(None),
        declarations: serde_json::from_str(&decl).unwrap_or_else(|_| serde_json::json!({})),
        enabled_model_ids: serde_json::from_str(&enabled).unwrap_or_default(),
        revision: r.get::<_, i64>(8).unwrap_or(1) as u64,
        secret_ref: r.get(9).unwrap_or(None),
        catalog: catalog.and_then(|c| serde_json::from_str(&c).ok()),
    }
}

fn write_provider(conn: &Connection, p: &ProviderRow) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO providers (id, label, url, api, provider_type, provider_type_version, declarations, enabled_model_ids, revision, secret_ref, catalog)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            p.id, p.label, p.url, p.api, p.provider_type, p.provider_type_version,
            serde_json::to_string(&p.declarations).unwrap_or_else(|_| "{}".into()),
            serde_json::to_string(&p.enabled_model_ids).unwrap_or_else(|_| "[]".into()),
            p.revision as i64, p.secret_ref,
            p.catalog.as_ref().map(|c| serde_json::to_string(c).unwrap_or_default()),
        ],
    )?;
    Ok(())
}

/// **The credential-transition journal** (`ARCHITECTURE` §8). A provider's
/// credential lives in a store (the keychain) that is NOT in the same
/// transaction as the row. Any operation that touches both records its intent
/// here FIRST, so a crash between the two stores leaves a **visible, recoverable**
/// state rather than a silent orphan credential or a row with no credential.
///
/// Rows are `pending` while an operation is in flight and deleted on success. A
/// boot sweep resolves any that remain: see [`recover_pending_provider_ops`].
pub const SCHEMA_PROVIDER_OPS: &str = r#"
CREATE TABLE IF NOT EXISTS provider_ops (
  id         TEXT PRIMARY KEY,
  provider   TEXT NOT NULL,
  op         TEXT NOT NULL,
  secret_ref TEXT,
  created_at TEXT NOT NULL
);
"#;

impl crate::Db {
    /// Record an in-flight credential transition (delete a pre-existing op with
    /// the same provider first, so one provider has at most one in-flight op).
    pub fn begin_provider_op(&self, provider: &str, op: &str, secret_ref: &str) -> Result<String, DbError> {
        let conn = self.lock();
        let id = format!("{provider}:{op}");
        conn.execute("DELETE FROM provider_ops WHERE provider = ?1", params![provider])?;
        conn.execute(
            "INSERT INTO provider_ops (id, provider, op, secret_ref, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, provider, op, secret_ref, crate::now_utc()],
        )?;
        Ok(id)
    }

    pub fn finish_provider_op(&self, provider: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute("DELETE FROM provider_ops WHERE provider = ?1", params![provider])?;
        Ok(())
    }

    /// Every in-flight credential transition (for the boot sweep).
    pub fn pending_provider_ops(&self) -> Result<Vec<(String, String, String)>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT provider, op, secret_ref FROM provider_ops")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod op_journal_tests {
    use super::*;
    use crate::Db;

    #[test]
    fn a_pending_op_is_visible_and_cleared() {
        let db = Db::open_in_memory().unwrap();
        db.begin_provider_op("p1", "create", "ns:provider-p1").unwrap();
        let pending = db.pending_provider_ops().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "p1");
        db.finish_provider_op("p1").unwrap();
        assert!(db.pending_provider_ops().unwrap().is_empty());
    }

    /// One provider has at most one in-flight op: beginning a new one replaces.
    #[test]
    fn a_provider_has_one_in_flight_op() {
        let db = Db::open_in_memory().unwrap();
        db.begin_provider_op("p1", "create", "r1").unwrap();
        db.begin_provider_op("p1", "delete", "r1").unwrap();
        let pending = db.pending_provider_ops().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1, "delete");
    }
}

#[cfg(test)]
mod instance_tests {
    use crate::Db;

    /// The instance id is PERSISTED: reopening the same database yields the same
    /// id, which is what makes the secret namespace survive a directory move.
    #[test]
    fn the_instance_id_is_persistent() {
        let dir = std::env::temp_dir().join(format!(
            "agent-hub-inst-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hub.sqlite");
        let a = Db::open(&path).unwrap().instance_id().unwrap();
        drop(Db::open(&path).unwrap());
        let b = Db::open(&path).unwrap().instance_id().unwrap();
        assert_eq!(a, b, "the instance id must persist across opens");
        assert_eq!(a.len(), 32);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
