//! Tests: signature.

use super::*;
use crate::slack_delivery::SlackSurface;
use crate::storage::StorageBackend;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use everruns_core::channel::{InboundAttachment, SessionBinding};
use everruns_platform::{SlackChannelConfig, SlackReplyMode};
use everruns_worker::AgentRunner;
use std::sync::Arc;

use super::tests_support::*;

#[test]
fn test_verify_slack_signature_valid() {
    let secret = "test_signing_secret";
    let body = r#"{"type":"url_verification","challenge":"abc123"}"#;
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let signature = make_signature(secret, &timestamp, body);

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Slack-Request-Timestamp",
        HeaderValue::from_str(&timestamp).unwrap(),
    );
    headers.insert(
        "X-Slack-Signature",
        HeaderValue::from_str(&signature).unwrap(),
    );

    assert!(verify_slack_signature(&headers, body.as_bytes(), secret).is_ok());
}

#[test]
fn test_verify_slack_signature_invalid() {
    let secret = "test_signing_secret";
    let body = r#"{"type":"url_verification","challenge":"abc123"}"#;
    let timestamp = chrono::Utc::now().timestamp().to_string();

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Slack-Request-Timestamp",
        HeaderValue::from_str(&timestamp).unwrap(),
    );
    headers.insert("X-Slack-Signature", HeaderValue::from_static("v0=deadbeef"));

    assert!(verify_slack_signature(&headers, body.as_bytes(), secret).is_err());
}

#[test]
fn test_verify_slack_signature_rejects_empty_secret() {
    let body = "body";
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let signature = make_signature("", &timestamp, body);

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Slack-Request-Timestamp",
        HeaderValue::from_str(&timestamp).unwrap(),
    );
    headers.insert(
        "X-Slack-Signature",
        HeaderValue::from_str(&signature).unwrap(),
    );

    assert!(verify_slack_signature(&headers, body.as_bytes(), "").is_err());
    assert!(verify_slack_signature(&headers, body.as_bytes(), "  ").is_err());
}

#[test]
fn test_verify_slack_signature_missing_headers() {
    let headers = HeaderMap::new();
    assert!(verify_slack_signature(&headers, b"body", "secret").is_err());
}

#[test]
fn test_verify_slack_signature_old_timestamp() {
    let secret = "test_signing_secret";
    let body = "body";
    let old_timestamp = (chrono::Utc::now().timestamp() - 600).to_string();
    let signature = make_signature(secret, &old_timestamp, body);

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Slack-Request-Timestamp",
        HeaderValue::from_str(&old_timestamp).unwrap(),
    );
    headers.insert(
        "X-Slack-Signature",
        HeaderValue::from_str(&signature).unwrap(),
    );

    assert!(verify_slack_signature(&headers, body.as_bytes(), secret).is_err());
}

#[test]
fn test_slack_event_envelope_url_verification() {
    let json = r#"{"type":"url_verification","challenge":"test_challenge_123"}"#;
    let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
    assert_eq!(envelope.event_type, "url_verification");
    assert_eq!(envelope.challenge.unwrap(), "test_challenge_123");
}

#[test]
fn test_slack_event_envelope_event_callback() {
    let json = r#"{
        "type": "event_callback",
        "team_id": "T0123456789",
        "event": {
            "type": "message",
            "user": "U0123456789",
            "text": "Hello bot",
            "channel": "C0123456789",
            "ts": "1234567890.123456",
            "thread_ts": "1234567890.000000"
        }
    }"#;
    let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
    assert_eq!(envelope.event_type, "event_callback");
    let event = envelope.event.unwrap();
    assert_eq!(event.event_type, "message");
    assert_eq!(event.user.unwrap(), "U0123456789");
    assert_eq!(event.text.unwrap(), "Hello bot");
    assert_eq!(event.channel.unwrap(), "C0123456789");
    assert!(event.thread_ts.is_some());
}

