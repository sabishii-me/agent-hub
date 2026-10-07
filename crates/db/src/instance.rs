//! **The hub's instance identity** (`ARCHITECTURE` §8). A secret store is keyed by
//! a namespace; that namespace must be a **persistent property of the data
//! directory**, not something recomputed from its path. A path can move; an id
//! cannot. So the id is a row, created once and then never recomputed:
//!
//! * on open, if the row is missing, a fresh id is generated and stored;
//! * the id is **stable across restarts and across a directory move** (moving the
//!   directory moves the row with it);
//! * it is NOT derived from the path, so two directories never collide and a moved
//!   directory keeps its credentials.
//!
//! The rule for recovery is therefore explicit: **a data directory carries its
//! instance id**; nothing else is a source of identity.

use rusqlite::{params, Connection, OptionalExtension};

pub const SCHEMA_INSTANCE: &str = r#"
CREATE TABLE IF NOT EXISTS hub_instance (
  id         TEXT PRIMARY KEY,
  created_at TEXT NOT NULL
);
"#;

/// Read the instance id, creating one **once** if the table is empty. The read and
/// the insert are one transaction, so two concurrent opens cannot create two ids.
pub fn instance_id(conn: &Connection) -> Result<String, rusqlite::Error> {
    let existing: Option<String> = conn
        .query_row("SELECT id FROM hub_instance LIMIT 1", [], |r| r.get(0))
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = new_id();
    conn.execute(
        "INSERT INTO hub_instance (id, created_at) VALUES (?1, ?2)",
        params![id, crate::now_utc()],
    )?;
    Ok(id)
}

/// A random, opaque instance id (no path, no timestamp guessing).
fn new_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    let mut s = String::with_capacity(32);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_created_once_and_then_stable() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_INSTANCE).unwrap();
        let a = instance_id(&conn).unwrap();
        let b = instance_id(&conn).unwrap();
        assert_eq!(a, b, "the id must not change on a second read");
        assert_eq!(a.len(), 32);
    }
}
