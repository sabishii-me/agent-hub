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
use agent_hub_transport::{finish, routes, Admission, Transport};

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
    let plugin_state = PluginsState::new(plugins, transport.clone());

    // One axum app: the transport surface plus the domain routes.
    let app = finish(routes(), transport)
        .merge(plugin_routes().with_state(plugin_state));

    let addr = std::env::var("AGENT_HUB_ADDR").unwrap_or_else(|_| "127.0.0.1:0".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local = listener.local_addr()?;
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
