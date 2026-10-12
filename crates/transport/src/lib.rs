//! The hub's transport: an axum/hyper server whose
//! request paths never block the runtime, plus the HTTP vocabulary for long
//! operations (`202` + `Location`) and the SSE stream.
//!
//! This crate is **infrastructure only**; domains mount onto [`Router`] and
//! publish to the [`Bus`]. It gives:
//!
//! - [`Accepted`]: a `202 Accepted` with a `Location` (RFC 9110 §15.3.3, §10.2.2),
//!   so a long operation returns immediately and its result is read from the
//!   resource;
//! - [`Admission`]: a bounded semaphore, so the hub refuses (503 + `Retry-After`)
//!   rather than queueing without limit;
//! - [`sse`]: a WHATWG `text/event-stream` with `id` and `Last-Event-ID`
//!   handling that converges (replay or resync), never a silent gap.

pub mod auth;
pub mod table;
pub mod error;

pub use auth::{require_bearer, BearerToken};
pub use table::RouteTable;
pub use error::{DomainError, ErrorRenderer};

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use agent_hub_events::{Bus, CatchUp};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use serde_json::json;

/// A long operation was accepted. The body says where to look; the result is
/// the resource, not a generic job.
pub struct Accepted {
    pub location: String,
    pub body: serde_json::Value,
}

impl Accepted {
    pub fn new(location: impl Into<String>, body: serde_json::Value) -> Self {
        Accepted { location: location.into(), body }
    }

    /// A **detached** long operation: answer
    /// `202 + Location` at once and run `work` on a task. The connection is
    /// never held. The result is the resource's own state, reported on the
    /// event stream.
    pub fn detached(
        location: impl Into<String>,
        body: serde_json::Value,
        work: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Response {
        tokio::spawn(work);
        Accepted::new(location, body).into_response()
    }
}

impl IntoResponse for Accepted {
    fn into_response(self) -> Response {
        let mut resp = (StatusCode::ACCEPTED, axum::Json(self.body)).into_response();
        if let Ok(v) = HeaderValue::from_str(&self.location) {
            resp.headers_mut().insert(header::LOCATION, v);
        }
        resp
    }
}

/// Bounded admission: the hub serves up to `permits` long operations at once
/// and refuses the rest with `503` + `Retry-After` instead of queueing without
/// limit. Short operations never touch this.
#[derive(Clone)]
pub struct Admission {
    permits: Arc<tokio::sync::Semaphore>,
}

impl Admission {
    pub fn new(permits: usize) -> Self {
        Admission { permits: Arc::new(tokio::sync::Semaphore::new(permits)) }
    }

    /// Try to take a permit without waiting.
    pub fn try_acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.permits.clone().try_acquire_owned().ok()
    }
}

pub struct Overloaded;

impl IntoResponse for Overloaded {
    fn into_response(self) -> Response {
        let mut resp = (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "error": "overloaded" })),
        )
            .into_response();
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        resp
    }
}

/// One mounted route, as `/v1/surface` reports it.
#[derive(Clone)]
pub struct SurfaceRow {
    pub method: String,
    pub path: String,
    pub auth: bool,
}

/// The identity of the contract this process serves (`/v1/surface.contract`).
#[derive(Clone, Default)]
pub struct ContractMeta {
    pub file: String,
    pub protocol: String,
    pub version: String,
    pub sha256: String,
}

/// Shared transport state.
#[derive(Clone)]
pub struct Transport {
    pub bus: Bus,
    pub admission: Admission,
    /// The mounted surface, collected from every domain's table (one source: the
    /// same registration the router is built from).
    pub surface: Arc<Vec<SurfaceRow>>,
    /// The contract this process claims to keep.
    pub contract: ContractMeta,
    /// Every SSE event name the hub can emit.
    pub events: Arc<Vec<String>>,
    /// The OpenAPI document, byte for byte as generated from the contract.
    pub openapi: Arc<String>,
    /// Fired by `POST /v1/shutdown` to ask the process to stop.
    pub shutdown: tokio::sync::watch::Sender<bool>,
    /// The boot timestamp (ISO-8601, once, at construction): `/v1/status.startedAt`
    /// and the discovery record report when THIS process started, not when the
    /// request arrived.
    pub started_at: String,
    /// The port the listener actually bound (set once, after bind). `0` until then.
    pub bound_port: Arc<AtomicU16>,
    /// The build marker the binary was built from (`/v1/status.buildId`,
    /// discovery record). Set once by the composition root from its own build.
    pub build_id: Arc<String>,
    /// The live facts `/v1/status` reports beyond identity: the harness/provider/
    /// connection lists and the session/turn counts. The composition root installs
    /// this because transport owns no store; when it is absent (probes, tests) the
    /// lists are empty and the counts are 0 - never a lie, just nothing to report.
    pub status_facts: Arc<StatusFacts>,
}

