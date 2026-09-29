//! The agent-hub process (`ARCHITECTURE` §17, the frame).
//!
//! This is the executable shell: a non-blocking axum server over the shared
//! [`Transport`]. Domains mount their routes onto [`transport::routes`] as they
//! are implemented; today it serves the transport-owned surface (`/v1/hub/status`
//! and the `/v1/hub/events` SSE stream).

use agent_hub_events::Bus;
use agent_hub_transport::{routes, Admission, Transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let bus = Bus::new(1024, 256);
    let admission = Admission::new(64);
    let state = Transport::new(bus, admission);

    let app = routes().with_state(state);

    let addr = std::env::var("AGENT_HUB_ADDR").unwrap_or_else(|_| "127.0.0.1:0".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local = listener.local_addr()?;
    tracing::info!(%local, "agent-hub listening");

    // Print the bound address for a supervisor that asked for port 0.
    println!("{local}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
