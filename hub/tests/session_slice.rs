//! The session slice through the REAL hub: spawn the `agent-hub` binary against a
//! temporary data dir and drive the OFFICIAL `/v1` surface (auth, boot, install,
//! the whole app) - not a sessions-only router (TASK-048 N6/R5).
//!
//! Gated on `AGENT_HUB_TEST_PLUGIN_DIR` (a plugin dir with its runtime in place)
//! and `AGENT_HUB_TEST_HARNESS` (default `pi`). Without a plugin dir the test
//! prints SKIP and returns; it never fabricates a product result.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn plugin_dir() -> Option<PathBuf> {
    std::env::var("AGENT_HUB_TEST_PLUGIN_DIR").ok().map(PathBuf::from)
}

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
async fn real_hub_session_202_start_close() {
    let Some(pdir) = plugin_dir() else {
        eprintln!("SKIP session_slice: set AGENT_HUB_TEST_PLUGIN_DIR");
        return;
    };
    let harness = std::env::var("AGENT_HUB_TEST_HARNESS").unwrap_or_else(|_| "pi".into());
    install_provider();

    let data = std::env::temp_dir().join(format!("agent-hub-slice-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    // Place the plugin under the hub's own plugins root.
    let plugins_root = data.join("plugins");
    std::fs::create_dir_all(&plugins_root).unwrap();
    let dest = plugins_root.join(&harness);
    copy_tree(&pdir, &dest);

    // Spawn the REAL hub binary.
    let exe = env!("CARGO_BIN_EXE_agent-hub");
    let child = Command::new(exe)
        .env("AGENT_HUB_DATA_DIR", &data)
        .env("AGENT_HUB_ADDR", "127.0.0.1:0")
        .env("AGENT_HUB_CONTRACT_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/../contract"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let mut hub = Hub(child);

    // Read the bound address from stdout, and the token from endpoint.json.
    let (addr, token) = wait_ready(&data, Duration::from_secs(20)).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // POST /v1/sessions -> 202 + Location (with the bearer).
    let resp = client
        .post(format!("{base}/v1/sessions"))
        .header("authorization", format!("Bearer {token}"))
        .header("idempotency-key", "slice-real")
        .json(&serde_json::json!({ "harnessId": harness }))
        .send()
        .await
        .unwrap();
    if resp.status() != 202 {
        let body = resp.text().await.unwrap();
        panic!("expected 202, got: {body}");
    }
    let location = resp.headers()["location"].to_str().unwrap().to_string();
    let created: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(created["session"]["status"], "starting");
    let id = created["session"]["id"].as_str().unwrap().to_string();
    assert_eq!(location, format!("/v1/sessions/{id}"));

    // GET reads starting -> active with a real native ref.
    let mut status = String::new();
    for _ in 0..400 {
        let got: serde_json::Value = client
            .get(format!("{base}/v1/sessions/{id}"))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        status = got["session"]["status"].as_str().unwrap_or("").to_string();
        if status != "starting" {
            eprintln!("status after poll: {status}");
            let native = got["session"]["nativeRef"].as_str().unwrap_or("");
            if status == "active" {
                assert!(PathBuf::from(native).exists(), "a real ref file");
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if status != "active" {
        let got: serde_json::Value = client
            .get(format!("{base}/v1/sessions/{id}"))
            .header("authorization", format!("Bearer {token}"))
            .send().await.unwrap().json().await.unwrap();
        panic!("expected active, got {status}: startError={}", got["session"]["startError"]);
    }

    // Same key -> same session; different body -> conflict.
    let replay: serde_json::Value = client
        .post(format!("{base}/v1/sessions"))
        .header("authorization", format!("Bearer {token}"))
        .header("idempotency-key", "slice-real")
        .json(&serde_json::json!({ "harnessId": harness }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay["session"]["id"], id);

    let conflict = client
        .post(format!("{base}/v1/sessions"))
        .header("authorization", format!("Bearer {token}"))
        .header("idempotency-key", "slice-real")
        .json(&serde_json::json!({ "harnessId": "other" }))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);

    // The turn lifecycle: admit (202) with a durable identity; a retry returns the
    // same turn; a different body under the same key is a conflict. The prompt
    // itself reaches the adapter (a real model call is not asserted - without a
    // credential the turn ends failed, which is honest, not a fake success).
    let turn_resp = client
        .post(format!("{base}/v1/sessions/{id}/turns"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "idempotencyKey": "turn-1",
            "content": [{ "type": "text", "text": "hello" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(turn_resp.status(), 202, "a turn is a long command: 202");
    let turn: serde_json::Value = turn_resp.json().await.unwrap();
    let turn_id = turn["turn"]["id"].as_str().unwrap().to_string();

    let turn_replay: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/turns"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "idempotencyKey": "turn-1",
            "content": [{ "type": "text", "text": "hello" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(turn_replay["turn"]["id"], turn_id, "same turn key returns the same turn");

    let turn_conflict = client
        .post(format!("{base}/v1/sessions/{id}/turns"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "idempotencyKey": "turn-1",
            "content": [{ "type": "text", "text": "different" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(turn_conflict.status(), 409);

    // The turn is listed and reaches a terminal state.
    let mut turn_ended = String::new();
    for _ in 0..200 {
        let turns: serde_json::Value = client
            .get(format!("{base}/v1/sessions/{id}/turns"))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(t) = turns["turns"].as_array().and_then(|a| a.first()) {
            let state = t["state"].as_str().unwrap_or("").to_string();
            if state == "ended" {
                turn_ended = t["ended"].as_str().unwrap_or("").to_string();
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!turn_ended.is_empty(), "the turn reaches a terminal state");

    // Close -> readonly.
    let closed: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/close"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(closed["session"]["status"], "readonly");

    let _ = hub.0.kill();
    let _ = std::fs::remove_dir_all(&data);
}

/// Read the bound address (the hub prints it) and the token from endpoint.json.
async fn wait_ready(data: &std::path::Path, timeout: Duration) -> (String, String) {
    let endpoint = data.join("endpoint.json");
    let start = std::time::Instant::now();
    loop {
        if let Ok(raw) = std::fs::read_to_string(&endpoint) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let (Some(url), Some(token)) = (v["url"].as_str(), v["token"].as_str()) {
                    let addr = url.trim_start_matches("http://").to_string();
                    return (addr, token.to_string());
                }
            }
        }
        if start.elapsed() > timeout {
            panic!("the hub did not become ready");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let to = dst.join(e.file_name());
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            copy_tree(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), &to).ok();
        }
    }
}
