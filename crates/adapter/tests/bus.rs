//! Adapter acceptance: spawn a **real subprocess** adapter, speak the agent-bus
//! protocol over its stdin/stdout, route capability calls, and receive its
//! notifications as events.

use std::fs;
use std::path::PathBuf;

use agent_hub_adapter::{Adapters, HarnessStatus};
use agent_hub_events::Bus;
use serde_json::json;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agent-hub-adapter-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// Write the fake adapter into a plugin directory named `id`.
fn install_fake(root: &std::path::Path, id: &str, capabilities: &[&str]) -> PathBuf {
    let dir = root.join(id);
    fs::create_dir_all(&dir).unwrap();
    // Copy the fixture next to the plugin, as a plugin would ship its code.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-adapter.mjs");
    fs::copy(&fixture, dir.join("adapter.mjs")).unwrap();
    let manifest = json!({
        "id": id,
        "pluginType": "harness-adapter",
        "protocol": 0,
        "command": ["node", "adapter.mjs"],
        "capabilities": capabilities,
        "name": "Fake"
    });
    fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    dir
}

#[tokio::test]
async fn spawns_a_real_adapter_and_routes_calls() {
    let root = tmp("spawn");
    install_fake(&root, "fake", &["models", "presets", "tools"]);
    let adapters = Adapters::new(vec![root.clone()], std::env::temp_dir(), Bus::new(64, 64));

    let found = adapters.scan();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "fake");
    assert_eq!(found[0].status, HarnessStatus::Enabled);

    // A declared capability routes to a real reply.
    let models = adapters
        .call("fake", "models", "models/list", json!({}))
        .await
        .unwrap();
    assert_eq!(models["known"], true);
    assert_eq!(models["models"][0]["id"], "fake-model");

    let presets = adapters
        .call("fake", "presets", "presets/list", json!({}))
        .await
        .unwrap();
    assert_eq!(presets["presets"][0]["id"], "default");

    // An undeclared capability is refused, not attempted.
    let err = adapters
        .call("fake", "providers", "connections/list", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, agent_hub_adapter::AdapterError::Unsupported(_)), "{err}");
}

#[tokio::test]
async fn a_protocol_error_surfaces_as_an_rpc_error() {
    let root = tmp("rpcerr");
    install_fake(&root, "fake", &["models"]);
    let adapters = Adapters::new(vec![root], std::env::temp_dir(), Bus::new(16, 16));
    adapters.scan();

    let err = adapters
        .call("fake", "models", "boom", json!({}))
        .await
        .unwrap_err();
    match err {
        agent_hub_adapter::AdapterError::Bus(agent_hub_adapter::BusError::Rpc { code, message }) => {
            assert_eq!(code, -32000);
            assert_eq!(message, "deliberate");
        }
        other => panic!("expected an rpc error, got {other}"),
    }
}

#[tokio::test]
async fn adapter_notifications_become_events() {
    let root = tmp("events");
    install_fake(&root, "fake", &["models"]);
    let bus = Bus::new(64, 64);
    let adapters = Adapters::new(vec![root], std::env::temp_dir(), bus.clone());
    adapters.scan();

    // Subscribe to the event stream, then prompt: the adapter emits turn.running
    // and message.* which the manager must publish.
    let mut sub = bus.subscribe(None);

    adapters
        .call("fake", "models", "session/start", json!({ "sid": "s1" }))
        .await
        .unwrap();
    adapters
        .call(
            "fake",
            "models",
            "session/prompt",
            json!({ "sid": "s1", "message": "hi", "clientMessageId": "c1" }),
        )
        .await
        .unwrap();

    // Collect the events the adapter emitted.
    let mut seen = Vec::new();
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), sub.receiver.recv()).await {
            Ok(Some(e)) => {
                seen.push(e.name.clone());
                if seen.iter().filter(|n| *n == "message.completed").count() == 1 {
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(seen.contains(&"turn.running".to_string()), "events: {seen:?}");
    assert!(seen.contains(&"message.delta".to_string()), "events: {seen:?}");
    assert!(seen.contains(&"message.completed".to_string()), "events: {seen:?}");
}

#[tokio::test]
async fn a_real_manifest_shape_parses_and_resolves_the_runtime_argv() {
    // The real manifests carry runtime.command as an ARGV ARRAY
    // (["node","runtime/dist/cli.js"]), and the hub resolves the parts against
    // the plugin directory to hand the adapter AGENT_HUB_RUNTIME_COMMAND.
    let root = tmp("realm");
    let dir = root.join("pi");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("manifest.json"),
        r#"{
          "id": "pi",
          "pluginType": "harness-adapter",
          "protocol": 0,
          "command": ["node", "pi-adapter.cjs"],
          "runtime": {"package": "@x/y", "version": "1.0.0", "command": ["node", "runtime/dist/cli.js"]},
          "capabilities": ["models"]
        }"#,
    )
    .unwrap();
    let adapters = Adapters::new(vec![root], std::env::temp_dir(), Bus::new(8, 8));
    let found = adapters.scan();
    assert_eq!(found.len(), 1, "the real manifest shape must parse");
    let argv = found[0].manifest.runtime_argv(&found[0].directory).unwrap();
    assert_eq!(argv[0], "node");
    assert!(argv[1].ends_with("runtime/dist/cli.js"));
    assert!(std::path::Path::new(&argv[1]).is_absolute(), "resolved to absolute: {argv:?}");
}

