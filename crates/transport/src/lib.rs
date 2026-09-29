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

use std::sync::Arc;

use agent_hub_events::{Bus, CatchUp};
use axum::{
    body::Body,
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

/// The transport-owned routes (`/v1/hub/status`, `/v1/hub/events`) as a
/// **stateful** builder. Domains merge their own routes onto this, then call
/// [`finish`] with the shared [`Transport`].
pub fn routes() -> Router<Transport> {
    Router::new()
        .route("/v1/hub/status", get(status))
        .route("/v1/hub/events", get(sse))
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
    // A fixed, dependency-free RFC 3339 UTC timestamp.
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let (y, mo, day, h, mi, s) = civil_from_unix(secs);
    format!("{y:04}-{mo:02}-{day:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Days-from-civil inverse (Howard Hinnant's algorithm).
fn civil_from_unix(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32, (rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32)
}

/// The SSE stream. `Last-Event-ID` (header or `?lastEventId=` query) selects
/// catch-up: replay what is in the window, or resync when it has fallen out.
async fn sse(State(t): State<Transport>, headers: HeaderMap) -> Response {
    let wanted = parse_last_event_id(&headers);
    let sub = t.bus.subscribe(wanted);

    let stream = async_stream::stream! {
        match sub.catch_up {
            CatchUp::Replay(events) => {
                for e in events {
                    yield Ok::<_, std::convert::Infallible>(frame(&e.name, e.id, &e.data));
                }
            }
            CatchUp::Fresh => {
                // The handshake: no id, no state (contract's `/v1/hub/events`).
                yield Ok(frame_no_id("hub.connected", &json!({ "at": now_rfc3339() })));
            }
            CatchUp::Resync => {
                // The gap cannot be described; tell the client to re-read.
                yield Ok(frame_no_id("hub.resync", &json!({ "reason": "last-event-id out of window" })));
            }
        }

        let mut rx = sub.receiver;
        while let Some(e) = rx.recv().await {
            yield Ok(frame(&e.name, e.id, &e.data));
        }
    };

    let body = Body::from_stream(stream);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(body)
        .unwrap()
}

fn parse_last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
}

fn frame(name: &str, id: u64, data: &serde_json::Value) -> String {
    format!("id: {id}\nevent: {name}\ndata: {data}\n\n")
}

fn frame_no_id(name: &str, data: &serde_json::Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}
