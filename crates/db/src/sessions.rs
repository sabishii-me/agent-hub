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
    /// The preset the session was created with (opaque id; null = harness default).
    pub preset_id: Option<String>,
    /// The preset the harness confirmed it applied (`applied.preset`).
    pub applied_preset: Option<String>,
    pub plan: Option<bool>,
    pub review: Option<bool>,
    /// The plan state the harness confirmed (`applied.plan`).
    pub applied_plan: Option<bool>,
    /// The review state the harness confirmed (`applied.review`).
    pub applied_review: Option<bool>,
    pub cwd: Option<String>,
    /// Extra roots the harness MAY activate (absolute), JSON-encoded.
    pub additional_dirs: Vec<String>,
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
  preset_id              TEXT,
  applied_preset         TEXT,
  plan                   INTEGER,
  review                 INTEGER,
  applied_plan           INTEGER,
  applied_review         INTEGER,
  cwd                    TEXT,
  additional_dirs        TEXT NOT NULL DEFAULT '[]',
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
  cancel_requested_at TEXT,
  running_at         TEXT,
  process_gen        INTEGER,
  terminal_intent    TEXT,
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
            "INSERT INTO sessions (id, harness_id, model_provider_id, model_id, applied_model, applied_provider, applied_route, preset_id, applied_preset, plan, review, applied_plan, applied_review, cwd, additional_dirs, title, status, created_at, updated_at, deleted, forked_from_session, forked_from_turn, native_ref, start_error)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
            params![
                s.id, s.harness_id, s.model_provider_id, s.model_id, s.applied_model, s.applied_provider, s.applied_route, s.preset_id, s.applied_preset,
                s.plan.map(|b| b as i64), s.review.map(|b| b as i64),
                s.applied_plan.map(|b| b as i64), s.applied_review.map(|b| b as i64),
                s.cwd, serde_json::to_string(&s.additional_dirs).unwrap_or_else(|_| "[]".into()), s.title,
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
            preset_id: r.get(7)?,
            applied_preset: r.get(8)?,
            plan: r.get::<_, Option<i64>>(9)?.map(|v| v != 0),
            review: r.get::<_, Option<i64>>(10)?.map(|v| v != 0),
            applied_plan: r.get::<_, Option<i64>>(11)?.map(|v| v != 0),
            applied_review: r.get::<_, Option<i64>>(12)?.map(|v| v != 0),
            cwd: r.get(13)?,
            additional_dirs: r
                .get::<_, Option<String>>(14)?
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            title: r.get(15)?,
            status: r.get(16)?,
            created_at: r.get(17)?,
            updated_at: r.get(18)?,
            deleted: r.get::<_, i64>(19)? != 0,
            forked_from_session: r.get(20)?,
            forked_from_turn: r.get(21)?,
            native_ref: r.get(22)?,
            start_error: r.get(23)?,
        })
    }

    const SESSION_COLS: &'static str = "id, harness_id, model_provider_id, model_id, applied_model, applied_provider, applied_route, preset_id, applied_preset, plan, review, applied_plan, applied_review, cwd, additional_dirs, title, status, created_at, updated_at, deleted, forked_from_session, forked_from_turn, native_ref, start_error";

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
            "UPDATE sessions SET model_provider_id=?2, model_id=?3, applied_model=?4, applied_provider=?5, applied_route=?6, preset_id=?7, applied_preset=?8, plan=?9, review=?10, applied_plan=?11, applied_review=?12, additional_dirs=?13, title=?14, status=?15, updated_at=?16, native_ref=?17, start_error=?18 WHERE id=?1",
            params![
                s.id, s.model_provider_id, s.model_id, s.applied_model, s.applied_provider, s.applied_route, s.preset_id, s.applied_preset,
                s.plan.map(|b| b as i64), s.review.map(|b| b as i64),
                s.applied_plan.map(|b| b as i64), s.applied_review.map(|b| b as i64),
                serde_json::to_string(&s.additional_dirs).unwrap_or_else(|_| "[]".into()),
                s.title, s.status, s.updated_at,
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

    /// Atomically claim the DISPATCH of a turn: `admitted` -> `running`, and only
    /// if the turn is STILL `admitted` (no cancel won the race) and non-terminal.
    /// Returns true for the caller that won; a `cancelling`/terminal turn returns
    /// false, so the winner is exactly one of dispatch or cancel (TASK-048 F4).
    /// Atomically claim `admitted` -> `running`, RECORDING the process generation
    /// that will run it. A later cancel compares the CURRENT generation to this
    /// value, so an abort is bound to the process the turn actually dispatched on
    /// (TASK-048 F4/S2).
    pub fn claim_running(&self, id: &str, process_generation: u64) -> Result<bool, DbError> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE turns SET state='running', running_at=?2, process_gen=?3 WHERE id=?1 AND state='admitted' AND ended IS NULL",
            params![id, crate::now_utc(), process_generation as i64],
        )?;
        Ok(n > 0)
    }

    /// The process generation recorded when this turn was dispatched (if any).
    pub fn turn_process_gen(&self, id: &str) -> Result<Option<u64>, DbError> {
        let conn = self.lock();
        let v: Option<i64> = conn
            .query_row("SELECT process_gen FROM turns WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        Ok(v.map(|x| x as u64))
    }

    /// Move a turn to a **non-terminal** state (`running`, `cancelling`). This
    /// refuses (returns `false`) if the turn is already terminal, so a late
    /// lifecycle write can never resurrect a settled turn (the previous bug let a
    /// cancel write `cancelling` after the terminal had been written).
    pub fn set_turn_state(&self, id: &str, state: &str) -> Result<bool, DbError> {
        let conn = self.lock();
        // Entering `cancelling` records WHEN, so a core-side cancel timeout can see
        // an unconfirmed stop (TASK-048 F4).
        let n = if state == "cancelling" {
            // A cancel wins over a not-yet-dispatched turn too (admitted), so it is
            // an atomic alternative to claim_running (TASK-048 F4).
            conn.execute(
                "UPDATE turns SET state = ?2, cancel_requested_at = ?3 WHERE id = ?1 AND ended IS NULL AND state IN ('admitted','running','cancelling')",
                params![id, state, crate::now_utc()],
            )?
        } else if state == "running" {
            conn.execute(
                "UPDATE turns SET state = ?2, running_at = ?3 WHERE id = ?1 AND ended IS NULL",
                params![id, state, crate::now_utc()],
            )?
        } else {
            conn.execute(
                "UPDATE turns SET state = ?2 WHERE id = ?1 AND ended IS NULL",
                params![id, state],
            )?
        };
        Ok(n > 0)
    }

    /// `(id, session_id, state)` of `cancelling` turns whose cancel was delivered
    /// but never confirmed by the harness. There is NO execution deadline for a
    /// `running` turn (a turn runs until the harness ends it); the core settles a
    /// `cancelling` turn on a real action (adapter-v1:387).
    pub fn unconfirmed_cancels(&self) -> Result<Vec<(String, String, String)>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, state FROM turns
             WHERE ended IS NULL AND state = 'cancelling'",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Commit the terminal state **once**. The guard is in the SQL: only a turn
    /// whose `ended` is still NULL can be settled, so two racing settles cannot
    /// both win and a decided terminal is never overwritten. Returns `true` when
    /// THIS call performed the settle (the caller publishes only on `true`).
    pub fn end_turn(&self, id: &str, ended: &str, at: &str) -> Result<bool, DbError> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE turns SET state='ended', ended=?2, ended_at=?3, terminal_intent=NULL WHERE id=?1 AND ended IS NULL AND state != 'ended'",
            params![id, ended, at],
        )?;
        Ok(n > 0)
    }

    /// Record the INTENDED terminal for a turn before attempting to write it. If
    /// the terminal write then fails, this durable intent lets a later reconcile
    /// finish the job instead of forgetting the known terminal (TASK-048 S3).
    pub fn set_terminal_intent(&self, id: &str, intent: &str) -> Result<(), DbError> {
        let conn = self.lock();
        conn.execute(
            "UPDATE turns SET terminal_intent=?2 WHERE id=?1 AND ended IS NULL",
            params![id, intent],
        )?;
        Ok(())
    }

    /// Every turn with an unapplied terminal intent (for reconciliation): the
    /// `(id, session_id, intent)` of each.
    pub fn pending_terminal_intents(&self) -> Result<Vec<(String, String, String)>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, terminal_intent FROM turns WHERE ended IS NULL AND terminal_intent IS NOT NULL",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Session ids that have a `cancelling` turn (for the stalled-cancel sweep).
    pub fn sessions_with_cancelling_turns(&self) -> Result<Vec<String>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT DISTINCT session_id FROM turns WHERE state = 'cancelling' AND ended IS NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
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
            preset_id: None,
            applied_preset: None,
            plan: None,
            review: None,
            applied_plan: None,
            applied_review: None,
            cwd: None,
            additional_dirs: Vec::new(),
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
            preset_id: None,
            applied_preset: None,
            plan: None,
            review: None,
            applied_plan: None,
            applied_review: None,
            cwd: None,
            additional_dirs: Vec::new(),
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
