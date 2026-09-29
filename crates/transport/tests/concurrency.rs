//! Transport-level component tests, against a real listener.
//!
//! **These test the transport primitives, not the product.** A cross-review
//! (TASK-048 F05) correctly refused the earlier framing: a test-only `/v1/heavy`
//! route and counting status *requests* do not demonstrate "heavy operations" or
//! "thousands of idle connections". What is asserted here is only:
//!
//! - N concurrent requests to a real listener are all answered;
//! - two requests that park on a gate both complete while a third is refused;
//! - the primitives do not deadlock.
//!
//! No product capability is claimed. The ADR-0009 numbers remain an open gate
//! (`docs/review/VERIFICATION-TASKS.md`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_hub_events::Bus;
use agent_hub_transport::{finish, routes, Admission, Overloaded, Transport};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

fn install_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// A router with a heavyweight route that parks on a gate (a stand-in for a
/// plugin install or a model turn). Returns the router and the gate.
fn hub_with_gate(permits: usize) -> (Router, Arc<tokio::sync::Notify>) {
    install_provider();
    let gate = Arc::new(tokio::sync::Notify::new());
    let g = gate.clone();
    let admission = Admission::new(permits);
    let handler_admission = admission.clone();
    let router = finish(
        routes().route(
            "/v1/heavy",
            post(move |State(_): State<Transport>| {
                let g = g.clone();
                let admission = handler_admission.clone();
                async move {
                    let Some(permit) = admission.try_acquire() else {
                        return Overloaded.into_response();
                    };
                    g.notified().await;
                    drop(permit);
                    axum::Json(serde_json::json!({ "done": true })).into_response()
                }
            }),
        ),
        Transport::new(Bus::new(64, 64), admission),
    );
    (router, gate)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_concurrent_requests_are_all_answered() {
    let (router, gate) = hub_with_gate(1);
    let base = serve(router).await;
    let client = reqwest::Client::new();
    let _ = gate; // no heavy work needed for this one

    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..200 {
        let c = client.clone();
        let b = base.clone();
        handles.push(tokio::spawn(async move {
            c.get(format!("{b}/v1/status")).send().await.unwrap().status()
        }));
    }
    let mut ok = 0;
    for h in handles {
        if h.await.unwrap() == 200 {
            ok += 1;
        }
    }
    let elapsed = started.elapsed();
    assert_eq!(ok, 200, "all 200 concurrent connections served");
    // The bound is generous; the point is that nothing stalls.
    assert!(elapsed < Duration::from_secs(5), "200 concurrent took {elapsed:?}");
    eprintln!("component: 200 concurrent status requests in {elapsed:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_gated_requests_both_complete_and_a_third_is_refused() {
    // Two permits: two heavy operations run at once; a third is refused.
    let (router, gate) = hub_with_gate(2);
    let base = serve(router).await;
    let client = reqwest::Client::new();

    let a = tokio::spawn({
        let c = client.clone();
        let b = base.clone();
        async move { c.post(format!("{b}/v1/heavy")).send().await.unwrap().status() }
    });
    let b2 = tokio::spawn({
        let c = client.clone();
        let b = base.clone();
        async move { c.post(format!("{b}/v1/heavy")).send().await.unwrap().status() }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // While both run, status polls keep answering.
    for _ in 0..20 {
        let r = client.get(format!("{base}/v1/status")).send().await.unwrap();
        assert_eq!(r.status(), 200);
    }

    // A third heavy op is refused (bounded), not queued.
    let third = client.post(format!("{base}/v1/heavy")).send().await.unwrap();
    assert_eq!(third.status(), 503);

    gate.notify_waiters();
    let (ra, rb) = tokio::join!(a, b2);
    assert_eq!((ra.unwrap().as_u16(), rb.unwrap().as_u16()), (200, 200));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_concurrent_requests_do_not_deadlock() {
    // 500 concurrent REQUESTS (which is not the same as 500 idle open
    // connections). This checks the transport does not deadlock; it does NOT
    // demonstrate an idle-connection count.
    let (router, _gate) = hub_with_gate(1);
    let base = serve(router).await;
    let client = reqwest::Client::new();

    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..500 {
        let c = client.clone();
        let b = base.clone();
        handles.push(tokio::spawn(async move {
            c.get(format!("{b}/v1/status")).send().await.unwrap().status()
        }));
    }
    for h in handles {
        assert_eq!(h.await.unwrap(), 200);
    }
    let elapsed = started.elapsed();
    eprintln!("component: 500 concurrent status requests in {elapsed:?}");
    assert!(elapsed < Duration::from_secs(10), "500 concurrent took {elapsed:?}");
}
