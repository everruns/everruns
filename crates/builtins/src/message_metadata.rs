//! MessageMetadata Capability — annotates user/agent messages with metadata
//! (currently the message timestamp) in the prompt-facing model view.
//!
//! Annotations are applied via [`ModelViewProvider`] at LLM message
//! construction time; stored messages are never modified. Timestamps come from
//! `Message::created_at`, which is immutable, so annotations are stable across
//! turns and do not invalidate provider prompt caches.

use std::sync::Arc;

use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};

use everruns_core::capabilities::{Capability, CapabilityLocalization, ModelViewContext, ModelViewProvider};
use everruns_core::message::{ContentPart, Message, MessageRole};

pub const MESSAGE_METADATA_CAPABILITY_ID: &str = "message_metadata";

/// A metadata field that can be annotated onto a message.
///
/// New fields (e.g. the LLM model that produced an agent message, once stored
/// messages record it) are added as variants here; each variant renders its
/// own bracketed segment and may return `None` when the message lacks the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageMetadataField {
    /// Message timestamp (`Message::created_at`), rendered as
    /// `[time <RFC3339 UTC>]`. For user messages this is when the message was
    /// received; for agent messages, when the reply was generated.
    Timestamp,
}

impl MessageMetadataField {
    fn render(&self, msg: &Message) -> Option<String> {
        match self {
            Self::Timestamp => Some(format!(
                "[time {}]",
                msg.created_at.to_rfc3339_opts(SecondsFormat::Secs, true)
            )),
        }
    }
}

/// Per-agent configuration for message metadata annotations.
///
/// User and agent messages are always annotated; only the rendered fields are
/// configurable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageMetadataConfig {
    /// Which metadata fields to annotate, in render order.
    #[serde(default = "default_fields")]
    pub fields: Vec<MessageMetadataField>,
}

impl Default for MessageMetadataConfig {
    fn default() -> Self {
        Self {
            fields: default_fields(),
        }
    }
}

fn default_fields() -> Vec<MessageMetadataField> {
    vec![MessageMetadataField::Timestamp]
}

impl MessageMetadataConfig {
    /// Parse from JSON value, falling back to defaults for invalid config.
    pub fn from_json(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

/// MessageMetadata capability — annotates conversation messages with metadata
/// (e.g. their timestamp) when they are sent to the LLM.
pub struct MessageMetadataCapability;

impl Capability for MessageMetadataCapability {
    fn id(&self) -> &str {
        MESSAGE_METADATA_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Message Metadata"
    }

    fn description(&self) -> &str {
        "Annotates user and agent messages with metadata (message timestamp, UTC) when building the LLM request, so the model can reason about timing and gaps between messages. Stored messages are unchanged."
    }

    fn icon(&self) -> Option<&str> {
        Some("clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Conversation messages carry a bracketed annotation added by the system, e.g. `[time 2026-06-11T09:15:42Z]` — the message's timestamp (UTC). Use it to reason about timing and gaps between messages. It is not part of what the author wrote; never emit such annotations in your replies.",
        )
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "fields": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "title": "Metadata field",
                        "description": "Metadata field rendered as a bracketed prefix on each message.",
                        "oneOf": [
                            { "const": "timestamp", "title": "Timestamp" }
                        ]
                    },
                    "default": ["timestamp"],
                    "title": "Metadata fields to annotate",
                    "description": "Which metadata fields are annotated onto user and agent messages, in render order. An empty list disables annotations."
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        serde_json::from_value::<MessageMetadataConfig>(config.clone())
            .map(|_| ())
            .map_err(|e| format!("invalid message_metadata config: {e}"))
    }

    fn model_view_provider(&self) -> Option<Arc<dyn ModelViewProvider>> {
        Some(Arc::new(MessageMetadataModelViewProvider))
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Choose which metadata fields are annotated onto messages sent to the LLM.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Метадані повідомлень"),
                description: Some(
                    "Додає до повідомлень користувача й агента метадані (часову позначку, UTC) \
                     під час формування запиту до LLM, щоб модель могла враховувати час і паузи \
                     між повідомленнями. Збережені повідомлення не змінюються.",
                ),
                config_description: Some(
                    "Визначає, які поля метаданих додаються до повідомлень, що надсилаються LLM.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "fields": {
                            "title": "Поля метаданих",
                            "description": "Які поля метаданих додаються до повідомлень користувача й агента, у порядку відображення. Порожній список вимикає анотації.",
                            "items": {
                                "title": "Поле метаданих",
                                "description": "Поле метаданих, що відображається як префікс у дужках для кожного повідомлення.",
                                "enum_labels": {
                                    "timestamp": "Часова позначка"
                                }
                            }
                        }
                    }
                })),
            },
        ]
    }
}

