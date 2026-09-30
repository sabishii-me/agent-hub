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
pub use providers::{PendingOp, ProviderRow};
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
    /// The on-disk database is a format this hub does not serve: it is refused,
    /// never migrated or overwritten (no-legacy: ARCHITECTURE 1).
    #[error("unsupported database format: {0}")]
    UnsupportedFormat(String),
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
    /// The release artifact this plugin was installed FROM, if any (the
    /// `{id,version,url,sha256,size}` the install named, stored as JSON). `None`
    /// for a git/local install. Recorded once at install; never re-derived.
    pub artifact: Option<String>,
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

    /// The CURRENT on-disk schema version. Bump ONLY with a deliberate format
    /// change. There is no migration and no compatibility: a database whose version
    /// is neither 0-and-empty nor this value is REFUSED (ARCHITECTURE 1: from
    /// scratch, no migration).
    pub const SCHEMA_VERSION: i64 = 1;

    fn init(conn: Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        // VERSION RECOGNITION COMES FIRST, before any schema write, journal sweep
        // or secret side effect: an unsupported existing format must be refused, not
        // silently rewritten or migrated.
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let has_tables: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' LIMIT 1",
                [],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        match (version, has_tables) {
            (v, _) if v == Self::SCHEMA_VERSION => {}
            (0, false) => {
                // A brand-new, empty database: create the CURRENT schema and stamp
                // the version.
                conn.execute_batch(SCHEMA)?;
                conn.execute_batch(sessions::SCHEMA_SESSIONS)?;
                conn.execute_batch(providers::SCHEMA_PROVIDERS)?;
                conn.execute_batch(instance::SCHEMA_INSTANCE)?;
                conn.execute_batch(connections::SCHEMA_CONNECTIONS)?;
                conn.execute_batch(SCHEMA_HARNESS_STATUS)?;
                conn.execute_batch(SCHEMA_HARNESS_EXTENSIONS)?;
                conn.execute_batch(providers::SCHEMA_PROVIDER_OPS)?;
                conn.pragma_update(None, "user_version", Self::SCHEMA_VERSION)?;
            }
            (0, true) => {
                return Err(DbError::UnsupportedFormat(
                    "the database predates the schema version; this hub does not migrate or overwrite an unsupported format".into(),
                ));
            }
            (other, _) => {
                return Err(DbError::UnsupportedFormat(format!(
                    "the database declares schema version {other}, but this hub expects {}; refusing rather than migrating",
                    Self::SCHEMA_VERSION
                )));
            }
        }

        instance::instance_id(&conn)?;
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
        "id, name, summary, plugin_type, source, reference, commit_ref, state, detail, installed_at, artifact";

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
            artifact: r.get(10)?,
        })
    }

    pub fn upsert_plugin(&self, row: &PluginRow) -> Result<(), DbError> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO plugins (id, name, summary, plugin_type, source, reference, commit_ref, state, detail, installed_at, artifact)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                   name=excluded.name, summary=excluded.summary, plugin_type=excluded.plugin_type,
                   source=excluded.source, reference=excluded.reference, commit_ref=excluded.commit_ref,
                   state=excluded.state, detail=excluded.detail, installed_at=excluded.installed_at,
                   artifact=excluded.artifact",
                params![
                    row.id, row.name, row.summary, row.plugin_type, row.source,
                    row.reference, row.commit, row.state.as_str(), row.detail, row.installed_at,
                    row.artifact
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
  installed_at TEXT,
  artifact     TEXT
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

/// A harness's enable/disable status (`enabled` | `disabled`), the hub's own
/// lifecycle fact. It must survive a restart: a disabled harness refuses session
/// create/turns, and a rescan must not silently re-enable it.
pub const SCHEMA_HARNESS_STATUS: &str = r#"
CREATE TABLE IF NOT EXISTS harness_status (
  harness_id TEXT PRIMARY KEY,
  status     TEXT NOT NULL
);
"#;

impl Db {
    /// Every recorded harness status.
    pub fn harness_statuses(&self) -> Result<Vec<(String, String)>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT harness_id, status FROM harness_status")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn set_harness_status(&self, id: &str, status: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO harness_status (harness_id, status) VALUES (?1, ?2)
             ON CONFLICT(harness_id) DO UPDATE SET status = excluded.status",
            rusqlite::params![id, status],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod harness_status_tests {
    use crate::Db;

    /// The status is durable: a disabled harness reads back as disabled after a
    /// reopen of the same database.
    #[test]
    fn harness_status_survives_a_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "agent-hub-hs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hub.sqlite");
        {
            let db = Db::open(&path).unwrap();
            db.set_harness_status("pi", "disabled").unwrap();
        }
        let db = Db::open(&path).unwrap();
        let statuses = db.harness_statuses().unwrap();
        assert_eq!(
            statuses,
            vec![("pi".to_string(), "disabled".to_string())],
            "a disabled harness survives a reopen"
        );
        // Re-enabling overwrites.
        db.set_harness_status("pi", "enabled").unwrap();
        assert_eq!(db.harness_statuses().unwrap(), vec![("pi".to_string(), "enabled".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The extensions the hub installs for a harness (the selected set). Durable: a
/// PATCH survives a restart and is what the next start installs. Absent = the
/// default (every shipped extension).
pub const SCHEMA_HARNESS_EXTENSIONS: &str = r#"
CREATE TABLE IF NOT EXISTS harness_extensions (
  harness_id TEXT PRIMARY KEY,
  extensions TEXT NOT NULL
);
"#;

impl Db {
    /// The selected extension ids for a harness, as JSON. `None` = never set
    /// (the default applies).
    pub fn harness_extensions(&self, id: &str) -> Result<Option<Vec<String>>, DbError> {
        let conn = self.lock();
        let raw: Option<String> = conn
            .query_row(
                "SELECT extensions FROM harness_extensions WHERE harness_id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }

    pub fn set_harness_extensions(&self, id: &str, extensions: &[String]) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO harness_extensions (harness_id, extensions) VALUES (?1, ?2)
             ON CONFLICT(harness_id) DO UPDATE SET extensions = excluded.extensions",
            rusqlite::params![id, serde_json::to_string(extensions).unwrap_or_else(|_| "[]".into())],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod harness_extensions_tests {
    use crate::Db;

    #[test]
    fn extensions_survive_a_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "agent-hub-hx-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hub.sqlite");
        {
            let db = Db::open(&path).unwrap();
            assert_eq!(db.harness_extensions("pi").unwrap(), None, "unset = default");
            db.set_harness_extensions("pi", &["plan".into()]).unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(
            db.harness_extensions("pi").unwrap(),
            Some(vec!["plan".to_string()])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod schema_version_tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "agent-hub-ver-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.join("hub.sqlite")
    }

    /// A brand-new database is created at the CURRENT version.
    #[test]
    fn a_fresh_database_is_stamped_with_the_current_version() {
        let path = tmp_path("fresh");
        {
            let db = Db::open(&path).unwrap();
            let conn = db.conn.lock().unwrap();
            let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
            assert_eq!(v, Db::SCHEMA_VERSION);
            // The current tables exist.
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='provider_ops'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1);
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The CURRENT format reopens normally.
    #[test]
    fn a_current_database_reopens() {
        let path = tmp_path("reopen");
        Db::open(&path).unwrap();
        Db::open(&path).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A database with tables but NO version (a prior, unsupported format) is
    /// REFUSED: no migration, no overwrite.
    #[test]
    fn an_unversioned_existing_database_is_refused() {
        let path = tmp_path("unversioned");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE something (id TEXT PRIMARY KEY);").unwrap();
        }
        assert!(
            matches!(Db::open(&path), Err(DbError::UnsupportedFormat(_))),
            "an unversioned existing database must be refused"
        );
        // The original table is untouched (never overwritten).
        let conn = rusqlite::Connection::open(&path).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='something'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the unsupported database must not be rewritten");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A FUTURE/other version is refused (not migrated down/up).
    #[test]
    fn a_foreign_schema_version_is_refused() {
        let path = tmp_path("foreign");
        {
            let db = Db::open(&path).unwrap();
            let conn = db.conn.lock().unwrap();
            conn.pragma_update(None, "user_version", 999_i64).unwrap();
        }
        assert!(
            matches!(Db::open(&path), Err(DbError::UnsupportedFormat(_))),
            "a foreign schema version must be refused"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
