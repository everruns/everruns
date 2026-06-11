//! MessageMetadata Capability — annotates user/agent messages with metadata
//! (currently the message sent time) in the prompt-facing model view.
//!
//! Annotations are applied via [`ModelViewProvider`] at LLM message
//! construction time; stored messages are never modified. Timestamps come from
//! `Message::created_at`, which is immutable, so annotations are stable across
//! turns and do not invalidate provider prompt caches.

use std::sync::Arc;

use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};

use super::{Capability, ModelViewContext, ModelViewProvider};
use crate::message::{ContentPart, Message, MessageRole};

pub const MESSAGE_METADATA_CAPABILITY_ID: &str = "message_metadata";

/// Per-agent configuration for message metadata annotations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageMetadataConfig {
    /// Annotate user messages with their sent time.
    #[serde(default = "default_true")]
    pub user_messages: bool,
    /// Annotate agent messages with their sent time.
    #[serde(default = "default_true")]
    pub agent_messages: bool,
}

impl Default for MessageMetadataConfig {
    fn default() -> Self {
        Self {
            user_messages: default_true(),
            agent_messages: default_true(),
        }
    }
}

fn default_true() -> bool {
    true
}

impl MessageMetadataConfig {
    /// Parse from JSON value, falling back to defaults for invalid config.
    pub fn from_json(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

/// MessageMetadata capability — annotates conversation messages with sent-time
/// metadata when they are sent to the LLM.
pub struct MessageMetadataCapability;

impl Capability for MessageMetadataCapability {
    fn id(&self) -> &str {
        MESSAGE_METADATA_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Message Metadata"
    }

    fn description(&self) -> &str {
        "Annotates user and agent messages with metadata (sent time, UTC) when building the LLM request, so the model can reason about timing and gaps between messages. Stored messages are unchanged."
    }

    fn icon(&self) -> Option<&str> {
        Some("clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Utilities")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Conversation messages carry a bracketed annotation added by the system, e.g. `[sent 2026-06-11T09:15:42Z]` (UTC). Use it to reason about timing and gaps between messages. It is not part of what the author wrote; never emit such annotations in your replies.",
        )
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "user_messages": {
                    "type": "boolean",
                    "default": true,
                    "title": "Annotate user messages"
                },
                "agent_messages": {
                    "type": "boolean",
                    "default": true,
                    "title": "Annotate agent messages"
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
            let annotate = match msg.role {
                MessageRole::User => config.user_messages,
                MessageRole::Agent => config.agent_messages,
                MessageRole::System | MessageRole::ToolResult => false,
            };
            if annotate {
                annotate_message(msg);
            }
        }
        messages
    }

    /// After compaction masking (50) so annotations land on the final view.
    fn priority(&self) -> i32 {
        100
    }
}

/// Render the metadata annotation for a message.
pub fn sent_annotation(msg: &Message) -> String {
    format!(
        "[sent {}]",
        msg.created_at.to_rfc3339_opts(SecondsFormat::Secs, true)
    )
}

fn annotate_message(msg: &mut Message) {
    let annotation = sent_annotation(msg);
    if let Some(ContentPart::Text(t)) = msg
        .content
        .iter_mut()
        .find(|p| matches!(p, ContentPart::Text(_)))
    {
        t.text = format!("{annotation} {}", t.text);
    } else {
        // No text part (e.g. tool-call-only agent message): carry the
        // annotation as its own leading text part.
        msg.content.insert(0, ContentPart::text(annotation));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;
    use crate::message::ToolCallContentPart;
    use crate::typed_id::SessionId;

    fn ctx() -> ModelViewContext<'static> {
        ModelViewContext {
            session_id: SessionId::new(),
            prior_usage: None,
        }
    }

    fn apply(messages: Vec<Message>, config: serde_json::Value) -> Vec<Message> {
        MessageMetadataModelViewProvider.apply_model_view(messages, &config, &ctx())
    }

    #[test]
    fn test_capability_metadata() {
        let cap = MessageMetadataCapability;
        assert_eq!(cap.id(), "message_metadata");
        assert_eq!(cap.name(), "Message Metadata");
        assert_eq!(cap.category(), Some("Utilities"));
        assert!(cap.system_prompt_addition().is_some());
        assert!(cap.tools().is_empty());
    }

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
        let expected_user = sent_annotation(&user);
        let expected_agent = sent_annotation(&agent);

        let out = apply(vec![user, agent], serde_json::json!({}));

        assert_eq!(
            out[0].text().unwrap(),
            format!("{expected_user} hello"),
            "user message gets sent-time prefix"
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
    fn test_config_disables_roles() {
        let user = Message::user("hello");
        let agent = Message::assistant("hi");
        let expected_agent = sent_annotation(&agent);

        let out = apply(
            vec![user, agent],
            serde_json::json!({"user_messages": false}),
        );

        assert_eq!(out[0].text().unwrap(), "hello");
        assert_eq!(out[1].text().unwrap(), format!("{expected_agent} hi"));
    }

    #[test]
    fn test_tool_call_only_agent_message_gets_text_part() {
        let mut agent = Message::assistant("");
        agent.content = vec![ContentPart::ToolCall(ToolCallContentPart::new(
            "call_1",
            "get_weather",
            serde_json::json!({}),
        ))];
        let expected = sent_annotation(&agent);

        let out = apply(vec![agent], serde_json::json!({}));

        assert_eq!(out[0].content.len(), 2);
        assert_eq!(out[0].text().unwrap(), expected);
        assert!(matches!(out[0].content[1], ContentPart::ToolCall(_)));
    }

    #[test]
    fn test_annotation_format_is_rfc3339_utc() {
        let user = Message::user("hello");
        let out = apply(vec![user], serde_json::json!({}));
        let text = out[0].text().unwrap();
        assert!(text.starts_with("[sent 2"), "got: {text}");
        assert!(text.contains("Z] hello"), "got: {text}");
    }

    #[test]
    fn test_validate_config() {
        let cap = MessageMetadataCapability;
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(cap.validate_config(&serde_json::json!({})).is_ok());
        assert!(
            cap.validate_config(&serde_json::json!({"user_messages": false}))
                .is_ok()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"user_messages": "nope"}))
                .is_err()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"unknown": true}))
                .is_err()
        );
    }
}
