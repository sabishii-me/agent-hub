//! Durable hub-level authorization operations. An auth sub-operation is a
//! real resource that OUTLIVES the starting request and SURVIVES a restart
//!: its state is read from the database, not from process memory.
//!
//! The row holds only what the contract's `authOperation` names: the status, the step
//! shown while pending, the error, and the account. The platform's raw flow data (the
//! device_code the hub polls with) is NOT part of the contract and is kept in the
//! `flow` column as opaque, hub-private state.

use rusqlite::params;

use crate::{now_utc, Db, DbError};

pub const SCHEMA_AUTH_OPS: &str = "
CREATE TABLE IF NOT EXISTS auth_ops (
    id           TEXT PRIMARY KEY,
    provider     TEXT NOT NULL,
    kind         TEXT NOT NULL,
    status       TEXT NOT NULL,
    next         TEXT,
    error        TEXT,
    account      TEXT,
    flow         TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
";

/// One durable auth operation row.
#[derive(Debug, Clone)]
pub struct AuthOpRow {
    pub id: String,
    pub provider: String,
    pub kind: String,
    pub status: String,
    pub next: Option<String>,
    pub error: Option<String>,
    pub account: Option<String>,
    pub flow: Option<String>,
}

impl Db {
    pub fn auth_op_create(
        &self,
        id: &str,
        provider: &str,
        kind: &str,
        next: &str,
        flow: &str,
    ) -> Result<(), DbError> {
        let now = now_utc();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO auth_ops (id, provider, kind, status, next, error, account, flow, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'pending', ?4, NULL, NULL, ?5, ?6, ?6)",
            params![id, provider, kind, next, flow, now],
        )?;
        Ok(())
    }

    /// Create a PENDING operation, minting its id from the EXISTING ROWS (never a
    /// process counter): the next free `{provider}-auth-<n>` is chosen by asking
    /// the table, and a lost race retries with the next `n`. A restart therefore
    /// never re-issues a live id, and a collision is invisible to the caller.
    /// Returns the id it created.
    pub fn auth_op_create_next(
        &self,
        provider: &str,
        kind: &str,
        next: &str,
        flow: &str,
    ) -> Result<String, DbError> {
        let prefix = format!("{provider}-auth-");
        // The highest n in use for this provider, derived from the rows (read under
        // the same lock as the insert, so two callers cannot pick the same n).
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let max_n: i64 = tx.query_row(
            "SELECT COALESCE(MAX(CAST(SUBSTR(id, ?1) AS INTEGER)), 0) \
             FROM auth_ops WHERE id LIKE ?2 || '%' AND CAST(SUBSTR(id, ?1) AS INTEGER) > 0",
            params![prefix.len() as i64 + 1, prefix],
            |r| r.get(0),
        )?;
        let now = now_utc();
        let mut n = max_n + 1;
        loop {
            let id = format!("{prefix}{n}");
            let res = tx.execute(
                "INSERT INTO auth_ops (id, provider, kind, status, next, error, account, flow, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 'pending', ?4, NULL, NULL, ?5, ?6, ?6)",
                params![id, provider, kind, next, flow, now],
            );
            match res {
                Ok(_) => {
                    tx.commit()?;
                    return Ok(id);
                }
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    // The id is taken (a concurrent create); try the next n.
                    n += 1;
                    if n > max_n + 10_000 {
                        return Err(DbError::Conflict(
                            "could not allocate an authorization operation id".into(),
                        ));
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    pub fn auth_op_get(&self, id: &str) -> Result<Option<AuthOpRow>, DbError> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT id, provider, kind, status, next, error, account, flow FROM auth_ops WHERE id = ?1",
                params![id],
                |r| {
                    Ok(AuthOpRow {
                        id: r.get(0)?,
                        provider: r.get(1)?,
                        kind: r.get(2)?,
                        status: r.get(3)?,
                        next: r.get(4)?,
                        error: r.get(5)?,
                        account: r.get(6)?,
                        flow: r.get(7)?,
                    })
                },
            )
            .ok();
        Ok(row)
    }

    /// Record a terminal (or updated) state. `next` is cleared on a terminal state.
    pub fn auth_op_finish(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
        account: Option<&str>,
    ) -> Result<(), DbError> {
        let now = now_utc();
        let conn = self.lock();
        conn.execute(
            "UPDATE auth_ops SET status = ?2, next = NULL, error = ?3, account = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, status, error, account, now],
        )?;
        Ok(())
    }

    /// Mark a pending operation cancelled. Idempotent; a terminal op is unchanged.
    pub fn auth_op_cancel(&self, id: &str) -> Result<bool, DbError> {
        let now = now_utc();
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE auth_ops SET status = 'cancelled', next = NULL, updated_at = ?2 WHERE id = ?1 AND status = 'pending'",
            params![id, now],
        )?;
        Ok(n > 0)
    }

    /// Operations left `pending` at boot: their poller died with the process. The
    /// operation itself SURVIVES (its state is readable); it is the pending work that
    /// cannot continue, so it is reported `failed` with a reason the caller can act on,
    /// rather than left `pending` forever.
    pub fn auth_op_pending(&self) -> Result<Vec<AuthOpRow>, DbError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, provider, kind, status, next, error, account, flow FROM auth_ops WHERE status = 'pending'",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AuthOpRow {
                    id: r.get(0)?,
                    provider: r.get(1)?,
                    kind: r.get(2)?,
                    status: r.get(3)?,
                    next: r.get(4)?,
                    error: r.get(5)?,
                    account: r.get(6)?,
                    flow: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
