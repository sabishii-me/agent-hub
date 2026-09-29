//! A **real** session start against the pi adapter, when a runtime is available.
//!
//! Gated on `AGENT_HUB_TEST_PLUGIN_DIR` (a plugin dir whose manifest has a
//! `runtime/`) and `AGENT_HUB_TEST_HARNESS` (the harness id, default `pi`). With
//! no runtime present the test is skipped and prints why - it never falls back to
//! a fake adapter (TASK-048). Run it with:
//!
//! ```text
//! AGENT_HUB_TEST_PLUGIN_DIR=/path/to/plugins/pi \
//! AGENT_HUB_TEST_HARNESS=pi \
//! cargo test -p agent-hub-sessions --test real_start -- --nocapture
//! ```

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

#[tokio::test]
async fn real_session_start_close_reopen() {
    let Some(pdir) = plugin_dir() else {
        eprintln!("SKIP real_start: set AGENT_HUB_TEST_PLUGIN_DIR to a plugin with a runtime/");
        return;
    };
    let harness = std::env::var("AGENT_HUB_TEST_HARNESS").unwrap_or_else(|_| "pi".into());

    let data = std::env::temp_dir().join(format!("agent-hub-realstart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();

    // A resolver from the real plugin directory (like the hub's).
    let roots = vec![pdir.parent().unwrap().to_path_buf()];
    let adapters = Arc::new(Adapters::new(roots, &data, Bus::new(16, 16)));
    adapters.scan();
    let registry = adapters.clone();
    let resolve = move |id: &str| -> Option<HarnessSpec> {
        let h = registry.get(id).ok()?;
        Some(HarnessSpec {
            id: h.id.clone(),
            command: h.manifest.command.clone()?,
            plugin_dir: h.directory.clone(),
            runtime_argv: h.manifest.runtime_argv(&h.directory),
        })
    };

    let db = Db::open(data.join("hub.sqlite")).unwrap();
    let sessions = Sessions::new(db, Bus::new(16, 16), &data, Box::new(resolve));
    let base = serve(SessionsState::new(sessions, errors())).await;
    let client = reqwest::Client::new();

    // Create: a REAL adapter start.
    let created: serde_json::Value = client
        .post(format!("{base}/v1/sessions"))
        .json(&serde_json::json!({ "harnessId": harness }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session = &created["session"];
    assert_eq!(session["status"], "active", "a real start must report active: {created}");
    let id = session["id"].as_str().unwrap().to_string();
    let native_ref = session["nativeRef"].as_str().expect("a real start names its native ref");
    assert!(PathBuf::from(native_ref).exists(), "the native ref must be a real file: {native_ref}");

    // GET confirms the same.
    let got: serde_json::Value = client
        .get(format!("{base}/v1/sessions/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["session"]["status"], "active");

    // Close stops the process; the record stays.
    let closed: serde_json::Value = client
        .post(format!("{base}/v1/sessions/{id}/close"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(closed["session"]["status"], "readonly");

    // Reopen re-attaches on the stored ref.
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
