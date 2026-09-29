//! Sessions domain acceptance over real HTTP: the control-state lifecycle and
//! turn admission's logical command identity (R1).

use agent_hub_db::Db;
use agent_hub_events::Bus;
use agent_hub_sessions::routes::{routes, SessionsState};
use agent_hub_sessions::Sessions;

fn install_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn serve(state: SessionsState) -> String {
    install_provider();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = routes().with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn state(dir: std::path::PathBuf) -> SessionsState {
    let db = Db::open_in_memory().unwrap();
    let bus = Bus::new(64, 64);
    SessionsState::new(Sessions::new(db, bus, dir), errors())
}

fn errors() -> agent_hub_transport::ErrorRenderer {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contract/errors.json"
    ))
    .unwrap();
    agent_hub_transport::ErrorRenderer::new(agent_hub_contract::ErrorTable::parse(&raw).unwrap())
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-sess-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn create_get_patch_close_reopen_delete() {
    let base = serve(state(tmp("crud"))).await;
    let client = reqwest::Client::new();

    // Create.
    let r = client
        .post(format!("{base}/v1/sessions"))
        .json(&serde_json::json!({ "harnessId": "pi", "title": "First" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let created: serde_json::Value = r.json().await.unwrap();
    let id = created["session"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["session"]["status"], "active");
    assert_eq!(created["session"]["harnessId"], "pi");

    // Patch the title.
    let r = client
        .patch(format!("{base}/v1/sessions/{id}"))
        .json(&serde_json::json!({ "title": "Renamed" }))
        .send()
        .await
        .unwrap();
    let patched: serde_json::Value = r.json().await.unwrap();
    assert_eq!(patched["session"]["title"], "Renamed");

    // Close, then reopen.
    let r = client
        .post(format!("{base}/v1/sessions/{id}/close"))
        .send()
        .await
        .unwrap();
    let closed: serde_json::Value = r.json().await.unwrap();
    assert_eq!(closed["session"]["status"], "readonly");

    let r = client
        .post(format!("{base}/v1/sessions/{id}/reopen"))
        .send()
        .await
        .unwrap();
    let reopened: serde_json::Value = r.json().await.unwrap();
    assert_eq!(reopened["session"]["status"], "active");

    // List.
    let list: serde_json::Value = client
        .get(format!("{base}/v1/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["sessions"].as_array().unwrap().len(), 1);

    // Delete (soft): it disappears from the list.
    client
        .delete(format!("{base}/v1/sessions/{id}"))
        .send()
        .await
        .unwrap();
    let list: serde_json::Value = client
        .get(format!("{base}/v1/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list["sessions"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn turn_idempotency_key_makes_retries_safe() {
    let base = serve(state(tmp("turn"))).await;
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{base}/v1/sessions"))
        .json(&serde_json::json!({ "harnessId": "pi" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["session"]["id"].as_str().unwrap().to_string();

    // First admission with key K.
    let t1: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/turns"))
        .json(&serde_json::json!({ "idempotencyKey": "K", "content": [] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let turn_id = t1["turn"]["id"].as_str().unwrap().to_string();
    assert_eq!(t1["turn"]["state"], "running");

    // A retry with the SAME key returns the SAME turn (no second turn).
    let t2: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/turns"))
        .json(&serde_json::json!({ "idempotencyKey": "K", "content": [] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(t2["turn"]["id"], turn_id, "a retry must not start a new turn");

    // A DIFFERENT key while a turn runs is refused (session_busy).
    let r = client
        .post(format!("{base}/v1/sessions/{id}/turns"))
        .json(&serde_json::json!({ "idempotencyKey": "K2", "content": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "session_busy");
}

#[tokio::test]
async fn fork_carries_the_source_relation() {
    let base = serve(state(tmp("fork"))).await;
    let client = reqwest::Client::new();
    let created: serde_json::Value = client
        .post(format!("{base}/v1/sessions"))
        .json(&serde_json::json!({ "harnessId": "pi" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["session"]["id"].as_str().unwrap().to_string();

    let forked: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/fork"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(forked["session"]["id"], id);
    assert_eq!(forked["forkedFrom"]["sessionId"], id);

    let list: serde_json::Value = client
        .get(format!("{base}/v1/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["sessions"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn missing_harness_is_a_validation_error() {
    let base = serve(state(tmp("validation"))).await;
    let r = reqwest::Client::new()
        .post(format!("{base}/v1/sessions"))
        .json(&serde_json::json!({ "harnessId": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}
