//! Session and turn rows (`ARCHITECTURE` §6: the hub's **control state**, kept
//! distinct from a harness's native conversation and the id/ref mapping).

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::DbError;

/// The hub's control state for a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub harness_id: String,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub applied_model: Option<String>,
    pub plan: Option<bool>,
    pub review: Option<bool>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted: bool,
    pub forked_from_session: Option<String>,
    pub forked_from_turn: Option<String>,
    /// The harness's native session ref (its own file/handle). The hub stores it
    /// to resume; it never interprets it.
    pub native_ref: Option<String>,
    /// Why the last start failed (`starting_failed`); null otherwise.
    pub start_error: Option<String>,
}

/// A turn record: the hub's view plus the mapping to the harness's native turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnRow {
    pub id: String,
    pub session_id: String,
    pub state: String,
    pub ended: Option<String>,
    pub native_turn_id: Option<String>,
    pub idempotency_key: String,
    pub created_at: String,
    pub ended_at: Option<String>,
}

pub const SCHEMA_SESSIONS: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
  id                     TEXT PRIMARY KEY,
  harness_id             TEXT NOT NULL,
  model_provider_id      TEXT,
  model_id               TEXT,
  applied_model          TEXT,
  plan                   INTEGER,
  review                 INTEGER,
  cwd                    TEXT,
  title                  TEXT,
  status                 TEXT NOT NULL,
  created_at             TEXT NOT NULL,
  updated_at             TEXT NOT NULL,
  deleted                INTEGER NOT NULL DEFAULT 0,
  forked_from_session    TEXT,
  forked_from_turn       TEXT,
  native_ref             TEXT,
  start_error            TEXT
);

-- The durable command -> result-resource association (ARCHITECTURE 11, R1):
-- reserved BEFORE any side effect, so a retry finds the original and a restart
-- keeps the link. `fingerprint` is the full semantic request.
CREATE TABLE IF NOT EXISTS session_commands (
  command_key  TEXT PRIMARY KEY,
  fingerprint  TEXT NOT NULL,
  session_id   TEXT NOT NULL,
  created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS turns (
  id               TEXT PRIMARY KEY,
  session_id       TEXT NOT NULL,
  state            TEXT NOT NULL,
  ended            TEXT,
  native_turn_id   TEXT,
  idempotency_key  TEXT NOT NULL,
  created_at       TEXT NOT NULL,
  ended_at         TEXT,
  UNIQUE(session_id, idempotency_key)
);
"#;

impl crate::Db {
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("db mutex")
    }

    pub fn insert_session(&self, s: &SessionRow) -> Result<(), DbError> {
        let conn = self.lock();
        Self::insert_session_tx(&conn, s)?;
        Ok(())
    }

    fn insert_session_tx(conn: &Connection, s: &SessionRow) -> Result<(), rusqlite::Error> {
        conn.execute(
            "INSERT INTO sessions (id, harness_id, model_provider_id, model_id, applied_model, plan, review, cwd, title, status, created_at, updated_at, deleted, forked_from_session, forked_from_turn, native_ref, start_error)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
                s.id, s.harness_id, s.model_provider_id, s.model_id, s.applied_model,
                s.plan.map(|b| b as i64), s.review.map(|b| b as i64), s.cwd, s.title,
                s.status, s.created_at, s.updated_at, s.deleted as i64,
                s.forked_from_session, s.forked_from_turn, s.native_ref, s.start_error
            ],
        )?;
        Ok(())
    }

    fn read_session(r: &rusqlite::Row<'_>) -> Result<SessionRow, rusqlite::Error> {
        Ok(SessionRow {
            id: r.get(0)?,
            harness_id: r.get(1)?,
            model_provider_id: r.get(2)?,
            model_id: r.get(3)?,
            applied_model: r.get(4)?,
            plan: r.get::<_, Option<i64>>(5)?.map(|v| v != 0),
            review: r.get::<_, Option<i64>>(6)?.map(|v| v != 0),
            cwd: r.get(7)?,
            title: r.get(8)?,
            status: r.get(9)?,
            created_at: r.get(10)?,
            updated_at: r.get(11)?,
            deleted: r.get::<_, i64>(12)? != 0,
            forked_from_session: r.get(13)?,
            forked_from_turn: r.get(14)?,
            native_ref: r.get(15)?,
            start_error: r.get(16)?,
        })
    }

    const SESSION_COLS: &'static str = "id, harness_id, model_provider_id, model_id, applied_model, plan, review, cwd, title, status, created_at, updated_at, deleted, forked_from_session, forked_from_turn, native_ref, start_error";

    pub fn session(&self, id: &str) -> Result<Option<SessionRow>, DbError> {
        let conn = self.lock();
        let sql = format!("SELECT {} FROM sessions WHERE id = ?1 AND deleted = 0", Self::SESSION_COLS);
        Ok(conn.query_row(&sql, params![id], Self::read_session).optional()?)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRow>, DbError> {
        let conn = self.lock();
        let sql = format!("SELECT {} FROM sessions WHERE deleted = 0 ORDER BY created_at DESC", Self::SESSION_COLS);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::read_session)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn update_session(&self, s: &SessionRow) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute(
            "UPDATE sessions SET model_provider_id=?2, model_id=?3, applied_model=?4, plan=?5, review=?6, title=?7, status=?8, updated_at=?9, native_ref=?10, start_error=?11 WHERE id=?1",
            params![
                s.id, s.model_provider_id, s.model_id, s.applied_model,
                s.plan.map(|b| b as i64), s.review.map(|b| b as i64), s.title, s.status, s.updated_at,
                s.native_ref, s.start_error
            ],
        )?;
        Ok(())
    }

    /// Hard-delete a session row (used to undo a reservation whose body was
    /// refused). Distinct from the soft delete a client asks for.
    pub fn delete_session_row(&self, id: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Remove a command association (undo a refused reservation).
    pub fn remove_session_command(&self, key: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute("DELETE FROM session_commands WHERE command_key = ?1", params![key])?;
        Ok(())
    }

    /// Soft-delete: the export flag is independent, so the row stays readable.
    pub fn mark_session_deleted(&self, id: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute("UPDATE sessions SET deleted = 1 WHERE id = ?1", params![id])?;
        Ok(())
    }
}

