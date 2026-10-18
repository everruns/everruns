#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::{EventCursor, EventReadLimit, EventReadRequest, EventReader};
use everruns_durable_object::{AuthorityClient, Executor, SessionBinding};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "authority-test-token-at-least-32-bytes";
const EXECUTOR_TOKEN: &str = "executor-test-token-at-least-32-bytes";

#[test]
fn authority_configuration_rejects_unsafe_origins_and_ambiguous_tenants() {
    for url in [
        "http://example.com",
        "https://user:password@example.com",
        "https://example.com/?token=x",
        "https://example.com/path",
    ] {
        assert!(AuthorityClient::new(url, "test", TOKEN).is_err(), "{url}");
    }
    assert!(AuthorityClient::new("https://example.com", "../other", TOKEN).is_err());
    assert!(AuthorityClient::new("https://example.com", "test", "short").is_err());
    assert!(AuthorityClient::new("http://127.0.0.1:8789", "test", TOKEN).is_ok());
}

#[tokio::test]
async fn cross_session_reads_fail_before_a_network_request() {
    let authority = AuthorityClient::new("http://127.0.0.1:1", "test", TOKEN)
        .unwrap()
        .session(SessionId::new());
    let cursor = EventCursor::after(SessionId::new(), 0).unwrap();
    let request =
        EventReadRequest::new(SessionId::new(), EventReadLimit::default()).with_cursor(cursor);
    assert!(authority.read_page(request).await.is_err());
}

#[tokio::test]
async fn unauthorized_or_wrong_build_activations_never_reconstruct_agent_code() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = calls.clone();
    let engine = everruns::Engine::builder()
        .hosting(everruns_durable_object::ObjectHosting::new(
            AuthorityClient::new("http://127.0.0.1:1", "test", TOKEN).unwrap(),
        ))
        .register(
            everruns::hosting::AgentBuild::new("test", "v1"),
            everruns::Agent::builder()
                .instructions("test")
                .model(everruns::Model::simulated("test"))
                .on_agent_start(move |_| {
                    seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async {}
                })
                .build()
                .unwrap(),
        )
        .build();
    assert!(
        Executor::new(
            AuthorityClient::new("http://127.0.0.1:1", "test", TOKEN).unwrap(),
            SessionBinding {
                agent: "test".into(),
                build_id: "v1".into()
            },
            TOKEN,
            engine.clone(),
            1,
        )
        .is_err(),
        "authority and executor credentials must be distinct"
    );
    let executor = Executor::new(
        AuthorityClient::new("http://127.0.0.1:1", "test", TOKEN).unwrap(),
        SessionBinding {
            agent: "test".into(),
            build_id: "v1".into(),
        },
        EXECUTOR_TOKEN,
        engine,
        1,
    )
    .unwrap();
    for (authorization, build, status) in [
        ("Bearer wrong", "v1", StatusCode::UNAUTHORIZED),
        (
            "Bearer executor-test-token-at-least-32-bytes",
            "v2",
            StatusCode::CONFLICT,
        ),
    ] {
        let request = Request::builder().method("POST").uri("/execute").header("content-type","application/json").header("authorization",authorization)
            .body(Body::from(serde_json::json!({"tenant":"test","session_id":SessionId::new(),"binding":{"agent":"test","build_id":build}}).to_string())).unwrap();
        assert_eq!(
            executor
                .clone()
                .router()
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            status
        );
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}
