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

/// A DURABLE registry of hub-level authorization operations (ADR-0012): the operation
/// outlives the starting request and SURVIVES a restart (ARCHITECTURE §438). Its state
/// is in the database; the in-process cancel signal is only a convenience for the
/// poller that is running right now.
pub struct AuthStore {
    db: agent_hub_db::Db,
    cancels: std::sync::Mutex<HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    seq: std::sync::atomic::AtomicU64,
}

impl AuthStore {
    pub fn new(db: agent_hub_db::Db) -> Self {
        AuthStore {
            db,
            cancels: std::sync::Mutex::new(HashMap::new()),
            seq: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Whether the descriptor declares an INTERACTIVE method the hub can run. A
    /// plain `api-key` is not interactive (the caller supplies the value directly);
    /// `device-code` / `browser` are.
    pub fn interactive_method(desc: &TypeDescriptor) -> Option<&str> {
        desc.auth_methods.iter().map(|s| s.as_str()).find(|m| matches!(*m, "device-code" | "browser"))
    }

    /// Create a PENDING operation and its durable row. Called ONLY by a real flow
    /// executor at the moment it begins work (TASK-048 G4): a route that cannot
    /// execute must not create one. `next` is the first step; `flow` is the hub's
    /// private poll state (the device_code), never part of the contract.
    pub fn create(&self, provider: &str, kind: &str, next: &serde_json::Value, flow: &str) -> Result<String, String> {
        let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let id = format!("{provider}-auth-{n}");
        self.db
            .auth_op_create(&id, provider, kind, &next.to_string(), flow)
            .map_err(|e| format!("could not record the authorization operation: {e}"))?;
        self.cancels
            .lock()
            .expect("auth cancels")
            .insert(id.clone(), std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
        Ok(id)
    }

    /// Read the operation as the contract's `authOperation` body.
    pub fn get(&self, id: &str) -> Option<serde_json::Value> {
        let row = self.db.auth_op_get(id).ok().flatten()?;
        let mut o = serde_json::json!({ "status": row.status });
        if let Some(n) = row.next.and_then(|n| serde_json::from_str::<serde_json::Value>(&n).ok()) {
            o["next"] = n;
        }
        if let Some(e) = row.error {
            o["error"] = serde_json::json!(e);
        }
        if let Some(a) = row.account {
            o["account"] = serde_json::json!(a);
        }
        Some(o)
    }

    /// The provider a stored operation belongs to (so the route can reject a mismatched
    /// `{id}`), and the flow state the poller needs.
    pub fn provider_of(&self, id: &str) -> Option<String> {
        self.db.auth_op_get(id).ok().flatten().map(|r| r.provider)
    }

    pub fn flow_of(&self, id: &str) -> Option<String> {
        self.db.auth_op_get(id).ok().flatten().and_then(|r| r.flow)
    }

    /// The cancel flag the poller observes. Present only while this process owns the
    /// running poller; absent after a restart (the operation is then resolved by the
    /// boot reconcile, not by cancelling a poller that no longer exists).
    pub fn cancel_flag(&self, id: &str) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.cancels
            .lock()
            .expect("auth cancels")
            .entry(id.to_string())
            .or_insert_with(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .clone()
    }

    /// Cancel a pending operation: flag the live poller AND record it durably.
    /// Idempotent. Returns whether the operation exists.
    pub fn cancel(&self, id: &str) -> bool {
        if let Some(flag) = self.cancels.lock().expect("auth cancels").get(id) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        match self.db.auth_op_get(id).ok().flatten() {
            Some(_) => {
                let _ = self.db.auth_op_cancel(id);
                true
            }
            None => false,
        }
    }

    /// Record a terminal outcome durably and drop the in-process cancel flag.
    pub fn finish(&self, id: &str, status: AuthStatus, error: Option<String>, account: Option<String>) {
        let _ = self.db.auth_op_finish(id, status.as_str(), error.as_deref(), account.as_deref());
        self.cancels.lock().expect("auth cancels").remove(id);
    }

    /// Boot reconcile: an operation left `pending` cannot continue (its poller died
    /// with the process), so it is reported `failed` with a reason the caller can act
    /// on. The operation row SURVIVES; only its pending work does not.
    pub fn reconcile_pending_at_boot(&self) -> usize {
        let pending = self.db.auth_op_pending().unwrap_or_default();
        let n = pending.len();
        for row in pending {
            let _ = self.db.auth_op_finish(
                &row.id,
                "failed",
                Some("the hub restarted while the sign-in was pending; start it again"),
                None,
            );
        }
        n
    }
}

/// The device-code flow a descriptor DECLARES as data (`configuration.auth`). The hub
/// runs it; it never calls plugin code in-process (the contract says so).
#[derive(Debug, Clone)]
pub struct DeviceCodeSpec {
    pub gateway: String,
    pub client_id: String,
    pub client_version: String,
    pub code_path: String,
    pub token_path: String,
    pub link_ack_path: Option<String>,
}

impl DeviceCodeSpec {
    /// Read the spec from a descriptor when it declares a `device-code` auth block.
    /// A type that declares device-code but ships no `auth` block is a CONFIGURATION
    /// error - not something to invent around.
    pub fn from_descriptor(desc: &TypeDescriptor) -> Result<Option<DeviceCodeSpec>, String> {
        if !desc.auth_methods.iter().any(|m| m == "device-code") {
            return Ok(None);
        }
        let auth = desc
            .configuration
            .get("auth")
            .ok_or_else(|| "the type declares device-code but ships no configuration.auth block".to_string())?;
        let g = |k: &str| auth.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let gateway = g("gateway").ok_or_else(|| "configuration.auth.gateway is required".to_string())?;
        let client_id = g("clientId").ok_or_else(|| "configuration.auth.clientId is required".to_string())?;
        Ok(Some(DeviceCodeSpec {
            gateway: gateway.trim_end_matches('/').to_string(),
            client_id,
            client_version: auth.get("clientVersion").and_then(|v| v.as_u64()).map(|n| n.to_string()).unwrap_or_else(|| "1".into()),
            code_path: g("codePath").unwrap_or_else(|| "/device/code".into()),
            token_path: g("tokenPath").unwrap_or_else(|| "/device/token".into()),
            link_ack_path: g("linkAckPath"),
        }))
    }
}

/// The result of a completed device-code flow.
pub struct AuthFinished {
    pub credential: String,
    pub url: Option<String>,
    pub api: Option<String>,
    pub account: Option<String>,
}

/// A stable `install_id` for this machine+provider (the platform refuses /device/code
/// without one; it lets the platform offer key replacement on reconnect).
fn install_id(provider: &str) -> String {
    use sha2::{Digest, Sha256};
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "host".into());
    let mut h = Sha256::new();
    h.update(format!("{host}|{provider}|hub-provider"));
    let d = hex::encode(h.finalize());
    format!("{}-{}-{}-{}-{}", &d[0..8], &d[8..12], &d[12..16], &d[16..20], &d[20..32])
}

/// START a device-code flow: POST the code request and return the FIRST step the
/// caller shows the human, plus what the poller needs.
pub async fn device_code_start(
    spec: &DeviceCodeSpec,
    provider: &str,
) -> Result<(serde_json::Value, String, u64, u64), String> {
    let body = serde_json::json!({
        "client_id": spec.client_id,
        "client_version": spec.client_version,
        "install_id": install_id(provider),
        "device_name": std::env::var("COMPUTERNAME").unwrap_or_else(|_| "host".into()),
        "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
    });
    let resp = reqwest::Client::new()
        .post(format!("{}{}", spec.gateway, spec.code_path))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("could not reach the platform: {e}"))?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("the code response is not JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!("the sign-in request was refused (HTTP {status})"));
    }
    let device_code = v.get("device_code").and_then(|x| x.as_str()).ok_or("the platform returned no device_code")?.to_string();
    let user_code = v.get("user_code").and_then(|x| x.as_str()).ok_or("the platform returned no user_code")?.to_string();
    let verify = v
        .get("verification_uri_complete")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("verification_uri").and_then(|x| x.as_str()))
        .ok_or("the platform returned no verification_uri")?
        .to_string();
    let expires = v.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(900);
    let interval = v.get("interval").and_then(|x| x.as_u64()).unwrap_or(5);
    let step = serde_json::json!({
        "kind": "device-code",
        "userCode": user_code,
        "verifyUrl": verify,
        "expiresInSeconds": expires,
        "intervalSeconds": interval,
    });
    Ok((step, device_code, expires, interval))
}

