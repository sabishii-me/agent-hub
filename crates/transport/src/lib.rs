//! The hub's transport (`ARCHITECTURE` §9, task T6): an axum/hyper server whose
//! request paths never block the runtime, plus the HTTP vocabulary for long
//! operations (`202` + `Location`) and the SSE stream (`ARCHITECTURE` §11).
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

use agent_hub_events::{Bus, CatchUp};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
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

    /// A **detached** long operation (ARCHITECTURE §13.2, ADR-0009): answer
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

/// Shared transport state.
#[derive(Clone)]
pub struct Transport {
    pub bus: Bus,
    pub admission: Admission,
}

impl Transport {
    pub fn new(bus: Bus, admission: Admission) -> Self {
        Transport { bus, admission }
    }
}

/// The transport-owned routes (`/v1/status`, `/v1/events`) as a
/// **stateful** builder. Domains merge their own routes onto this, then call
/// [`finish`] with the shared [`Transport`].
/// The routes this module mounts (the boot surface check reads every module's).
pub fn surface() -> &'static [&'static str] {
    &["GET /v1/status", "GET /v1/events"]
}

pub fn routes() -> Router<Transport> {
    Router::new()
        .route("/v1/status", get(status))
        .route("/v1/events", get(sse))
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
    axum::Json(json!({
        "pid": std::process::id(),
        "startedAt": now_rfc3339(),
        "events": { "currentId": t.bus.current_id() }
    }))
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
    // component), not hand-written strings (TASK-048 C5).
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
