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
    // A manifest with a `command` whose SCRIPT exists: the hub verifies the declared runtime
    // command is on disk before it calls an install `ready` (a fact, not a claim).
    fs::write(
        dir.join("manifest.json"),
        format!(
            r#"{{"id":"{id}","name":"{id}","pluginType":"harness-adapter","version":"{version}","command":["node","adapter.cjs"]}}"#
        ),
    )
    .unwrap();
    fs::write(dir.join("adapter.cjs"), "// the harness adapter entry\n").unwrap();
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
/// An adapter registry pointed at the plugin ROOT, so `scan()` registers a landed plugin and the
/// hub's `runtimeReady` check (the declared command exists on disk) is a real fact. An empty root
/// list would make that check fail for every install - the reason these tests used to wedge.
fn adapters_for(root: &std::path::Path) -> std::sync::Arc<agent_hub_adapter::Adapters> {
    std::sync::Arc::new(agent_hub_adapter::Adapters::new(
        vec![root.to_path_buf()],
        root.parent().unwrap_or(root).to_path_buf(),
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

    // Both releases this test installs, built and served up front. A release must be LISTED in
    // the registry (url + sha256) to be an authorized install source; a raw dir/git source is
    // refused (403). The registry can list several versions at once.
    let src = plugin_src(&dir, "alpha", "1.0.0");
    let zip = zip_dir(&src, "alpha-1.0.0");
    let sha = sha256_hex(&zip);
    let size = zip.len() as u64;
    let (url, _h) = serve_bytes(zip).await;
    let src2 = plugin_src(&dir, "alpha", "2.0.0");
    let zip2 = zip_dir(&src2, "alpha-2.0.0");
    let sha2 = sha256_hex(&zip2);
    let size2 = zip2.len() as u64;
    let (url2, _h2) = serve_bytes(zip2).await;
    authorize(&plugins, &dir, &[("alpha", "1.0.0", &url, &sha), ("alpha", "2.0.0", &url2, &sha2)]);

    let base = serve(PluginsState::new(plugins, transport, errors(), adapters_for(&root))).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({ "source": { "artifact": {
        "url": url, "sha256": sha, "id": "alpha", "version": "1.0.0",
        "pluginType": "harness-adapter", "size": size } } });

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

    // Replace with a new version (also a registered release): still 202; wait for the new manifest.
    let r = client
        .post(format!("{base}/v1/plugins"))
        .json(&serde_json::json!({ "source": { "artifact": {
            "url": url2, "sha256": sha2, "id": "alpha", "version": "2.0.0",
            "pluginType": "harness-adapter", "size": size2 } } }))
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

    // An AUTHORIZED release whose manifest cannot be honoured: the source is a registered
    // artifact (so it is not refused as unregistered) and the manifest is the fault under test.
    let bad = dir.join("bad");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("manifest.json"), r#"{"id":"bad","pluginType":"nonsense"}"#).unwrap();
    let zip = zip_dir(&bad, "bad-1.0.0");
    let sha = sha256_hex(&zip);
    let size = zip.len() as u64;
    let (url, _h) = serve_bytes(zip).await;
    authorize(&plugins, &dir, &[("bad", "1.0.0", &url, &sha)]);

    let base = serve(PluginsState::new(plugins, transport, errors(), adapters_for(&root))).await;

    let r = reqwest::Client::new()
        .post(format!("{base}/v1/plugins"))
        .json(&serde_json::json!({ "source": { "artifact": {
            "url": url, "sha256": sha, "id": "bad", "version": "1.0.0",
            "pluginType": "harness-adapter", "size": size } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202, "the resolve is detached; the refusal lands asynchronously");
    // The artifact is a REGISTERED release, so it is fetched - but its manifest cannot be
    // honoured, so the install fails during resolve and leaves NOTHING: no usable plugin and no
    // tree on disk. A refused artifact never half-lands.
    for _ in 0..100 {
        if !root.join("bad").join("manifest.json").exists() && r.status() == 202 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let g: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/v1/plugins/bad"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(g["plugin"]["state"], "ready", "an invalid manifest must never be `ready`: {g}");
    assert!(!root.join("bad").join("manifest.json").exists(), "no tree landed for a refused manifest");
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

    // A registered release to install (a raw source is refused; the change event must follow a
    // REAL install).
    let src = plugin_src(&dir, "beta", "1.0.0");
    let zip = zip_dir(&src, "beta-1.0.0");
    let sha = sha256_hex(&zip);
    let size = zip.len() as u64;
    let (url, _h) = serve_bytes(zip).await;
    authorize(&plugins, &dir, &[("beta", "1.0.0", &url, &sha)]);

    let base = serve(PluginsState::new(plugins, transport, errors(), adapters_for(&root))).await;
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
    client
        .post(format!("{base}/v1/plugins"))
        .json(&serde_json::json!({ "source": { "artifact": {
            "url": url, "sha256": sha, "id": "beta", "version": "1.0.0",
            "pluginType": "harness-adapter", "size": size } } }))
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

/// Build a release zip in memory: a single wrapper dir with a manifest.
fn build_artifact_zip(id: &str, version: &str, wrapper: &str) -> Vec<u8> {
    use std::io::Write;
    let mut buf = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        // A `command` whose script lands in the artifact: the hub verifies the declared runtime
        // command exists before it calls an install `ready`.
        let manifest = format!(
            r#"{{"id":"{id}","name":"{id}","pluginType":"harness-adapter","version":"{version}","command":["node","adapter.cjs"]}}"#
        );
        zw.start_file(format!("{wrapper}/manifest.json"), opts).unwrap();
        zw.write_all(manifest.as_bytes()).unwrap();
        zw.start_file(format!("{wrapper}/adapter.cjs"), opts).unwrap();
        zw.write_all(b"// the harness adapter entry\n").unwrap();
        zw.finish().unwrap();
    }
    buf
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Write the registry the hub reads for THIS test (its `registry.json` sits beside the plugin
/// root, which is where `Plugins::registry_file()` looks) and load it into memory. A release the
/// registry LISTS is an authorized install source; a source it does not list is refused (403
/// `plugin_not_in_registry`). `entries` are `(id, version, url, sha256)`.
fn authorize(plugins: &Plugins, dir: &std::path::Path, entries: &[(&str, &str, &str, &str)]) {
    let plugins_json: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, version, url, sha)| {
            serde_json::json!({
                "id": id,
                "pluginType": "harness-adapter",
                "versions": [{ "version": version, "url": url, "sha256": sha, "size": 0 }]
            })
        })
        .collect();
    let doc = serde_json::json!({ "plugins": plugins_json });
    fs::write(dir.join("registry.json"), doc.to_string()).unwrap();
    plugins.load_registry_from_file();
}

/// Build a one-plugin release zip from a source directory (the plugin tree under `wrapper/`).
fn zip_dir(src: &std::path::Path, wrapper: &str) -> Vec<u8> {
    use std::io::Write;
    let mut buf = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            zw.start_file(format!("{wrapper}/{name}"), opts).unwrap();
            zw.write_all(&fs::read(entry.path()).unwrap()).unwrap();
        }
        zw.finish().unwrap();
    }
    buf
}

/// Serve raw bytes once at a stable path, so the artifact URL is real HTTP.
async fn serve_bytes(bytes: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::response::Response;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/artifact.zip",
        axum::routing::get(move || {
            let bytes = bytes.clone();
            async move { Response::builder().status(StatusCode::OK).body(Body::from(bytes)).unwrap() }
        }),
    );
    let h = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/artifact.zip"), h)
}

#[tokio::test]
async fn install_from_a_release_artifact_verifies_and_records_it() {
    let dir = tmp("artifact");
    let db = Db::open_in_memory().unwrap();
    let bus = Bus::new(8, 8);
    let root = dir.join("plugins");
    fs::create_dir_all(&root).unwrap();
    let plugins = Plugins::new(db, &root, bus.clone());
    let transport = Transport::new(bus.clone(), agent_hub_transport::Admission::new(8));

    let zip = build_artifact_zip("art", "2.0.0", "art-2.0.0");
    let sha = sha256_hex(&zip);
    let size = zip.len() as u64;
    let (url, _h) = serve_bytes(zip).await;
    // The artifact must be a REGISTERED release (url + sha256): that is what authorizes an install.
    authorize(&plugins, &dir, &[("art", "2.0.0", &url, &sha)]);

    let base = serve(PluginsState::new(plugins, transport, errors(), adapters_for(&root))).await;
    let client = reqwest::Client::new();

    // A correct artifact lands the plugin and RECORDS the artifact on the row.
    let body = serde_json::json!({ "source": { "artifact": {
        "url": url, "sha256": sha, "id": "art", "version": "2.0.0",
        "pluginType": "harness-adapter", "size": size } } });
    let r = client.post(format!("{base}/v1/plugins")).json(&body).send().await.unwrap();
    assert_eq!(r.status(), 202, "artifact install is a long op");
    let plugin = wait_for_plugin(&client, &base, "art").await;
    assert_eq!(plugin["state"], "ready");
    assert_eq!(plugin["artifact"]["version"], "2.0.0");
    assert_eq!(plugin["artifact"]["sha256"], sha);
    // The wrapper dir was stripped: the manifest sits at the plugin root.
    let target = root.join("art").join("manifest.json");
    assert!(target.is_file(), "manifest must land at the plugin root");
}

#[tokio::test]
async fn a_wrong_digest_is_refused_before_unpacking() {
    let dir = tmp("artifact-bad");
    let db = Db::open_in_memory().unwrap();
    let bus = Bus::new(8, 8);
    let root = dir.join("plugins");
    fs::create_dir_all(&root).unwrap();
    let plugins = Plugins::new(db, &root, bus.clone());
    let transport = Transport::new(bus.clone(), agent_hub_transport::Admission::new(8));

    let zip = build_artifact_zip("bad", "1.0.0", "bad-1.0.0");
    let good = sha256_hex(&zip);
    let (url, _h) = serve_bytes(zip).await;
    // The registry lists this release by its TRUE digest. A caller that names a DIFFERENT digest
    // names a release the registry does not list, so the source is refused BEFORE anything is
    // fetched or unpacked: the sha256 is the anchor, and an unlisted digest is not authorized.
    authorize(&plugins, &dir, &[("bad", "1.0.0", &url, &good)]);

    let base = serve(PluginsState::new(plugins, transport, errors(), adapters_for(&root))).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({ "source": { "artifact": {
        "url": url, "sha256": "0".repeat(64), "id": "bad", "version": "1.0.0" } } });
    let r = client.post(format!("{base}/v1/plugins")).json(&body).send().await.unwrap();
    assert_eq!(r.status(), 403, "a digest the registry does not list is refused");
    let err: serde_json::Value = r.json().await.unwrap();
    assert_eq!(err["error"], "plugin_not_in_registry");
    assert!(!root.join("bad").exists(), "nothing lands on a refused artifact");
}

#[tokio::test]
async fn install_from_a_git_source_is_refused() {
    let dir = tmp("git");
    let db = Db::open_in_memory().unwrap();
    let bus = Bus::new(8, 8);
    let root = dir.join("plugins");
    fs::create_dir_all(&root).unwrap();
    let plugins = Plugins::new(db, &root, bus.clone());
    let transport = Transport::new(bus.clone(), agent_hub_transport::Admission::new(8));
    let base = serve(PluginsState::new(plugins, transport, errors(), adapters_for(&root))).await;
    let client = reqwest::Client::new();

    // A real local git repo with a manifest and a marker file.
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("manifest.json"),
        r#"{"id":"gitp","name":"GitP","pluginType":"harness-adapter","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(repo.join("marker.txt"), "hello").unwrap();
    let git = |args: &[&str]| {
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .expect("git");
        assert!(st.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&st.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]);

    // A git source carries no sha256, so it can never match a published release: the registry
    // refuses it (403) - the hub does not clone an unregistered tree. That is the honest truth of
    // the current contract; a git install is no longer an authorized source.
    let body = serde_json::json!({ "source": { "url": repo.to_string_lossy() } });
    let r = client.post(format!("{base}/v1/plugins")).json(&body).send().await.unwrap();
    assert_eq!(r.status(), 403, "a git source is not a registered release");
    let err: serde_json::Value = r.json().await.unwrap();
    assert_eq!(err["error"], "plugin_not_in_registry");
    assert!(!root.join("gitp").exists(), "nothing was cloned from an unregistered source");
}