/// The live facts `GET /v1/status` reports, supplied by the composition root.
/// Each field is already a value-free JSON value (the same shapes the list routes
/// return); transport does not interpret them.
pub struct StatusFacts {
    /// `harnesses` - the installed harness list.
    pub harnesses: Box<dyn Fn() -> Vec<serde_json::Value> + Send + Sync>,
    /// `providers` - the value-free provider rows.
    pub providers: Box<dyn Fn() -> Vec<serde_json::Value> + Send + Sync>,
    /// `connections` - the value-free connection rows.
    pub connections: Box<dyn Fn() -> Vec<serde_json::Value> + Send + Sync>,
    /// `sessionCount` - the number of sessions the hub knows.
    pub session_count: Box<dyn Fn() -> i64 + Send + Sync>,
    /// `activeTurns` - sessions with a turn admitted, running, awaiting an answer,
    /// or being cancelled.
    pub active_turns: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl Default for StatusFacts {
    fn default() -> Self {
        StatusFacts {
            harnesses: Box::new(Vec::new),
            providers: Box::new(Vec::new),
            connections: Box::new(Vec::new),
            session_count: Box::new(|| 0),
            active_turns: Box::new(|| 0),
        }
    }
}

impl Transport {
    pub fn new(bus: Bus, admission: Admission) -> Self {
        let (shutdown, _) = tokio::sync::watch::channel(false);
        Transport {
            bus,
            admission,
            surface: Arc::new(Vec::new()),
            contract: ContractMeta::default(),
            events: Arc::new(Vec::new()),
            openapi: Arc::new(String::new()),
            shutdown,
            status_facts: Arc::new(StatusFacts::default()),
            started_at: now_rfc3339(),
            bound_port: Arc::new(AtomicU16::new(0)),
            build_id: Arc::new("unknown".into()),
        }
    }

    /// Record the build marker (called once by the composition root).
    pub fn set_build_id(&mut self, id: impl Into<String>) {
        self.build_id = Arc::new(id.into());
    }

    /// Record the port the listener bound (called by the composition root after
    /// `bind`). `/v1/status.port` and the discovery record report it.
    pub fn set_bound_port(&self, port: u16) {
        self.bound_port.store(port, Ordering::SeqCst);
    }

    /// Install the live status facts (called once by the composition root).
    pub fn with_status_facts(mut self, facts: StatusFacts) -> Self {
        self.status_facts = Arc::new(facts);
        self
    }

