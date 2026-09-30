//! Plugins domain acceptance, over real HTTP.

use std::fs;
use std::path::PathBuf;

use agent_hub_db::Db;
use agent_hub_events::Bus;
use agent_hub_plugins::routes::{routes, PluginsState};
use agent_hub_plugins::Plugins;
use agent_hub_transport::{ErrorRenderer, Transport};

fn errors() -> ErrorRenderer {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contract/errors.json"
    ))
    .unwrap();
    ErrorRenderer::new(agent_hub_contract::ErrorTable::parse(&raw).unwrap())
}

fn install_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-plug-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// A plugin source directory with a manifest.
fn plugin_src(root: &std::path::Path, id: &str, version: &str) -> PathBuf {
    let dir = root.join(format!("src-{id}-{version}"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("manifest.json"),
        format!(
            r#"{{"id":"{id}","name":"{id}","pluginType":"harness-adapter","version":"{version}"}}"#
        ),
    )
    .unwrap();
    dir
}

async fn serve(state: PluginsState) -> String {
    install_provider();
    // Merge the transport routes with the plugins routes by nesting state:
    // build a router whose state is PluginsState, and mount transport handlers
    // through a oneshot-free path: we instead serve two routers is not possible,
    // so we build one router carrying PluginsState and re-export transport
    // handlers by wrapping them.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = finish_plugins(routes(), state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Merge the transport surface under the plugins state. Transport handlers need
/// `Transport`; we give them their own router and mount it, so both live in one
/// axum app with different state via `with_state` per sub-router.
/// An empty adapter registry (the plugin tests do not exercise enable/disable
/// against a real harness).
fn empty_adapters() -> std::sync::Arc<agent_hub_adapter::Adapters> {
    std::sync::Arc::new(agent_hub_adapter::Adapters::new(
        vec![],
        std::env::temp_dir(),
        Bus::new(8, 8),
    ))
}

fn finish_plugins(plugins: axum::Router<PluginsState>, state: PluginsState) -> axum::Router {
    let transport = agent_hub_transport::finish(agent_hub_transport::routes(), state.transport.clone());
    plugins.with_state(state).merge(transport)
}

#[tokio::test]
async fn install_list_replace_remove_over_http() {
    let dir = tmp("crud");
    let db = Db::open_in_memory().unwrap();
    let bus = Bus::new(64, 64);
    let root = dir.join("plugins");
    fs::create_dir_all(&root).unwrap();
    let plugins = Plugins::new(db, &root, bus.clone());
    let transport = Transport::new(bus.clone(), agent_hub_transport::Admission::new(8));
    let base = serve(PluginsState::new(plugins, transport, errors(), empty_adapters())).await;
    let client = reqwest::Client::new();

    let src = plugin_src(&dir, "alpha", "1.0.0");
    let body = serde_json::json!({ "source": { "url": src.to_string_lossy() } });

    // Install: a long operation is 202 + Location, not a held 200.
    let r = client
        .post(format!("{base}/v1/plugins"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202, "install should be accepted, not held");
    assert_eq!(r.headers()["location"], "/v1/plugins/alpha");
    // The work is detached; wait for the resource to become ready.
    let plugin = wait_for_plugin(&client, &base, "alpha").await;
    assert_eq!(plugin["state"], "ready");
    assert_eq!(plugin["origin"], "hub");

    // List shows it.
    let list: serde_json::Value = client
        .get(format!("{base}/v1/plugins"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(list["plugins"][0]["id"], "alpha");

    // Replace with a new version: still 202; wait for the new manifest.
    let src2 = plugin_src(&dir, "alpha", "2.0.0");
    let r = client
        .post(format!("{base}/v1/plugins"))
        .json(&serde_json::json!({ "source": { "url": src2.to_string_lossy() } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    wait_for_manifest(&root, "alpha", "2.0.0").await;

    // Remove: 202, then the directory goes.
    let r = client
        .delete(format!("{base}/v1/plugins/alpha"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    wait_for_gone(&root, "alpha").await;

    let list: serde_json::Value = client
        .get(format!("{base}/v1/plugins"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list["plugins"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn installing_an_invalid_manifest_is_refused() {
    let dir = tmp("invalid");
    let db = Db::open_in_memory().unwrap();
    let bus = Bus::new(16, 16);
    let root = dir.join("plugins");
    fs::create_dir_all(&root).unwrap();
    let plugins = Plugins::new(db, &root, bus.clone());
    let transport = Transport::new(bus, agent_hub_transport::Admission::new(4));
    let base = serve(PluginsState::new(plugins, transport, errors(), empty_adapters())).await;

    let bad = dir.join("bad");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("manifest.json"), r#"{"id":"bad","pluginType":"nonsense"}"#).unwrap();

    let r = reqwest::Client::new()
        .post(format!("{base}/v1/plugins"))
        .json(&serde_json::json!({ "source": { "url": bad.to_string_lossy() } }))
        .send()
        .await
        .unwrap();
    // The contract maps a manifest that cannot be honoured to
    // `plugin_archive_invalid` (502): the artifact is not a usable plugin.
    assert_eq!(r.status(), 502);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "plugin_archive_invalid");
}

/// Poll GET /v1/plugins until `id` is ready; return its view.
async fn wait_for_plugin(client: &reqwest::Client, base: &str, id: &str) -> serde_json::Value {
    for _ in 0..100 {
        let list: serde_json::Value = client
            .get(format!("{base}/v1/plugins"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(p) = list["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
        {
            if p["state"] == "ready" {
                return p.clone();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("plugin `{id}` never became ready");
}

async fn wait_for_manifest(root: &std::path::Path, id: &str, needle: &str) {
    for _ in 0..100 {
        if let Ok(m) = fs::read_to_string(root.join(id).join("manifest.json")) {
            if m.contains(needle) {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("manifest never contained {needle}");
}

async fn wait_for_gone(root: &std::path::Path, id: &str) {
    for _ in 0..100 {
        if !root.join(id).exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("{id} was never removed");
}

#[tokio::test]
async fn state_change_is_announced_on_the_event_stream() {
    let dir = tmp("sse");
    let db = Db::open_in_memory().unwrap();
    let bus = Bus::new(64, 64);
    let root = dir.join("plugins");
    fs::create_dir_all(&root).unwrap();
    let plugins = Plugins::new(db, &root, bus.clone());
    let transport = Transport::new(bus.clone(), agent_hub_transport::Admission::new(8));
    let base = serve(PluginsState::new(plugins, transport, errors(), empty_adapters())).await;
    let client = reqwest::Client::new();

    // Subscribe to the event stream.
    let mut stream: std::pin::Pin<Box<dyn futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>> =
        Box::pin(
            client
                .get(format!("{base}/v1/events"))
                .send()
                .await
                .unwrap()
                .bytes_stream(),
        );
    // Drain the handshake frame.
    use futures::StreamExt;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next()).await;

    // Install: a hub.plugins.changed frame must arrive.
    let src = plugin_src(&dir, "beta", "1.0.0");
    client
        .post(format!("{base}/v1/plugins"))
        .json(&serde_json::json!({ "source": { "url": src.to_string_lossy() } }))
        .send()
        .await
        .unwrap();

    let mut seen = String::new();
    for _ in 0..10 {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next()).await;
        if let Ok(Some(Ok(b))) = chunk {
            seen.push_str(&String::from_utf8_lossy(&b));
            if seen.contains("hub.plugins.changed") {
                break;
            }
        }
    }
    assert!(seen.contains("hub.plugins.changed"), "no change frame: {seen}");
    assert!(seen.contains("\"id\":\"beta\""), "frame: {seen}");
}
