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
    /// The provider identity the adapter CONFIRMED (`applied.modelProviderId`).
    pub applied_provider: Option<String>,
    /// The native route the adapter resolved (`applied.connectionId`).
    pub applied_route: Option<String>,
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
    pub intent: String,
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
  applied_provider       TEXT,
  applied_route          TEXT,
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
  intent           TEXT NOT NULL DEFAULT '',
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
            "INSERT INTO sessions (id, harness_id, model_provider_id, model_id, applied_model, applied_provider, applied_route, plan, review, cwd, title, status, created_at, updated_at, deleted, forked_from_session, forked_from_turn, native_ref, start_error)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
            params![
                s.id, s.harness_id, s.model_provider_id, s.model_id, s.applied_model, s.applied_provider, s.applied_route,
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
            applied_provider: r.get(5)?,
            applied_route: r.get(6)?,
            plan: r.get::<_, Option<i64>>(7)?.map(|v| v != 0),
            review: r.get::<_, Option<i64>>(8)?.map(|v| v != 0),
            cwd: r.get(9)?,
            title: r.get(10)?,
            status: r.get(11)?,
            created_at: r.get(12)?,
            updated_at: r.get(13)?,
            deleted: r.get::<_, i64>(14)? != 0,
            forked_from_session: r.get(15)?,
            forked_from_turn: r.get(16)?,
            native_ref: r.get(17)?,
            start_error: r.get(18)?,
        })
    }

    const SESSION_COLS: &'static str = "id, harness_id, model_provider_id, model_id, applied_model, applied_provider, applied_route, plan, review, cwd, title, status, created_at, updated_at, deleted, forked_from_session, forked_from_turn, native_ref, start_error";

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
            "UPDATE sessions SET model_provider_id=?2, model_id=?3, applied_model=?4, applied_provider=?5, applied_route=?6, plan=?7, review=?8, title=?9, status=?10, updated_at=?11, native_ref=?12, start_error=?13 WHERE id=?1",
            params![
                s.id, s.model_provider_id, s.model_id, s.applied_model, s.applied_provider, s.applied_route,
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

/// The outcome of admitting a turn (one atomic decision: identity + busy).
#[derive(Debug, Clone, PartialEq)]
pub enum TurnAdmission {
    /// A new turn row was inserted (the session was idle).
    Reserved,
    /// The session already has an active turn; no row was inserted.
    Busy(String),
    /// The same key with the SAME intent: return that turn.
    Replay(String),
    /// The same key with a DIFFERENT intent.
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

    /// **Atomically** admit a turn: in ONE transaction, decide the command
    /// identity AND whether the session is busy. There is no window where a turn
    /// is inserted and the busy check runs after (the previous bug admitted a
    /// second turn before noticing the first was running).
    ///
    /// * the key is new AND the session has no active turn -> `Reserved`;
    /// * the key is new but the session HAS an active turn -> `Busy`;
    /// * the key exists with the same intent -> `Replay(id)`;
    /// * the key exists with a different intent -> `Conflict`.
    ///
    /// A refused admission leaves **no** `admitted` row.
    pub fn admit_turn(
        &self,
        session_id: &str,
        idempotency_key: &str,
        intent: &str,
        turn_id: &str,
    ) -> Result<TurnAdmission, DbError> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT intent, id FROM turns WHERE session_id = ?1 AND idempotency_key = ?2",
                params![session_id, idempotency_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((it, id)) = existing {
            tx.commit()?;
            return Ok(if it == intent {
                TurnAdmission::Replay(id)
            } else {
                TurnAdmission::Conflict
            });
        }
        let busy: Option<String> = tx
            .query_row(
                "SELECT id FROM turns WHERE session_id = ?1 AND ended IS NULL LIMIT 1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(active) = busy {
            tx.commit()?;
            return Ok(TurnAdmission::Busy(active));
        }
        tx.execute(
            "INSERT INTO turns (id, session_id, state, ended, native_turn_id, idempotency_key, intent, created_at, ended_at)
             VALUES (?1, ?2, 'admitted', NULL, NULL, ?3, ?4, ?5, NULL)",
            params![turn_id, session_id, idempotency_key, intent, crate::now_utc()],
        )?;
        tx.commit()?;
        Ok(TurnAdmission::Reserved)
    }

    pub fn insert_turn(&self, t: &TurnRow) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO turns (id, session_id, state, ended, native_turn_id, idempotency_key, intent, created_at, ended_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![t.id, t.session_id, t.state, t.ended, t.native_turn_id, t.idempotency_key, t.intent, t.created_at, t.ended_at],
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
            intent: r.get(6)?,
            created_at: r.get(7)?,
            ended_at: r.get(8)?,
        })
    }

    const TURN_COLS: &'static str = "id, session_id, state, ended, native_turn_id, idempotency_key, intent, created_at, ended_at";

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

    /// Move a turn to a **non-terminal** state (`running`, `cancelling`). This
    /// refuses (returns `false`) if the turn is already terminal, so a late
    /// lifecycle write can never resurrect a settled turn (the previous bug let a
    /// cancel write `cancelling` after the terminal had been written).
    pub fn set_turn_state(&self, id: &str, state: &str) -> Result<bool, DbError> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE turns SET state = ?2 WHERE id = ?1 AND ended IS NULL",
            params![id, state],
        )?;
        Ok(n > 0)
    }

    /// Commit the terminal state **once**. The guard is in the SQL: only a turn
    /// whose `ended` is still NULL can be settled, so two racing settles cannot
    /// both win and a decided terminal is never overwritten. Returns `true` when
    /// THIS call performed the settle (the caller publishes only on `true`).
    pub fn end_turn(&self, id: &str, ended: &str, at: &str) -> Result<bool, DbError> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE turns SET state='ended', ended=?2, ended_at=?3 WHERE id=?1 AND ended IS NULL AND state != 'ended'",
            params![id, ended, at],
        )?;
        Ok(n > 0)
    }

    /// Session ids that have a `cancelling` turn (for the stalled-cancel sweep).
    pub fn sessions_with_cancelling_turns(&self) -> Result<Vec<String>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT DISTINCT session_id FROM turns WHERE state = 'cancelling' AND ended IS NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }


}