/// The outcome of an atomic command reservation.
pub enum ReserveOutcome {
    /// The command was newly reserved together with this `starting` session row.
    Reserved,
    /// The key already maps to this session with the SAME fingerprint (replay).
    Replay(String),
    /// The key maps to a session with a DIFFERENT fingerprint (conflict).
    Conflict,
}

impl crate::Db {
    /// **Atomically** reserve a command key and (when new) insert the `starting`
    /// session row, in ONE transaction. This is the single decision point: there
    /// is no separate lookup that a concurrent request can race.
    ///
    /// * the key is new → `Reserved` (the row was inserted);
    /// * the key exists with the same fingerprint → `Replay(session_id)`;
    /// * the key exists with a different fingerprint → `Conflict`.
    ///
    /// A database error is an error, never silently read as "not found".
    pub fn reserve_session_command(
        &self,
        command_key: &str,
        fingerprint: &str,
        session: &SessionRow,
    ) -> Result<ReserveOutcome, DbError> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT fingerprint, session_id FROM session_commands WHERE command_key = ?1",
                params![command_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let outcome = match existing {
            Some((fp, sid)) if fp == fingerprint => ReserveOutcome::Replay(sid),
            Some(_) => ReserveOutcome::Conflict,
            None => {
                Self::insert_session_tx(&tx, session)?;
                tx.execute(
                    "INSERT INTO session_commands (command_key, fingerprint, session_id, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![command_key, fingerprint, session.id, crate::now_utc()],
                )?;
                ReserveOutcome::Reserved
            }
        };
        tx.commit()?;
        Ok(outcome)
    }

    // --- turns ---

