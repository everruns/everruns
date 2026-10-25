// Unit tests for the channel record types in `mod.rs`.

use super::*;

#[test]
fn test_app_status_mapping() {
    // (variant, wire string) for Display, FromStr and serde over the same enum mapping.
    let cases = [
        (AppStatus::Draft, "draft"),
        (AppStatus::Published, "published"),
        (AppStatus::Archived, "archived"),
        (AppStatus::Deleted, "deleted"),
    ];
    for (variant, s) in cases {
        assert_eq!(variant.to_string(), s, "Display mismatch for {variant:?}");
        assert_eq!(AppStatus::from(s), variant, "from_str mismatch for {s:?}");
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(
            json,
            format!("\"{s}\""),
            "serde wire format mismatch for {variant:?}"
        );
        let parsed: AppStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, variant, "serde roundtrip mismatch for {variant:?}");
    }
    // Unrecognized/empty input falls back to Draft.
    assert_eq!(AppStatus::from("unknown"), AppStatus::Draft);
    assert_eq!(AppStatus::from(""), AppStatus::Draft);
}

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

fn test_app(channels: Vec<AgentChannel>) -> App {
    App {
        public_id: AppId::from_uuid(Uuid::nil()),
        internal_id: Uuid::nil(),
        org_id: 1,
        name: "test".into(),
        description: None,
        harness_id: HarnessId::from_uuid(Uuid::nil()),
        agent_id: Some(AgentId::from_uuid(Uuid::nil())),
        virtual_user_id: None,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        owner: None,
        effective_owner: None,
        channels,
        status: AppStatus::Draft,
        published_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
    }
}

fn test_channel(channel_type: ChannelType, config: serde_json::Value) -> AgentChannel {
    AgentChannel {
        public_id: AgentChannelId::from_uuid(Uuid::nil()),
        internal_id: Uuid::nil(),
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
fn test_app_slack_channel_lookup() {
    let ch = test_channel(
        ChannelType::Slack,
        serde_json::json!({"signing_secret": "s", "bot_token": "t"}),
    );
    let app = test_app(vec![ch]);
    assert!(app.slack_channel().is_some());
}

#[test]
fn test_app_ag_ui_channel_lookup() {
    let config = serde_json::json!({"anonymous": true});
    let ch = test_channel(ChannelType::AgUi, config);
    let app = test_app(vec![ch]);
    assert!(app.ag_ui_channel().is_some());
}

#[test]
fn test_app_fcp_channel_lookup() {
    let ch = test_channel(ChannelType::Fcp, serde_json::json!({}));
    let app = test_app(vec![ch]);
    assert!(app.fcp_channel().is_some());
}

#[test]
fn test_app_schedule_channel_lookup() {
    let ch = test_channel(
        ChannelType::Schedule,
        serde_json::json!({
            "cron_expression": "0 * * * * * *",
            "message": "Run checks"
        }),
    );
    let app = test_app(vec![ch]);
    assert!(app.schedule_channel().is_some());
}

#[test]
fn test_app_webhook_channel_lookup() {
    let ch = test_channel(
        ChannelType::Webhook,
        serde_json::json!({
            "token": "secret",
            "message": "{{payload.ref}}"
        }),
    );
    let app = test_app(vec![ch]);
    assert!(app.webhook_channel().is_some());
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
fn test_app_a2a_channel_lookup() {
    let ch = test_channel(
        ChannelType::A2a,
        serde_json::json!({
            "api_key_hash": "h",
            "api_key_prefix": "evra2a_h...",
            "message": "{{a2a.text}}"
        }),
    );
    let app = test_app(vec![ch]);
    assert!(app.a2a_channel().is_some());
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

#[test]
fn test_app_public_chat_channel_lookup() {
    let ch = test_channel(
        ChannelType::PublicChat,
        serde_json::json!({"branding": {"display_name": "Helpdesk"}}),
    );
    let app = test_app(vec![ch]);
    assert!(app.public_chat_channel().is_some());
}

#[test]
fn test_app_serde_skips_internal_fields() {
    let app = test_app(vec![]);
    let json = serde_json::to_value(&app).unwrap();
    assert!(json.get("id").is_some()); // public_id serialized as "id"
    assert!(json.get("internal_id").is_none()); // skipped
    assert!(json.get("org_id").is_none()); // skipped
    assert!(json.get("published_at").is_none()); // None skipped
}