struct MessageMetadataModelViewProvider;

impl ModelViewProvider for MessageMetadataModelViewProvider {
    fn apply_model_view(
        &self,
        mut messages: Vec<Message>,
        config: &serde_json::Value,
        _context: &ModelViewContext<'_>,
    ) -> Vec<Message> {
        let config = MessageMetadataConfig::from_json(config);
        for msg in &mut messages {
            if matches!(msg.role, MessageRole::User | MessageRole::Agent) {
                annotate_message(msg, &config.fields);
            }
        }
        messages
    }

    /// After compaction masking (50) so annotations land on the final view.
    fn priority(&self) -> i32 {
        100
    }
}

/// Strip synthetic leading `[time <RFC3339 UTC>]` annotations from generated
/// assistant text before it is persisted.
///
/// The model view prepends `[time …]` to messages so the model can reason about
/// timing, with a system-prompt instruction never to emit it. Models
/// nevertheless echo the annotation into their replies (EVE-710); the echoed
/// text is then stored and shown to the user, breaking exact-output prompts.
/// This removes only the exact synthetic pattern — one or more leading
/// `[time <timestamp>]` segments, each followed by an optional single space —
/// validating the bracket contents as a real RFC3339 timestamp so legitimate
/// user-authored text like `[time to go]` is left untouched. Only leading
/// occurrences are removed; a bracketed timestamp later in the text is treated
/// as authored content and preserved.
pub fn strip_leading_timestamp_annotations(text: &str) -> String {
    let mut rest = text;
    while let Some(after) = strip_one_timestamp_annotation(rest) {
        rest = after;
    }
    rest.to_string()
}

/// Strip a single leading `[time <rfc3339>]` (plus one optional following space)
/// from `text`, returning the remainder when it matches the synthetic format.
fn strip_one_timestamp_annotation(text: &str) -> Option<&str> {
    const PREFIX: &str = "[time ";
    let rest = text.strip_prefix(PREFIX)?;
    let close = rest.find(']')?;
    // Only strip when the bracket holds a real RFC3339 timestamp — the exact
    // synthetic format produced by `MessageMetadataField::Timestamp::render`.
    chrono::DateTime::parse_from_rfc3339(&rest[..close]).ok()?;
    let after = &rest[close + 1..];
    Some(after.strip_prefix(' ').unwrap_or(after))
}

/// Render the combined metadata annotation for a message, e.g.
/// `[time 2026-06-11T09:15:42Z]`. Returns `None` when no field yields a value.
pub fn render_annotation(msg: &Message, fields: &[MessageMetadataField]) -> Option<String> {
    let segments: Vec<String> = fields.iter().filter_map(|f| f.render(msg)).collect();
    if segments.is_empty() {
        None
    } else {
        Some(segments.join(" "))
    }
}

fn annotate_message(msg: &mut Message, fields: &[MessageMetadataField]) {
    let Some(annotation) = render_annotation(msg, fields) else {
        return;
    };
    if let Some(ContentPart::Text(t)) = msg
        .content
        .iter_mut()
        .find(|p| matches!(p, ContentPart::Text(_)))
    {
        t.text = if t.text.is_empty() {
            annotation
        } else {
            format!("{annotation} {}", t.text)
        };
    } else {
        // No text part (e.g. tool-call-only agent message): carry the
        // annotation as its own leading text part.
        msg.content.insert(0, ContentPart::text(annotation));
    }
}

#[cfg(test)]
mod tests {
    use everruns_core::capabilities::*;
    use everruns_core::capabilities::CapabilityRegistry;
    use everruns_core::message::ToolCallContentPart;
    use everruns_core::typed_id::SessionId;

