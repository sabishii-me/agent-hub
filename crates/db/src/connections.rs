//! Connection rows: the hub-managed connections as **relationship state in the
//! database** (ARCHITECTURE 5, 8). A connection is `{id, name, scheme, endpoint,
//! envName, state}`; its **credential** is NOT here - only a **secret reference**
//! (a keychain key), never a value.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::DbError;

/// A stored connection. The credential is a reference, never a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionRow {
    pub id: String,
    pub name: String,
    /// The auth scheme (opaque label; the adapter decides what it means).
    pub scheme: Option<String>,
    pub endpoint: Option<String>,
    /// The env var name a session's adapter receives the token under.
    pub env_name: Option<String>,
    /// `enabled` | `disabled`. Disabled == cut-off == zero materialization.
    pub state: String,
    /// The keychain reference for this connection's credential, when one is stored.
    pub secret_ref: Option<String>,
    /// A fresh id minted on every INSERT (a stale save is refused).
    pub incarnation: String,
    /// Bumped on every material change.
    pub revision: u64,
}

pub const SCHEMA_CONNECTIONS: &str = r#"
CREATE TABLE IF NOT EXISTS connections (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  scheme       TEXT,
  endpoint     TEXT,
  env_name     TEXT,
  state        TEXT NOT NULL DEFAULT 'enabled',
  secret_ref   TEXT,
  incarnation  TEXT NOT NULL DEFAULT '',
  revision     INTEGER NOT NULL DEFAULT 1
);
"#;

impl crate::Db {
    pub fn list_connections(&self) -> Result<Vec<ConnectionRow>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, scheme, endpoint, env_name, state, secret_ref, incarnation, revision FROM connections ORDER BY id",
        )?;
        let rows = stmt.query_map([], read_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn connection(&self, id: &str) -> Result<Option<ConnectionRow>, DbError> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT id, name, scheme, endpoint, env_name, state, secret_ref, incarnation, revision FROM connections WHERE id = ?1",
                params![id],
                read_row,
            )
            .optional()?)
    }

    /// Insert a connection; a duplicate id is refused (no side effect). Returns
    /// the stored row (with the minted incarnation).
    pub fn insert_connection(&self, c: &ConnectionRow) -> Result<ConnectionRow, DbError> {
        let conn = self.lock();
        let exists: bool = conn
            .query_row("SELECT 1 FROM connections WHERE id = ?1", params![c.id], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if exists {
            return Err(DbError::Conflict(format!("connection `{}` already exists", c.id)));
        }
        let incarnation = new_incarnation();
        conn.execute(
            "INSERT INTO connections (id, name, scheme, endpoint, env_name, state, secret_ref, incarnation, revision)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![c.id, c.name, c.scheme, c.endpoint, c.env_name, c.state, c.secret_ref, incarnation, c.revision as i64],
        )?;
        drop(conn);
        self.connection(&c.id)?.ok_or_else(|| DbError::NotFound(c.id.clone()))
    }

    /// Save a connection, guarded by its incarnation (a stale row is refused).
    pub fn save_connection(&self, c: &ConnectionRow) -> Result<(), DbError> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE connections SET name=?2, scheme=?3, endpoint=?4, env_name=?5, state=?6, secret_ref=?7, revision=?8 WHERE id=?1 AND incarnation=?9",
            params![c.id, c.name, c.scheme, c.endpoint, c.env_name, c.state, c.secret_ref, c.revision as i64, c.incarnation],
        )?;
        if n == 0 {
            let exists: bool = conn
                .query_row("SELECT 1 FROM connections WHERE id = ?1", params![c.id], |_| Ok(true))
                .optional()?
                .unwrap_or(false);
            return Err(if exists {
                DbError::Conflict(format!("connection `{}` changed since it was read", c.id))
            } else {
                DbError::NotFound(c.id.clone())
            });
        }
        Ok(())
    }

    pub fn delete_connection(&self, id: &str) -> Result<(), DbError> {
        let conn = self.lock();
        let n = conn.execute("DELETE FROM connections WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(DbError::NotFound(id.into()));
        }
        Ok(())
    }
}

fn read_row(r: &rusqlite::Row<'_>) -> Result<ConnectionRow, rusqlite::Error> {
    Ok(ConnectionRow {
        id: r.get(0)?,
        name: r.get(1)?,
        scheme: r.get(2)?,
        endpoint: r.get(3)?,
        env_name: r.get(4)?,
        state: r.get(5)?,
        secret_ref: r.get(6)?,
        incarnation: r.get(7)?,
        revision: r.get::<_, i64>(8)? as u64,
    })
}

fn new_incarnation() -> String {
    use rand::RngCore;
    let mut b = [0u8; 8];
    rand::rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}
