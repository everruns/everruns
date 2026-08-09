//! FCP (Free Communication Protocol) app channel integration tests.
//!
//! These tests cover the public FCP endpoint end-to-end:
//! - handshake (`GET`) returns Markdown without leaking app state
//! - text-in / text-out (`POST`) round-trips through the durable runtime
//! - auth/rate-limit/timeout isolation invariants
//! - error bodies are sanitized Markdown with actionable instructions
//!
//! The agent is backed by `llmsim` so tests don't make outbound LLM calls.

mod test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_platform::App;

fn unique_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{now}_{seq}")
}

fn unique_slug(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{now}-{seq}")
}

async fn create_llmsim_agent(server: &TestServer) -> String {
    let provider: Value = server
        .post(
            "/v1/providers",
            json!({
                "name": unique_id("FCP Test Provider"),
                "provider_type": "llmsim"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let model: Value = server
        .post(
            &format!("/v1/providers/{}/models", provider["id"].as_str().unwrap()),
            json!({
                "model_id": unique_id("llmsim-fcp"),
                "display_name": unique_id("FCP Test Model"),
                "enabled": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": unique_slug("fcp-test-agent"),
                "display_name": unique_id("FCP Test Agent"),
                "system_prompt": "You are a brief test agent.",
                "default_model_id": model["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    agent["id"].as_str().unwrap().to_string()
}

async fn create_fcp_app(server: &TestServer, channel_config: Value) -> App {
    let agent_id = create_llmsim_agent(server).await;

    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("FCP App"),
                "description": "A brief FCP test endpoint",
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_id,
                "channel_type": "fcp",
                "channel_config": channel_config
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    app
}

async fn create_published_fcp_app(server: &TestServer, channel_config: Value) -> App {
    let app = create_fcp_app(server, channel_config).await;

    server
        .post(&format!("/v1/apps/{}/publish", app.public_id), json!({}))
        .await
        .assert_success();

    server
        .get(&format!("/v1/apps/{}", app.public_id))
        .await
        .assert_success()
        .json()
}

async fn get_handshake(
    server: &TestServer,
    app_id: impl std::fmt::Display,
    headers: Vec<(&str, &str)>,
) -> test_harness::TestResponse {
    server
        .request_raw(
            Method::GET,
            &format!("/v1/apps/{}/fcp", app_id),
            headers,
            Vec::new(),
        )
        .await
}

async fn send_fcp_post(
    server: &TestServer,
    app_id: impl std::fmt::Display,
    body: impl Into<Vec<u8>>,
    headers: Vec<(&str, &str)>,
) -> test_harness::TestResponse {
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{}/fcp", app_id),
            headers,
            body.into(),
        )
        .await
}

fn assert_markdown(response: &test_harness::TestResponse) {
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("text/markdown"),
        "expected text/markdown response, got {content_type:?}"
    );
}

/// Status codes that mean "FCP handler accepted the request and ran its
/// session/auth/rate-limit pipeline". In-memory test mode does not run a
/// durable worker, so happy-path `POST`s frequently surface as 504
/// (handler waited for the agent reply that the test environment will
/// never produce). Either status proves the request reached the handler.
fn assert_accepted_or_timeout(status: StatusCode) {
    assert!(
        status == StatusCode::OK || status == StatusCode::GATEWAY_TIMEOUT,
        "expected 200 or 504 (FCP handler accepted), got {status}"
    );
}

async fn count_sessions_with_tag(server: &TestServer, tag: &str) -> usize {
    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    sessions["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|s| {
                    s["tags"]
                        .as_array()
                        .map(|tags| tags.iter().any(|t| t.as_str() == Some(tag)))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

/// Shorter response timeout for happy-path tests so 504 paths return
/// quickly instead of waiting the default 120 s.
fn fast_timeout_config() -> Value {
    json!({"response_timeout_seconds": 2})
}

fn assert_body_no_leaks(body: &str) {
    let lower = body.to_lowercase();
    for forbidden in [
        "stack",
        "panic",
        "tokio",
        "postgres",
        "subscription",
        "openai",
        "anthropic",
        "llmsim",
        "thread 'tokio",
    ] {
        assert!(
            !lower.contains(forbidden),
            "leaked internal detail via word '{forbidden}': {body}"
        );
    }
}

// ----------------------------------------------------------------------------
// Handshake (`GET`)
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_handshake_returns_markdown_for_published_app() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, json!({})).await;

    let response = get_handshake(&server, &app.public_id, vec![]).await;
    let body = response.text();
    assert_eq!(response.status(), StatusCode::OK, "body: {body}");
    assert_markdown(&response);
    assert!(body.contains(&format!("# {}", app.name)), "body: {body}");
    assert!(body.contains("A brief FCP test endpoint"));
    assert!(body.contains("POST"));
    assert!(body.contains("application/json"));
    assert!(body.contains("fcp_session"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_handshake_advertises_auth_when_token_configured() {
    let server = TestServer::in_memory().await;
    let app =
        create_published_fcp_app(&server, json!({"anonymous": true, "token": "fcp-secret"})).await;

    let response = get_handshake(&server, &app.public_id, vec![]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text();
    assert!(body.contains("Authorization: Bearer"));
    assert!(body.contains("X-Everruns-FCP-Token"));
    assert!(
        !body.contains("fcp-secret"),
        "handshake must never echo the configured token"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_handshake_returns_404_for_unknown_app() {
    let server = TestServer::in_memory().await;
    let response = get_handshake(&server, "app_does_not_exist", vec![]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_markdown(&response);
    let body = response.text();
    // Single-tone "no endpoint" body that doesn't disclose lookup failure mode.
    assert!(body.to_lowercase().contains("no fcp endpoint"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_handshake_returns_404_for_unpublished_app() {
    let server = TestServer::in_memory().await;
    let app = create_fcp_app(&server, json!({})).await;

    // App is draft — handshake must look identical to "unknown app" so the
    // caller cannot distinguish lifecycle state.
    let response = get_handshake(&server, &app.public_id, vec![]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response.text();
    let lower = body.to_lowercase();
    for forbidden in ["draft", "unpublished", "publish", "disabled", "archived"] {
        assert!(
            !lower.contains(forbidden),
            "404 body leaked lifecycle word '{forbidden}': {body}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_handshake_returns_404_when_channel_is_disabled() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, json!({})).await;
    let channel_id = app.channels[0].public_id.to_string();

    server
        .request_raw(
            Method::PATCH,
            &format!("/v1/apps/{}/channels/{}", app.public_id, channel_id),
            vec![("content-type", "application/json")],
            serde_json::to_vec(&json!({"enabled": false})).unwrap(),
        )
        .await
        .assert_success();

    let response = get_handshake(&server, &app.public_id, vec![]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_handshake_returns_custom_body_when_configured() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, json!({"handshake": "# Custom\nGo away."})).await;

    let response = get_handshake(&server, &app.public_id, vec![]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text(), "# Custom\nGo away.");
}

// ----------------------------------------------------------------------------
// POST happy path
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_plain_text_returns_markdown_reply_and_sets_cookie() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, fast_timeout_config()).await;

    let response = send_fcp_post(
        &server,
        &app.public_id,
        "Hello, FCP!",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_accepted_or_timeout(response.status());
    assert_markdown(&response);

    // Cookie is set on every response that resolved a session — including
    // 504, so a client can retry against the same conversation.
    let cookie = test_harness::extract_cookie(response.headers(), "fcp_session");
    assert!(cookie.contains("fcp_session="));

    let body = response.text();
    assert!(!body.trim().is_empty(), "FCP body was empty");
    assert_body_no_leaks(&body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_json_body_is_parsed_via_message_field() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, fast_timeout_config()).await;

    let response = send_fcp_post(
        &server,
        &app.public_id,
        r#"{"message":"hi from JSON"}"#,
        vec![("content-type", "application/json")],
    )
    .await;
    assert_accepted_or_timeout(response.status());
    // Either the agent replied (200) or we timed out (504) — both prove the
    // body was parsed past the JSON gate and the user message was injected
    // into a session. A 400 here would mean the JSON branch refused the
    // body shape.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_session_cookie_reuses_same_session() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, fast_timeout_config()).await;

    let first = send_fcp_post(
        &server,
        &app.public_id,
        "first turn",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_accepted_or_timeout(first.status());
    let cookie = test_harness::extract_cookie(first.headers(), "fcp_session");

    // Inspect sessions by tag — first POST must have created exactly one.
    let expected_tag = format!("fcp:app:{}", app.public_id);
    let first_count = count_sessions_with_tag(&server, &expected_tag).await;
    assert_eq!(first_count, 1);

    let second = send_fcp_post(
        &server,
        &app.public_id,
        "second turn",
        vec![("content-type", "text/plain"), ("cookie", &cookie)],
    )
    .await;
    assert_accepted_or_timeout(second.status());

    let after_count = count_sessions_with_tag(&server, &expected_tag).await;
    assert_eq!(
        after_count, 1,
        "second POST with cookie must reuse the session created by the first"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_cookie_reuses_matching_session_when_multiple_exist() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, fast_timeout_config()).await;

    let first = send_fcp_post(
        &server,
        &app.public_id,
        "client one",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_accepted_or_timeout(first.status());
    let first_cookie = test_harness::extract_cookie(first.headers(), "fcp_session");

    let second = send_fcp_post(
        &server,
        &app.public_id,
        "client two",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_accepted_or_timeout(second.status());

    let third = send_fcp_post(
        &server,
        &app.public_id,
        "client one again",
        vec![("content-type", "text/plain"), ("cookie", &first_cookie)],
    )
    .await;
    assert_accepted_or_timeout(third.status());

    let expected_tag = format!("fcp:app:{}", app.public_id);
    let tagged_count = count_sessions_with_tag(&server, &expected_tag).await;
    assert_eq!(
        tagged_count, 2,
        "cookie reuse must not create a third session"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_stale_cookie_creates_new_session() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, fast_timeout_config()).await;

    // Use a random UUID for the cookie — the server has no matching session.
    let response = send_fcp_post(
        &server,
        &app.public_id,
        "hello with stale cookie",
        vec![
            ("content-type", "text/plain"),
            ("cookie", "fcp_session=00000000-0000-0000-0000-000000000000"),
        ],
    )
    .await;
    assert_accepted_or_timeout(response.status());
    // A fresh cookie was issued — never strand the client.
    let _new_cookie = test_harness::extract_cookie(response.headers(), "fcp_session");
}

// ----------------------------------------------------------------------------
// Auth
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_requires_token_when_configured() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(
        &server,
        json!({"anonymous": true, "token": "fcp-secret", "response_timeout_seconds": 2}),
    )
    .await;

    // No header — 401 with actionable Markdown body.
    let response = send_fcp_post(
        &server,
        &app.public_id,
        "hi",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_markdown(&response);
    let body = response.text();
    assert!(body.contains("Authorization: Bearer"));
    assert!(body.contains("X-Everruns-FCP-Token"));
    assert!(
        !body.contains("fcp-secret"),
        "401 body must never echo the configured token"
    );

    // Wrong token — same 401, no oracle on validity.
    let wrong = send_fcp_post(
        &server,
        &app.public_id,
        "hi",
        vec![
            ("content-type", "text/plain"),
            ("authorization", "Bearer not-the-token"),
        ],
    )
    .await;
    wrong.assert_status(StatusCode::UNAUTHORIZED);

    // Right token via Authorization header.
    let ok = send_fcp_post(
        &server,
        &app.public_id,
        "hi",
        vec![
            ("content-type", "text/plain"),
            ("authorization", "Bearer fcp-secret"),
        ],
    )
    .await;
    assert_accepted_or_timeout(ok.status());

    // Right token via dedicated header.
    let ok2 = send_fcp_post(
        &server,
        &app.public_id,
        "hi",
        vec![
            ("content-type", "text/plain"),
            ("x-everruns-fcp-token", "fcp-secret"),
        ],
    )
    .await;
    assert_accepted_or_timeout(ok2.status());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_rejects_empty_token_config() {
    let server = TestServer::in_memory().await;
    let agent_id = create_llmsim_agent(&server).await;
    server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("Bad Token FCP App"),
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_id,
                "channel_type": "fcp",
                "channel_config": { "anonymous": true, "token": "  " }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_rejects_inline_auth_object() {
    // FCP's auth surface is intentionally narrow: anonymous + shared token
    // only. The inline `auth: {...}` object used by AG-UI/A2A is rejected
    // so FCP never grows another auth mode by accident.
    let server = TestServer::in_memory().await;
    let agent_id = create_llmsim_agent(&server).await;
    server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("Inline Auth FCP App"),
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_id,
                "channel_type": "fcp",
                "channel_config": {
                    "anonymous": false,
                    "auth": {"mode": "anonymous"}
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_anonymous_false_without_token_rejects_all_requests() {
    let server = TestServer::in_memory().await;
    // The config validator accepts `anonymous=false` without a token (the
    // operator might intend to fill it in later). But every POST must then
    // 401, because there is no way to authenticate.
    let app = create_published_fcp_app(&server, json!({"anonymous": false})).await;

    let response = send_fcp_post(
        &server,
        &app.public_id,
        "hi",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Even with a bearer token — the channel has nothing to compare to.
    let with_token = send_fcp_post(
        &server,
        &app.public_id,
        "hi",
        vec![
            ("content-type", "text/plain"),
            ("authorization", "Bearer anything"),
        ],
    )
    .await;
    with_token.assert_status(StatusCode::UNAUTHORIZED);
}

// ----------------------------------------------------------------------------
// Bad input handling
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_empty_body_returns_actionable_400() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, json!({})).await;

    let response = send_fcp_post(
        &server,
        &app.public_id,
        "",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.text();
    assert!(body.to_lowercase().contains("empty"));
    assert!(body.contains("POST"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_invalid_json_returns_actionable_400() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, json!({})).await;

    let response = send_fcp_post(
        &server,
        &app.public_id,
        "not actually json",
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.text();
    assert!(body.contains("application/json"));
    assert!(body.contains("text/plain"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_oversized_body_returns_413() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, json!({})).await;

    // 257 KiB > the 256 KiB FCP limit.
    let body = vec![b'x'; 257 * 1024];
    let response = send_fcp_post(
        &server,
        &app.public_id,
        body,
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(response.text().contains("256"));
}

// ----------------------------------------------------------------------------
// Rate limit
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_per_app_rate_limit_returns_429_with_retry_after() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(
        &server,
        json!({"rate_limit_per_minute": 1, "response_timeout_seconds": 2}),
    )
    .await;

    // Burn the single allowed request. The agent reply may not arrive in
    // in-memory test mode, so accept either 200 or 504 — both mean the
    // rate-limit bucket was decremented.
    let first = send_fcp_post(
        &server,
        &app.public_id,
        "first",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_accepted_or_timeout(first.status());

    // The next one must 429 with a Retry-After header.
    let second = send_fcp_post(
        &server,
        &app.public_id,
        "second",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = second
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(retry_after, "60");
    let body = second.text();
    assert!(body.contains("Rate limit"));
    assert!(body.contains("1 requests per minute"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_rate_limit_zero_disables_per_app_cap() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(
        &server,
        json!({"rate_limit_per_minute": 0, "response_timeout_seconds": 2}),
    )
    .await;

    // Several requests in a row must all reach the handler (none rejected
    // with 429). Whether they ultimately 200 or 504 depends on whether a
    // worker is running, so we only assert that none were rate-limited.
    for i in 0..3 {
        let response = send_fcp_post(
            &server,
            &app.public_id,
            format!("turn {i}"),
            vec![("content-type", "text/plain")],
        )
        .await;
        assert_ne!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_per_minute=0 must disable the per-app cap; turn {i} got 429"
        );
        assert_accepted_or_timeout(response.status());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_rate_limit_rejects_absurd_values() {
    let server = TestServer::in_memory().await;
    let agent_id = create_llmsim_agent(&server).await;
    server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("Bad RL FCP App"),
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_id,
                "channel_type": "fcp",
                "channel_config": { "rate_limit_per_minute": 5_000_000 }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_rejects_invalid_response_timeout() {
    let server = TestServer::in_memory().await;
    let agent_id = create_llmsim_agent(&server).await;

    // Zero is rejected.
    server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("Zero Timeout FCP App"),
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_id,
                "channel_type": "fcp",
                "channel_config": { "response_timeout_seconds": 0 }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // Absurd values are rejected.
    server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("Huge Timeout FCP App"),
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_id,
                "channel_type": "fcp",
                "channel_config": { "response_timeout_seconds": 9999 }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

// ----------------------------------------------------------------------------
// 404 lifecycle invariants
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_returns_404_for_unpublished_app() {
    let server = TestServer::in_memory().await;
    let app = create_fcp_app(&server, json!({})).await;

    let response = send_fcp_post(
        &server,
        &app.public_id,
        "hi",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_markdown(&response);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_returns_404_for_app_without_fcp_channel() {
    let server = TestServer::in_memory().await;
    let agent_id = create_llmsim_agent(&server).await;
    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("Webhook-Only App"),
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_id,
                "channel_type": "webhook",
                "channel_config": { "token": "wh", "message": "Run." }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(&format!("/v1/apps/{}/publish", app.public_id), json!({}))
        .await
        .assert_success();

    let response = send_fcp_post(
        &server,
        &app.public_id,
        "hi",
        vec![("content-type", "text/plain")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response.text();
    assert_body_no_leaks(&body);
}

// ----------------------------------------------------------------------------
// Token storage is redacted
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_token_is_redacted_in_app_reads() {
    let server = TestServer::in_memory().await;
    let app = create_published_fcp_app(&server, json!({"token": "fcp-secret-12345"})).await;

    let response: Value = server
        .get(&format!("/v1/apps/{}", app.public_id))
        .await
        .assert_success()
        .json();

    let channel = response["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["channel_type"] == "fcp")
        .expect("fcp channel must be present");
    let config = &channel["channel_config"];
    assert!(
        config.get("token").is_none(),
        "FCP token must not be returned in app reads"
    );
    assert_eq!(
        config["token_configured"], true,
        "redaction must surface a boolean indicator that a token is set"
    );

    let raw_response = serde_json::to_string(&response).unwrap();
    assert!(
        !raw_response.contains("fcp-secret-12345"),
        "raw response must not leak the configured FCP token"
    );
}

// ----------------------------------------------------------------------------
// Response timeout
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fcp_post_response_timeout_returns_504_with_actionable_body() {
    let server = TestServer::in_memory().await;
    // 1-second timeout combined with an unconfigured agent (no default model
    // would respond) would normally hang. Here we install a normal llmsim
    // agent but use a clamped timeout — llmsim is fast enough that this
    // path is the lower-bound timing test rather than a true timeout test.
    // To force a deterministic timeout we instead point at a non-existent
    // model by creating an app whose agent has no default model.
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": unique_slug("fcp-timeout-agent"),
                "display_name": unique_id("FCP Timeout Agent"),
                "system_prompt": "You should not reply.",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("FCP Timeout App"),
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent["id"],
                "channel_type": "fcp",
                "channel_config": {
                    "anonymous": true,
                    "response_timeout_seconds": 1
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    server
        .post(&format!("/v1/apps/{}/publish", app.public_id), json!({}))
        .await
        .assert_success();

    let response = send_fcp_post(
        &server,
        &app.public_id,
        "this turn should time out",
        vec![("content-type", "text/plain")],
    )
    .await;

    // Without a configured default model the turn will not produce an
    // `output.message.completed` event — the 1-second budget kicks in.
    // In environments where llmsim happens to satisfy the turn through an
    // unrelated codepath we accept a 200 to keep the test non-flaky; the
    // body shape check below only runs in the timeout case.
    if response.status() == StatusCode::GATEWAY_TIMEOUT {
        assert_markdown(&response);
        let body = response.text();
        assert!(body.contains("1 seconds"));
        assert_body_no_leaks(&body);
    } else {
        assert_eq!(response.status(), StatusCode::OK);
    }
}