#[test]
fn test_slack_event_bot_message_detected() {
    let json = r#"{
        "type": "event_callback",
        "event": {
            "type": "message",
            "bot_id": "B0123456789",
            "text": "I am a bot",
            "channel": "C0123456789",
            "ts": "1234567890.123456"
        }
    }"#;
    let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
    let event = envelope.event.unwrap();
    assert!(event.bot_id.is_some());
}

// Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

#[test]
fn test_slack_channel_config_defaults() {
    let json = r#"{"signing_secret": "sec", "bot_token": "tok"}"#;
    let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.session_strategy, SessionBinding::Thread);
    assert_eq!(config.reply_mode, SlackReplyMode::AllMessages);
    assert!(config.channel_id.is_none());
    assert!(config.team_id.is_none());
}

#[test]
fn test_event_matches_slack_scope_unrestricted() {
    let config = test_config(SessionBinding::Thread);
    let event = test_event("C123", Some("1234.5678"), Some("1234.0000"));
    assert!(event_matches_slack_scope(&config, Some("T123"), &event));
}

#[test]
fn test_event_matches_slack_scope_rejects_team_mismatch() {
    let mut config = test_config(SessionBinding::Thread);
    config.team_id = Some("T123".to_string());
    let event = test_event("C123", Some("1234.5678"), Some("1234.0000"));
    assert!(!event_matches_slack_scope(&config, Some("T999"), &event));
    assert!(!event_matches_slack_scope(&config, None, &event));
}

#[test]
fn test_event_matches_slack_scope_rejects_channel_mismatch() {
    let mut config = test_config(SessionBinding::Thread);
    config.channel_id = Some("C123".to_string());
    let event = test_event("C999", Some("1234.5678"), Some("1234.0000"));
    assert!(!event_matches_slack_scope(&config, Some("T123"), &event));
}

#[test]
fn test_event_matches_slack_scope_accepts_matching_team_and_channel() {
    let mut config = test_config(SessionBinding::Thread);
    config.team_id = Some("T123".to_string());
    config.channel_id = Some("C123".to_string());
    let event = test_event("C123", Some("1234.5678"), Some("1234.0000"));
    assert!(event_matches_slack_scope(&config, Some("T123"), &event));
}
#[test]
fn test_supported_slack_message_subtypes() {
    for subtype in [None, Some("file_share"), Some("thread_broadcast")] {
        assert!(
            is_supported_slack_message_subtype(subtype),
            "expected {subtype:?} to be supported"
        );
    }
}

#[test]
fn test_unsupported_slack_message_subtypes() {
    for subtype in [
        "bot_message",
        "channel_join",
        "channel_leave",
        "message_changed",
        "message_deleted",
        "channel_topic",
        "unknown_future_subtype",
    ] {
        assert!(
            !is_supported_slack_message_subtype(Some(subtype)),
            "expected {subtype} to be unsupported"
        );
    }
}

#[test]
fn test_build_session_tags_per_thread() {
    let app = test_app();
    let config = test_config(SessionBinding::Thread);
    let event = test_event("C123", Some("1234.5678"), Some("1234.0000"));

    let tags = build_session_tags(
        &app,
        &app.channels[0],
        &config,
        &event,
        SlackSurface::Channel,
    );
    assert_eq!(tags.len(), 3);
    assert!(tags[0].starts_with("slack:app:"));
    assert!(tags[1].starts_with("slack:endpoint:"));
    assert_eq!(tags[2], "slack:thread:1234.0000"); // uses thread_ts
}

#[test]
fn test_build_session_tags_per_thread_no_thread_ts() {
    let app = test_app();
    let config = test_config(SessionBinding::Thread);
    let event = test_event("C123", Some("1234.5678"), None);

    let tags = build_session_tags(
        &app,
        &app.channels[0],
        &config,
        &event,
        SlackSurface::Channel,
    );
    assert_eq!(tags[2], "slack:thread:1234.5678"); // falls back to ts
}

#[test]
fn test_build_session_tags_per_channel() {
    let app = test_app();
    let config = test_config(SessionBinding::Conversation);
    let event = test_event("C123", Some("1234.5678"), None);

    let tags = build_session_tags(
        &app,
        &app.channels[0],
        &config,
        &event,
        SlackSurface::Channel,
    );
    assert_eq!(tags[2], "slack:channel:C123");
}