/// POLL the token endpoint until the human approves, it expires, or the operation is
/// cancelled. Only a 200 with a usable credential returns Ok - never a fake success.
pub async fn device_code_poll(
    spec: &DeviceCodeSpec,
    device_code: &str,
    expires_secs: u64,
    interval_secs: u64,
    cancelled: impl Fn() -> bool,
) -> Result<AuthFinished, String> {
    let client = reqwest::Client::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(expires_secs);
    let mut interval = interval_secs.max(1);
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        if cancelled() {
            return Err("cancelled".into());
        }
        let resp = client
            .post(format!("{}{}", spec.gateway, spec.token_path))
            .json(&serde_json::json!({ "client_id": spec.client_id, "device_code": device_code }))
            .send()
            .await
            .map_err(|e| format!("token poll failed: {e}"))?;
        let status = resp.status();
        let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        if status.as_u16() == 200 {
            let secret = v
                .pointer("/api_key/secret")
                .and_then(|x| x.as_str())
                .ok_or("the sign-in response carried no credential")?
                .to_string();
            let url = v.pointer("/endpoints/openai_base_url").and_then(|x| x.as_str()).map(str::to_string);
            let api = url.as_ref().map(|_| "openai-completions".to_string());
            let account = v
                .pointer("/user/email")
                .and_then(|x| x.as_str())
                .or_else(|| v.pointer("/org/name").and_then(|x| x.as_str()))
                .map(str::to_string);
            if let (Some(path), Some(link)) = (&spec.link_ack_path, v.get("link_token").and_then(|x| x.as_str())) {
                let _ = client
                    .post(format!("{}{}", spec.gateway, path))
                    .json(&serde_json::json!({}))
                    .bearer_auth(link)
                    .send()
                    .await;
            }
            return Ok(AuthFinished { credential: secret, url, api, account });
        }
        let code = v
            .pointer("/error/code")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("error").and_then(|x| x.as_str()))
            .unwrap_or("");
        if code == "slow_down" {
            interval += 2;
            continue;
        }
        if matches!(code, "expired_token" | "access_denied" | "authorization_declined") {
            return Err(format!("the sign-in did not complete: {code}"));
        }
    }
    Err("the sign-in window expired".into())
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

    fn db() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "authops-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("h.sqlite")
    }

    /// An EMPTY store has no operations: with no executor, none is ever created, so
    /// the status route reads `not_found` (honest, not a fabricated pending).
    #[test]
    fn an_empty_store_has_no_operations() {
        let store = AuthStore::new(agent_hub_db::Db::open(db()).unwrap());
        assert!(store.get("anything").is_none());
        assert!(!store.cancel("anything"));
    }

    /// The executor path: create -> pending -> cancel is idempotent, and the row
    /// SURVIVES (a fresh store over the same db reads the same state - ADR-0012).
    #[test]
    fn an_operation_is_pending_then_cancelled_idempotently_and_survives() {
        let path = db();
        let store = AuthStore::new(agent_hub_db::Db::open(&path).unwrap());
        let step = serde_json::json!({ "kind": "device-code", "userCode": "AB-12", "verifyUrl": "https://x", "expiresInSeconds": 900, "intervalSeconds": 5 });
        let id = store.create("p1", "device-code", &step, "device-code-secret").unwrap();
        assert_eq!(store.get(&id).unwrap()["status"], "pending");
        assert_eq!(store.provider_of(&id).as_deref(), Some("p1"));
        assert!(store.cancel(&id));
        assert_eq!(store.get(&id).unwrap()["status"], "cancelled");
        assert!(store.cancel(&id)); // idempotent
        assert_eq!(store.get(&id).unwrap()["status"], "cancelled");
    }

    /// A recreated store (a restart) still reads the operation and its flow state.
    #[test]
    fn an_operation_survives_a_restart() {
        let path = db();
        let step = serde_json::json!({ "kind": "device-code", "verifyUrl": "https://x" });
        let id = AuthStore::new(agent_hub_db::Db::open(&path).unwrap())
            .create("p1", "device-code", &step, "flow-xyz")
            .unwrap();
        // A NEW store over the SAME database (a restart) sees the operation.
        let store2 = AuthStore::new(agent_hub_db::Db::open(&path).unwrap());
        assert_eq!(store2.get(&id).unwrap()["status"], "pending");
        assert_eq!(store2.flow_of(&id).as_deref(), Some("flow-xyz"));
        // The boot reconcile ends a pending op a restart cannot continue.
        assert_eq!(store2.reconcile_pending_at_boot(), 1);
        assert_eq!(store2.get(&id).unwrap()["status"], "failed");
    }
}
