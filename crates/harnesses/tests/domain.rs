//! Harnesses domain acceptance: the projection and the capability gating over a
//! real subprocess adapter.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use agent_hub_adapter::Adapters;
use agent_hub_events::Bus;
use agent_hub_harnesses::routes::{routes, HarnessesState};
use agent_hub_harnesses::Harnesses;

fn errors() -> agent_hub_transport::ErrorRenderer {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contract/errors.json"
    ))
    .unwrap();
    agent_hub_transport::ErrorRenderer::new(agent_hub_contract::ErrorTable::parse(&raw).unwrap())
}

fn install_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-harn-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn install_fake(root: &std::path::Path, id: &str, capabilities: &[&str]) {
    let dir = root.join(id);
    fs::create_dir_all(&dir).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../adapter/tests/fixtures/fake-adapter.mjs");
    fs::copy(&fixture, dir.join("adapter.mjs")).unwrap();
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string(&serde_json::json!({
            "id": id, "pluginType": "harness-adapter", "protocol": 0,
            "command": ["node", "adapter.mjs"], "name": "Fake",
            "version": "0.1.0",
            "runtime": {"package": "@x/y", "version": "1.2.3", "command": ["node", "runtime/cli.js"]},
            "capabilities": capabilities
        }))
        .unwrap(),
    )
    .unwrap();
}

async fn serve(state: HarnessesState) -> String {
    install_provider();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, routes().with_state(state)).await.unwrap();
    });
    format!("http://{addr}")
}

fn state(root: PathBuf, data: PathBuf) -> HarnessesState {
    let adapters = Arc::new(Adapters::new(vec![root], data, Bus::new(64, 64)));
    adapters.scan();
    HarnessesState::new(Harnesses::new(adapters), errors())
}

#[tokio::test]
async fn lists_a_registered_harness_with_both_versions() {
    let root = tmp("list");
    install_fake(&root, "fake", &["models", "presets"]);
    let base = serve(state(root, tmp("list-data"))).await;

    let list: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/v1/harnesses"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let h = &list["harnesses"][0];
    assert_eq!(h["id"], "fake");
    assert_eq!(h["status"], "enabled");
    // ADR-0004: adapter and runtime versions are two independent facts.
    assert_eq!(h["adapterVersion"], "0.1.0");
    assert_eq!(h["runtimeVersion"], "1.2.3");
    assert_eq!(h["runtimePackage"], "@x/y");
}

#[tokio::test]
async fn a_declared_capability_is_routed_to_the_adapter() {
    let root = tmp("declared");
    install_fake(&root, "fake", &["models", "presets", "tools"]);
    let base = serve(state(root, tmp("declared-data"))).await;
    let client = reqwest::Client::new();

    let presets: serde_json::Value = client
        .get(format!("{base}/v1/harnesses/fake/presets"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(presets["known"], true);
    assert_eq!(presets["presets"][0]["id"], "default");
}

#[tokio::test]
async fn an_undeclared_capability_is_known_false_never_a_fake_list() {
    // The harness declares only `models`; presets/tools must be known:false.
    let root = tmp("undeclared");
    install_fake(&root, "fake", &["models"]);
    let base = serve(state(root, tmp("undeclared-data"))).await;
    let client = reqwest::Client::new();

    let presets: serde_json::Value = client
        .get(format!("{base}/v1/harnesses/fake/presets"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(presets["known"], false, "must be known:false, not a fake empty list: {presets}");
    assert_eq!(presets["presets"].as_array().unwrap().len(), 0);

    let tools: serde_json::Value = client
        .get(format!("{base}/v1/harnesses/fake/tools"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tools["known"], false);
}

#[tokio::test]
async fn unknown_harness_is_a_contract_code() {
    let root = tmp("unknown");
    install_fake(&root, "fake", &["models"]);
    let base = serve(state(root, tmp("unknown-data"))).await;

    let r = reqwest::Client::new()
        .get(format!("{base}/v1/harnesses/nope/presets"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "harness_not_found");
}

#[tokio::test]
async fn enable_disable_reflects_in_the_projection() {
    let root = tmp("enable");
    install_fake(&root, "fake", &["models"]);
    let base = serve(state(root, tmp("enable-data"))).await;
    let client = reqwest::Client::new();

    client
        .post(format!("{base}/v1/harnesses/fake/disable"))
        .send()
        .await
        .unwrap();
    let list: serde_json::Value = client
        .get(format!("{base}/v1/harnesses"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["harnesses"][0]["status"], "disabled");
}
