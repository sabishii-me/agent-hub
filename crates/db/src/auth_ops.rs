//! Durable hub-level authorization operations (ADR-0012). An auth sub-operation is a
//! real resource that OUTLIVES the starting request and SURVIVES a restart
//! (ARCHITECTURE §438): its state is read from the database, not from process memory.
//!
//! The row holds only what the contract's `authOperation` names: the status, the step
//! shown while pending, the error, and the account. The platform's raw flow data (the
//! device_code the hub polls with) is NOT part of the contract and is kept in the
//! `flow` column as opaque, hub-private state (ADR-0011: implementation is private).

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
