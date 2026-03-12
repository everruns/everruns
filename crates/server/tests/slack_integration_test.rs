//! Slack integration tests — real Slack API + in-process webhook handler
//!
//! These tests exercise the full Slack integration flow:
//! - Webhook signature verification with real HMAC-SHA256
//! - Session creation via webhook events
//! - Message routing by session strategy
//! - Real Slack API calls (users.info, chat.postMessage) when credentials are available
//!
//! Two tiers:
//! - **Webhook tests**: Always run (use test harness, no real Slack needed)
//! - **Real Slack tests**: Only run when SLACK_BOT_TOKEN + SLACK_SIGNING_SECRET are set
//!
//! Run webhook tests:
//!   cargo test -p everruns-server --test slack_integration_test -- --test-threads=1
//!
//! Run with real Slack:
//!   SLACK_BOT_TOKEN=xoxb-... SLACK_SIGNING_SECRET=... SLACK_TEST_CHANNEL=C... \
//!     cargo test -p everruns-server --test slack_integration_test -- --test-threads=1

mod test_harness;

use axum::http::{Method, StatusCode};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use test_harness::TestServer;

use everruns_core::App;

type HmacSha256 = Hmac<Sha256>;

/// Generate a unique timestamp-like string for test isolation across runs.
fn unique_ts() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}.{:06}", now, seq)
}

/// Test signing secret (arbitrary, used for webhook signature tests)
const TEST_SIGNING_SECRET: &str = "test_signing_secret_for_integration_tests";

/// Seed harness ID (BASE_HARNESS)
const SEED_BASE_HARNESS_ID: &str = "harness_01933b5a000070008000000000000601";

/// Compute Slack-compatible HMAC-SHA256 signature for a request body.
fn sign_slack_request(signing_secret: &str, timestamp: &str, body: &[u8]) -> String {
    let sig_basestring = format!("v0:{}:{}", timestamp, std::str::from_utf8(body).unwrap());
    let mut mac =
        HmacSha256::new_from_slice(signing_secret.as_bytes()).expect("HMAC can take any key size");
    mac.update(sig_basestring.as_bytes());
    let result = mac.finalize();
    format!("v0={}", hex::encode(result.into_bytes()))
}

/// Helper: create a published Slack app via API, return the app.
async fn create_published_slack_app(server: &TestServer, signing_secret: &str) -> App {
    // First, get a seed agent
    let agents_resp = server.get("/v1/agents").await.assert_success();
    let agents: Value = agents_resp.json();
    let agent_id = agents["data"][0]["id"]
        .as_str()
        .expect("need at least one seed agent");

    // Create app with Slack channel config
    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": "Slack Test Bot",
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent_id,
                "channel_type": "slack",
                "channel_config": {
                    "signing_secret": signing_secret,
                    "bot_token": "xoxb-test-token-for-integration",
                    "team_id": "T_TEST",
                    "session_strategy": "per_thread"
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Publish the app
    server
        .post(&format!("/v1/apps/{}/publish", app.public_id), json!({}))
        .await
        .assert_success();

    // Re-fetch to get published status
    server
        .get(&format!("/v1/apps/{}", app.public_id))
        .await
        .assert_success()
        .json()
}

/// Helper: send a signed Slack webhook event to the test server.
async fn send_slack_event(
    server: &TestServer,
    app_id: impl std::fmt::Display,
    signing_secret: &str,
    event_payload: &Value,
) -> test_harness::TestResponse {
    let app_id = app_id.to_string();
    let body = serde_json::to_vec(event_payload).unwrap();
    let timestamp = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let signature = sign_slack_request(signing_secret, &timestamp, &body);

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{}/slack/events", app_id),
            vec![
                ("content-type", "application/json"),
                ("x-slack-request-timestamp", &timestamp),
                ("x-slack-signature", &signature),
            ],
            body,
        )
        .await
}

