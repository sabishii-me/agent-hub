//! Component tests for the sessions slice: a REAL adapter process, driven through
//! the sessions HTTP router.
//!
//! **Limited purpose (recorded):** these exercise the sessions domain's routes
//! (`crates/sessions`) against a real adapter, but they build the sessions router
//! directly - they are NOT the whole hub product (no auth, install, boot). The
//! product acceptance is the real hub run recorded in the slice's delivery
//! evidence; these guard regressions in the domain. Gated on
//! `AGENT_HUB_TEST_PLUGIN_DIR`; without it they print SKIP and do not assert.
//!
//! Run: `AGENT_HUB_TEST_PLUGIN_DIR=/path/to/plugins/pi cargo test -p agent-hub-sessions --test real_start -- --nocapture`

use std::path::PathBuf;
use std::sync::Arc;

use agent_hub_adapter::Adapters;
use agent_hub_db::Db;
use agent_hub_events::Bus;
use agent_hub_sessions::routes::{routes, SessionsState};
use agent_hub_sessions::{HarnessSpec, Sessions};

fn plugin_dir() -> Option<PathBuf> {
    std::env::var("AGENT_HUB_TEST_PLUGIN_DIR").ok().map(PathBuf::from)
}

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

async fn serve(state: SessionsState) -> String {
    install_provider();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, routes().with_state(state)).await.unwrap();
    });
    format!("http://{addr}")
}

/// Build a sessions state whose resolver runs the adapter's OWN placement
/// (harness_env), exactly as the hub does - no bypass.
fn state(pdir: &PathBuf, data: &std::path::Path) -> SessionsState {
    let roots = vec![pdir.parent().unwrap().to_path_buf()];
    let adapters = Arc::new(Adapters::new(roots, data, Bus::new(16, 16)));
    adapters.scan();
    let registry = adapters.clone();
    let resolve = move |id: &str| -> Result<HarnessSpec, String> {
        let h = registry.get(id).map_err(|e| e.to_string())?;
        let command = h.manifest.command.clone().ok_or("no command")?;
        let runtime_argv = h.manifest.runtime_argv(&h.directory);
        let env = registry.harness_env(&h.id, None).map_err(|e| e.to_string())?;
        Ok(HarnessSpec {
            id: h.id.clone(),
            command,
            plugin_dir: h.directory.clone(),
            runtime_argv,
            harness_dir: env.harness_dir,
            skills_dir: env.skills_dir,
            extensions_dir: env.extensions_dir,
        })
    };
    let db = Db::open(data.join("hub.sqlite")).unwrap();
    SessionsState::new(Sessions::new(db, Bus::new(16, 16), data, Box::new(resolve)), errors())
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Poll GET until the session leaves `starting`.
async fn wait_status(client: &reqwest::Client, base: &str, id: &str) -> String {
    for _ in 0..200 {
        let got: serde_json::Value = client
            .get(format!("{base}/v1/sessions/{id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let status = got["session"]["status"].as_str().unwrap_or("").to_string();
        if status != "starting" {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    "still-starting".into()
}

#[tokio::test]
async fn real_start_is_202_then_active_then_close_releases() {
    let Some(pdir) = plugin_dir() else {
        eprintln!("SKIP real_start: set AGENT_HUB_TEST_PLUGIN_DIR");
        return;
    };
    let harness = std::env::var("AGENT_HUB_TEST_HARNESS").unwrap_or_else(|_| "pi".into());
    let data = tmp("realstart");
    let base = serve(state(&pdir, &data)).await;
    let client = reqwest::Client::new();

    // 202 + Location.
    let resp = client
        .post(format!("{base}/v1/sessions"))
        .header("idempotency-key", "A")
        .json(&serde_json::json!({ "harnessId": harness }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202, "a long command answers 202");
    let location = resp.headers()["location"].to_str().unwrap().to_string();
    let created: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(created["session"]["status"], "starting");
    let id = created["session"]["id"].as_str().unwrap().to_string();
    assert_eq!(location, format!("/v1/sessions/{id}"));

    // GET reaches active with a real native ref.
    let status = wait_status(&client, &base, &id).await;
    assert_eq!(status, "active", "a real start reaches active");
    let got: serde_json::Value = client
        .get(format!("{base}/v1/sessions/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let native_ref = got["session"]["nativeRef"].as_str().unwrap();
    assert!(PathBuf::from(native_ref).exists(), "the ref must be a real file");

    // Same key returns the same session.
    let replay: serde_json::Value = client
        .post(format!("{base}/v1/sessions"))
        .header("idempotency-key", "A")
        .json(&serde_json::json!({ "harnessId": harness }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay["session"]["id"], id);

    // Same key, different body -> conflict.
    let conflict = client
        .post(format!("{base}/v1/sessions"))
        .header("idempotency-key", "A")
        .json(&serde_json::json!({ "harnessId": "other" }))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);

    // Unsupported config refused.
    let unsupported = client
        .post(format!("{base}/v1/sessions"))
        .header("idempotency-key", "B")
        .json(&serde_json::json!({ "harnessId": harness, "plan": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported.status(), 501);

    // Close releases (status readonly).
    let closed: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/close"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(closed["session"]["status"], "readonly");

    // Reopening while running is refused; after close it re-attaches.
    let reopened: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/reopen"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reopened["session"]["status"], "active");

    let _ = std::fs::remove_dir_all(&data);
}