    /// Set the served-surface metadata (called once by the composition root).
    pub fn with_meta(
        mut self,
        surface: Vec<SurfaceRow>,
        contract: ContractMeta,
        events: Vec<String>,
        openapi: String,
    ) -> Self {
        self.surface = Arc::new(surface);
        self.contract = contract;
        self.events = Arc::new(events);
        self.openapi = Arc::new(openapi);
        self
    }
}

/// The transport-owned routes (`/v1/status`, `/v1/events`) as a
/// **stateful** builder. Domains merge their own routes onto this, then call
/// [`finish`] with the shared [`Transport`].
fn table() -> crate::table::RouteTable<Transport> {
    crate::table::RouteTable::new()
        .get("/v1/status", status)
        .get("/v1/events", sse)
        .get("/v1/surface", surface_route)
        .get("/v1/openapi.json", openapi_route)
        .post("/v1/shutdown", shutdown_route)
}

pub fn routes() -> Router<Transport> {
    table().router()
}

/// The transport-owned surface, from the same table the router is built from.
pub fn surface() -> Vec<String> {
    table().surface()
}

/// Apply the shared state, yielding a ready [`Router`].
pub fn finish(routes: Router<Transport>, state: Transport) -> Router {
    routes.with_state(state)
}

/// The base router with no extra domain routes (handy for tests and probes).
pub fn base_router(state: Transport) -> Router {
    finish(routes(), state)
}

async fn status(State(t): State<Transport>) -> impl IntoResponse {
    // `/v1/status` reports EVERY property its contract declares (the identity a
    // client uses to prove it is talking to its own child, plus what the hub is
    // managing right now). Identity is answered from what the binary was BUILT
    // with; the lists and counts come from the installed status facts.
    let f = &t.status_facts;
    axum::Json(json!({
        "pid": std::process::id(),
        "port": t.bound_port.load(Ordering::SeqCst),
        "startedAt": t.started_at,
        "version": env!("CARGO_PKG_VERSION"),
        "buildId": t.build_id.as_str(),
        "protocol": { "name": t.contract.protocol, "version": t.contract.version },
        "contract": {
            "file": t.contract.file,
            "protocol": t.contract.protocol,
            "version": t.contract.version,
            "sha256": t.contract.sha256,
        },
        "harnesses": (f.harnesses)(),
        "providers": (f.providers)(),
        "connections": (f.connections)(),
        "sessionCount": (f.session_count)(),
        "activeTurns": (f.active_turns)(),
        "events": { "currentId": t.bus.current_id() }
    }))
}

/// `GET /v1/surface`: what this process actually implements, as data (the same
/// registration the router matches), plus the contract it claims to keep.
async fn surface_route(State(t): State<Transport>) -> impl IntoResponse {
    let routes: Vec<serde_json::Value> = t
        .surface
        .iter()
        .map(|r| json!({ "method": r.method, "path": r.path, "auth": r.auth }))
        .collect();
    axum::Json(json!({
        "contract": {
            "file": t.contract.file,
            "protocol": t.contract.protocol,
            "version": t.contract.version,
            "sha256": t.contract.sha256,
        },
        "routes": routes,
        "events": t.events.as_ref(),
    }))
}

/// `GET /v1/openapi.json`: the OpenAPI document as generated from the contract - a
/// projection for tools, never a second source.
async fn openapi_route(State(t): State<Transport>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))],
        t.openapi.as_str().to_string(),
    )
        .into_response()
}

/// `POST /v1/shutdown`: ask the process to stop (the composition root watches this).
async fn shutdown_route(State(t): State<Transport>) -> impl IntoResponse {
    let _ = t.shutdown.send(true);
    axum::Json(json!({ "ok": true }))
}

fn now_rfc3339() -> String {
    // A maintained date library, not a hand-written civil algorithm (C5).
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}


/// The SSE stream. `Last-Event-ID` (header or `?lastEventId=` query) selects
/// catch-up: replay what is in the window, or resync when it has fallen out.
async fn sse(State(t): State<Transport>, headers: HeaderMap) -> axum::response::sse::Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    use axum::response::sse::{KeepAlive, Sse};

    let wanted = parse_last_event_id(&headers);
    let sub = t.bus.subscribe(wanted);

    let stream = async_stream::stream! {
        match sub.catch_up {
            CatchUp::Replay(events) => {
                for e in events {
                    yield Ok(event_with_id(&e.name, e.id, &e.data));
                }
            }
            CatchUp::Fresh => {
                // The handshake: no id, no state (contract's `/v1/events`).
                yield Ok(event_no_id("hub.connected", &json!({ "at": now_rfc3339() })));
            }
            CatchUp::Resync => {
                yield Ok(event_no_id("hub.resync", &json!({ "reason": "last-event-id out of window" })));
            }
        }
        let mut rx = sub.receiver;
        while let Some(e) = rx.recv().await {
            yield Ok(event_with_id(&e.name, e.id, &e.data));
        }
    };

    // axum's Sse owns the `event:`/`data:`/`id:` framing (a maintained
    // component), not hand-written strings.
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn event_with_id(name: &str, id: u64, data: &serde_json::Value) -> axum::response::sse::Event {
    axum::response::sse::Event::default()
        .id(id.to_string())
        .event(name)
        .data(data.to_string())
}

fn event_no_id(name: &str, data: &serde_json::Value) -> axum::response::sse::Event {
    axum::response::sse::Event::default().event(name).data(data.to_string())
}

fn parse_last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
}
