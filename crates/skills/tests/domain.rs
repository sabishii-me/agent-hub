//! Skills domain acceptance over HTTP: store bytes, list, read, and the
//! traversal refusal. A skill is a directory; the hub is a courier.

use agent_hub_skills::routes::{routes, SkillsState};
use agent_hub_skills::Skills;

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

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-skills-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

async fn serve(dir: std::path::PathBuf) -> String {
    install_provider();
    let state = SkillsState::new(Skills::new(dir), errors());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, routes().with_state(state)).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn write_list_read_delete_over_http() {
    let base = serve(tmp("crud")).await;
    let client = reqwest::Client::new();

    // Write two files of a skill.
    let r = client
        .put(format!("{base}/v1/skills/deploy/files/SKILL.md"))
        .json(&serde_json::json!({ "content": "# Deploy\n" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    client
        .put(format!("{base}/v1/skills/deploy/files/scripts/run.sh"))
        .json(&serde_json::json!({ "content": "echo hi\n" }))
        .send()
        .await
        .unwrap();

    // List.
    let list: serde_json::Value = client
        .get(format!("{base}/v1/skills"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["skills"][0]["id"], "deploy");
    assert_eq!(list["skills"][0]["hasManifest"], true);
    assert_eq!(list["skills"][0]["files"], 2);

    // Read one file back.
    let got: serde_json::Value = client
        .get(format!("{base}/v1/skills/deploy/files/SKILL.md"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["content"], "# Deploy\n");

    // Delete.
    let r = client
        .delete(format!("{base}/v1/skills/deploy"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let list: serde_json::Value = client
        .get(format!("{base}/v1/skills"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list["skills"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn traversal_is_refused() {
    let base = serve(tmp("traversal")).await;
    let client = reqwest::Client::new();
    // A `..` segment must be an invalid input, not a read outside the root.
    let r = client
        .put(format!("{base}/v1/skills/x/files/../../../../etc/evil"))
        .json(&serde_json::json!({ "content": "x" }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error() || r.status() == 404, "status {}", r.status());
}

#[tokio::test]
async fn reading_a_missing_skill_is_a_contract_code() {
    let base = serve(tmp("missing")).await;
    let r = reqwest::Client::new()
        .get(format!("{base}/v1/skills/nope/files/SKILL.md"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "not_found");
}
