//! The hub's data layer (`ARCHITECTURE` §8, task T3): a `rusqlite`-backed store
//! and the plugin install/replace **recovery rules** from §10.
//!
//! A plugin install touches two stores that are **not** one transaction: a
//! directory tree (rename moves) and a database row. SQLite gives atomicity for
//! the row; it says nothing about the tree. So the design records, at each
//! boundary, **what is kept, advanced or rolled back**, and a boot sweep resumes
//! from the recorded state rather than from scratch.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub mod connections;
pub mod instance;
pub mod providers;
pub mod recovery;
pub mod sessions;

pub use recovery::{install, recover, Layout, RecoveryOutcome};
pub use connections::ConnectionRow;
pub use providers::ProviderRow;
pub use sessions::{ReserveOutcome, SessionRow, TurnAdmission, TurnRow};

/// The externally visible state of a plugin (mirrors the contract's `plugin.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginState {
    Absent,
    Installing,
    Preparing,
    Removing,
    Failed,
    Ready,
}

impl PluginState {
    pub fn as_str(self) -> &'static str {
        match self {
            PluginState::Absent => "absent",
            PluginState::Installing => "installing",
            PluginState::Preparing => "preparing",
            PluginState::Removing => "removing",
            PluginState::Failed => "failed",
            PluginState::Ready => "ready",
        }
    }
}

/// Where an in-flight install/replace stands. Recorded durably before each move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStep {
    Staged,
    OldMovedAside,
    NewInPlace,
    Committed,
    RecoveryRetained,
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("`{0}` not found")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginRow {
    pub id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub plugin_type: Option<String>,
    pub source: Option<String>,
    pub reference: Option<String>,
    pub commit: Option<String>,
    pub state: PluginState,
    pub detail: Option<String>,
    pub installed_at: Option<String>,
}

/// The store. One `Connection` behind a mutex; long work must **not** hold the
/// lock (ARCHITECTURE §12: DB locks are not held across long work).
pub struct Db {
    conn: std::sync::Mutex<Connection>,
}

