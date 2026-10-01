//! Hub-level provider authorization OPERATIONS. The hub owns the flow; a type that
//! declares no interactive auth method is refused with `501` rather than a fabricated
//! flow (contract: `POST /v1/model-providers/{id}/auth`). The operation store is
//! in-memory: an operation outlives the REQUEST/caller, and its durable form is not
//! specified by the contract.
//!
//! The STEP schema of a device-code/browser flow is NOT written down in the contract or
//! architecture, so the hub does not invent one: START reads the type descriptor and,
//! until a descriptor that declares an interactive method pins the steps, answers `501`
//! with the real reason. STATUS and CANCEL are implemented over the operation store.

use std::collections::HashMap;

use crate::types::TypeDescriptor;

/// The lifecycle of a hub-level authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatus {
    Pending,
    Approved,
    Failed,
    Expired,
    Cancelled,
}

impl AuthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthStatus::Pending => "pending",
            AuthStatus::Approved => "approved",
            AuthStatus::Failed => "failed",
            AuthStatus::Expired => "expired",
            AuthStatus::Cancelled => "cancelled",
        }
    }
}

/// One operation.
#[derive(Debug, Clone)]
pub struct AuthOperation {
    pub id: String,
    pub provider: String,
    pub status: AuthStatus,
    pub next: Option<serde_json::Value>,
    pub error: Option<String>,
    pub account: Option<String>,
}

impl AuthOperation {
    pub fn to_json(&self) -> serde_json::Value {
        let mut o = serde_json::json!({ "status": self.status.as_str() });
        if let Some(n) = &self.next {
            o["next"] = n.clone();
        }
        if let Some(e) = &self.error {
            o["error"] = serde_json::json!(e);
        }
        if let Some(a) = &self.account {
            o["account"] = serde_json::json!(a);
        }
        o
    }
}

/// An in-memory registry of hub-level authorization operations.
#[derive(Default)]
pub struct AuthStore {
    ops: std::sync::Mutex<HashMap<String, AuthOperation>>,
    seq: std::sync::atomic::AtomicU64,
}

impl AuthStore {
    pub fn new() -> Self {
        AuthStore::default()
    }

    /// Whether the descriptor declares an INTERACTIVE method the hub can run. A
    /// plain `api-key` is not interactive (the caller supplies the value directly);
    /// `device-code` / `browser` are.
    pub fn interactive_method(desc: &TypeDescriptor) -> Option<&str> {
        desc.auth_methods
            .iter()
            .map(|s| s.as_str())
            .find(|m| matches!(*m, "device-code" | "browser"))
    }

    /// Create an operation. Called ONLY by a real flow EXECUTOR at the moment it
    /// begins work; a route that cannot execute must NOT create one (a pending
    /// operation with no executor is a placeholder disguised as a resource,
    /// TASK-048 G4).
    #[allow(dead_code)] // used by the flow executor once the step contract lands
    pub fn create(&self, provider: &str) -> AuthOperation {
        let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let op = AuthOperation {
            id: format!("{provider}-auth-{n}"),
            provider: provider.to_string(),
            status: AuthStatus::Pending,
            next: None,
            error: None,
            account: None,
        };
        self.ops
            .lock()
            .expect("auth ops")
            .insert(op.id.clone(), op.clone());
        op
    }

    pub fn get(&self, id: &str) -> Option<AuthOperation> {
        self.ops.lock().expect("auth ops").get(id).cloned()
    }

    /// Cancel a pending operation. Idempotent: cancelling an already-cancelled
    /// operation is a no-op success; cancelling a terminal one is also a no-op.
    pub fn cancel(&self, id: &str) -> bool {
        let mut ops = self.ops.lock().expect("auth ops");
        match ops.get_mut(id) {
            Some(op) => {
                if op.status == AuthStatus::Pending {
                    op.status = AuthStatus::Cancelled;
                }
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(methods: &[&str]) -> TypeDescriptor {
        TypeDescriptor {
            id: "t".into(),
            version: 1,
            owner: "hub".into(),
            name: Default::default(),
            auth_methods: methods.iter().map(|s| s.to_string()).collect(),
            configuration: serde_json::json!({}),
            catalog: serde_json::json!({}),
        }
    }

    #[test]
    fn only_interactive_methods_count() {
        assert_eq!(AuthStore::interactive_method(&desc(&["api-key"])), None);
        assert_eq!(AuthStore::interactive_method(&desc(&["device-code"])), Some("device-code"));
        assert_eq!(AuthStore::interactive_method(&desc(&["browser"])), Some("browser"));
    }

    /// An EMPTY store has no operations: with no executor, none is ever created, so
    /// the status route reads `not_found` (honest, not a fabricated pending).
    #[test]
    fn an_empty_store_has_no_operations() {
        let store = AuthStore::new();
        assert!(store.get("anything").is_none());
        assert!(!store.cancel("anything"));
    }

    /// The executor path: create -> pending -> cancel is idempotent. This is the API
    /// the flow executor uses once the step contract lands.
    #[test]
    fn an_executor_operation_is_pending_then_cancelled_idempotently() {
        let store = AuthStore::new();
        let op = store.create("p1");
        assert_eq!(store.get(&op.id).unwrap().status, AuthStatus::Pending);
        assert!(store.cancel(&op.id));
        assert_eq!(store.get(&op.id).unwrap().status, AuthStatus::Cancelled);
        assert!(store.cancel(&op.id)); // idempotent
        assert_eq!(store.get(&op.id).unwrap().status, AuthStatus::Cancelled);
    }
}
