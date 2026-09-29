//! Transport acceptance: the semantics the contract claims, exercised against a
//! real listener (task T6 + the `202`/SSE parts of T4).

use std::sync::Arc;
use std::time::Duration;

use agent_hub_events::Bus;
use agent_hub_transport::{base_router, finish, routes, Accepted, Admission, Overloaded, Transport};
use axum::extract::State;
use axum::routing::post;
use axum::response::IntoResponse;
use axum::Router;

/// Install a rustls crypto provider once per process (a workspace build may
/// unify reqwest's TLS feature without a provider).
fn install_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Boot the router on an ephemeral loopback port; return its base URL.
async fn serve(router: Router) -> String {
    install_provider();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

fn state() -> Transport {
    Transport::new(Bus::new(64, 64), Admission::new(4))
}

#[tokio::test]
async fn accepted_returns_202_and_location() {
    let t = state();
    let router = finish(
        routes().route(
            "/v1/things",
            post(|State(_): State<Transport>| async {
                Accepted::new("/v1/things/42", serde_json::json!({ "state": "installing" }))
            }),
        ),
        t,
    );
    let base = serve(router).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/v1/things"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    assert_eq!(resp.headers()["location"], "/v1/things/42");
}

#[tokio::test]
async fn overload_is_refused_with_retry_after() {
    let t = state();
    // A route that holds every permit until we say so.
    let gate = Arc::new(tokio::sync::Notify::new());
    let g = gate.clone();
    let permits = Admission::new(2);
    let handler_permits = permits.clone();
    let router = finish(
        routes().route(
            "/v1/long",
            post(move |State(_): State<Transport>| {
                let g = g.clone();
                let permits = handler_permits.clone();
                async move {
                    let Some(permit) = permits.try_acquire() else {
                        return Overloaded.into_response();
                    };
                    g.notified().await;
                    drop(permit);
                    axum::Json(serde_json::json!({ "ok": true })).into_response()
                }
            }),
        ),
        Transport::new(t.bus.clone(), permits.clone()),
    );
    let base = serve(router).await;

    let client = reqwest::Client::new();
    // Fill the two permits.
    let a = tokio::spawn({
        let c = client.clone();
        let b = base.clone();
        async move { c.post(format!("{b}/v1/long")).send().await.unwrap().status() }
    });
    let b2 = tokio::spawn({
        let c = client.clone();
        let b = base.clone();
        async move { c.post(format!("{b}/v1/long")).send().await.unwrap().status() }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A third is refused, not queued forever.
    let third = client
        .post(format!("{base}/v1/long"))
        .send()
        .await
        .unwrap();
    assert_eq!(third.status(), 503);
    assert_eq!(third.headers()["retry-after"], "1");

    gate.notify_waiters();
    let _ = tokio::join!(a, b2);
}

#[tokio::test]
async fn sse_delivers_and_replays_by_last_event_id() {
    let bus = Bus::new(64, 64);
    let t = Transport::new(bus.clone(), Admission::new(4));
    let router = base_router(t);
    let base = serve(router).await;
    let client = reqwest::Client::new();

    // Subscribe first, then publish; the open stream receives the change.
    let mut stream: ByteStream = Box::pin(
        client
            .get(format!("{base}/v1/events"))
            .send()
            .await
            .unwrap()
            .bytes_stream(),
    );
    // The handshake frame (no id).
    let first = next_chunk(&mut stream).await;
    assert!(first.contains("event: hub.connected"), "handshake: {first}");

    let id = bus.publish("hub.plugins.changed", serde_json::json!({ "id": "p", "state": "ready" }));
    let frame = next_chunk_containing(&mut stream, "hub.plugins.changed").await;
    assert!(frame.contains(&format!("id: {id}")), "frame: {frame}");
    drop(stream);

    // Reconnect with Last-Event-ID: the change is replayed.
    let mut replay: ByteStream = Box::pin(
        client
            .get(format!("{base}/v1/events"))
            .header("last-event-id", "0")
            .send()
            .await
            .unwrap()
            .bytes_stream(),
    );
    let frame = next_chunk_containing(&mut replay, "hub.plugins.changed").await;
    assert!(frame.contains("hub.plugins.changed"), "replay: {frame}");
}

#[tokio::test]
async fn sse_resyncs_when_last_event_id_fell_out_of_window() {
    let bus = Bus::new(2, 8);
    for i in 0..5 {
        bus.publish("x", serde_json::json!(i));
    }
    let t = Transport::new(bus, Admission::new(4));
    let base = serve(base_router(t)).await;

    let mut stream: ByteStream = Box::pin(
        reqwest::Client::new()
            .get(format!("{base}/v1/events"))
            .header("last-event-id", "1")
            .send()
            .await
            .unwrap()
            .bytes_stream(),
    );
    let text = next_chunk_containing(&mut stream, "hub.resync").await;
    assert!(text.contains("hub.resync"), "resync: {text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_concurrent_connections_are_served_while_one_op_runs() {
    // T6: >= 100 concurrent connections, and one in-flight operation does not
    // time another out. A real listener, real sockets.
    let t = state();
    let permits = Admission::new(1);
    let slow_gate = Arc::new(tokio::sync::Notify::new());
    let g = slow_gate.clone();
    let p = permits.clone();
    let router = finish(
        routes().route(
            "/v1/slow",
            post(move |State(_): State<Transport>| {
                let g = g.clone();
                let p = p.clone();
                async move {
                    let Some(permit) = p.try_acquire() else {
                        return Overloaded.into_response();
                    };
                    g.notified().await;
                    drop(permit);
                    axum::Json(serde_json::json!({ "done": true })).into_response()
                }
            }),
        ),
        Transport::new(t.bus.clone(), permits.clone()),
    );
    let base = serve(router).await;
    let client = reqwest::Client::new();

    // Start one long operation.
    let slow = tokio::spawn({
        let c = client.clone();
        let b = base.clone();
        async move { c.post(format!("{b}/v1/slow")).send().await.unwrap().status() }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 150 concurrent status polls must all succeed, each fast.
    let started = std::time::Instant::now();
    let mut handles = Vec::new();
    for _ in 0..150 {
        let c = client.clone();
        let b = base.clone();
        handles.push(tokio::spawn(async move {
            let r = c.get(format!("{b}/v1/status")).send().await.unwrap();
            (r.status(), r.text().await.unwrap())
        }));
    }
    for h in handles {
        let (status, body) = h.await.unwrap();
        assert_eq!(status, 200, "a status poll failed while a slow op ran");
        assert!(body.contains("\"pid\""), "body: {body}");
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "150 concurrent polls took {:?}",
        started.elapsed()
    );

    slow_gate.notify_waiters();
    let _ = slow.await;
}

// --- tiny SSE reader helpers ---------------------------------------------

type ByteStream = std::pin::Pin<Box<dyn futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>;

async fn next_chunk(stream: &mut ByteStream) -> String {
    use futures::StreamExt;
    let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timeout")
        .expect("stream ended")
        .expect("chunk error");
    String::from_utf8_lossy(&chunk).to_string()
}

async fn next_chunk_containing(stream: &mut ByteStream, needle: &str) -> String {
    for _ in 0..20 {
        let chunk = next_chunk(stream).await;
        if chunk.contains(needle) {
            return chunk;
        }
    }
    panic!("event `{needle}` not seen");
}