    pub fn insert_turn(&self, t: &TurnRow) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO turns (id, session_id, state, ended, native_turn_id, idempotency_key, created_at, ended_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![t.id, t.session_id, t.state, t.ended, t.native_turn_id, t.idempotency_key, t.created_at, t.ended_at],
        )?;
        Ok(())
    }

    fn read_turn(r: &rusqlite::Row<'_>) -> Result<TurnRow, rusqlite::Error> {
        Ok(TurnRow {
            id: r.get(0)?,
            session_id: r.get(1)?,
            state: r.get(2)?,
            ended: r.get(3)?,
            native_turn_id: r.get(4)?,
            idempotency_key: r.get(5)?,
            created_at: r.get(6)?,
            ended_at: r.get(7)?,
        })
    }

    const TURN_COLS: &'static str = "id, session_id, state, ended, native_turn_id, idempotency_key, created_at, ended_at";

    /// A turn by its logical command identity (R1): the same key returns the
    /// same turn, so a retried `202` never starts a second turn.
    pub fn turn_by_key(&self, session_id: &str, key: &str) -> Result<Option<TurnRow>, DbError> {
        let conn = self.lock();
        let sql = format!("SELECT {} FROM turns WHERE session_id = ?1 AND idempotency_key = ?2", Self::TURN_COLS);
        Ok(conn.query_row(&sql, params![session_id, key], Self::read_turn).optional()?)
    }

    pub fn turn(&self, id: &str) -> Result<Option<TurnRow>, DbError> {
        let conn = self.lock();
        let sql = format!("SELECT {} FROM turns WHERE id = ?1", Self::TURN_COLS);
        Ok(conn.query_row(&sql, params![id], Self::read_turn).optional()?)
    }

    pub fn list_turns(&self, session_id: &str) -> Result<Vec<TurnRow>, DbError> {
        let conn = self.lock();
        let sql = format!("SELECT {} FROM turns WHERE session_id = ?1 ORDER BY created_at", Self::TURN_COLS);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![session_id], Self::read_turn)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The turn currently running in a session, if any.
    pub fn active_turn(&self, session_id: &str) -> Result<Option<TurnRow>, DbError> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {} FROM turns WHERE session_id = ?1 AND ended IS NULL ORDER BY created_at DESC LIMIT 1",
            Self::TURN_COLS
        );
        Ok(conn.query_row(&sql, params![session_id], Self::read_turn).optional()?)
    }

    pub fn set_turn_state(&self, id: &str, state: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute("UPDATE turns SET state = ?2 WHERE id = ?1", params![id, state])?;
        Ok(())
    }

    pub fn end_turn(&self, id: &str, ended: &str, at: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute(
            "UPDATE turns SET state='ended', ended=?2, ended_at=?3 WHERE id=?1",
            params![id, ended, at],
        )?;
        Ok(())
    }
}

/// The upgrade path: `CREATE TABLE IF NOT EXISTS` never alters an existing table,
/// so a column added after a database was created must be added explicitly. This
/// is idempotent and additive only (no data loss); a database created by an older
/// build gains the new columns on open.
pub fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
    let add = |table: &str, column: &str, decl: &str| -> Result<(), rusqlite::Error> {
        let exists: bool = conn
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|name| name == column);
        if !exists {
            conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"), [])?;
        }
        Ok(())
    };
    // Sessions columns added after the first release.
    add("sessions", "native_ref", "TEXT")?;
    add("sessions", "start_error", "TEXT")?;
    Ok(())
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    #[test]
    fn an_old_sessions_table_gains_the_new_columns() {
        // Simulate a database created before start_error existed.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY, harness_id TEXT NOT NULL,
                model_provider_id TEXT, model_id TEXT, applied_model TEXT,
                plan INTEGER, review INTEGER, cwd TEXT, title TEXT,
                status TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                deleted INTEGER NOT NULL DEFAULT 0,
                forked_from_session TEXT, forked_from_turn TEXT
            );",
        )
        .unwrap();
        // The new code path runs the migration, then the columns exist.
        migrate(&conn).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(sessions)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(cols.contains(&"native_ref".to_string()));
        assert!(cols.contains(&"start_error".to_string()));
        // Idempotent: a second call is a no-op.
        migrate(&conn).unwrap();
    }
}
