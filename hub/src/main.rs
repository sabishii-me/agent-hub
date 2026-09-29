//! The agent-hub process (`ARCHITECTURE` §17).
//!
//! The executable shell: a non-blocking axum server over the shared
//! [`Transport`], with the plugins domain mounted. As more domains are
//! implemented they mount here the same way.

use std::path::PathBuf;

use agent_hub_db::Db;
use agent_hub_events::Bus;
use agent_hub_plugins::routes::{routes as plugin_routes, PluginsState};
use agent_hub_plugins::Plugins;
use agent_hub_adapter::Adapters;
use agent_hub_harnesses::routes::{routes as harness_routes, HarnessesState};
use agent_hub_harnesses::Harnesses;
use agent_hub_skills::routes::{routes as skill_routes, SkillsState};
use agent_hub_skills::Skills;
use agent_hub_humans::routes::{routes as human_routes, HumansState};
use agent_hub_humans::Humans;
use agent_hub_sessions::routes::{routes as session_routes, SessionsState};
use agent_hub_sessions::{HarnessSpec, Sessions};
use agent_hub_providers::routes::{routes as provider_routes, ProvidersState};
use agent_hub_providers::{ProviderStore, Providers};
use agent_hub_transport::{finish, require_bearer, routes, Admission, BearerToken, ErrorRenderer, Transport};

mod selfcheck;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let data_dir: PathBuf = std::env::var("AGENT_HUB_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("agent-hub"));
    std::fs::create_dir_all(&data_dir)?;

    let bus = Bus::new(1024, 256);
    let transport = Transport::new(bus.clone(), Admission::new(64));

    // The error master table is the single source of truth for every failure
    // code (ARCHITECTURE 13.7); the transport maps a code to its status.
    let contract_dir: PathBuf = std::env::var("AGENT_HUB_CONTRACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../contract"));
    let error_table = agent_hub_contract::ErrorTable::load(&contract_dir.join("errors.json"))
        .map_err(|e| format!("load contract/errors.json: {e}"))?;
    // Every code a domain can return must be declared in the contract table
    // (ARCHITECTURE 13.7); a domain inventing a code refuses to start.
    let declared_codes = [
        // plugins
        "not_found", "plugin_remove_failed", "plugin_in_use", "plugin_dir_busy",
        "plugin_archive_invalid", "idempotency_conflict", "plugin_install_failed",
        "internal_error",
        // sessions
        "unknown_session", "session_closed", "session_busy", "validation_failed",
        // providers
        "provider_not_found", "already_exists", "provider_catalog_failed",
    ];
    selfcheck::check_error_codes(&declared_codes, &error_table)
        .map_err(|e| format!("self-check failed: {e}"))?;

    // The surface check (ARCHITECTURE 13.4): serving a route the contract does
    // NOT declare is a refusal to start; a contract route not mounted yet is
    // unfinished work (reported by check_surface, not fatal).
    let contract_raw = std::fs::read_to_string(contract_dir.join("v1.json"))
        .map_err(|e| format!("read contract/v1.json: {e}"))?;
    let contract_v1: serde_json::Value = serde_json::from_str(&contract_raw)
        .map_err(|e| format!("parse contract/v1.json: {e}"))?;
    let mounted = selfcheck::mounted_surface();
    selfcheck::check_surface(&mounted, &contract_v1)
        .map_err(|e| format!("self-check failed: {e}"))?;
    let errors = ErrorRenderer::new(error_table);

    // The data layer and the plugins domain. Recovery runs before serving.
    let db = Db::open(data_dir.join("hub.sqlite"))?;
    let plugins_root = data_dir.join("plugins");
    std::fs::create_dir_all(&plugins_root)?;
    match agent_hub_db::recover(&db, &plugins_root) {
        Ok(done) if !done.is_empty() => {
            tracing::info!(recovered = done.len(), "finished unfinished plugin operations");
        }
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "plugin recovery failed"),
    }
    let plugins = Plugins::new(db, &plugins_root, bus.clone());
    let plugin_state = PluginsState::new(plugins, transport.clone(), errors.clone());

    // Harnesses first: the adapter registry is where a session resolves the
    // adapter argv and plugin dir it must start.
    let adapters = std::sync::Arc::new(Adapters::new(vec![plugins_root.clone()], &data_dir, bus.clone()));
    adapters.scan();
    let harness_state = HarnessesState::new(Harnesses::new(adapters.clone()), errors.clone());

    // Sessions: a session resolves its harness through the registry and starts
    // its OWN adapter process with the resolved argv + plugin dir.
    let sessions_db = Db::open(data_dir.join("hub.sqlite"))?;
    let registry = adapters.clone();
    let resolve = move |id: &str| -> Option<HarnessSpec> {
        let h = registry.get(id).ok()?;
        let command = h.manifest.command.clone()?;
        let runtime_argv = h.manifest.runtime_argv(&h.directory);
        Some(HarnessSpec {
            id: h.id.clone(),
            command,
            plugin_dir: h.directory.clone(),
            runtime_argv,
        })
    };
    let sessions = Sessions::new(sessions_db, bus.clone(), &data_dir, Box::new(resolve));
    let session_state = SessionsState::new(sessions, errors.clone());

    // Providers: one JSON file per provider under the data dir.
    let providers = Providers::new(ProviderStore::new(data_dir.join("providers")));
    let provider_state = ProvidersState::new(providers, errors.clone());

    // Skills: the hub stores the bytes and installs the effective set per harness.
    let skills = Skills::new(&data_dir);
    let skill_state = SkillsState::new(skills, errors.clone());

    // Humans: approvals and questions.
    let human_state = HumansState::shared(Humans::new(bus.clone()), errors.clone());

    // Every route requires the bearer token (o5: no anonymous discovery).
    let bearer = BearerToken::from_env_or_generate();

    // One axum app: the transport surface plus the domain routes.
    let app = finish(routes(), transport)
        .merge(plugin_routes().with_state(plugin_state))
        .merge(session_routes().with_state(session_state))
        .merge(provider_routes().with_state(provider_state))
        .merge(harness_routes().with_state(harness_state))
        .merge(skill_routes().with_state(skill_state))
        .merge(human_routes().with_state(human_state));

    // The bearer guard wraps the whole app: a missing or wrong token is 401.
    let app = app.layer(axum::middleware::from_fn_with_state(bearer.clone(), require_bearer));

    let addr = std::env::var("AGENT_HUB_ADDR").unwrap_or_else(|_| "127.0.0.1:0".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local = listener.local_addr()?;

    // endpoint.json: the URL and bearer a client reads. Discovery is NOT
    // anonymous (o5), so the token is required on every route; the file is how a
    // conforming client learns it.
    let endpoint = serde_json::json!({
        "url": format!("http://{local}"),
        "token": bearer.0,
        "pid": std::process::id(),
    });
    std::fs::write(
        data_dir.join("endpoint.json"),
        serde_json::to_string_pretty(&endpoint).unwrap(),
    )
    .map_err(|e| format!("write endpoint.json: {e}"))?;

    tracing::info!(%local, data_dir = %data_dir.display(), "agent-hub listening");
    println!("{local}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