#[test]
fn test_build_session_tags_per_user() {
    let app = test_app();
    let config = test_config(SessionBinding::Requester);
    let mut event = test_event("C123", Some("1234.5678"), None);
    event.user = Some("U999".to_string());

    let tags = build_session_tags(
        &app,
        &app.channels[0],
        &config,
        &event,
        SlackSurface::Channel,
    );
    assert_eq!(tags[2], "slack:user:U999");
}

#[test]
fn test_desired_session_tags_adds_reply_mode_for_report_progress_only() {
    let tags = desired_session_tags(
        &[
            "slack:app:app_123".to_string(),
            "slack:thread:1234.0000".to_string(),
        ],
        SlackReplyMode::ReportProgressOnly,
    );

    assert_eq!(
        tags,
        vec![
            "slack:app:app_123".to_string(),
            "slack:thread:1234.0000".to_string(),
            "slack:reply_mode:report_progress_only".to_string(),
            "channel:reply_mode:report_progress_only".to_string(),
        ]
    );
}

#[test]
fn test_extract_response_text() {
    let data = serde_json::json!({
        "message": {
            "content": [
                {"type": "text", "text": "Hello from the agent!"},
                {"type": "text", "text": "More text."}
            ]
        }
    });
    let text = extract_response_text(&data);
    assert_eq!(text.unwrap(), "Hello from the agent!\nMore text.");
}

#[test]
fn test_extract_response_text_no_message() {
    let data = serde_json::json!({});
    assert!(extract_response_text(&data).is_none());
}

#[test]
fn test_extract_response_text_empty_content() {
    let data = serde_json::json!({"message": {"content": []}});
    assert!(extract_response_text(&data).is_none());
}

#[tokio::test]
async fn test_resolve_slack_user_name_cache_hit() {
    let cache: SlackUserCache = new_slack_user_cache();
    cache.insert("U123".to_string(), Some("Alice".to_string()));

    let result = resolve_slack_user_name(&cache, "xoxb-fake", "U123").await;
    assert_eq!(result, Some("Alice".to_string()));
}

#[tokio::test]
async fn test_resolve_slack_user_name_cache_hit_none() {
    // Cached failure (e.g., missing_scope) should return None without API call
    let cache: SlackUserCache = new_slack_user_cache();
    cache.insert("U123".to_string(), None);

    let result = resolve_slack_user_name(&cache, "xoxb-fake", "U123").await;
    assert_eq!(result, None);
}

#[tokio::test]
async fn transport_failure_does_not_cache_user_lookup() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        // Close after receiving the request, without a response.
    });
    let cache = new_slack_user_cache();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        resolve_slack_user_name_base(&format!("http://{address}"), &cache, "test-token", "U999"),
    )
    .await
    .expect("lookup must terminate");
    server.await.unwrap();
    assert_eq!(result, None);
    assert!(cache.get("U999").is_none());

    let retry_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/users.info"))
        .and(wiremock::matchers::query_param("user", "U999"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "user": {"profile": {"display_name": "Recovered User"}}
            })),
        )
        .expect(1)
        .mount(&retry_server)
        .await;
    let recovered =
        resolve_slack_user_name_base(&retry_server.uri(), &cache, "test-token", "U999").await;
    assert_eq!(recovered.as_deref(), Some("Recovered User"));
    assert_eq!(cache.get("U999"), Some(recovered));
}

#[test]
fn test_external_actor_display_label_with_name() {
    let actor = everruns_core::ExternalActor {
        actor_id: "U123".to_string(),
        actor_name: Some("Alice".to_string()),
        source: "slack".to_string(),
        metadata: None,
    };
    assert_eq!(actor.display_label(), "Alice");
}

#[test]
fn test_external_actor_display_label_fallback_to_id() {
    let actor = everruns_core::ExternalActor {
        actor_id: "U0123456789".to_string(),
        actor_name: None,
        source: "slack".to_string(),
        metadata: None,
    };
    assert_eq!(actor.display_label(), "U0123456789");
}

