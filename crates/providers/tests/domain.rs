//! Providers domain acceptance: CRUD, the model selection, revision staleness,
//! and a **real catalog fetch** against a local HTTP server.

use std::sync::Arc;

use agent_hub_providers::routes::{routes, ProvidersState};
use agent_hub_providers::{ProviderStore, Providers};
use axum::routing::get;
use axum::Router;

fn install_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-prov-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

async fn serve_providers(dir: std::path::PathBuf) -> String {
    install_provider();
    let state = ProvidersState::new(Providers::new(ProviderStore::new(dir)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, routes().with_state(state)).await.unwrap();
    });
    format!("http://{addr}")
}

/// A tiny upstream that answers an OpenAI-shaped /models.
async fn serve_upstream(models: serde_json::Value) -> String {
    let app = Router::new().route(
        "/models",
        get(move || {
            let models = models.clone();
            async move { axum::Json(models) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn crud_lifecycle() {
    let base = serve_providers(tmp("crud")).await;
    let client = reqwest::Client::new();

    // Create.
    let r = client
        .post(format!("{base}/v1/hub/providers"))
        .json(&serde_json::json!({ "id": "acme", "label": "Acme", "url": "https://api.acme.test", "api": "openai-completions", "token": "sk-secret" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let created: serde_json::Value = r.json().await.unwrap();
    assert_eq!(created["provider"]["id"], "acme");
    assert_eq!(created["provider"]["tokenConfigured"], true);
    // The token itself never appears.
    assert!(created["provider"].get("token").is_none(), "token leaked");

    // Duplicate id refused.
    let r = client
        .post(format!("{base}/v1/hub/providers"))
        .json(&serde_json::json!({ "id": "acme", "url": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);

    // List.
    let list: serde_json::Value = client
        .get(format!("{base}/v1/hub/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["providers"].as_array().unwrap().len(), 1);

    // Logout keeps the row but drops the credential.
    let r = client
        .post(format!("{base}/v1/hub/providers/acme/logout"))
        .send()
        .await
        .unwrap();
    let out: serde_json::Value = r.json().await.unwrap();
    assert_eq!(out["provider"]["tokenConfigured"], false);

    // Delete.
    let r = client
        .delete(format!("{base}/v1/hub/providers/acme"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let list: serde_json::Value = client
        .get(format!("{base}/v1/hub/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list["providers"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn changing_url_or_api_bumps_revision_and_marks_catalog_stale() {
    let base = serve_providers(tmp("revision")).await;
    let client = reqwest::Client::new();
    let upstream = serve_upstream(serde_json::json!({
        "data": [ { "id": "m1" }, { "id": "m2" } ]
    }))
    .await;

    client
        .post(format!("{base}/v1/hub/providers"))
        .json(&serde_json::json!({ "id": "p", "url": upstream, "api": "openai-completions" }))
        .send()
        .await
        .unwrap();

    // Refresh: the catalog is now present and fresh.
    let r: serde_json::Value = client
        .post(format!("{base}/v1/hub/providers/p/models/refresh"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["stale"], false, "just refreshed: {r}");
    assert_eq!(r["models"].as_array().unwrap().len(), 2);
    // First refresh enables new models.
    assert_eq!(r["models"][0]["enabled"], true);

    // Change the api -> revision bump -> stale.
    let r: serde_json::Value = client
        .patch(format!("{base}/v1/hub/providers/p"))
        .json(&serde_json::json!({ "api": "anthropic-messages" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _ = r;
    let r: serde_json::Value = client
        .get(format!("{base}/v1/hub/providers/p/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["stale"], true, "url/api change must mark the catalog stale");
}

#[tokio::test]
async fn selection_is_stored_and_validated() {
    let base = serve_providers(tmp("selection")).await;
    let client = reqwest::Client::new();
    let upstream = serve_upstream(serde_json::json!({
        "data": [ { "id": "m1" }, { "id": "m2" }, { "id": "m3" } ]
    }))
    .await;

    client
        .post(format!("{base}/v1/hub/providers"))
        .json(&serde_json::json!({ "id": "p", "url": upstream }))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{base}/v1/hub/providers/p/models/refresh"))
        .send()
        .await
        .unwrap();

    // Select a subset.
    let r: serde_json::Value = client
        .patch(format!("{base}/v1/hub/providers/p/models"))
        .json(&serde_json::json!({ "enabledModelIds": ["m1", "m3"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let enabled: Vec<String> = r["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["enabled"] == true)
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(enabled, vec!["m1", "m3"]);

    // An unknown id is refused.
    let r = client
        .patch(format!("{base}/v1/hub/providers/p/models"))
        .json(&serde_json::json!({ "enabledModelIds": ["nope"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);

    // The selection survives a refresh (it lives on the provider).
    client
        .post(format!("{base}/v1/hub/providers/p/models/refresh"))
        .send()
        .await
        .unwrap();
    let r: serde_json::Value = client
        .get(format!("{base}/v1/hub/providers/p/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let enabled: Vec<String> = r["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["enabled"] == true)
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(enabled, vec!["m1", "m3"], "refresh must not change the selection");
}

#[tokio::test]
async fn a_corrupt_provider_file_is_reported_not_dropped() {
    let dir = tmp("corrupt");
    std::fs::write(dir.join("bad.json"), "{ not json").unwrap();
    let base = serve_providers(dir).await;
    let list: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/v1/hub/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["providers"].as_array().unwrap().len(), 0);
    assert_eq!(list["broken"].as_array().unwrap().len(), 1);
    assert_eq!(list["broken"][0]["file"], "bad.json");
}
