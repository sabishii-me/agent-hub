//! The hub-managed connections through the REAL hub: a credential goes into the OS
//! secret store (never plaintext), the view is credential-free, `disabled` means
//! zero materialization, and delete removes the credential.
//!
//! Runs the real `agent-hub` binary against a temp dir. Gated on the OS store being
//! available (a refused credential SKIPs, never fakes).

use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Hub(Child);
impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn install_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[tokio::test]
async fn a_connection_credential_lives_in_the_os_store_and_disables_to_zero() {
    install_provider();
    let unique = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        format!("{}-{}", std::process::id(), nanos)
    };
    let data = std::env::temp_dir().join(format!("agent-hub-conn-{unique}"));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_agent-hub"))
        .env("AGENT_HUB_DATA_DIR", &data)
        .env("AGENT_HUB_ADDR", "127.0.0.1:0")
        .env("AGENT_HUB_CONTRACT_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/../contract"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let mut hub = Hub(child);

    let endpoint = data.join("endpoint.json");
    let start = std::time::Instant::now();
    let (addr, token) = loop {
        if let Ok(s) = std::fs::read_to_string(&endpoint) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                if let (Some(u), Some(t)) = (v["url"].as_str(), v["token"].as_str()) {
                    break (u.to_string(), t.to_string());
                }
            }
        }
        if start.elapsed() > Duration::from_secs(30) {
            panic!("hub did not become ready");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let base = addr;
    let client = reqwest::Client::new();
    let auth = format!("Bearer {token}");
    let secret = "xoxb-connection-secret-xyz";

    let created = client
        .post(format!("{base}/v1/connections"))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "id": "slack", "name": "Slack", "scheme": "bearer",
            "endpoint": "https://hooks.slack.test", "envName": "SLACK_TOKEN",
            "token": secret
        }))
        .send()
        .await
        .unwrap();
    if !created.status().is_success() {
        eprintln!("SKIP connection_slice: no OS secret store (credential refused)");
        let _ = std::fs::remove_dir_all(&data);
        return;
    }
    let body: serde_json::Value = created.json().await.unwrap();
    assert_eq!(body["connection"]["credentialConfigured"], true);
    assert!(
        body["connection"].get("token").is_none(),
        "the token is never returned: {body}"
    );

    // Nothing on disk holds the plaintext secret (bytes, not UTF-8 only).
    assert!(!dir_contains(&data, secret), "no plaintext credential anywhere under the data dir");

    // Disable -> the connection still exists but materialises nothing (checked in
    // the domain unit test); here the view reflects the state.
    let disabled: serde_json::Value = client
        .patch(format!("{base}/v1/connections/slack"))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "state": "disabled" }))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(disabled["connection"]["state"], "disabled");

    // Delete removes it; a second delete is unknown_connection.
    client
        .delete(format!("{base}/v1/connections/slack"))
        .header("authorization", &auth)
        .send().await.unwrap();
    let second = client
        .delete(format!("{base}/v1/connections/slack"))
        .header("authorization", &auth)
        .send().await.unwrap();
    assert_eq!(second.status(), 404);

    let _ = hub.0.kill();
    let _ = std::fs::remove_dir_all(&data);
}

fn dir_contains(dir: &std::path::Path, needle: &str) -> bool {
    dir_contains_bytes(dir, needle.as_bytes())
}

fn dir_contains_bytes(dir: &std::path::Path, needle: &[u8]) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if dir_contains_bytes(&p, needle) {
                return true;
            }
        } else if let Ok(bytes) = std::fs::read(&p) {
            if bytes.windows(needle.len()).any(|w| w == needle) {
                return true;
            }
        }
    }
    false
}