/// The upgrade path: `CREATE TABLE IF NOT EXISTS` never alters an existing table,
/// so a column added after a database was created must be added explicitly. This
/// is idempotent and additive only (no data loss); a database created by an older
/// build gains the new columns on open.
pub fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
    let add = |table: &str, column: &str, decl: &str| -> Result<(), rusqlite::Error> {
        // A table that does not exist yet is created fresh by the schema; only an
        // EXISTING table needs an added column.
        let table_exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !table_exists {
            return Ok(());
        }
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
    add("sessions", "applied_provider", "TEXT")?;
    add("sessions", "applied_route", "TEXT")?;
    add("turns", "intent", "TEXT NOT NULL DEFAULT ''")?;
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

#[cfg(test)]
mod turn_admission_tests {
    use super::*;
    use crate::Db;

    fn session(db: &Db, id: &str) {
        db.insert_session(&SessionRow {
            id: id.into(),
            harness_id: "pi".into(),
            model_provider_id: None,
            model_id: None,
            applied_model: None,
            applied_provider: None,
            applied_route: None,
            plan: None,
            review: None,
            cwd: None,
            title: None,
            status: "active".into(),
            created_at: crate::now_utc(),
            updated_at: crate::now_utc(),
            deleted: false,
            forked_from_session: None,
            forked_from_turn: None,
            native_ref: None,
            start_error: None,
        })
        .unwrap();
    }

    /// A second turn is refused WITHOUT inserting an admitted row: the busy check
    /// and the admission are one decision.
    #[test]
    fn a_second_turn_is_refused_and_leaves_no_row() {
        let db = Db::open_in_memory().unwrap();
        session(&db, "s1");
        assert_eq!(
            db.admit_turn("s1", "k1", "turn:hello", "t1").unwrap(),
            TurnAdmission::Reserved
        );
        // The session is busy: a DELIBERATE new key is refused, and no row exists.
        assert!(matches!(
            db.admit_turn("s1", "k2", "turn:again", "t2").unwrap(),
            TurnAdmission::Busy(_)
        ));
        assert!(db.turn("t2").unwrap().is_none(), "a refused turn must not be inserted");
        assert_eq!(db.list_turns("s1").unwrap().len(), 1);
    }

    #[test]
    fn the_same_key_replays_and_a_different_intent_conflicts() {
        let db = Db::open_in_memory().unwrap();
        session(&db, "s1");
        assert_eq!(db.admit_turn("s1", "k", "turn:a", "t1").unwrap(), TurnAdmission::Reserved);
        assert_eq!(
            db.admit_turn("s1", "k", "turn:a", "t2").unwrap(),
            TurnAdmission::Replay("t1".into())
        );
        assert_eq!(db.admit_turn("s1", "k", "turn:b", "t3").unwrap(), TurnAdmission::Conflict);
    }

    /// The terminal is committed once; a late non-terminal write cannot resurrect.
    #[test]
    fn the_terminal_is_committed_once_and_not_resurrected() {
        let db = Db::open_in_memory().unwrap();
        session(&db, "s1");
        db.admit_turn("s1", "k", "turn:a", "t1").unwrap();
        assert!(db.end_turn("t1", "completed", &crate::now_utc()).unwrap(), "first settle wins");
        assert!(!db.end_turn("t1", "cancelled", &crate::now_utc()).unwrap(), "second settle loses");
        // A late `cancelling` write is refused because the turn is terminal.
        assert!(!db.set_turn_state("t1", "cancelling").unwrap());
        let t = db.turn("t1").unwrap().unwrap();
        assert_eq!(t.state, "ended");
        assert_eq!(t.ended.as_deref(), Some("completed"), "the terminal is not overwritten");
    }

    /// After a terminal, the session is idle again: a new turn is admitted.
    #[test]
    fn a_finished_turn_frees_the_session() {
        let db = Db::open_in_memory().unwrap();
        session(&db, "s1");
        db.admit_turn("s1", "k1", "turn:a", "t1").unwrap();
        db.end_turn("t1", "completed", &crate::now_utc()).unwrap();
        assert_eq!(db.admit_turn("s1", "k2", "turn:b", "t2").unwrap(), TurnAdmission::Reserved);
    }
}

#[cfg(test)]
mod cancel_hold_tests {
    use super::*;
    use crate::Db;

    /// A `cancelling` turn still counts as ACTIVE (`ended IS NULL`): an
    /// unconfirmed cancel must NOT release busy. It appears in the stalled sweep.
    #[test]
    fn a_cancelling_turn_holds_the_session_and_is_swept() {
        let db = Db::open_in_memory().unwrap();
        db.insert_session(&SessionRow {
            id: "s1".into(),
            harness_id: "pi".into(),
            model_provider_id: None,
            model_id: None,
            applied_model: None,
            applied_provider: None,
            applied_route: None,
            plan: None,
            review: None,
            cwd: None,
            title: None,
            status: "active".into(),
            created_at: crate::now_utc(),
            updated_at: crate::now_utc(),
            deleted: false,
            forked_from_session: None,
            forked_from_turn: None,
            native_ref: None,
            start_error: None,
        })
        .unwrap();
        db.admit_turn("s1", "k", "turn:a", "t1").unwrap();
        db.set_turn_state("t1", "cancelling").unwrap();

        // Still active: a new turn is refused (busy held).
        assert!(matches!(
            db.admit_turn("s1", "k2", "turn:b", "t2").unwrap(),
            TurnAdmission::Busy(_)
        ));
        assert_eq!(db.sessions_with_cancelling_turns().unwrap(), vec!["s1".to_string()]);

        // After the stalled-cancel sweep settles it, the session frees up.
        assert!(db.end_turn("t1", "interrupted", &crate::now_utc()).unwrap());
        assert!(db.sessions_with_cancelling_turns().unwrap().is_empty());
        assert_eq!(db.admit_turn("s1", "k3", "turn:c", "t3").unwrap(), TurnAdmission::Reserved);
    }
}
