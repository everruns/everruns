//! Tests: signature.

use super::*;
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use crate::storage::StorageBackend;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use everruns_core::channel::{InboundAttachment, SessionBinding};

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

#[test]
fn slack_message_metadata_includes_system_app_id() {
    let app = test_app();
    let event = test_event("C123", Some("1234.5678"), None);

    let metadata = slack_message_metadata(&app, &app.channels[0], &event);

    assert_eq!(
        metadata.get("slack_thread_ts"),
        Some(&serde_json::json!("1234.5678"))
    );
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