    fn ctx() -> ModelViewContext<'static> {
        ModelViewContext {
            session_id: SessionId::new(),
            prior_usage: None,
        }
    }

    fn apply(messages: Vec<Message>, config: serde_json::Value) -> Vec<Message> {
        MessageMetadataModelViewProvider.apply_model_view(messages, &config, &ctx())
    }

    fn time_annotation(msg: &Message) -> String {
        render_annotation(msg, &[MessageMetadataField::Timestamp]).unwrap()
    }

    // Metadata constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_capability_in_registry() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get(MESSAGE_METADATA_CAPABILITY_ID).unwrap();
        assert!(cap.model_view_provider().is_some());
    }

    #[test]
    fn test_annotates_user_and_agent_messages() {
        let user = Message::user("hello");
        let agent = Message::assistant("hi there");
        let expected_user = time_annotation(&user);
        let expected_agent = time_annotation(&agent);

        let out = apply(vec![user, agent], serde_json::json!({}));

        assert_eq!(
            out[0].text().unwrap(),
            format!("{expected_user} hello"),
            "user message gets timestamp prefix"
        );
        assert_eq!(out[1].text().unwrap(), format!("{expected_agent} hi there"));
    }

    #[test]
    fn test_skips_system_and_tool_result_messages() {
        let system = Message::system("you are a bot");
        let tool = Message::tool_result("call_1", Some(serde_json::json!({"ok": true})), None);

        let out = apply(vec![system, tool], serde_json::json!({}));

        assert_eq!(out[0].text().unwrap(), "you are a bot");
        assert!(out[1].text().is_none());
    }

    #[test]
    fn test_explicit_fields_config() {
        let user = Message::user("hello");
        let expected = time_annotation(&user);
        let out = apply(vec![user], serde_json::json!({"fields": ["timestamp"]}));
        assert_eq!(out[0].text().unwrap(), format!("{expected} hello"));
    }

    #[test]
    fn test_empty_fields_disable_annotations() {
        let user = Message::user("hello");
        let out = apply(vec![user], serde_json::json!({"fields": []}));
        assert_eq!(out[0].text().unwrap(), "hello");
        assert_eq!(out[0].content.len(), 1);
    }

    #[test]
    fn test_tool_call_only_agent_message_gets_text_part() {
        let mut agent = Message::assistant("");
        agent.content = vec![ContentPart::ToolCall(ToolCallContentPart::new(
            "call_1",
            "get_weather",
            serde_json::json!({}),
        ))];
        let expected = time_annotation(&agent);

        let out = apply(vec![agent], serde_json::json!({}));

        assert_eq!(out[0].content.len(), 2);
        assert_eq!(out[0].text().unwrap(), expected);
        assert!(matches!(out[0].content[1], ContentPart::ToolCall(_)));
    }

    #[test]
    fn test_empty_text_part_gets_annotation_without_trailing_space() {
        let agent = Message::assistant("");
        let expected = time_annotation(&agent);

        let out = apply(vec![agent], serde_json::json!({}));

        assert_eq!(out[0].text().unwrap(), expected);
    }

    #[test]
    fn test_annotation_format_is_rfc3339_utc() {
        let user = Message::user("hello");
        let out = apply(vec![user], serde_json::json!({}));
        let text = out[0].text().unwrap();
        assert!(text.starts_with("[time 2"), "got: {text}");
        assert!(text.contains("Z] hello"), "got: {text}");
    }

    // EVE-710: models echo the synthetic `[time …]` prefix into their replies;
    // it must be stripped before the assistant text is persisted.
    #[test]
    fn strip_removes_single_leading_annotation() {
        assert_eq!(
            strip_leading_timestamp_annotations("[time 2026-07-10T05:38:28Z] cobalt"),
            "cobalt"
        );
    }

    // EVE-710: duplicated prefixes (observed in the wild) are all removed.
    #[test]
    fn strip_removes_repeated_leading_annotations() {
        assert_eq!(
            strip_leading_timestamp_annotations(
                "[time 2026-07-10T05:38:21Z] [time 2026-07-10T05:38:21Z] Understood"
            ),
            "Understood"
        );
    }

    // EVE-710: an annotation with no following space still strips cleanly.
    #[test]
    fn strip_handles_annotation_without_trailing_space() {
        assert_eq!(
            strip_leading_timestamp_annotations("[time 2026-07-10T05:38:28Z]hi"),
            "hi"
        );
    }

    // EVE-710: legitimate user-authored bracket text that is not a timestamp is
    // preserved.
    #[test]
    fn strip_preserves_non_timestamp_bracket_text() {
        assert_eq!(
            strip_leading_timestamp_annotations("[time to go] home"),
            "[time to go] home"
        );
        assert_eq!(
            strip_leading_timestamp_annotations("hello world"),
            "hello world"
        );
    }

    // EVE-710: a real timestamp appearing later in the text is authored content,
    // not the synthetic leading prefix, and is kept.
    #[test]
    fn strip_only_touches_leading_annotation() {
        assert_eq!(
            strip_leading_timestamp_annotations("see [time 2026-07-10T05:38:28Z]"),
            "see [time 2026-07-10T05:38:28Z]"
        );
    }

    // EVE-710: stripping is the inverse of the model-view annotation, so an
    // agent reply that merely echoed the injected prefix round-trips to the
    // author's original text.
    #[test]
    fn strip_inverts_render_annotation() {
        let agent = Message::assistant("the answer");
        let annotated = format!("{} the answer", time_annotation(&agent));
        assert_eq!(
            strip_leading_timestamp_annotations(&annotated),
            "the answer"
        );
    }

    #[test]
    fn test_validate_config() {
        let cap = MessageMetadataCapability;
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(cap.validate_config(&serde_json::json!({})).is_ok());
        assert!(
            cap.validate_config(&serde_json::json!({"fields": ["timestamp"]}))
                .is_ok()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"fields": []}))
                .is_ok()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"fields": ["model"]}))
                .is_err(),
            "unknown metadata fields are rejected until implemented"
        );
        assert!(
            cap.validate_config(&serde_json::json!({"fields": "timestamp"}))
                .is_err(),
            "fields must be an array"
        );
        assert!(
            cap.validate_config(&serde_json::json!({"user_messages": false}))
                .is_err(),
            "role toggles were removed; user/agent messages are always annotated"
        );
        assert!(
            cap.validate_config(&serde_json::json!({"unknown": true}))
                .is_err()
        );
    }

    /// Keeps `config_schema()` honest against the serde config shape, since
    /// there is no compile-time link between the two.
    #[test]
    fn test_config_schema_matches_config_shape() {
        let cap = MessageMetadataCapability;
        let schema = cap.config_schema().expect("capability exposes a schema");

        assert_eq!(schema["type"], "object");
        assert_eq!(
            schema["additionalProperties"], false,
            "schema must reject unknown keys like validate_config does"
        );

        // Schema properties == serde fields of the default config.
        let schema_keys: std::collections::BTreeSet<&str> = schema["properties"]
            .as_object()
            .expect("properties object")
            .keys()
            .map(String::as_str)
            .collect();
        let config_value = serde_json::to_value(MessageMetadataConfig::default()).unwrap();
        let config_keys: std::collections::BTreeSet<&str> = config_value
            .as_object()
            .expect("config serializes to object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(schema_keys, config_keys);

        // The schema's labeled oneOf of field names matches
        // MessageMetadataField's serde names, and its default parses as a
        // valid config.
        let enum_values: Vec<serde_json::Value> = schema["properties"]["fields"]["items"]["oneOf"]
            .as_array()
            .expect("fields oneOf")
            .iter()
            .map(|option| option["const"].clone())
            .collect();
        for value in &enum_values {
            assert!(
                serde_json::from_value::<MessageMetadataField>(value.clone()).is_ok(),
                "schema oneOf const {value} is not a known MessageMetadataField"
            );
        }
        assert_eq!(
            enum_values.len(),
            1,
            "add new MessageMetadataField variants to the schema oneOf"
        );
        let schema_default = serde_json::json!({
            "fields": schema["properties"]["fields"]["default"]
        });
        assert!(cap.validate_config(&schema_default).is_ok());
    }
}
