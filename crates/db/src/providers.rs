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
    /// A fresh id minted on every INSERT: a row read before a delete+recreate of
    /// the same id has a DIFFERENT incarnation, so a stale save is refused
    /// (TASK-048 F2: an old refresh cannot overwrite a rebuilt provider).
    pub incarnation: String,
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
  incarnation           TEXT NOT NULL DEFAULT '',
  catalog               TEXT
);
"#;

impl crate::Db {
    pub fn list_providers(&self) -> Result<Vec<ProviderRow>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, label, url, api, provider_type, provider_type_version, declarations, enabled_model_ids, revision, secret_ref, incarnation, catalog FROM providers ORDER BY id",
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
                "SELECT id, label, url, api, provider_type, provider_type_version, declarations, enabled_model_ids, revision, secret_ref, incarnation, catalog FROM providers WHERE id = ?1",
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
        let mut p = p.clone();
        p.incarnation = new_incarnation();
        write_provider(&conn, &p)?;
        Ok(())
    }

    /// Save a provider row, **guarded by its incarnation**: the UPDATE matches
    /// only when the row currently has the SAME incarnation the caller read. A
    /// row read before a delete+recreate of the same id has a different
    /// incarnation and the save is refused with `Conflict` (TASK-048 F2), so a
    /// stale object can never overwrite a rebuilt one.
    pub fn save_provider(&self, p: &ProviderRow) -> Result<(), DbError> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE providers SET label=?2, url=?3, api=?4, provider_type=?5, provider_type_version=?6, declarations=?7, enabled_model_ids=?8, revision=?9, secret_ref=?10, catalog=?11 WHERE id=?1 AND incarnation=?12",
            params![
                p.id, p.label, p.url, p.api, p.provider_type, p.provider_type_version,
                serde_json::to_string(&p.declarations).unwrap_or_else(|_| "{}".into()),
                serde_json::to_string(&p.enabled_model_ids).unwrap_or_else(|_| "[]".into()),
                p.revision as i64, p.secret_ref,
                p.catalog.as_ref().map(|c| serde_json::to_string(c).unwrap_or_default()),
                p.incarnation,
            ],
        )?;
        if n == 0 {
            // Either the id is gone or the incarnation changed: report a conflict
            // so the caller re-reads rather than clobbering.
            let exists: bool = conn
                .query_row("SELECT 1 FROM providers WHERE id = ?1", params![p.id], |_| Ok(true))
                .optional()?
                .unwrap_or(false);
            return Err(if exists {
                DbError::Conflict(format!("provider `{}` changed since it was read", p.id))
            } else {
                DbError::NotFound(p.id.clone())
            });
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
    let catalog: Option<String> = r.get(11).unwrap_or(None);
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
        incarnation: r.get(10).unwrap_or_default(),
        catalog: catalog.and_then(|c| serde_json::from_str(&c).ok()),
    }
}

fn write_provider(conn: &Connection, p: &ProviderRow) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO providers (id, label, url, api, provider_type, provider_type_version, declarations, enabled_model_ids, revision, secret_ref, incarnation, catalog)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            p.id, p.label, p.url, p.api, p.provider_type, p.provider_type_version,
            serde_json::to_string(&p.declarations).unwrap_or_else(|_| "{}".into()),
            serde_json::to_string(&p.enabled_model_ids).unwrap_or_else(|_| "[]".into()),
            p.revision as i64, p.secret_ref, p.incarnation,
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
  expected_incarnation TEXT,
  expected_revision    INTEGER,
  intent_version       INTEGER,
  created_at TEXT NOT NULL
);
"#;

    /// A pending credential transition, with the ROW STATE the operation intended
    /// to write (`expected_incarnation`/`expected_revision`). Recovery may only
    /// "confirm" the credential when the row STILL matches that intent; a row that
    /// changed under a failed write must NOT be blessed with the new credential.
    #[derive(Debug, Clone)]
    pub struct PendingOp {
        pub id: String,
        pub provider: String,
        pub op: String,
        pub secret_ref: String,
        pub expected_incarnation: Option<String>,
        pub expected_revision: Option<u64>,
        /// The SEMANTICS of `expected_incarnation`/`expected_revision`:
        /// `Some(2)` = the row state the op intended to WRITE (post-write intent);
        /// `None`/`Some(1)` = a LEGACY entry written before the intent was the
        /// post-write state (its revision is the PRE-write one). Recovery must not
        /// interpret a legacy revision with the new semantics (TASK-048 F1).
        pub intent_version: Option<i64>,
    }

/// The current journal intent semantics. Bump when the meaning of
/// `expected_incarnation`/`expected_revision` changes, so a newer hub can
/// recognise an older entry instead of misreading it.
pub const INTENT_VERSION: i64 = 2;

