//! Tests: routing.

use super::*;
use crate::api::messages::InputContentPart;
use crate::storage::StorageBackend;
use everruns_platform::ConversationStarter;
use everruns_worker::AgentRunner;
use std::sync::Arc;

use super::tests_support::*;

#[test]
fn test_truncate_display_name_short() {
    assert_eq!(truncate_display_name("My Bot"), "My Bot");
}

#[test]
fn test_truncate_display_name_exact_80() {
    let exact = "a".repeat(80);
    assert_eq!(truncate_display_name(&exact), exact);
}

#[test]
fn test_truncate_display_name_long() {
    let long = "a".repeat(100);
    assert_eq!(truncate_display_name(&long).len(), 80);
}

#[test]
fn test_truncate_display_name_empty() {
    assert_eq!(truncate_display_name(""), "");
}

#[test]
fn test_truncate_display_name_multibyte() {
    // 27 emoji × 4 bytes = 108 bytes, should truncate to 80 bytes (20 emoji)
    let name = "🤖".repeat(27);
    let truncated = truncate_display_name(&name);
    assert!(truncated.len() <= 80);
    assert!(truncated.is_char_boundary(truncated.len()));
}

#[test]
fn test_manifest_yaml_contains_event_subscriptions() {
    let yaml = build_manifest_yaml(
        "My Bot",
        "My Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );

    assert!(
        yaml.contains(&format!("    request_url: \"{TEST_REQUEST_URL}\"")),
        "manifest must name this server's webhook:\n{yaml}"
    );
    for event in [
        "      - app_mention",
        "      - message.channels",
        "      - message.groups",
        "      - message.im",
        "      - message.mpim",
    ] {
        assert!(
            yaml.contains(event),
            "manifest must subscribe {event}:\n{yaml}"
        );
    }
    // event_subscriptions has to sit under `settings`, not at the root, or
    // Slack rejects the manifest.
    let settings = yaml.find("settings:").expect("settings section");
    let subs = yaml
        .find("  event_subscriptions:")
        .expect("event_subscriptions section");
    assert!(
        subs > settings,
        "event_subscriptions must nest under settings"
    );
}

#[test]
fn test_manifest_yaml_parses_as_yaml_with_expected_shape() {
    let yaml = build_manifest_yaml(
        "My Bot",
        "My Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("manifest is valid YAML");

    let subs = &parsed["settings"]["event_subscriptions"];
    assert_eq!(subs["request_url"].as_str(), Some(TEST_REQUEST_URL));
    let events: Vec<&str> = subs["bot_events"]
        .as_sequence()
        .expect("bot_events sequence")
        .iter()
        .map(|v| v.as_str().expect("event is a string"))
        .collect();
    assert_eq!(
        events,
        vec![
            "app_mention",
            "message.channels",
            "message.groups",
            "message.im",
            "message.mpim"
        ]
    );
}

/// Authored starters reach the manifest as `{title, message}` pairs.
#[test]
fn test_manifest_yaml_agent_view_carries_suggested_prompts() {
    let starters = vec![
        test_starter("Triage the newest P1"),
        test_starter("Summarize this channel"),
    ];
    let yaml = build_manifest_yaml(
        "Bot",
        "Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        true,
        &starters,
    );
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("manifest is valid YAML");

    let prompts = parsed["features"]["agent_view"]["suggested_prompts"]
        .as_sequence()
        .expect("suggested_prompts must be a list");
    assert_eq!(prompts.len(), 2, "{yaml}");
    assert_eq!(prompts[0]["title"].as_str(), Some("Triage the newest P1"));
    assert_eq!(prompts[0]["message"].as_str(), Some("Triage the newest P1"));
    assert_eq!(prompts[1]["title"].as_str(), Some("Summarize this channel"));
}

/// Nothing authored must stay nothing — an empty pane beats generic prompts
/// nobody wrote, and an empty `suggested_prompts` key is not valid either.
#[test]
fn test_manifest_yaml_omits_suggested_prompts_when_unauthored() {
    for starters in [vec![], vec![test_starter("   ")]] {
        let yaml = build_manifest_yaml(
            "Bot",
            "Bot",
            None,
            TEST_REQUEST_URL,
            TEST_INTERACTIVITY_URL,
            true,
            &starters,
        );
        assert!(
            yaml.contains("agent_view"),
            "the agent surface itself must stay on: {yaml}"
        );
        assert!(
            !yaml.contains("suggested_prompts"),
            "unauthored starters must emit no key: {yaml}"
        );
        serde_yaml::from_str::<serde_yaml::Value>(&yaml).expect("manifest is valid YAML");
    }
}

/// Slack renders at most four prompts; Platform Chat allows up to eight.
#[test]
fn test_manifest_yaml_caps_suggested_prompts_at_slack_limit() {
    let starters: Vec<ConversationStarter> = (0..8)
        .map(|i| test_starter(&format!("Prompt {i}")))
        .collect();
    let yaml = build_manifest_yaml(
        "Bot",
        "Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        true,
        &starters,
    );
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("manifest is valid YAML");

    let prompts = parsed["features"]["agent_view"]["suggested_prompts"]
        .as_sequence()
        .expect("suggested_prompts must be a list");
    assert_eq!(prompts.len(), SLACK_SUGGESTED_PROMPT_MAX, "{yaml}");
    assert_eq!(prompts[0]["title"].as_str(), Some("Prompt 0"));
    assert_eq!(prompts[3]["title"].as_str(), Some("Prompt 3"));
}

/// A long starter keeps a legible chip while the full text still rides in
/// `message`, which is what Slack inserts into the composer.
#[test]
fn test_manifest_yaml_truncates_prompt_title_but_not_message() {
    let long = "a".repeat(200);
    let yaml = build_manifest_yaml(
        "Bot",
        "Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        true,
        &[test_starter(&long)],
    );
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("manifest is valid YAML");
    let prompt = &parsed["features"]["agent_view"]["suggested_prompts"][0];

    assert_eq!(
        prompt["title"].as_str().unwrap().chars().count(),
        SLACK_SUGGESTED_PROMPT_TITLE_MAX
    );
    assert_eq!(prompt["message"].as_str(), Some(long.as_str()));
}

/// Starter text is operator-authored, so it must not break out of the YAML
/// string it is embedded in.
#[test]
fn test_manifest_yaml_escapes_prompt_text() {
    let yaml = build_manifest_yaml(
        "Bot",
        "Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        true,
        &[test_starter(r#"Say "hi" \ now"#)],
    );
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&yaml).expect("manifest with quoted starter is valid YAML");
    assert_eq!(
        parsed["features"]["agent_view"]["suggested_prompts"][0]["message"].as_str(),
        Some(r#"Say "hi" \ now"#)
    );
}

/// The agent surface is the only thing that renders prompts, so they must
/// not leak into a channel-bot manifest.
#[test]
fn test_manifest_yaml_agent_surface_off_carries_no_prompts() {
    let yaml = build_manifest_yaml(
        "Bot",
        "Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[test_starter("Triage the newest P1")],
    );
    assert!(!yaml.contains("suggested_prompts"), "{yaml}");
    assert!(!yaml.contains("Triage the newest P1"), "{yaml}");
}

#[test]
fn test_manifest_yaml_agent_surface_off_is_unchanged() {
    let yaml = build_manifest_yaml(
        "My Bot",
        "My Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );

    // Existing apps must be untouched by the feature existing.
    assert!(!yaml.contains("agent_view"), "{yaml}");
    assert!(!yaml.contains("assistant:write"), "{yaml}");
    for event in [
        "app_home_opened",
        "app_context_changed",
        "agent_session_stopped",
        "agent_session_title_changed",
    ] {
        assert!(
            !yaml.contains(event),
            "{event} leaked with surface off:\n{yaml}"
        );
    }
}

#[test]
fn test_manifest_yaml_agent_surface_on() {
    let yaml = build_manifest_yaml(
        "My Bot",
        "My Bot",
        Some("Answers questions"),
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        true,
        &[],
    );
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("manifest is valid YAML");

    // `agent_view`, not the legacy `assistant_view`: Slack only accepts the
    // former for new apps, and every manifest we generate is for a new app.
    let agent_view = &parsed["features"]["agent_view"];
    assert!(
        !agent_view.is_null(),
        "features.agent_view missing:\n{yaml}"
    );
    assert!(parsed["features"]["assistant_view"].is_null());
    let description = agent_view["agent_description"]
        .as_str()
        .expect("agent_description is required by Slack");
    assert!(description.contains("My Bot"));
    assert!(description.len() <= SLACK_AGENT_DESC_MAX);

    // The channel bot is untouched — the surface is additive.
    let scopes: Vec<&str> = parsed["oauth_config"]["scopes"]["bot"]
        .as_sequence()
        .expect("bot scopes")
        .iter()
        .map(|v| v.as_str().expect("scope is a string"))
        .collect();
    assert!(scopes.contains(&"assistant:write"), "{scopes:?}");
    assert!(scopes.contains(&"chat:write"), "{scopes:?}");

    let events: Vec<&str> = parsed["settings"]["event_subscriptions"]["bot_events"]
        .as_sequence()
        .expect("bot_events")
        .iter()
        .map(|v| v.as_str().expect("event is a string"))
        .collect();
    for event in [
        "app_home_opened",
        "app_context_changed",
        "agent_session_stopped",
        "agent_session_title_changed",
        // message.im is the pane's inbound channel and was already present.
        "message.im",
        // Channel events survive: one app serves both surfaces.
        "app_mention",
        "message.channels",
    ] {
        assert!(events.contains(&event), "{event} missing from {events:?}");
    }
}

#[test]
fn test_agent_description_respects_slack_limit() {
    // Slack rejects an agent_description over 300 characters.
    let long = "d".repeat(500);
    let yaml = build_manifest_yaml(
        "Bot",
        "Bot",
        Some(&long),
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        true,
        &[],
    );
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("manifest is valid YAML");

    let description = parsed["features"]["agent_view"]["agent_description"]
        .as_str()
        .expect("agent_description");
    assert_eq!(description.chars().count(), SLACK_AGENT_DESC_MAX);
}

#[test]
fn test_agent_description_is_char_safe() {
    // Truncation must not split a multi-byte character.
    let long = "é".repeat(500);
    let yaml = build_manifest_yaml(
        "Bot",
        "Bot",
        Some(&long),
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        true,
        &[],
    );
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("manifest is valid YAML");
    assert_eq!(
        parsed["features"]["agent_view"]["agent_description"]
            .as_str()
            .expect("agent_description")
            .chars()
            .count(),
        SLACK_AGENT_DESC_MAX
    );
}

#[test]
fn test_manifest_yaml_escapes_request_url() {
    // The URL is server-configured, but a quote in it must not break out of
    // the YAML string and corrupt the rest of the manifest.
    let yaml = build_manifest_yaml(
        "My Bot",
        "My Bot",
        None,
        r#"https://x/"evil"#,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("manifest is valid YAML");
    assert_eq!(
        parsed["settings"]["event_subscriptions"]["request_url"].as_str(),
        Some(r#"https://x/"evil"#)
    );
}

#[test]
fn test_slack_webhook_url_shape() {
    assert_eq!(
        slack_webhook_url("https://example.com/api", "channel_abc"),
        "https://example.com/api/v1/e/channel_abc/slack/events"
    );
    // A configured base with a trailing slash must not double up.
    assert_eq!(
        slack_webhook_url("https://example.com/api/", "channel_abc"),
        "https://example.com/api/v1/e/channel_abc/slack/events"
    );
}

#[test]
fn test_manifest_yaml_contains_description_and_long_description() {
    let yaml = build_manifest_yaml(
        "My Bot",
        "My Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );
    assert!(yaml.contains(r#"description: "My Bot (Powered by Everruns)""#));
    assert!(yaml.contains("AI agent powered by Everruns"));
    assert!(yaml.contains("https://everruns.com"));
}

#[test]
fn test_manifest_yaml_with_app_description() {
    let yaml = build_manifest_yaml(
        "My Bot",
        "My Bot",
        Some("A helpful assistant"),
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );
    assert!(yaml.contains("A helpful assistant"));
    assert!(yaml.contains("AI agent powered by Everruns"));
}

#[test]
fn test_manifest_yaml_description_within_slack_limit() {
    // Slack description limit is 140 chars. Worst case: 35-char app name
    // (Slack's name limit) + " (Powered by Everruns)" = 57 chars.
    let long_name = "a".repeat(35);
    let yaml = build_manifest_yaml(
        &long_name,
        &long_name,
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );
    // Extract the description value
    let desc_prefix = "description: \"";
    let desc_start = yaml.find(desc_prefix).unwrap() + desc_prefix.len();
    let desc_end = yaml[desc_start..].find('"').unwrap();
    let description = &yaml[desc_start..desc_start + desc_end];
    assert!(
        description.len() <= 140,
        "Description '{}' is {} chars, exceeds Slack's 140 limit",
        description,
        description.len()
    );
}

#[test]
fn test_manifest_yaml_escapes_special_chars_in_name() {
    let yaml = build_manifest_yaml(
        r#"Bot "Special""#,
        "Bot Special",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );
    assert!(yaml.contains(r#"name: "Bot \"Special\"""#));
    assert!(yaml.contains(r#"description: "Bot \"Special\" (Powered by Everruns)""#));
}

#[test]
fn test_build_long_description_meets_minimum() {
    // No description — should still be >= 174
    let desc = build_long_description("Bot", None);
    assert!(
        desc.len() >= SLACK_LONG_DESC_MIN,
        "Long description is {} chars, need at least {}",
        desc.len(),
        SLACK_LONG_DESC_MIN
    );
}

#[test]
fn test_build_long_description_with_app_desc() {
    let desc = build_long_description("Bot", Some("My awesome bot"));
    assert!(desc.contains("My awesome bot"));
    assert!(desc.len() >= SLACK_LONG_DESC_MIN);
}

#[test]
fn test_build_long_description_empty_desc_treated_as_none() {
    let desc = build_long_description("Bot", Some("   "));
    // Empty/whitespace description is ignored
    assert!(!desc.contains("   |"));
    assert!(desc.len() >= SLACK_LONG_DESC_MIN);
}

#[test]
fn test_build_long_description_truncates_at_max() {
    let long = "x".repeat(5000);
    let desc = build_long_description("Bot", Some(&long));
    assert!(desc.len() <= SLACK_LONG_DESC_MAX);
}

#[test]
fn test_yaml_escape() {
    assert_eq!(yaml_escape(r#"hello "world""#), r#"hello \"world\""#);
    assert_eq!(yaml_escape(r#"back\slash"#), r#"back\\slash"#);
}

#[test]
fn test_yaml_escape_no_special_chars() {
    assert_eq!(yaml_escape("plain text"), "plain text");
}

#[test]
fn test_yaml_escape_empty() {
    assert_eq!(yaml_escape(""), "");
}

#[test]
fn test_urlencoding_encode() {
    assert_eq!(urlencoding_encode("hello world"), "hello%20world");
    assert_eq!(urlencoding_encode("a=b&c=d"), "a%3Db%26c%3Dd");
    assert_eq!(urlencoding_encode("safe-_.~"), "safe-_.~");
}

#[test]
fn test_urlencoding_encode_empty() {
    assert_eq!(urlencoding_encode(""), "");
}

#[test]
fn test_urlencoding_encode_newlines() {
    assert_eq!(urlencoding_encode("a\nb"), "a%0Ab");
}

#[test]
fn test_validated_image_url_requires_https() {
    assert!(validated_image_url("http://example.com/image.png").is_none());
    assert_eq!(
        validated_image_url("https://example.com/image.png").as_deref(),
        Some("https://example.com/image.png")
    );
}

// ==========================================
// File attachment content part building
// ==========================================

#[test]
fn test_build_file_content_parts_image_with_url() {
    let files = vec![test_slack_file(
        "photo.png",
        "image/png",
        "png",
        Some("https://files.slack.com/files-pri/T0/photo.png"),
    )];
    let parts = build_file_content_parts(&files);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Image(img) => {
            assert_eq!(
                img.url.as_deref().unwrap(),
                "https://files.slack.com/files-pri/T0/photo.png"
            );
        }
        other => panic!("Expected Image content part, got {:?}", other),
    }
}

#[test]
fn test_build_file_content_parts_image_no_url() {
    let files = vec![test_slack_file("photo.png", "image/png", "png", None)];
    let parts = build_file_content_parts(&files);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Text(t) => {
            assert!(t.text.contains("photo.png"));
            assert!(t.text.contains("no download URL"));
        }
        other => panic!("Expected Text fallback, got {:?}", other),
    }
}

#[test]
fn test_build_file_content_parts_image_unsafe_url_blocked() {
    let files = vec![test_slack_file(
        "photo.png",
        "image/png",
        "png",
        Some("http://169.254.169.254/latest/meta-data/"),
    )];
    let parts = build_file_content_parts(&files);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Text(t) => {
            assert!(t.text.contains("blocked unsafe image URL"));
        }
        other => panic!("Expected Text fallback, got {:?}", other),
    }
}

#[test]
fn test_build_file_content_parts_unsupported_type() {
    let files = vec![test_slack_file(
        "video.mp4",
        "video/mp4",
        "mp4",
        Some("https://files.slack.com/files-pri/T0/video.mp4"),
    )];
    let parts = build_file_content_parts(&files);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Text(t) => {
            assert!(t.text.contains("Attached file"));
            assert!(t.text.contains("video.mp4"));
            assert!(t.text.contains("mp4"));
        }
        other => panic!("Expected Text description, got {:?}", other),
    }
}

#[test]
fn test_build_file_content_parts_multiple_mixed() {
    let files = vec![
        test_slack_file(
            "photo.jpg",
            "image/jpeg",
            "jpg",
            Some("https://files.slack.com/photo.jpg"),
        ),
        test_slack_file(
            "doc.pdf",
            "application/pdf",
            "pdf",
            Some("https://files.slack.com/doc.pdf"),
        ),
        test_slack_file(
            "screenshot.webp",
            "image/webp",
            "webp",
            Some("https://files.slack.com/screenshot.webp"),
        ),
    ];
    let parts = build_file_content_parts(&files);
    assert_eq!(parts.len(), 3);
    assert!(matches!(&parts[0], InputContentPart::Image(_)));
    assert!(matches!(&parts[1], InputContentPart::Text(_)));
    assert!(matches!(&parts[2], InputContentPart::Image(_)));
}

#[test]
fn test_build_file_content_parts_empty() {
    let parts = build_file_content_parts(&[]);
    assert!(parts.is_empty());
}

#[test]
fn test_build_file_content_parts_all_supported_image_types() {
    for (mime, ext) in [
        ("image/png", "png"),
        ("image/jpeg", "jpeg"),
        ("image/gif", "gif"),
        ("image/webp", "webp"),
    ] {
        let files = vec![test_slack_file(
            &format!("test.{ext}"),
            mime,
            ext,
            Some("https://example.com/file"),
        )];
        let parts = build_file_content_parts(&files);
        assert!(
            matches!(&parts[0], InputContentPart::Image(_)),
            "{mime} should produce Image content part"
        );
    }
}

#[test]
fn test_slack_event_with_files_deserialization() {
    let json = r#"{
        "type": "event_callback",
        "event": {
            "type": "message",
            "user": "U0123456789",
            "text": "Check this out",
            "channel": "C0123456789",
            "ts": "1234567890.123456",
            "files": [
                {
                    "id": "F0123",
                    "name": "image.png",
                    "mimetype": "image/png",
                    "filetype": "png",
                    "url_private": "https://files.slack.com/image.png",
                    "size": 2048
                },
                {
                    "id": "F0456",
                    "name": "report.pdf",
                    "mimetype": "application/pdf",
                    "filetype": "pdf",
                    "url_private": "https://files.slack.com/report.pdf",
                    "size": 10240
                }
            ]
        }
    }"#;
    let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
    let event = envelope.event.unwrap();
    assert_eq!(event.files.len(), 2);
    assert_eq!(event.files[0].name.as_deref().unwrap(), "image.png");
    assert_eq!(event.files[0].mimetype.as_deref().unwrap(), "image/png");
    assert_eq!(event.files[1].name.as_deref().unwrap(), "report.pdf");
}

#[test]
fn test_slack_event_without_files_deserialization() {
    let json = r#"{
        "type": "event_callback",
        "event": {
            "type": "message",
            "user": "U0123456789",
            "text": "No files here",
            "channel": "C0123456789",
            "ts": "1234567890.123456"
        }
    }"#;
    let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
    let event = envelope.event.unwrap();
    assert!(event.files.is_empty());
}

#[test]
fn test_build_file_content_parts_missing_fields() {
    // File with no name, no mimetype, no filetype
    let file = SlackFile {
        id: None,
        name: None,
        mimetype: None,
        filetype: None,
        url_private: Some("https://example.com/file".to_string()),
        size: None,
    };
    let parts = build_file_content_parts(&[file]);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Text(t) => {
            assert!(t.text.contains("unnamed file"));
            assert!(t.text.contains("unknown"));
        }
        other => panic!("Expected Text for unknown mimetype, got {:?}", other),
    }
}

// ==========================================
// Legacy attachment content part building
// ==========================================

#[test]
fn test_build_attachment_content_parts_with_image() {
    let mut att = test_slack_attachment();
    att.image_url = Some("https://example.com/preview.png".to_string());
    att.title = Some("Preview".to_string());

    let parts = build_attachment_content_parts(&[att]);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Image(img) => {
            assert_eq!(
                img.url.as_deref().unwrap(),
                "https://example.com/preview.png"
            );
        }
        other => panic!("Expected Image, got {:?}", other),
    }
}

#[test]
fn test_build_attachment_content_parts_unsafe_image_url_blocked() {
    let mut att = test_slack_attachment();
    att.image_url = Some("http://127.0.0.1/private.png".to_string());
    att.title = Some("Preview".to_string());

    let parts = build_attachment_content_parts(&[att]);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Text(t) => {
            assert!(t.text.contains("Attachment image blocked"));
            assert!(t.text.contains("Preview"));
        }
        other => panic!("Expected Text fallback, got {:?}", other),
    }
}

#[test]
fn test_build_attachment_content_parts_link_unfurl() {
    let mut att = test_slack_attachment();
    att.service_name = Some("GitHub".to_string());
    att.title = Some("Fix bug #123".to_string());
    att.title_link = Some("https://github.com/org/repo/pull/123".to_string());
    att.text = Some("Fixes a critical issue".to_string());

    let parts = build_attachment_content_parts(&[att]);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Text(t) => {
            assert!(t.text.contains("GitHub"));
            assert!(t.text.contains("Fix bug #123"));
            assert!(t.text.contains("github.com"));
            assert!(t.text.contains("Fixes a critical issue"));
        }
        other => panic!("Expected Text, got {:?}", other),
    }
}

#[test]
fn test_build_attachment_content_parts_fallback_only() {
    let mut att = test_slack_attachment();
    att.fallback = Some("Canvas: Project Plan".to_string());

    let parts = build_attachment_content_parts(&[att]);
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        InputContentPart::Text(t) => {
            assert!(t.text.contains("Canvas: Project Plan"));
        }
        other => panic!("Expected Text fallback, got {:?}", other),
    }
}

#[test]
fn test_build_attachment_content_parts_empty_attachment() {
    let att = test_slack_attachment();
    let parts = build_attachment_content_parts(&[att]);
    assert!(parts.is_empty());
}

#[test]
fn test_build_attachment_content_parts_empty_vec() {
    let parts = build_attachment_content_parts(&[]);
    assert!(parts.is_empty());
}

#[test]
fn test_build_attachment_content_parts_multiple() {
    let mut att1 = test_slack_attachment();
    att1.image_url = Some("https://example.com/img.png".to_string());

    let mut att2 = test_slack_attachment();
    att2.title = Some("Linked doc".to_string());
    att2.text = Some("Some content".to_string());

    let parts = build_attachment_content_parts(&[att1, att2]);
    assert_eq!(parts.len(), 2);
    assert!(matches!(&parts[0], InputContentPart::Image(_)));
    assert!(matches!(&parts[1], InputContentPart::Text(_)));
}

#[test]
fn test_slack_event_with_attachments_deserialization() {
    let json = r#"{
        "type": "event_callback",
        "event": {
            "type": "message",
            "user": "U0123456789",
            "text": "Check this link",
            "channel": "C0123456789",
            "ts": "1234567890.123456",
            "attachments": [
                {
                    "service_name": "GitHub",
                    "title": "PR #42",
                    "title_link": "https://github.com/org/repo/pull/42",
                    "text": "Add feature X",
                    "fallback": "GitHub: PR #42"
                }
            ]
        }
    }"#;
    let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
    let event = envelope.event.unwrap();
    assert_eq!(event.attachments.len(), 1);
    assert_eq!(event.attachments[0].title.as_deref().unwrap(), "PR #42");
    assert_eq!(
        event.attachments[0].service_name.as_deref().unwrap(),
        "GitHub"
    );
}

#[test]
fn test_slack_event_with_files_and_attachments() {
    let json = r#"{
        "type": "event_callback",
        "event": {
            "type": "message",
            "user": "U0123456789",
            "text": "",
            "channel": "C0123456789",
            "ts": "1234567890.123456",
            "files": [{"id": "F01", "name": "photo.png", "mimetype": "image/png", "filetype": "png"}],
            "attachments": [{"title": "Link", "text": "A link"}]
        }
    }"#;
    let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
    let event = envelope.event.unwrap();
    assert_eq!(event.files.len(), 1);
    assert_eq!(event.attachments.len(), 1);
    // Both empty text + has files/attachments = should not be skipped
    assert!(event.text.as_deref().unwrap().is_empty());
}

// ==========================================
// Thread context — SlackReplyMessage parsing
// ==========================================

#[test]
fn test_slack_reply_message_deserialization() {
    let json = r#"[
        {"user": "U123", "text": "Hello", "ts": "1234.0000"},
        {"bot_id": "B123", "text": "Hi there", "ts": "1234.0001"},
        {"user": "U456", "text": "Nice", "ts": "1234.0002", "subtype": "thread_broadcast"}
    ]"#;
    let replies: Vec<SlackReplyMessage> = serde_json::from_str(json).unwrap();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0].user.as_deref(), Some("U123"));
    assert!(replies[0].bot_id.is_none());
    assert!(replies[1].bot_id.is_some());
    assert_eq!(replies[2].subtype.as_deref(), Some("thread_broadcast"));
}

#[test]
fn test_slack_reply_message_minimal() {
    let json = r#"{"ts": "1234.0000"}"#;
    let reply: SlackReplyMessage = serde_json::from_str(json).unwrap();
    assert!(reply.user.is_none());
    assert!(reply.text.is_none());
    assert!(reply.bot_id.is_none());
}

#[test]
fn test_should_skip_thread_reply_for_bot_messages() {
    let reply = SlackReplyMessage {
        user: None,
        text: Some("Bot says hi".to_string()),
        ts: Some("1234.0001".to_string()),
        bot_id: Some("B999".to_string()),
        subtype: Some("bot_message".to_string()),
    };
    assert!(should_skip_thread_reply(&reply, None));
}

#[test]
fn test_should_not_skip_thread_reply_for_human_message() {
    let reply = SlackReplyMessage {
        user: Some("U123".to_string()),
        text: Some("hello".to_string()),
        ts: Some("1234.0002".to_string()),
        bot_id: None,
        subtype: None,
    };
    assert!(!should_skip_thread_reply(&reply, None));
}

#[tokio::test]
async fn test_inject_thread_context_empty_replies() {
    // When fetch returns empty, inject_thread_context should succeed as no-op
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
    let session_id = setup_test_session(&state.db).await;

    // inject with no network (fetch will fail gracefully → empty replies → Ok)
    let result = inject_thread_context(
        &state,
        "xoxb-fake",
        "C123",
        "1234.0000",
        session_id,
        Some("1234.9999"),
    )
    .await;
    assert!(result.is_ok());
}

/// EVE-1025: Slack verifies `settings.interactivity.request_url` when the
/// manifest is saved, which is why it is generated rather than added by hand.
#[test]
fn manifest_declares_the_interactivity_request_url() {
    let yaml = build_manifest_yaml(
        "My Bot",
        "My Bot",
        None,
        TEST_REQUEST_URL,
        TEST_INTERACTIVITY_URL,
        false,
        &[],
    );
    assert!(
        yaml.contains("  interactivity:"),
        "manifest must declare an interactivity block: {yaml}"
    );
    assert!(
        yaml.contains("    is_enabled: true"),
        "interactivity must be enabled or Slack ignores the URL: {yaml}"
    );
    assert!(
        yaml.contains(&format!("    request_url: \"{TEST_INTERACTIVITY_URL}\"")),
        "manifest must point interactivity at this endpoint: {yaml}"
    );
    // The events subscription must survive alongside it: both live under
    // `settings`, and an interactivity block that replaced them would silently
    // stop every incoming message.
    assert!(
        yaml.contains(&format!("    request_url: \"{TEST_REQUEST_URL}\"")),
        "manifest must keep the events request_url: {yaml}"
    );
}
