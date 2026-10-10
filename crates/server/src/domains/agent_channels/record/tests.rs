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
fn test_session_strategy_default() {
    assert_eq!(SessionBinding::default(), SessionBinding::Thread);
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
        ChannelType::ApiEndpoint,
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
fn test_ag_ui_channel_config_defaults_to_anonymous() {
    let config: AgUiChannelConfig = serde_json::from_str("{}").unwrap();
    assert!(config.anonymous);
    assert_eq!(
        config.session_expiration_seconds,
        DEFAULT_SESSION_EXPIRATION_SECONDS
    );
    assert!(config.rate_limit_per_minute.is_none());
    assert!(config.token.is_none());
    assert!(config.auth.is_none());
    assert_eq!(config.tool_visibility, PublicToolVisibility::Generic);
    assert_eq!(config.generic_tool_text, DEFAULT_AG_UI_GENERIC_TOOL_TEXT);
    assert!(!config.reasoning_summary_visible && !config.subagents_visible);
    assert!(!config.state_visible);
}

#[test]
fn test_ag_ui_channel_config_roundtrip() {
    let config = AgUiChannelConfig {
        anonymous: true,
        token: Some("agui-token".to_string()),
        session_expiration_seconds: 3600,
        rate_limit_per_minute: Some(120),
        tool_visibility: PublicToolVisibility::None,
        generic_tool_text: "Please wait".to_string(),
        reasoning_summary_visible: true,
        tool_approval_interrupts: true,
        usage_visible: true,
        subagents_visible: true,
        state_visible: true,
        auth: None,
    };
    let json = serde_json::to_string(&config).unwrap();
    let parsed: AgUiChannelConfig = serde_json::from_str(&json).unwrap();
    assert!(parsed.anonymous);
    assert_eq!(parsed.token.as_deref(), Some("agui-token"));
    assert_eq!(parsed.session_expiration_seconds, 3600);
    assert_eq!(parsed.rate_limit_per_minute, Some(120));
    assert_eq!(parsed.tool_visibility, PublicToolVisibility::None);
    assert_eq!(parsed.generic_tool_text, "Please wait");
    assert!(parsed.reasoning_summary_visible);
    assert!(parsed.tool_approval_interrupts && parsed.usage_visible);
    assert!(parsed.subagents_visible && parsed.state_visible);
}

#[test]
fn test_ag_ui_channel_config_zero_disables_expiration() {
    let config: AgUiChannelConfig =
        serde_json::from_str(r#"{"session_expiration_seconds": 0}"#).unwrap();
    assert_eq!(config.session_expiration_seconds, 0);
}

#[test]
fn test_ag_ui_channel_config_omits_rate_limit_when_unset() {
    let config: AgUiChannelConfig = serde_json::from_str("{}").unwrap();
    let json = serde_json::to_value(&config).unwrap();
    assert!(json.get("rate_limit_per_minute").is_none());
    assert!(json.get("generic_tool_text").is_none());
}

/// EVE-1005: `SessionBinding::default()` is `Thread`, which is correct for
/// messaging and wrong for invocations. Every invocation-shaped config must
/// therefore carry an explicit field default of `Shared` — the former
/// `shared_session`. A config that silently fell back to `Thread` would
/// route every schedule firing to a different session.
#[test]
fn invocation_configs_default_to_the_shared_binding() {
    assert_eq!(default_invocation_binding(), SessionBinding::Shared);
    assert_ne!(SessionBinding::default(), SessionBinding::Shared);
}

#[test]
fn test_schedule_channel_config_defaults() {
    let config: ScheduleChannelConfig =
        serde_json::from_str(r#"{"cron_expression":"0 * * * * * *","message":"Run checks"}"#)
            .unwrap();
    assert_eq!(config.timezone, "UTC");
    assert_eq!(config.session_mode, SessionBinding::Shared);
}

#[test]
fn test_webhook_channel_config_defaults() {
    let config: WebhookChannelConfig =
        serde_json::from_str(r#"{"token":"top-secret","message":"{{payload.action}}"}"#).unwrap();
    assert_eq!(config.session_mode, SessionBinding::Shared);
    // EVE-627: rate limit is optional and absent by default.
    assert!(config.rate_limit_per_minute.is_none());
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
fn test_channel_slack_config_valid() {
    let ch = test_channel(
        ChannelType::Slack,
        serde_json::json!({"signing_secret": "sec", "bot_token": "tok"}),
    );
    let config = ch.slack_config().unwrap();
    assert_eq!(config.signing_secret, "sec");
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
fn test_app_slack_channel_none_when_empty() {
    let app = test_app(vec![]);
    assert!(app.slack_channel().is_none());
}

#[test]
fn test_channel_ag_ui_config_valid() {
    let config = serde_json::json!({"anonymous": true});
    let ch = test_channel(ChannelType::AgUi, config);
    let config = ch.ag_ui_config().unwrap();
    assert!(config.anonymous);
}

#[test]
fn test_app_ag_ui_channel_lookup() {
    let config = serde_json::json!({"anonymous": true});
    let ch = test_channel(ChannelType::AgUi, config);
    let app = test_app(vec![ch]);
    assert!(app.ag_ui_channel().is_some());
}

#[test]
fn test_channel_fcp_config_defaults() {
    let ch = test_channel(ChannelType::Fcp, serde_json::json!({}));
    let config = ch.fcp_config().unwrap();
    assert!(config.anonymous);
    assert!(config.token.is_none());
    assert!(config.handshake.is_none());
    assert_eq!(
        config.session_expiration_seconds,
        DEFAULT_SESSION_EXPIRATION_SECONDS
    );
    assert_eq!(
        config.response_timeout_seconds,
        DEFAULT_FCP_RESPONSE_TIMEOUT_SECONDS
    );
}

#[test]
fn test_app_fcp_channel_lookup() {
    let ch = test_channel(ChannelType::Fcp, serde_json::json!({}));
    let app = test_app(vec![ch]);
    assert!(app.fcp_channel().is_some());
}

#[test]
fn test_channel_schedule_config_valid() {
    let ch = test_channel(
        ChannelType::Schedule,
        serde_json::json!({
            "cron_expression": "0 * * * * * *",
            "message": "Run checks"
        }),
    );
    let config = ch.schedule_config().unwrap();
    assert_eq!(config.message, "Run checks");
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
fn test_channel_webhook_config_valid() {
    let ch = test_channel(
        ChannelType::Webhook,
        serde_json::json!({
            "token": "secret",
            "message": "{{payload.ref}}"
        }),
    );
    let config = ch.webhook_config().unwrap();
    assert_eq!(config.token, "secret");
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
fn test_a2a_channel_config_defaults() {
    let config: A2aChannelConfig = serde_json::from_str(
        r#"{"api_key_hash":"abc","api_key_prefix":"evra2a_abc1...","message":"{{a2a.text}}"}"#,
    )
    .unwrap();
    assert_eq!(config.session_mode, SessionBinding::Shared);
    assert!(config.agent_card_name.is_none());
    assert!(config.agent_card_description.is_none());
    assert!(config.rate_limit_per_minute.is_none());
    assert!(config.auth.is_none());
    assert!(config.signing_secret.is_none());
}

#[test]
fn test_a2a_channel_config_roundtrip() {
    let config = A2aChannelConfig {
        api_key_hash: "deadbeef".into(),
        api_key_prefix: "evra2a_dead...".into(),
        session_mode: SessionBinding::Ephemeral,
        message: "{{a2a.text}}".into(),
        agent_card_name: Some("Inbox triage".into()),
        agent_card_description: Some("Triages github events".into()),
        rate_limit_per_minute: Some(120),
        auth: None,
        signing_secret: None,
        pact: None,
    };
    let json = serde_json::to_string(&config).unwrap();
    let parsed: A2aChannelConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.api_key_hash, "deadbeef");
    assert_eq!(parsed.session_mode, SessionBinding::Ephemeral);
    assert_eq!(parsed.agent_card_name.as_deref(), Some("Inbox triage"));
    assert_eq!(parsed.rate_limit_per_minute, Some(120));
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
fn test_channel_a2a_config_valid() {
    let ch = test_channel(
        ChannelType::A2a,
        serde_json::json!({
            "api_key_hash": "h",
            "api_key_prefix": "evra2a_h...",
            "message": "{{a2a.text}}"
        }),
    );
    let config = ch.a2a_config().unwrap();
    assert_eq!(config.api_key_prefix, "evra2a_h...");
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
fn test_public_chat_channel_config_defaults() {
    let config: PublicChatChannelConfig = serde_json::from_str("{}").unwrap();
    assert!(config.anonymous);
    assert!(config.token.is_none());
    assert_eq!(
        config.session_expiration_seconds,
        DEFAULT_SESSION_EXPIRATION_SECONDS
    );
    assert!(config.rate_limit_per_minute.is_none());
    assert_eq!(config.tool_visibility, PublicToolVisibility::Generic);
    assert_eq!(config.generic_tool_text, DEFAULT_AG_UI_GENERIC_TOOL_TEXT);
    assert!(config.auth.is_none());
    assert!(config.branding.is_empty());
    assert!(config.captcha.is_none());
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
fn test_public_chat_channel_config_full_roundtrip() {
    let json = r##"{
        "anonymous": false,
        "token": "shared-secret",
        "session_expiration_seconds": 3600,
        "rate_limit_per_minute": 30,
        "tool_visibility": "narrated",
        "generic_tool_text": "Thinking...",
        "branding": {
            "display_name": "Support",
            "logo_url": "https://example.com/logo.png",
            "primary_color": "#0A1636",
            "welcome_message": "How can I help?"
        },
        "captcha": {
            "provider": "turnstile",
            "enabled": true,
            "site_key": "1x00000000000000000000AA",
            "secret_key": "1x0000000000000000000000000000000AA"
        }
    }"##;
    let config: PublicChatChannelConfig = serde_json::from_str(json).unwrap();
    assert!(!config.anonymous);
    assert_eq!(config.token.as_deref(), Some("shared-secret"));
    assert_eq!(config.session_expiration_seconds, 3600);
    assert_eq!(config.rate_limit_per_minute, Some(30));
    assert_eq!(config.tool_visibility, PublicToolVisibility::Narrated);
    let branding = &config.branding;
    assert_eq!(branding.display_name.as_deref(), Some("Support"));
    assert_eq!(branding.primary_color.as_deref(), Some("#0A1636"));
    let captcha = config.captcha.unwrap();
    assert_eq!(captcha.provider, CaptchaProvider::Turnstile);
    assert!(captcha.enabled);
    assert_eq!(captcha.site_key, "1x00000000000000000000AA");
    assert!(captcha.secret_key.is_some());
}

#[test]
fn test_channel_public_chat_config_valid() {
    let ch = test_channel(
        ChannelType::PublicChat,
        serde_json::json!({"anonymous": true}),
    );
    let config = ch.public_chat_config().unwrap();
    assert!(config.anonymous);
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
