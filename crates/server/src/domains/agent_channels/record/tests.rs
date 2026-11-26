// Unit tests for the channel record types in `mod.rs`.

use super::*;

#[test]
fn test_channel_type_mapping() {
    // (variant, wire string) for Display, from_str_opt and serde over the same enum mapping.
    let cases = [
        (ChannelType::Slack, "slack"),
        (ChannelType::AgUi, "ag_ui"),
        (ChannelType::Schedule, "schedule"),
        (ChannelType::Webhook, "webhook"),
        (ChannelType::A2a, "a2a"),
        (ChannelType::Fcp, "fcp"),
        (ChannelType::PublicChat, "public_chat"),
        (ChannelType::Voice, "voice"),
        (ChannelType::Poppy, "poppy"),
    ];
    for (variant, s) in cases {
        assert_eq!(variant.to_string(), s, "Display mismatch for {variant:?}");
        assert_eq!(
            ChannelType::from_str_opt(s),
            Some(variant.clone()),
            "from_str_opt mismatch for {s:?}"
        );
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(
            json,
            format!("\"{s}\""),
            "serde wire format mismatch for {variant:?}"
        );
        let parsed: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, variant, "serde roundtrip mismatch for {variant:?}");
    }
    assert_eq!(ChannelType::from_str_opt("unknown"), None);
    assert_eq!(ChannelType::from_str_opt(""), None);
}

#[test]
fn test_session_strategy_serde() {
    let json = serde_json::to_string(&SessionBinding::Conversation).unwrap();
    assert_eq!(json, r#""per_channel""#);
    let parsed: SessionBinding = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, SessionBinding::Conversation);

    let json = serde_json::to_string(&SessionBinding::Requester).unwrap();
    assert_eq!(json, r#""per_user""#);
}

/// EVE-1005: a transport offers only the bindings that mean something on it.
/// This replaces the old "two enums" split — messaging could not express
/// `shared_session` and triggers could not express `per_thread` because they
/// used different types, not because anything checked.
#[test]
fn transports_only_offer_bindings_that_mean_something() {
    assert!(ChannelType::Slack.allows_binding(SessionBinding::Thread));
    assert!(ChannelType::Slack.allows_binding(SessionBinding::Conversation));
    assert!(ChannelType::Slack.allows_binding(SessionBinding::Requester));
    assert!(!ChannelType::Slack.allows_binding(SessionBinding::Shared));
    assert!(!ChannelType::Slack.allows_binding(SessionBinding::Ephemeral));

    for transport in [
        ChannelType::Schedule,
        ChannelType::Webhook,
        ChannelType::A2a,
    ] {
        assert!(
            transport.allows_binding(SessionBinding::Shared),
            "{transport}"
        );
        assert!(
            transport.allows_binding(SessionBinding::Ephemeral),
            "{transport}"
        );
        // Nothing is listening on a thread, so these have no meaning here.
        for message_keyed in SessionBinding::MESSAGE_KEYED {
            assert!(
                !transport.allows_binding(message_keyed),
                "{transport} must not offer {message_keyed:?}"
            );
        }
    }
}

#[test]
fn test_ag_ui_channel_config_omits_rate_limit_when_unset() {
    let config: AgUiChannelConfig = serde_json::from_str("{}").unwrap();
    let json = serde_json::to_value(&config).unwrap();
    assert!(json.get("rate_limit_per_minute").is_none());
    assert!(json.get("generic_tool_text").is_none());
}

#[test]
fn test_webhook_channel_config_rate_limit_roundtrip() {
    // EVE-627: an explicit per-channel rate limit parses and re-serializes.
    let config: WebhookChannelConfig =
        serde_json::from_str(r#"{"token":"t","message":"m","rate_limit_per_minute":120}"#).unwrap();
    assert_eq!(config.rate_limit_per_minute, Some(120));
    let json = serde_json::to_value(&config).unwrap();
    assert_eq!(
        json.get("rate_limit_per_minute").and_then(|v| v.as_u64()),
        Some(120)
    );

    // Absent when None (skip_serializing_if).
    let none_cfg: WebhookChannelConfig =
        serde_json::from_str(r#"{"token":"t","message":"m"}"#).unwrap();
    let none_json = serde_json::to_value(&none_cfg).unwrap();
    assert!(none_json.get("rate_limit_per_minute").is_none());
}

fn test_channel(channel_type: ChannelType, config: serde_json::Value) -> AgentChannel {
    AgentChannel {
        public_id: AgentChannelId::from_uuid(Uuid::nil()),
        internal_id: Uuid::nil(),
        agent_id: None,
        channel_type,
        channel_config: config,
        auth: None,
        enabled: true,
        status: ChannelStatus::Live,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn test_channel_slack_config_invalid_json() {
    let ch = test_channel(
        ChannelType::Slack,
        serde_json::json!({"signing_secret": 42}),
    );
    assert!(ch.slack_config().is_none());
}

#[test]
fn test_a2a_channel_config_omits_optional_fields() {
    let config = A2aChannelConfig {
        api_key_hash: "h".into(),
        api_key_prefix: "evra2a_h...".into(),
        session_mode: SessionBinding::Shared,
        message: "m".into(),
        agent_card_name: None,
        agent_card_description: None,
        rate_limit_per_minute: None,
        auth: None,
        signing_secret: None,
        pact: None,
    };
    let json = serde_json::to_value(&config).unwrap();
    assert!(json.get("agent_card_name").is_none());
    assert!(json.get("agent_card_description").is_none());
    assert!(json.get("rate_limit_per_minute").is_none());
    assert!(json.get("signing_secret").is_none());
}

#[test]
fn test_public_chat_channel_config_omits_empty_branding_and_defaults() {
    let config: PublicChatChannelConfig = serde_json::from_str("{}").unwrap();
    let json = serde_json::to_value(&config).unwrap();
    assert!(json.get("branding").is_none());
    assert!(json.get("captcha").is_none());
    assert!(json.get("generic_tool_text").is_none());
    assert!(json.get("rate_limit_per_minute").is_none());
}