#[test]
fn test_external_actor_with_metadata() {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("channel".to_string(), "C123".to_string());
    metadata.insert("team_id".to_string(), "T456".to_string());

    let actor = everruns_core::ExternalActor {
        actor_id: "U123".to_string(),
        actor_name: Some("Alice".to_string()),
        source: "slack".to_string(),
        metadata: Some(metadata),
    };

    assert_eq!(actor.source, "slack");
    let meta = actor.metadata.as_ref().unwrap();
    assert_eq!(meta.get("channel").unwrap(), "C123");
    assert_eq!(meta.get("team_id").unwrap(), "T456");
}

// Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

/// The agent pane is inherently one thread, so `per_channel`/`per_user` have
/// no meaning there — but the same app still honours them in channels, which
/// is why the combination is resolved at runtime instead of rejected in config.
#[test]
fn test_pane_forces_per_thread_routing() {
    let app = test_app();
    let mut config = test_config(SessionBinding::Conversation);
    config.agent_surface_enabled = true;
    let event = test_event("D_PANE", Some("1234.5678"), None);

    let pane_tags = build_session_tags(&app, &app.channels[0], &config, &event, SlackSurface::Pane);
    assert!(
        pane_tags.iter().any(|t| t == "slack:thread:1234.5678"),
        "pane must route per thread, got {pane_tags:?}"
    );
    assert!(
        !pane_tags.iter().any(|t| t.starts_with("slack:channel:")),
        "pane must not route per channel, got {pane_tags:?}"
    );

    // Same config, channel surface: per_channel still means per_channel.
    let channel_tags = build_session_tags(
        &app,
        &app.channels[0],
        &config,
        &event,
        SlackSurface::Channel,
    );
    assert!(
        channel_tags.iter().any(|t| t == "slack:channel:D_PANE"),
        "channel surface must keep the configured strategy, got {channel_tags:?}"
    );
}

#[test]
fn slack_message_metadata_includes_system_app_id() {
    let app = test_app();
    let event = test_event("C123", Some("1234.5678"), None);

    let metadata = slack_message_metadata(&app, &app.channels[0], &event, None);

    assert_eq!(
        metadata.get("_app_id"),
        Some(&serde_json::Value::String(app.public_id.to_string()))
    );
    assert_eq!(
        metadata.get("_app_channel_id"),
        Some(&serde_json::Value::String(
            app.channels[0].public_id.to_string()
        ))
    );
    assert_eq!(
        metadata.get("slack_ts"),
        Some(&serde_json::Value::String("1234.5678".to_string()))
    );
}

// ==========================================
// InboundChannelEvent parsing (channel abstractions)
// ==========================================

#[test]
fn test_parse_slack_inbound_event_basic() {
    let config = test_config(SessionBinding::Thread);
    let mut event = test_event("C123", Some("1234.5678"), Some("1234.0000"));
    event.user = Some("U001".to_string());

    let inbound = parse_slack_inbound_event(&event, &config, Some("Alice".to_string()));

    assert_eq!(inbound.actor.actor_id, "U001");
    assert_eq!(inbound.actor.actor_name.as_deref(), Some("Alice"));
    assert_eq!(inbound.actor.source, "slack");
    assert_eq!(inbound.text, "Hello");
    assert_eq!(inbound.dedup_key, "1234.5678");
    assert_eq!(inbound.thread_ref.as_deref(), Some("1234.0000"));
    assert_eq!(
        inbound.routing_metadata.get("thread_ref").unwrap(),
        "1234.0000"
    );
    assert_eq!(inbound.routing_metadata.get("channel_id").unwrap(), "C123");
    assert_eq!(inbound.routing_metadata.get("user_id").unwrap(), "U001");
}