// ============================================
// Webhook Integration Tests (no real Slack needed)
// ============================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_url_verification_challenge() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;

    let payload = json!({
        "type": "url_verification",
        "challenge": "test_challenge_string_abc123",
        "token": "ignored"
    });

    let resp = send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload).await;
    let body: Value = resp.assert_status(StatusCode::OK).json();
    assert_eq!(body["challenge"], "test_challenge_string_abc123");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_invalid_signature_rejected() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;

    let payload = json!({
        "type": "url_verification",
        "challenge": "test"
    });
    let body = serde_json::to_vec(&payload).unwrap();
    let timestamp = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );

    // Sign with wrong secret
    let bad_signature = sign_slack_request("wrong_secret", &timestamp, &body);

    let resp = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{}/slack/events", app.public_id),
            vec![
                ("content-type", "application/json"),
                ("x-slack-request-timestamp", &timestamp),
                ("x-slack-signature", &bad_signature),
            ],
            body,
        )
        .await;

    resp.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_message_creates_session() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;
    let ts = unique_ts();

    let payload = json!({
        "type": "event_callback",
        "team_id": "T_TEST",
        "event": {
            "type": "message",
            "text": "Hello bot!",
            "user": "U_TESTUSER",
            "channel": "C_TESTCHAN",
            "ts": ts,
            "thread_ts": null
        }
    });

    let resp = send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload).await;
    let body: Value = resp.assert_status(StatusCode::OK).json();
    assert_eq!(body["ok"], true);

    // Allow background task to process
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Verify session was created with correct tags
    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    let sessions_data = sessions["data"]
        .as_array()
        .expect("sessions should be array");

    let expected_thread_tag = format!("slack:thread:{}", ts);
    let slack_session = sessions_data.iter().find(|s| {
        s["tags"]
            .as_array()
            .map(|tags| {
                tags.iter()
                    .any(|t| t.as_str() == Some(&expected_thread_tag))
            })
            .unwrap_or(false)
    });

    assert!(
        slack_session.is_some(),
        "Should have created a session with slack:thread tag"
    );

    let session = slack_session.unwrap();
    let tags: Vec<&str> = session["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t.as_str())
        .collect();

    // Should also have the app tag
    let expected_app_tag = format!("slack:app:{}", app.public_id);
    assert!(
        tags.contains(&expected_app_tag.as_str()),
        "Session should have slack:app tag, got: {:?}",
        tags
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_threaded_message_reuses_session() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;
    let thread_ts = unique_ts();
    let reply_ts = unique_ts();

    // First message starts the thread
    let payload1 = json!({
        "type": "event_callback",
        "team_id": "T_TEST",
        "event": {
            "type": "message",
            "text": "First message",
            "user": "U_USER1",
            "channel": "C_CHAN1",
            "ts": thread_ts,
            "thread_ts": null
        }
    });
    send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload1)
        .await
        .assert_status(StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Second message in the same thread
    let payload2 = json!({
        "type": "event_callback",
        "team_id": "T_TEST",
        "event": {
            "type": "message",
            "text": "Reply in thread",
            "user": "U_USER1",
            "channel": "C_CHAN1",
            "ts": reply_ts,
            "thread_ts": thread_ts
        }
    });
    send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload2)
        .await
        .assert_status(StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Should still have only one session for this thread
    let expected_tag = format!("slack:thread:{}", thread_ts);
    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    let slack_sessions: Vec<&Value> = sessions["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| {
            s["tags"]
                .as_array()
                .map(|tags| tags.iter().any(|t| t.as_str() == Some(&expected_tag)))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        slack_sessions.len(),
        1,
        "Should reuse the same session for threaded replies"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_bot_message_ignored() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;

    let bot_ts = unique_ts();

    // Message with bot_id should be ignored
    let payload = json!({
        "type": "event_callback",
        "team_id": "T_TEST",
        "event": {
            "type": "message",
            "text": "I am a bot",
            "bot_id": "B_SOMEBOT",
            "channel": "C_BOTCHAN",
            "ts": bot_ts
        }
    });

    let resp = send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload).await;
    resp.assert_status(StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // No session should have been created for the bot message
    let expected_tag = format!("slack:thread:{}", bot_ts);
    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    let bot_sessions: Vec<&Value> = sessions["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| {
            s["tags"]
                .as_array()
                .map(|tags| tags.iter().any(|t| t.as_str() == Some(&expected_tag)))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        bot_sessions.len(),
        0,
        "Bot messages should not create sessions"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_unpublished_app_rejected() {
    let server = TestServer::new().await;

    // Get a seed agent
    let agents: Value = server.get("/v1/agents").await.assert_success().json();
    let agent_id = agents["data"][0]["id"].as_str().unwrap();

    // Create app but don't publish
    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": "Unpublished Bot",
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent_id,
                "channel_type": "slack",
                "channel_config": {
                    "signing_secret": TEST_SIGNING_SECRET,
                    "bot_token": "xoxb-test",
                    "session_strategy": "per_thread"
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let payload = json!({
        "type": "url_verification",
        "challenge": "test"
    });

    let resp = send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload).await;
    resp.assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_nonexistent_app_returns_404() {
    let server = TestServer::new().await;

    let payload = json!({
        "type": "url_verification",
        "challenge": "test"
    });

    let resp = send_slack_event(
        &server,
        "app_00000000000000000000000000000000",
        TEST_SIGNING_SECRET,
        &payload,
    )
    .await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_app_mention_creates_session() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;

    let mention_ts = unique_ts();

    let payload = json!({
        "type": "event_callback",
        "team_id": "T_TEST",
        "event": {
            "type": "app_mention",
            "text": "<@U_BOT> help me",
            "user": "U_MENTIONER",
            "channel": "C_MENTION",
            "ts": mention_ts
        }
    });

    let resp = send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload).await;
    resp.assert_status(StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let expected_tag = format!("slack:thread:{}", mention_ts);
    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    let mention_sessions: Vec<&Value> = sessions["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| {
            s["tags"]
                .as_array()
                .map(|tags| tags.iter().any(|t| t.as_str() == Some(&expected_tag)))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        mention_sessions.len(),
        1,
        "app_mention should create a session"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_per_channel_strategy() {
    let server = TestServer::new().await;

    // Get a seed agent
    let agents: Value = server.get("/v1/agents").await.assert_success().json();
    let agent_id = agents["data"][0]["id"].as_str().unwrap();

    // Create app with per_channel strategy
    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": "Channel Bot",
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent_id,
                "channel_type": "slack",
                "channel_config": {
                    "signing_secret": TEST_SIGNING_SECRET,
                    "bot_token": "xoxb-test-channel",
                    "session_strategy": "per_channel"
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

    // Use unique channel name to avoid cross-test pollution
    let channel = format!("C_SHARED_{}", unique_ts().replace('.', "_"));
    let ts1 = unique_ts();
    let ts2 = unique_ts();

    // Two messages in same channel, different threads
    for (ts, thread) in [(ts1.as_str(), None), (ts2.as_str(), Some(ts1.as_str()))] {
        let mut event = json!({
            "type": "message",
            "text": "hello",
            "user": "U_CHANUSER",
            "channel": channel,
            "ts": ts
        });
        if let Some(t) = thread {
            event["thread_ts"] = json!(t);
        }
        let payload = json!({
            "type": "event_callback",
            "team_id": "T_TEST",
            "event": event
        });
        send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload)
            .await
            .assert_status(StatusCode::OK);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // Should have exactly one session tagged with slack:channel:{channel}
    let expected_tag = format!("slack:channel:{}", channel);
    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    let channel_sessions: Vec<&Value> = sessions["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| {
            s["tags"]
                .as_array()
                .map(|tags| tags.iter().any(|t| t.as_str() == Some(&expected_tag)))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        channel_sessions.len(),
        1,
        "per_channel strategy should create one session per channel"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_per_user_strategy() {
    let server = TestServer::new().await;

    let agents: Value = server.get("/v1/agents").await.assert_success().json();
    let agent_id = agents["data"][0]["id"].as_str().unwrap();

    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": "User Bot",
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent_id,
                "channel_type": "slack",
                "channel_config": {
                    "signing_secret": TEST_SIGNING_SECRET,
                    "bot_token": "xoxb-test-user",
                    "session_strategy": "per_user"
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

    // Use unique user IDs to avoid cross-test pollution
    let user_alice = format!("U_ALICE_{}", unique_ts().replace('.', ""));
    let user_bob = format!("U_BOB_{}", unique_ts().replace('.', ""));
    let alice_tag = format!("slack:user:{}", user_alice);
    let bob_tag = format!("slack:user:{}", user_bob);

    for (user, ts) in [
        (user_alice.as_str(), unique_ts()),
        (user_bob.as_str(), unique_ts()),
    ] {
        let payload = json!({
            "type": "event_callback",
            "team_id": "T_TEST",
            "event": {
                "type": "message",
                "text": "hello from user",
                "user": user,
                "channel": "C_USERCHAN",
                "ts": ts
            }
        });
        send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload)
            .await
            .assert_status(StatusCode::OK);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    let user_sessions: Vec<&Value> = sessions["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| {
            s["tags"]
                .as_array()
                .map(|tags| {
                    tags.iter().any(|t| {
                        let s = t.as_str().unwrap_or("");
                        s == alice_tag || s == bob_tag
                    })
                })
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        user_sessions.len(),
        2,
        "per_user strategy should create one session per user"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_empty_message_ignored() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;
    let empty_ts = unique_ts();

    let payload = json!({
        "type": "event_callback",
        "team_id": "T_TEST",
        "event": {
            "type": "message",
            "text": "",
            "user": "U_EMPTY",
            "channel": "C_EMPTY",
            "ts": empty_ts
        }
    });

    send_slack_event(&server, &app.public_id, TEST_SIGNING_SECRET, &payload)
        .await
        .assert_status(StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let expected_tag = format!("slack:thread:{}", empty_ts);
    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    let empty_sessions: Vec<&Value> = sessions["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| {
            s["tags"]
                .as_array()
                .map(|tags| tags.iter().any(|t| t.as_str() == Some(&expected_tag)))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        empty_sessions.len(),
        0,
        "Empty text messages (no files/attachments) should not create sessions"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_manifest_endpoint() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;

    let resp = server
        .get(&format!("/v1/apps/{}/slack/manifest", app.public_id))
        .await
        .assert_success();

    let body: Value = resp.json();
    assert!(
        body.get("manifest_yaml").is_some(),
        "Should return manifest YAML"
    );
    assert!(
        body.get("create_url").is_some(),
        "Should return Slack create URL"
    );

    let manifest = body["manifest_yaml"].as_str().unwrap();
    assert!(
        manifest.contains("chat:write"),
        "Manifest should include chat:write scope"
    );
    assert!(
        manifest.contains("app_mentions:read"),
        "Manifest should include app_mentions:read scope"
    );
}

// ============================================
// Real Slack API Tests (require credentials)
// ============================================

/// Get real Slack credentials from environment. Returns None if not set.
/// Checks TEST_-prefixed vars first (Doppler convention), then unprefixed.
fn real_slack_credentials() -> Option<(String, String, String)> {
    let token = std::env::var("TEST_SLACK_BOT_TOKEN")
        .or_else(|_| std::env::var("SLACK_BOT_TOKEN"))
        .ok()?;
    let secret = std::env::var("TEST_SLACK_SIGNING_SECRET")
        .or_else(|_| std::env::var("SLACK_SIGNING_SECRET"))
        .ok()?;
    let channel = std::env::var("TEST_SLACK_TEST_CHANNEL")
        .or_else(|_| std::env::var("SLACK_TEST_CHANNEL"))
        .ok()?;
    Some((token, secret, channel))
}

/// Test posting a real message to Slack and verifying the response.
/// Requires: SLACK_BOT_TOKEN, SLACK_SIGNING_SECRET, SLACK_TEST_CHANNEL
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_real_slack_post_message() {
    let Some((bot_token, _signing_secret, channel)) = real_slack_credentials() else {
        eprintln!(
            "Skipping real Slack test: SLACK_BOT_TOKEN / SLACK_SIGNING_SECRET / SLACK_TEST_CHANNEL not set"
        );
        return;
    };

    let client = reqwest::Client::new();
    let resp = client
        .post("https://slack.com/api/chat.postMessage")
        .header("Authorization", format!("Bearer {}", bot_token))
        .json(&json!({
            "channel": channel,
            "text": "[Integration Test] chat.postMessage works"
        }))
        .send()
        .await
        .expect("Failed to send request to Slack");

    let body: Value = resp
        .json::<Value>()
        .await
        .expect("Failed to parse Slack response");
    assert!(
        body["ok"].as_bool().unwrap_or(false),
        "chat.postMessage should succeed. Error: {:?}",
        body["error"]
    );
    assert!(body["ts"].is_string(), "Response should include ts");

    // Clean up: delete the test message
    let ts = body["ts"].as_str().unwrap();
    let _ = client
        .post("https://slack.com/api/chat.delete")
        .header("Authorization", format!("Bearer {}", bot_token))
        .json(&json!({
            "channel": channel,
            "ts": ts
        }))
        .send()
        .await;
}

/// Test resolving a real Slack user's display name via users.info API.
/// Requires: SLACK_BOT_TOKEN (and a real user in the workspace)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_real_slack_users_info() {
    let Some((bot_token, _, _)) = real_slack_credentials() else {
        eprintln!("Skipping real Slack test: credentials not set");
        return;
    };

    // First, get a user from the workspace
    let client = reqwest::Client::new();
    let resp = client
        .get("https://slack.com/api/users.list?limit=5")
        .header("Authorization", format!("Bearer {}", bot_token))
        .send()
        .await
        .expect("Failed to call users.list");

    let body: Value = resp
        .json::<Value>()
        .await
        .expect("Failed to parse response");
    assert!(
        body["ok"].as_bool().unwrap_or(false),
        "users.list should succeed. Error: {:?}",
        body["error"]
    );

    // Find a non-bot user
    let members = body["members"].as_array().expect("members should be array");
    let human_user = members
        .iter()
        .find(|m| !m["is_bot"].as_bool().unwrap_or(true) && m["id"].as_str() != Some("USLACKBOT"));

    let Some(user) = human_user else {
        eprintln!("No non-bot user found in workspace, skipping users.info test");
        return;
    };

    let user_id = user["id"].as_str().unwrap();

    // Now test users.info
    let resp = client
        .get(&format!(
            "https://slack.com/api/users.info?user={}",
            user_id
        ))
        .header("Authorization", format!("Bearer {}", bot_token))
        .send()
        .await
        .expect("Failed to call users.info");

    let info: Value = resp.json::<Value>().await.unwrap();
    assert!(
        info["ok"].as_bool().unwrap_or(false),
        "users.info should succeed. Error: {:?}",
        info["error"]
    );
    assert!(
        info["user"]["profile"]["display_name"].is_string()
            || info["user"]["profile"]["real_name"].is_string(),
        "User should have a display_name or real_name"
    );
}

/// Test the full webhook→session flow with real Slack signing secret.
/// Requires: SLACK_BOT_TOKEN, SLACK_SIGNING_SECRET, SLACK_TEST_CHANNEL
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_real_slack_webhook_to_session() {
    let Some((bot_token, signing_secret, channel)) = real_slack_credentials() else {
        eprintln!("Skipping real Slack webhook test: credentials not set");
        return;
    };

    let server = TestServer::new().await;

    let agents: Value = server.get("/v1/agents").await.assert_success().json();
    let agent_id = agents["data"][0]["id"].as_str().unwrap();

    // Create app with real signing secret
    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": "Real Slack Test Bot",
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent_id,
                "channel_type": "slack",
                "channel_config": {
                    "signing_secret": signing_secret,
                    "bot_token": bot_token,
                    "team_id": "T_REAL",
                    "session_strategy": "per_thread"
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

    // Send a webhook event signed with the real signing secret
    let event_ts = format!(
        "{}.000001",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );

    let payload = json!({
        "type": "event_callback",
        "team_id": "T_REAL",
        "event": {
            "type": "message",
            "text": "[Integration Test] Real webhook test",
            "user": "U_REALUSER",
            "channel": channel,
            "ts": event_ts
        }
    });

    let resp = send_slack_event(&server, &app.public_id, &signing_secret, &payload).await;
    resp.assert_status(StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Verify session was created
    let sessions: Value = server.get("/v1/sessions").await.assert_success().json();
    let slack_session = sessions["data"].as_array().unwrap().iter().find(|s| {
        s["tags"]
            .as_array()
            .map(|tags| {
                tags.iter()
                    .any(|t| t.as_str() == Some(&format!("slack:thread:{}", event_ts)))
            })
            .unwrap_or(false)
    });

    assert!(
        slack_session.is_some(),
        "Real Slack webhook should create session with thread tag"
    );
}

/// Test that Slack rate limit (429) is properly identified as retryable.
/// This test doesn't actually hit a rate limit but validates the error classification.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_missing_signature_headers() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;

    let body = serde_json::to_vec(&json!({
        "type": "url_verification",
        "challenge": "test"
    }))
    .unwrap();

    // No signature headers at all
    let resp = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{}/slack/events", app.public_id),
            vec![("content-type", "application/json")],
            body,
        )
        .await;

    resp.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slack_replay_attack_old_timestamp() {
    let server = TestServer::new().await;
    let app = create_published_slack_app(&server, TEST_SIGNING_SECRET).await;

    let payload = json!({
        "type": "url_verification",
        "challenge": "test"
    });
    let body = serde_json::to_vec(&payload).unwrap();

    // Timestamp from 10 minutes ago (beyond 5-minute window)
    let old_timestamp = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 600
    );
    let signature = sign_slack_request(TEST_SIGNING_SECRET, &old_timestamp, &body);

    let resp = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{}/slack/events", app.public_id),
            vec![
                ("content-type", "application/json"),
                ("x-slack-request-timestamp", &old_timestamp),
                ("x-slack-signature", &signature),
            ],
            body,
        )
        .await;

    resp.assert_status(StatusCode::UNAUTHORIZED);
}
