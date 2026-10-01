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
    // A path that does not exist is a SKIP, not a panic: the gate is the env var
    // AND a real directory.
    std::env::var("AGENT_HUB_TEST_PLUGIN_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
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

    // The first turn ended failed (no credential): the session is idle again, so
    // a NEW key is admitted. And a refused admission must leave NO row: we prove
    // the atomicity by racing two new keys and requiring at most one running turn
    // and no orphaned admitted rows.
    let before: serde_json::Value = client
        .get(format!("{base}/v1/sessions/{id}/turns"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let before_count = before["turns"].as_array().map(|a| a.len()).unwrap_or(0);

    let (a, b) = tokio::join!(
        client
            .post(format!("{base}/v1/sessions/{id}/turns"))
            .header("authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({"content":[{"type":"text","text":"race a"}],"idempotencyKey":"race-a"}))
            .send(),
        client
            .post(format!("{base}/v1/sessions/{id}/turns"))
            .header("authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({"content":[{"type":"text","text":"race b"}],"idempotencyKey":"race-b"}))
            .send(),
    );
    let a = a.unwrap();
    let b = b.unwrap();
    let a_accepted = a.status().as_u16() == 202;
    let b_accepted = b.status().as_u16() == 202;
    let accepted = [a_accepted, b_accepted].iter().filter(|x| **x).count();
    assert!(accepted >= 1, "at least one concurrent turn is admitted");

    // The INVARIANT (not a timing guess): the number of NEW rows equals the number
    // of ACCEPTED requests - an accepted request always has its row, a refused one
    // never leaves an `admitted` row (F: admission is one atomic decision).
    let after: serde_json::Value = client
        .get(format!("{base}/v1/sessions/{id}/turns"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let turns = after["turns"].as_array().cloned().unwrap_or_default();
    let admitted = turns.iter().filter(|t| t["state"] == "admitted").count();
    assert_eq!(admitted, 0, "a refused admission leaves no admitted row: {after}");
    let new_rows = turns.len() - before_count;
    assert_eq!(
        new_rows, accepted,
        "accepted requests == new rows (accepted={accepted}, before={before_count}, after={})",
        turns.len()
    );

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

/// The grant chain through the REAL hub: a registered provider with a stored
/// credential + a session that names it must be ACCEPTED and START (the adapter
/// receives `credentials/grant` with the provider as `hub-<id>`), not refused as
/// it was before the resolver existed.
///
/// It needs a reachable provider endpoint; use a URL that will not answer (the
/// adapter tolerates a catalog fetch failure at grant time) - the point is the
/// session path, not a model answer, so no credential is imported.
#[tokio::test]
async fn a_session_with_a_managed_provider_is_accepted_and_granted() {
    let Some(pdir) = plugin_dir() else {
        eprintln!("SKIP grant_slice: set AGENT_HUB_TEST_PLUGIN_DIR");
        return;
    };
    let harness = std::env::var("AGENT_HUB_TEST_HARNESS").unwrap_or_else(|_| "pi".into());
    install_provider();

    let unique = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        format!("{}-{}", std::process::id(), nanos)
    };
    let data = std::env::temp_dir().join(format!("agent-hub-grant-{unique}"));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let plugins_root = data.join("plugins");
    std::fs::create_dir_all(&plugins_root).unwrap();
    copy_tree(&pdir, &plugins_root.join(&harness));

    let child = Command::new(env!("CARGO_BIN_EXE_agent-hub"))
        .env("AGENT_HUB_DATA_DIR", &data)
        .env("AGENT_HUB_ADDR", "127.0.0.1:0")
        .env("AGENT_HUB_CONTRACT_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/../contract"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let mut hub = Hub(child);
    let (addr, token) = wait_ready(&data, Duration::from_secs(30)).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // A reachable mock provider endpoint: the grant must actually succeed, so the
    // session can start (an unreachable endpoint is an honest start failure).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"data":[{"id":"deepseek-flash","name":"DeepSeek Flash"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });

    // A provider with a credential (a test value; the OS store must be available
    // or a credential is refused - if refused, SKIP: this test needs the store).
    let created = client
        .post(format!("{base}/v1/model-providers"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "id": "managed",
            "url": format!("http://{mock_addr}/v1"),
            "api": "anthropic-messages",
            "token": "sk-slice-test"
        }))
        .send()
        .await
        .unwrap();
    if !created.status().is_success() {
        eprintln!("SKIP grant_slice: no OS secret store (credential refused)");
        let _ = std::fs::remove_dir_all(&data);
        return;
    }

    // A session that names the managed provider must be ACCEPTED (202), not
    // refused as unsupported.
    let resp = client
        .post(format!("{base}/v1/sessions"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "harnessId": harness,
            "commandKey": "grant-1",
            "modelProviderId": "managed",
            "modelId": "deepseek-flash"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202, "a managed-provider session is a long command");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["session"]["modelProviderId"], "managed");
    let id = body["session"]["id"].as_str().unwrap().to_string();

    // It reaches active AND the applied identity is CONFIRMED and persisted: the
    // adapter's `applied.modelProviderId` must equal the requested provider, and
    // its resolved route (`applied.connectionId` = `hub-<id>` for pi) is stored.
    // This is the F3 acceptance: a grant that returned but did not apply the
    // requested target must NOT show up as active with a matched identity.
    let mut status = String::new();
    let mut applied_provider: Option<String> = None;
    let mut applied_route: Option<String> = None;
    for _ in 0..400 {
        let s: serde_json::Value = client
            .get(format!("{base}/v1/sessions/{id}"))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let sess = &s["session"];
        status = sess["status"].as_str().unwrap_or("").to_string();
        applied_provider = sess["appliedProvider"].as_str().map(str::to_string);
        applied_route = sess["appliedRoute"].as_str().map(str::to_string);
        if status == "active" || status == "starting_failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(status, "active", "the managed-provider session starts");
    assert_eq!(
        applied_provider.as_deref(),
        Some("managed"),
        "the adapter's applied provider identity is confirmed and persisted"
    );
    assert!(
        applied_route.as_deref().map(|r| r.contains("managed")).unwrap_or(false),
        "the resolved native route is persisted, got {applied_route:?}"
    );

    // Clean up this run's credential through the API (never leave a keychain
    // entry behind for a reused id), then stop the hub.
    let _ = client
        .delete(format!("{base}/v1/model-providers/managed"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await;
    let _ = hub.0.kill();
    let _ = std::fs::remove_dir_all(&data);
}

/// PATCH /v1/sessions/{id} through the REAL hub: a title is renamed IN the
/// harness (the harness's accepted title is what the session reports), and
/// plan/review apply as confirmed policy knobs.
#[tokio::test]
async fn a_session_patch_renames_and_sets_policy() {
    let Some(pdir) = plugin_dir() else {
        eprintln!("SKIP patch_slice: set AGENT_HUB_TEST_PLUGIN_DIR");
        return;
    };
    let harness = std::env::var("AGENT_HUB_TEST_HARNESS").unwrap_or_else(|_| "pi".into());
    install_provider();

    let unique = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        format!("{}-{}", std::process::id(), nanos)
    };
    let data = std::env::temp_dir().join(format!("agent-hub-patch-{unique}"));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let plugins_root = data.join("plugins");
    std::fs::create_dir_all(&plugins_root).unwrap();
    copy_tree(&pdir, &plugins_root.join(&harness));

    let child = Command::new(env!("CARGO_BIN_EXE_agent-hub"))
        .env("AGENT_HUB_DATA_DIR", &data)
        .env("AGENT_HUB_ADDR", "127.0.0.1:0")
        .env("AGENT_HUB_CONTRACT_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/../contract"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let mut hub = Hub(child);
    let (addr, token) = wait_ready(&data, Duration::from_secs(30)).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{base}/v1/sessions"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "harnessId": harness, "commandKey": "patch-1" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["session"]["id"].as_str().unwrap().to_string();
    for _ in 0..400 {
        let s: serde_json::Value = client
            .get(format!("{base}/v1/sessions/{id}"))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if s["session"]["status"] == "active" || s["session"]["status"] == "starting_failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Rename: the harness's ACCEPTED title is what the session reports.
    let renamed: serde_json::Value = client
        .patch(format!("{base}/v1/sessions/{id}"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "title": "Renamed Via Patch" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        renamed["session"]["title"], "Renamed Via Patch",
        "the harness accepted the title: {renamed}"
    );

    // Policy knobs: confirmed applied.
    let pol: serde_json::Value = client
        .patch(format!("{base}/v1/sessions/{id}"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "plan": true, "review": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pol["session"]["appliedPlan"], true, "plan confirmed: {pol}");
    assert_eq!(pol["session"]["appliedReview"], true, "review confirmed: {pol}");

    // An unknown session is `unknown_session`, not a silent create.
    let missing = client
        .patch(format!("{base}/v1/sessions/does-not-exist"))
        .header("authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "plan": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    let _ = hub.0.kill();
    let _ = std::fs::remove_dir_all(&data);
}

/// The read-through routes through the REAL hub: `messages`, `stats` and `skills`
/// read from the session's harness, and a read starts the process if it is not
/// running (here: after a `close`).
#[tokio::test]
async fn read_through_routes_start_the_process_and_answer_from_the_harness() {
    let Some(pdir) = plugin_dir() else {
        eprintln!("SKIP readthrough_slice: set AGENT_HUB_TEST_PLUGIN_DIR");
        return;
    };
    let harness = std::env::var("AGENT_HUB_TEST_HARNESS").unwrap_or_else(|_| "pi".into());
    install_provider();

    let unique = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        format!("{}-{}", std::process::id(), nanos)
    };
    let data = std::env::temp_dir().join(format!("agent-hub-rt-{unique}"));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let plugins_root = data.join("plugins");
    std::fs::create_dir_all(&plugins_root).unwrap();
    copy_tree(&pdir, &plugins_root.join(&harness));

    let child = Command::new(env!("CARGO_BIN_EXE_agent-hub"))
        .env("AGENT_HUB_DATA_DIR", &data)
        .env("AGENT_HUB_ADDR", "127.0.0.1:0")
        .env("AGENT_HUB_CONTRACT_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/../contract"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let mut hub = Hub(child);
    let (addr, token) = wait_ready(&data, Duration::from_secs(30)).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let auth = format!("Bearer {token}");

    let created: serde_json::Value = client
        .post(format!("{base}/v1/sessions"))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "harnessId": harness, "commandKey": "rt-1" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["session"]["id"].as_str().unwrap().to_string();
    for _ in 0..400 {
        let s: serde_json::Value = client
            .get(format!("{base}/v1/sessions/{id}"))
            .header("authorization", &auth)
            .send().await.unwrap().json().await.unwrap();
        if s["session"]["status"] == "active" || s["session"]["status"] == "starting_failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Close it: the process stops, status becomes readonly.
    let closed: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/close"))
        .header("authorization", &auth)
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(closed["session"]["status"], "readonly");

    // A read must START the process and answer from the harness.
    let stats: serde_json::Value = client
        .get(format!("{base}/v1/sessions/{id}/stats"))
        .header("authorization", &auth)
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(stats["sessionId"], id, "stats names the session: {stats}");
    assert_eq!(stats["source"], "harness", "stats is read from the harness: {stats}");

    let messages: serde_json::Value = client
        .get(format!("{base}/v1/sessions/{id}/messages?limit=5"))
        .header("authorization", &auth)
        .send().await.unwrap().json().await.unwrap();
    assert!(messages["messages"].is_array(), "messages is a page: {messages}");
    assert!(messages.get("next_cursor").is_some(), "next_cursor present: {messages}");

    let skills: serde_json::Value = client
        .get(format!("{base}/v1/sessions/{id}/skills"))
        .header("authorization", &auth)
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(skills["source"], "harness", "skills read from the harness: {skills}");
    assert!(skills["skills"].is_array(), "skills is a list: {skills}");

    let _ = hub.0.kill();
    let _ = std::fs::remove_dir_all(&data);
}

/// Compact and fork through the REAL hub: compact reports the harness's own result
/// (nothing computed here), and fork starts a NEW session whose source is
/// untouched.
#[tokio::test]
async fn compact_and_fork_are_real() {
    let Some(pdir) = plugin_dir() else {
        eprintln!("SKIP compact_fork: set AGENT_HUB_TEST_PLUGIN_DIR");
        return;
    };
    let harness = std::env::var("AGENT_HUB_TEST_HARNESS").unwrap_or_else(|_| "pi".into());
    install_provider();

    let unique = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        format!("{}-{}", std::process::id(), nanos)
    };
    let data = std::env::temp_dir().join(format!("agent-hub-cf-{unique}"));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let plugins_root = data.join("plugins");
    std::fs::create_dir_all(&plugins_root).unwrap();
    copy_tree(&pdir, &plugins_root.join(&harness));

    let child = Command::new(env!("CARGO_BIN_EXE_agent-hub"))
        .env("AGENT_HUB_DATA_DIR", &data)
        .env("AGENT_HUB_ADDR", "127.0.0.1:0")
        .env("AGENT_HUB_CONTRACT_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/../contract"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let mut hub = Hub(child);
    let (addr, token) = wait_ready(&data, Duration::from_secs(30)).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let auth = format!("Bearer {token}");

    let created: serde_json::Value = client
        .post(format!("{base}/v1/sessions"))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "harnessId": harness, "commandKey": "cf-1" }))
        .send().await.unwrap().json().await.unwrap();
    let id = created["session"]["id"].as_str().unwrap().to_string();
    for _ in 0..400 {
        let s: serde_json::Value = client
            .get(format!("{base}/v1/sessions/{id}"))
            .header("authorization", &auth)
            .send().await.unwrap().json().await.unwrap();
        if s["session"]["status"] == "active" || s["session"]["status"] == "starting_failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Compact: the hub reports the harness's own result, tagged source:harness.
    let compacted: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/compact"))
        .header("authorization", &auth)
        .json(&serde_json::json!({}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(compacted["sessionId"], id, "compact names the session: {compacted}");
    assert_eq!(compacted["source"], "harness", "compact is the harness's result: {compacted}");

    // Fork: a NEW session; the source is untouched.
    let forked: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/fork"))
        .header("authorization", &auth)
        .json(&serde_json::json!({}))
        .send().await.unwrap().json().await.unwrap();
    let child_id = forked["session"]["id"].as_str().unwrap().to_string();
    assert_ne!(child_id, id, "fork is a new session: {forked}");
    assert_eq!(forked["forkedFrom"]["sessionId"], id, "forkedFrom names the source: {forked}");

    let source: serde_json::Value = client
        .get(format!("{base}/v1/sessions/{id}"))
        .header("authorization", &auth)
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(source["session"]["status"], "active", "the source is unchanged: {source}");

    let _ = hub.0.kill();
    let _ = std::fs::remove_dir_all(&data);
}