impl Db {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, DbError> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, DbError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(sessions::SCHEMA_SESSIONS)?;
        conn.execute_batch(providers::SCHEMA_PROVIDERS)?;
        conn.execute_batch(instance::SCHEMA_INSTANCE)?;
        conn.execute_batch(connections::SCHEMA_CONNECTIONS)?;
        conn.execute_batch(providers::SCHEMA_PROVIDER_OPS)?;
        instance::instance_id(&conn)?;
        // A pre-existing table is not extended by CREATE TABLE IF NOT EXISTS, so
        // a column added after a database was created must be added explicitly.
        // This is the upgrade path: adding a column that is missing (idempotent).
        sessions::migrate(&conn)?;
        providers::migrate(&conn)?;
        Ok(Db { conn: std::sync::Mutex::new(conn) })
    }

    /// The persistent instance id (created once at open). The secret store is
    /// namespaced by this, so a moved data dir keeps its credentials.
    pub fn instance_id(&self) -> Result<String, DbError> {
        let conn = self.conn.lock().expect("db mutex");
        Ok(instance::instance_id(&conn)?)
    }

    fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, rusqlite::Error>,
    ) -> Result<T, DbError> {
        let conn = self.conn.lock().expect("db mutex");
        Ok(f(&conn)?)
    }

    const COLUMNS: &'static str =
        "id, name, summary, plugin_type, source, reference, commit_ref, state, detail, installed_at";

    fn read_row(r: &rusqlite::Row<'_>) -> Result<PluginRow, rusqlite::Error> {
        Ok(PluginRow {
            id: r.get(0)?,
            name: r.get(1)?,
            summary: r.get(2)?,
            plugin_type: r.get(3)?,
            source: r.get(4)?,
            reference: r.get(5)?,
            commit: r.get(6)?,
            state: parse_state(&r.get::<_, String>(7)?),
            detail: r.get(8)?,
            installed_at: r.get(9)?,
        })
    }

    pub fn upsert_plugin(&self, row: &PluginRow) -> Result<(), DbError> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO plugins (id, name, summary, plugin_type, source, reference, commit_ref, state, detail, installed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                   name=excluded.name, summary=excluded.summary, plugin_type=excluded.plugin_type,
                   source=excluded.source, reference=excluded.reference, commit_ref=excluded.commit_ref,
                   state=excluded.state, detail=excluded.detail, installed_at=excluded.installed_at",
                params![
                    row.id, row.name, row.summary, row.plugin_type, row.source,
                    row.reference, row.commit, row.state.as_str(), row.detail, row.installed_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn plugin(&self, id: &str) -> Result<Option<PluginRow>, DbError> {
        self.with(|conn| {
            let sql = format!("SELECT {} FROM plugins WHERE id = ?1", Self::COLUMNS);
            conn.query_row(&sql, params![id], Self::read_row).optional()
        })
    }

    pub fn delete_plugin(&self, id: &str) -> Result<(), DbError> {
        self.with(|conn| {
            conn.execute("DELETE FROM plugins WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    pub fn list_plugins(&self) -> Result<Vec<PluginRow>, DbError> {
        self.with(|conn| {
            let sql = format!("SELECT {} FROM plugins ORDER BY id", Self::COLUMNS);
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], Self::read_row)?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    // --- the operation record (T3 recovery spine) ---------------------------

    pub fn set_step(&self, id: &str, step: InstallStep) -> Result<(), DbError> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO plugin_ops (plugin_id, step, updated_at, backup_retained)
                 VALUES (?1, ?2, datetime('now'), 0)
                 ON CONFLICT(plugin_id) DO UPDATE SET step=excluded.step, updated_at=excluded.updated_at",
                params![id, step_name(step)],
            )?;
            Ok(())
        })
    }

    pub fn mark_backup_retained(&self, id: &str) -> Result<(), DbError> {
        self.with(|conn| {
            conn.execute(
                "UPDATE plugin_ops SET backup_retained = 1 WHERE plugin_id = ?1",
                params![id],
            )?;
            Ok(())
        })
    }

    pub fn backup_retained(&self, id: &str) -> Result<bool, DbError> {
        self.with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT backup_retained FROM plugin_ops WHERE plugin_id = ?1",
                    params![id],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
                .map(|v| v != 0)
                .unwrap_or(false))
        })
    }

    pub fn step(&self, id: &str) -> Result<Option<InstallStep>, DbError> {
        self.with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT step FROM plugin_ops WHERE plugin_id = ?1",
                    params![id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .map(|s| parse_step(&s)))
        })
    }

    pub fn clear_step(&self, id: &str) -> Result<(), DbError> {
        self.with(|conn| {
            conn.execute("DELETE FROM plugin_ops WHERE plugin_id = ?1", params![id])?;
            Ok(())
        })
    }

    pub fn unfinished_ops(&self) -> Result<Vec<(String, InstallStep, bool)>, DbError> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT plugin_id, step, backup_retained FROM plugin_ops ORDER BY plugin_id",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        parse_step(&r.get::<_, String>(1)?),
                        r.get::<_, i64>(2)? != 0,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}

fn parse_state(s: &str) -> PluginState {
    match s {
        "absent" => PluginState::Absent,
        "installing" => PluginState::Installing,
        "preparing" => PluginState::Preparing,
        "removing" => PluginState::Removing,
        "failed" => PluginState::Failed,
        _ => PluginState::Ready,
    }
}

pub(crate) fn step_name(s: InstallStep) -> &'static str {
    match s {
        InstallStep::Staged => "staged",
        InstallStep::OldMovedAside => "old_moved_aside",
        InstallStep::NewInPlace => "new_in_place",
        InstallStep::Committed => "committed",
        InstallStep::RecoveryRetained => "recovery_retained",
    }
}

fn parse_step(s: &str) -> InstallStep {
    match s {
        "staged" => InstallStep::Staged,
        "old_moved_aside" => InstallStep::OldMovedAside,
        "new_in_place" => InstallStep::NewInPlace,
        "committed" => InstallStep::Committed,
        _ => InstallStep::RecoveryRetained,
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS plugins (
  id           TEXT PRIMARY KEY,
  name         TEXT,
  summary      TEXT,
  plugin_type  TEXT,
  source       TEXT,
  reference    TEXT,
  commit_ref   TEXT,
  state        TEXT NOT NULL,
  detail       TEXT,
  installed_at TEXT
);

CREATE TABLE IF NOT EXISTS plugin_ops (
  plugin_id       TEXT PRIMARY KEY,
  step            TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  backup_retained INTEGER NOT NULL DEFAULT 0
);
"#;

/// RFC 3339 UTC, dependency-free (shared by the domains).

/// RFC 3339 UTC (a maintained date library, shared by the domains).
pub fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