#[test]
fn test_parse_slack_inbound_event_with_image_files() {
    let config = test_config(SessionBinding::Thread);
    let mut event = test_event("C123", Some("1234.5678"), None);
    event.files = vec![test_slack_file(
        "photo.png",
        "image/png",
        "png",
        Some("https://files.slack.com/photo.png"),
    )];

    let inbound = parse_slack_inbound_event(&event, &config, None);

    assert_eq!(inbound.attachments.len(), 1);
    match &inbound.attachments[0] {
        InboundAttachment::Image { url, alt_text } => {
            assert_eq!(url, "https://files.slack.com/photo.png");
            assert_eq!(alt_text.as_deref(), Some("photo.png"));
        }
        other => panic!("Expected Image attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_slack_inbound_event_with_non_image_file() {
    let config = test_config(SessionBinding::Thread);
    let mut event = test_event("C123", Some("1234.5678"), None);
    event.files = vec![test_slack_file("doc.pdf", "application/pdf", "pdf", None)];

    let inbound = parse_slack_inbound_event(&event, &config, None);

    assert_eq!(inbound.attachments.len(), 1);
    match &inbound.attachments[0] {
        InboundAttachment::FileDescription { name, mime_type } => {
            assert_eq!(name, "doc.pdf");
            assert_eq!(mime_type.as_deref(), Some("application/pdf"));
        }
        other => panic!("Expected FileDescription, got {:?}", other),
    }
}

#[test]
fn test_parse_slack_inbound_event_routing_metadata_no_thread_ts() {
    let config = test_config(SessionBinding::Thread);
    let event = test_event("C123", Some("1234.5678"), None);

    let inbound = parse_slack_inbound_event(&event, &config, None);

    // Without thread_ts, thread_ref falls back to ts
    assert_eq!(inbound.thread_ref.as_deref(), Some("1234.5678"));
    assert_eq!(
        inbound.routing_metadata.get("thread_ref").unwrap(),
        "1234.5678"
    );
}

#[test]
fn test_build_session_tags_uses_generic_routing() {
    // Verify that build_session_tags produces the same tags via
    // build_session_routing_tag() as the old hand-rolled implementation
    let app = test_app();
    let config = test_config(SessionBinding::Thread);
    let event = test_event("C123", Some("1234.5678"), Some("1234.0000"));

    let tags = build_session_tags(
        &app,
        &app.channels[0],
        &config,
        &event,
        SlackSurface::Channel,
    );
    assert_eq!(tags[2], "slack:thread:1234.0000");

    let config_channel = test_config(SessionBinding::Conversation);
    let tags_channel = build_session_tags(
        &app,
        &app.channels[0],
        &config_channel,
        &event,
        SlackSurface::Channel,
    );
    assert_eq!(tags_channel[2], "slack:channel:C123");

    let mut event_user = test_event("C123", Some("1234.5678"), None);
    event_user.user = Some("U999".to_string());
    let config_user = test_config(SessionBinding::Requester);
    let tags_user = build_session_tags(
        &app,
        &app.channels[0],
        &config_user,
        &event_user,
        SlackSurface::Channel,
    );
    assert_eq!(tags_user[2], "slack:user:U999");
}

// ==========================================
// DB-level dedup via has_event_with_slack_ts
// ==========================================

#[tokio::test]
async fn test_has_event_with_slack_ts_no_match() {
    let db = StorageBackend::in_memory();
    let session_id = setup_test_session(&db).await;
    let result = db
        .has_event_with_slack_ts(session_id, "1234567890.123456")
        .await
        .unwrap();
    assert!(!result);
}

#[tokio::test]
async fn test_has_event_with_slack_ts_match() {
    use crate::storage::models::CreateEventRow;

    let db = StorageBackend::in_memory();
    let session_id = setup_test_session(&db).await;

    // Insert an input.message event with slack_ts in metadata
    let event = CreateEventRow {
        session_id,
        event_type: "input.message".to_string(),
        ts: chrono::Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({
            "message": {
                "id": "message_00000000000000000000000000000001",
                "role": "user",
                "content": [{"type": "text", "text": "hello"}],
                "metadata": {
                    "slack_ts": "1234567890.123456",
                    "slack_user": "U123",
                    "slack_channel": "C456"
                },
                "created_at": "2024-01-01T00:00:00Z"
            }
        }),
        metadata: None,
        tags: None,
    };
    db.create_event(event).await.unwrap();

    // Same slack_ts should be found
    assert!(
        db.has_event_with_slack_ts(session_id, "1234567890.123456")
            .await
            .unwrap()
    );

    // Different slack_ts should NOT be found
    assert!(
        !db.has_event_with_slack_ts(session_id, "9999999999.999999")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_has_event_with_slack_ts_wrong_session() {
    use crate::storage::models::CreateEventRow;

    let db = StorageBackend::in_memory();
    let session_id = setup_test_session(&db).await;
    let other_session_id = everruns_provider::typed_id::SessionId::from_uuid(uuid::Uuid::now_v7());

    // Insert event in session_id
    let event = CreateEventRow {
        session_id,
        event_type: "input.message".to_string(),
        ts: chrono::Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({
            "message": {
                "id": "message_00000000000000000000000000000001",
                "role": "user",
                "content": [{"type": "text", "text": "hello"}],
                "metadata": { "slack_ts": "1234567890.123456" },
                "created_at": "2024-01-01T00:00:00Z"
            }
        }),
        metadata: None,
        tags: None,
    };
    db.create_event(event).await.unwrap();

    // Different session should NOT find it
    assert!(
        !db.has_event_with_slack_ts(other_session_id, "1234567890.123456")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_has_event_with_slack_ts_ignores_non_input_events() {
    use crate::storage::models::CreateEventRow;

    let db = StorageBackend::in_memory();
    let session_id = setup_test_session(&db).await;

    // Insert an output.message.completed event (not input.message)
    let event = CreateEventRow {
        session_id,
        event_type: "output.message.completed".to_string(),
        ts: chrono::Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({
            "message": {
                "id": "message_00000000000000000000000000000001",
                "role": "assistant",
                "content": [{"type": "text", "text": "hi"}],
                "metadata": { "slack_ts": "1234567890.123456" },
                "created_at": "2024-01-01T00:00:00Z"
            }
        }),
        metadata: None,
        tags: None,
    };
    db.create_event(event).await.unwrap();

    // Should NOT match — only input.message events count
    assert!(
        !db.has_event_with_slack_ts(session_id, "1234567890.123456")
            .await
            .unwrap()
    );
}

// ==========================================
// Response text extraction — last vs first
// ==========================================

#[test]
fn test_extract_response_text_ignores_non_text_parts() {
    let data = serde_json::json!({
        "message": {
            "content": [
                {"type": "image", "url": "https://example.com/img.png"},
                {"type": "text", "text": "Only this."}
            ]
        }
    });
    assert_eq!(extract_response_text(&data).unwrap(), "Only this.");
}

/// Helper: create an in-memory session for dedup tests
/// EVE-975: a rename in the pane is the user naming their own thread. We push
/// titles the other way, so ignoring it would silently revert them.
mod pane_rename_tests {
    use super::*;
    use crate::storage::models::CreateSessionRow;

    const PANE_CHANNEL: &str = "D_PANE";
    const PANE_TS: &str = "1700000000.000100";

    fn pane_config() -> SlackChannelConfig {
        let mut config = test_config(SessionBinding::Thread);
        config.agent_surface_enabled = true;
        config
    }

    fn rename_event(title: Option<&str>) -> SlackEvent {
        let mut event = test_event(PANE_CHANNEL, Some(PANE_TS), Some(PANE_TS));
        event.event_type = "agent_session_title_changed".to_string();
        event.assistant_thread = Some(SlackAssistantThread {
            channel_id: Some(PANE_CHANNEL.to_string()),
            thread_ts: Some(PANE_TS.to_string()),
            context: None,
            title: title.map(str::to_string),
        });
        event
    }

    async fn state_with_pane_session(
        app: &TestIngress,
        stored_title: &str,
    ) -> (SlackState, everruns_provider::typed_id::SessionId) {
        let db = Arc::new(StorageBackend::in_memory());
        let runner: Arc<dyn AgentRunner> = Arc::new(NoopRunner);
        let state = SlackState::new(
            db,
            None,
            runner,
            None,
            false,
            crate::event_delivery::EventDelivery::in_memory(),
            "https://example.com/api".to_string(),
        );

        let tags = build_session_tags(
            app,
            &app.channels[0],
            &pane_config(),
            &rename_event(None),
            SlackSurface::Pane,
        );
        let session = state
            .db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                workspace_id: None,
                org_id: app.org_id,
                app_id: Some(app.internal_id),
                endpoint_id: None,
                harness_id: Some(everruns_provider::typed_id::HarnessId::from_uuid(
                    uuid::Uuid::nil(),
                )),
                agent_id: None,
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: None,
                owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: Some(stored_title.to_string()),
                locale: None,
                tags,
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::Value::Array(vec![]),
                hints: None,
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                network_access: None,
                parent_session_id: None,
                budget_root_session_id: None,
            })
            .await
            .expect("create pane session");
        (state, session.id)
    }

    async fn title_events(
        state: &SlackState,
        session: everruns_provider::typed_id::SessionId,
    ) -> usize {
        state
            .db
            .list_events(
                session,
                None,
                None,
                &[everruns_core::SESSION_TITLE_UPDATED.to_string()],
                &[],
                None,
                None,
            )
            .await
            .expect("list title events")
            .len()
    }

    #[tokio::test]
    async fn a_rename_becomes_the_session_title() {
        let app = test_app();
        let (state, session) =
            state_with_pane_session(&app, "Slack thread 1700000000.000100").await;

        handle_agent_session_title_changed(
            &state,
            &app,
            &app.channels[0],
            &pane_config(),
            // Slack pads nothing, but a user can; the stored title should not.
            &rename_event(Some("  Refund policy for EU orders  ")),
        )
        .await
        .expect("apply rename");

        let row = state
            .db
            .get_session(app.org_id, session)
            .await
            .expect("load session")
            .expect("session exists");
        assert_eq!(row.title.as_deref(), Some("Refund policy for EU orders"));
        assert_eq!(title_events(&state, session).await, 1);
    }

    /// The pane echoes back the title we just pushed it. Writing that again
    /// would emit an event, which the dispatcher would push to Slack, which
    /// would echo — so a no-op rename has to stay a no-op.
    #[tokio::test]
    async fn a_rename_to_the_current_title_changes_nothing() {
        let app = test_app();
        let (state, session) = state_with_pane_session(&app, "Refund policy").await;

        handle_agent_session_title_changed(
            &state,
            &app,
            &app.channels[0],
            &pane_config(),
            &rename_event(Some("Refund policy")),
        )
        .await
        .expect("apply rename");

        assert_eq!(title_events(&state, session).await, 0);
    }

    #[tokio::test]
    async fn an_empty_rename_is_ignored() {
        let app = test_app();
        let (state, session) = state_with_pane_session(&app, "Original").await;

        for title in [Some("   "), None] {
            handle_agent_session_title_changed(
                &state,
                &app,
                &app.channels[0],
                &pane_config(),
                &rename_event(title),
            )
            .await
            .expect("apply rename");
        }

        let row = state
            .db
            .get_session(app.org_id, session)
            .await
            .expect("load session")
            .expect("session exists");
        assert_eq!(row.title.as_deref(), Some("Original"));
        assert_eq!(title_events(&state, session).await, 0);
    }

    /// Same scoping as the stop button (EVE-976): a rename can only touch a
    /// session the receiving app owns.
    #[tokio::test]
    async fn a_rename_for_another_apps_thread_is_ignored() {
        let app = test_app();
        let (state, session) = state_with_pane_session(&app, "Original").await;

        let mut other_app = test_app();
        other_app.internal_id = uuid::Uuid::from_u128(9_999);
        other_app.public_id =
            everruns_provider::typed_id::AppId::from_uuid(uuid::Uuid::from_u128(9_999));

        handle_agent_session_title_changed(
            &state,
            &other_app,
            &other_app.channels[0],
            &pane_config(),
            &rename_event(Some("Hijacked")),
        )
        .await
        .expect("apply rename");

        let row = state
            .db
            .get_session(app.org_id, session)
            .await
            .expect("load session")
            .expect("session exists");
        assert_eq!(row.title.as_deref(), Some("Original"));
    }
}