impl crate::Db {
    /// Record an in-flight credential transition. It REFUSES (Conflict) when a
    /// pending op already exists for this provider: an unrelated write must never
    /// overwrite an unresolved intent (TASK-048 F1). The caller resolves the
    /// pending op first (boot sweep or an explicit resolution), then begins.
    ///
    /// The insert and the duplicate check are one transaction.
    pub fn begin_provider_op(
        &self,
        provider: &str,
        op: &str,
        secret_ref: &str,
        expected_incarnation: &str,
        expected_revision: u64,
    ) -> Result<String, DbError> {
        let mut conn = self.conn.lock().expect("db mutex");
        let tx = conn.transaction()?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM provider_ops WHERE provider = ?1",
                params![provider],
                |r| r.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(DbError::Conflict(format!(
                "provider `{provider}` has an unresolved credential transition; resolve it before a new write"
            )));
        }
        let id = format!("{provider}:{op}");
        tx.execute(
            "INSERT INTO provider_ops (id, provider, op, secret_ref, expected_incarnation, expected_revision, intent_version, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, provider, op, secret_ref, expected_incarnation, expected_revision as i64, INTENT_VERSION, crate::now_utc()],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Clear the op the caller actually began, by ITS id. A different pending op
    /// (a later write that began after ours, or an unresolved one) is untouched.
    pub fn finish_provider_op_id(&self, id: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute("DELETE FROM provider_ops WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Whether this provider has an unresolved credential transition.
    pub fn has_pending_provider_op(&self, provider: &str) -> Result<bool, DbError> {
        let conn = self.lock();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM provider_ops WHERE provider = ?1",
            params![provider],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Every in-flight credential transition (for the boot sweep).
    pub fn pending_provider_ops(&self) -> Result<Vec<PendingOp>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, provider, op, secret_ref, expected_incarnation, expected_revision, intent_version FROM provider_ops",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PendingOp {
                    id: r.get(0)?,
                    provider: r.get(1)?,
                    op: r.get(2)?,
                    secret_ref: r.get(3)?,
                    expected_incarnation: r.get(4)?,
                    expected_revision: r.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                    intent_version: r.get::<_, Option<i64>>(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod op_journal_tests {
    use super::*;
    use crate::Db;

    #[test]
    fn a_pending_op_is_visible_and_cleared_by_its_id() {
        let db = Db::open_in_memory().unwrap();
        let id = db.begin_provider_op("p1", "create", "ns:provider-p1", "inc1", 1).unwrap();
        let pending = db.pending_provider_ops().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].provider, "p1");
        assert_eq!(pending[0].expected_incarnation.as_deref(), Some("inc1"));
        db.finish_provider_op_id(&id).unwrap();
        assert!(db.pending_provider_ops().unwrap().is_empty());
    }

    /// A new op is REFUSED while an unresolved op exists for the same provider: an
    /// unrelated write must not overwrite an unresolved intent (F1).
    #[test]
    fn a_second_op_is_refused_while_one_is_pending() {
        let db = Db::open_in_memory().unwrap();
        db.begin_provider_op("p1", "create", "r1", "inc", 1).unwrap();
        assert!(matches!(
            db.begin_provider_op("p1", "delete", "r1", "inc", 1),
            Err(DbError::Conflict(_))
        ));
        let pending = db.pending_provider_ops().unwrap();
        assert_eq!(pending.len(), 1, "the unresolved op is not replaced");
        assert_eq!(pending[0].op, "create");
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

/// A fresh incarnation id for a newly inserted provider.
fn new_incarnation() -> String {
    use rand::RngCore;
    let mut b = [0u8; 8];
    rand::rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod incarnation_tests {
    use super::*;
    use crate::Db;

    fn row(id: &str, url: &str) -> ProviderRow {
        ProviderRow {
            id: id.into(),
            label: None,
            url: Some(url.into()),
            api: None,
            provider_type: None,
            provider_type_version: None,
            declarations: serde_json::json!({}),
            enabled_model_ids: vec![],
            revision: 1,
            secret_ref: None,
            incarnation: String::new(),
            catalog: None,
        }
    }

    /// A row read before a delete+recreate of the same id carries a different
    /// incarnation, so its save is refused (F2: no stale overwrite).
    #[test]
    fn a_stale_incarnation_cannot_overwrite_a_rebuilt_provider() {
        let db = Db::open_in_memory().unwrap();
        db.insert_provider(&row("p", "old")).unwrap();
        let stale = db.provider("p").unwrap().unwrap();
        db.delete_provider("p").unwrap();
        db.insert_provider(&row("p", "new")).unwrap();

        // The stale object (old incarnation) must not overwrite the rebuilt one.
        let mut bad = stale.clone();
        bad.url = Some("clobbered".into());
        assert!(matches!(db.save_provider(&bad), Err(DbError::Conflict(_))));

        let current = db.provider("p").unwrap().unwrap();
        assert_eq!(current.url.as_deref(), Some("new"), "the rebuilt row is intact");

        // A save with the CURRENT row succeeds.
        let mut ok = current.clone();
        ok.revision += 1;
        db.save_provider(&ok).unwrap();
        assert_eq!(db.provider("p").unwrap().unwrap().revision, 2);
    }
}

