//! Humans domain acceptance: an approval is decided over HTTP, only from the
//! offered options; a question carries answers back.

use std::sync::Arc;

use agent_hub_events::Bus;
use agent_hub_humans::routes::{routes, HumansState};
use agent_hub_humans::Humans;

fn errors() -> agent_hub_transport::ErrorRenderer {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contract/errors.json"
    ))
    .unwrap();
    agent_hub_transport::ErrorRenderer::new(agent_hub_contract::ErrorTable::parse(&raw).unwrap())
}

fn install_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn serve(humans: Arc<Humans>) -> String {
    install_provider();
    let state = HumansState::new(humans, errors());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, routes().with_state(state)).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn approval_decided_over_http_and_only_from_offered_options() {
    let bus = Bus::new(64, 64);
    let humans = Arc::new(Humans::new(bus));
    let base = serve(humans.clone()).await;
    let client = reqwest::Client::new();

    let ap = humans.raise_approval(
        "s1",
        "confirm",
        serde_json::json!({ "cmd": "rm -rf build" }),
        Some(vec!["Allow Once".into(), "Reject".into()]),
    );

    // It is listed.
    let list: serde_json::Value = client
        .get(format!("{base}/v1/sessions/s1/approvals"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["approvals"].as_array().unwrap().len(), 1);
    assert_eq!(list["approvals"][0]["id"], ap.id);
    assert_eq!(list["approvals"][0]["tool"], "confirm");

    // A decision NOT offered is refused (fail closed).
    let r = client
        .post(format!("{base}/v1/sessions/s1/approvals/{}", ap.id))
        .json(&serde_json::json!({ "decision": "Allow Always" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "validation_failed");

    // An offered decision is accepted.
    let r = client
        .post(format!("{base}/v1/sessions/s1/approvals/{}", ap.id))
        .json(&serde_json::json!({ "decision": "Allow Once" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let decided: serde_json::Value = r.json().await.unwrap();
    assert_eq!(decided["approval"]["decision"], "Allow Once");
}

#[tokio::test]
async fn a_question_carries_answers_back() {
    let bus = Bus::new(64, 64);
    let humans = Arc::new(Humans::new(bus));
    let base = serve(humans.clone()).await;
    let client = reqwest::Client::new();

    let q = humans.raise_question(
        "s2",
        serde_json::json!([{ "id": "q1", "question": "Which branch?" }]),
    );

    let r = client
        .post(format!("{base}/v1/sessions/s2/questions/{}", q.id))
        .json(&serde_json::json!({ "answers": [{ "id": "q1", "custom": "main" }] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let answered: serde_json::Value = r.json().await.unwrap();
    assert_eq!(answered["question"]["answers"][0]["custom"], "main");
}

#[tokio::test]
async fn unknown_approval_is_a_contract_code() {
    let bus = Bus::new(16, 16);
    let humans = Arc::new(Humans::new(bus));
    let base = serve(humans).await;
    let r = reqwest::Client::new()
        .post(format!("{base}/v1/sessions/s1/approvals/nope"))
        .json(&serde_json::json!({ "decision": "Allow Once" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "approval_not_found");
}