#[tokio::test]
async fn the_adapter_receives_the_agent_hub_environment() {
    // The env the hub builds names the dirs it created and the runtime argv.
    let root = tmp("env");
    install_fake(&root, "fake", &["models"]);
    let data = tmp("env-data");
    let adapters = Adapters::new(vec![root], data.clone(), Bus::new(8, 8));
    adapters.scan();
    let env = adapters.adapter_env(&adapters.get("fake").unwrap());
    let get = |k: &str| -> Option<String> { env.iter().find(|(n, _): &&(String, String)| n == k).map(|(_, v)| v.clone()) };
    assert!(get("AGENT_HUB_HARNESS_DIR").unwrap().contains("agents"));
    assert!(get("AGENT_HUB_CWD").is_some());
    assert!(get("AGENT_HUB_INSTALLED_SKILLS_DIR").unwrap().contains("skills"));
    // The secret key is never forwarded.
    assert!(get("AGENT_HUB_SECRET_KEY").is_none());
}

#[tokio::test]
async fn the_adapter_env_installs_the_plugins_shipped_extensions() {
    // A plugin ships `extensions/plan`; the hub writes it into the harness dir
    // it hands over as AGENT_HUB_INSTALLED_EXTENSIONS_DIR.
    let root = tmp("extenv");
    let dir = root.join("fake");
    fs::create_dir_all(dir.join("extensions/plan")).unwrap();
    fs::write(dir.join("extensions/plan/plan.js"), "// plan
").unwrap();
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-adapter.mjs"),
        dir.join("adapter.mjs"),
    )
    .unwrap();
    fs::write(
        dir.join("manifest.json"),
        r#"{"id":"fake","pluginType":"harness-adapter","protocol":0,
            "command":["node","adapter.mjs"],"capabilities":["models"],
            "extensions":["plan"]}"#,
    )
    .unwrap();

    let data = tmp("extenv-data");
    let adapters = Adapters::new(vec![root], data.clone(), Bus::new(8, 8));
    adapters.scan();
    let env = adapters.adapter_env(&adapters.get("fake").unwrap());
    let ext = env
        .iter()
        .find(|(k, _)| k == "AGENT_HUB_INSTALLED_EXTENSIONS_DIR")
        .map(|(_, v)| v.clone())
        .unwrap();
    let installed = PathBuf::from(&ext).join("plan/plan.js");
    assert!(installed.exists(), "the shipped extension must be installed: {installed:?}");
}

#[tokio::test]
async fn a_bad_protocol_version_is_refused() {
    let root = tmp("protocol");
    let dir = root.join("bad");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("manifest.json"),
        r#"{"id":"bad","pluginType":"harness-adapter","protocol":99,"command":["node","x"]}"#,
    )
    .unwrap();
    let adapters = Adapters::new(vec![root], std::env::temp_dir(), Bus::new(8, 8));
    // The wrong protocol means the plugin is not registered as an adapter.
    assert!(adapters.scan().is_empty());
}

#[tokio::test]
async fn disabled_harness_is_not_started() {
    let root = tmp("disabled");
    install_fake(&root, "fake", &["models"]);
    let adapters = Adapters::new(vec![root], std::env::temp_dir(), Bus::new(8, 8));
    adapters.scan();

    adapters.set_status("fake", HarnessStatus::Disabled).unwrap();
    let err = adapters
        .call("fake", "models", "models/list", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, agent_hub_adapter::AdapterError::Unsupported(_)), "{err}");
}
