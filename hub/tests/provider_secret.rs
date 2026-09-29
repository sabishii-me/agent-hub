//! The provider credential path through the REAL hub: a token goes into the OS
//! secret store, the record file carries no secret, and logout deletes it.
//!
//! Gated on the OS store being available (it probes it through the binary's own
//! log line is not read; instead the test asserts the contract behaviour and SKIPs
//! when the store refuses). Runs the real `agent-hub` binary against a temp dir.

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
async fn a_provider_token_goes_to_the_os_store_not_disk() {
    install_provider();
    let data = std::env::temp_dir().join(format!("agent-hub-sec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();

    let exe = env!("CARGO_BIN_EXE_agent-hub");
    let child = Command::new(exe)
        .env("AGENT_HUB_DATA_DIR", &data)
        .env("AGENT_HUB_ADDR", "127.0.0.1:0")
        .env("AGENT_HUB_CONTRACT_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/../contract"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let _hub = Hub(child);

    let (addr, token) = wait_ready(&data, Duration::from_secs(20)).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let secret = "sk-TEST-SECRET-do-not-persist";

    // Create a provider with a credential.
    let resp = client
        .post(format!("{base}/v1/model-providers"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "id": "acme", "url": "https://api.acme.test", "token": secret }))
        .send()
        .await
        .unwrap();
    if resp.status() == 501 {
        eprintln!("SKIP provider_secret: the OS secret store is unavailable on this host");
        let _ = std::fs::remove_dir_all(&data);
        return;
    }
    assert_eq!(resp.status(), 200, "{}", resp.text().await.unwrap());
    let created: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(created["provider"]["tokenConfigured"], true);

    // The record file must NOT contain the secret.
    let record = std::fs::read_to_string(data.join("providers/acme.json")).unwrap();
    assert!(!record.contains(secret), "the credential must not be on disk: {record}");
    assert!(!record.contains("token"), "no token field in the record: {record}");

    // Nothing in the whole data dir has the plaintext secret.
    assert!(!dir_contains(&data, secret), "no plaintext secret anywhere under the data dir");

    // Logout deletes it.
    let out: serde_json::Value = client
        .post(format!("{base}/v1/model-providers/acme/logout"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(out["provider"]["tokenConfigured"], false);

    let _ = std::fs::remove_dir_all(&data);
}

fn dir_contains(dir: &std::path::Path, needle: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if dir_contains(&p, needle) {
                return true;
            }
        } else if std::fs::read_to_string(&p).map(|s| s.contains(needle)).unwrap_or(false) {
            return true;
        }
    }
    false
}

async fn wait_ready(data: &std::path::Path, timeout: Duration) -> (String, String) {
    let endpoint = data.join("endpoint.json");
    let start = std::time::Instant::now();
    loop {
        if let Ok(raw) = std::fs::read_to_string(&endpoint) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let (Some(url), Some(tok)) = (v["url"].as_str(), v["token"].as_str()) {
                    return (url.trim_start_matches("http://").to_string(), tok.to_string());
                }
            }
        }
        if start.elapsed() > timeout {
            panic!("the hub did not become ready");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
