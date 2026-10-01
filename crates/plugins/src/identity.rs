//! Logical **command identity** (ARCHITECTURE §11, R1).
//!
//! A resource identity (`{id}`) locates a plugin and serializes conflicting
//! commands on it; it does **not** identify an intent. A retry of a lost `202`
//! and a deliberate second command (install v1, then v2) can share the same
//! target, so a separate identity distinguishes them.

use std::collections::HashMap;
use std::sync::Mutex;

/// The outcome of presenting a command identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Idempotency {
    /// First time this identity is seen; run the command.
    New,
    /// Seen before with the **same** request: return the original result.
    Replay,
    /// Seen before with **different** parameters: an explicit conflict.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    /// A canonical fingerprint of the semantic request.
    request: String,
}

/// A bounded store of seen command identities, with a retention window.
pub struct CommandIds {
    seen: Mutex<HashMap<String, Record>>,
    capacity: usize,
    order: Mutex<Vec<String>>,
}

impl CommandIds {
    pub fn new(capacity: usize) -> Self {
        CommandIds {
            seen: Mutex::new(HashMap::new()),
            capacity,
            order: Mutex::new(Vec::new()),
        }
    }

    /// Present `key` with a request fingerprint. `New` means "run it"; `Replay`
    /// means "return what you produced before"; `Conflict` means refuse.
    pub fn present(&self, key: &str, request: &str) -> Idempotency {
        let mut seen = self.seen.lock().expect("ids mutex");
        if let Some(rec) = seen.get(key) {
            return if rec.request == request {
                Idempotency::Replay
            } else {
                Idempotency::Conflict
            };
        }
        seen.insert(key.to_string(), Record { request: request.to_string() });

        let mut order = self.order.lock().expect("order mutex");
        order.push(key.to_string());
        while order.len() > self.capacity {
            let evicted = order.remove(0);
            seen.remove(&evicted);
        }
        Idempotency::New
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_same_request_replays() {
        let ids = CommandIds::new(8);
        assert_eq!(ids.present("k", "install:v1"), Idempotency::New);
        assert_eq!(ids.present("k", "install:v1"), Idempotency::Replay);
    }

    #[test]
    fn same_key_different_request_conflicts() {
        let ids = CommandIds::new(8);
        assert_eq!(ids.present("k", "install:v1"), Idempotency::New);
        assert_eq!(ids.present("k", "install:v2"), Idempotency::Conflict);
    }

    #[test]
    fn a_new_key_is_a_new_intent_even_for_the_same_target() {
        let ids = CommandIds::new(8);
        assert_eq!(ids.present("k1", "install:p@v1"), Idempotency::New);
        // A deliberate upgrade is a new identity, not a retry.
        assert_eq!(ids.present("k2", "install:p@v2"), Idempotency::New);
    }

    #[test]
    fn eviction_makes_an_old_key_new_again() {
        let ids = CommandIds::new(1);
        assert_eq!(ids.present("k1", "a"), Idempotency::New);
        assert_eq!(ids.present("k2", "b"), Idempotency::New);
        // k1 fell out of the window.
        assert_eq!(ids.present("k1", "a"), Idempotency::New);
    }
}
