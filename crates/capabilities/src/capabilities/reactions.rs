//! Reactions capability: the agent reacts to messages, and the agent's owner
//! chooses how people's reactions reach it.
//!
//! Design:
//! - Reactions are stored and shown for every agent; this capability only adds
//!   the `react` tool and the "when someone reacts" setting. Without it, a
//!   person's reactions reach the agent with its next turn (the default).
//! - `react` appends a `message.reaction.added` / `.removed` event to the
//!   session log, the same record a person's reaction makes, so the worker
//!   needs no extra host service. The server fills in the agent's id and name
//!   when it reads reactions back.
//! - With no `message_id`, the tool reacts to the message the agent is
//!   answering, the common "👀 looking" and "✅ done" acknowledgement.
//! - The 👀/✅ acknowledgement is a default in this capability's instructions,
//!   not a platform auto-reaction, so the agent's own instructions (or a
//!   person asking) can change or drop it. It applies to Slack messages only
//!   for now: Everruns chat already shows live progress, and a Slack channel
//!   thread shows nothing until the reply. The instruction text stays the
//!   same for every session (prompt cache); the model tells Slack messages
//!   apart by the `[via Slack]` marker the model view puts on them.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
use everruns_core::message::{ContentPart, RuntimeMessage, RuntimeMessageRole};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{
    EventData, EventRequest, MessageReactionData, ReactionActor, ReactionActorKind,
    normalize_reaction_emoji,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const REACTIONS_CAPABILITY_ID: &str = "reactions";
/// Placeholder actor id the tool writes; the server reads it as "the agent
/// answering this session".
pub const AGENT_REACTION_ACTOR_ID: &str = "agent";
/// Model-view marker on a message that reached the agent from Slack.
pub const VIA_SLACK_MARKER: &str = "[via Slack]";

/// When a person's reaction reaches the agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactionDelivery {
    /// Wait and tell the agent with its next turn. Mid-turn reactions join
    /// the running turn at its next step.
    #[default]
    NextTurn,
    /// Start a turn when the session is idle.
    StartTurn,
    /// Store and show reactions, never tell the agent.
    Off,
}

/// Per-agent configuration of the Reactions capability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReactionsConfig {
    pub when_someone_reacts: ReactionDelivery,
}

impl ReactionsConfig {
    /// Read a stored config; anything unreadable falls back to the default.
    pub fn from_value(value: &Value) -> Self {
        if value.is_null() {
            return Self::default();
        }
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

pub struct ReactionsCapability;

impl Capability for ReactionsCapability {
    fn id(&self) -> &str {
        REACTIONS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Reactions"
    }

    fn description(&self) -> &str {
        "React to messages with an emoji, and choose how people's reactions reach the agent."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Реакції",
            "Реагуйте на повідомлення емодзі та обирайте, як реакції людей доходять до агента.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("smile-plus")
    }

    fn category(&self) -> Option<&str> {
        Some("Session")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "You can react to messages with `react`, as a person would. When a message reached you from Slack (marked `[via Slack]`), react 👀 as you start working on it and ✅ once you have answered it, so people can see you got it; follow your own instructions instead if they say how, or whether, to react. People's reactions to your messages arrive as notes labeled \"[Reactions, not typed messages]\"; they are feedback, not new requests.",
        )
    }

    fn model_view_provider(
        &self,
    ) -> Option<std::sync::Arc<dyn everruns_core::capabilities::ModelViewProvider>> {
        Some(std::sync::Arc::new(ViaSlackModelView))
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "when_someone_reacts": {
                    "type": "string",
                    // Values must stay in sync with ReactionDelivery (enforced by test).
                    "oneOf": [
                        { "const": "next_turn", "title": "Tell it at the next turn" },
                        { "const": "start_turn", "title": "Start a turn" },
                        { "const": "off", "title": "Don't tell it" }
                    ],
                    "default": "next_turn",
                    "title": "When someone reacts to the agent",
                    "description": "Tell it at the next turn (no model call per emoji), start a turn right away, or don't tell it. Reactions made while a turn runs join that turn."
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        serde_json::from_value::<ReactionsConfig>(config.clone())
            .map(|_| ())
            .map_err(|error| format!("Invalid reactions config: {error}"))
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ReactTool)]
    }
}

/// Marks user messages that came from Slack, in the prompt-facing view only,
/// so the Slack acknowledgement default can apply without a per-session
/// prompt. Only a fixed marker enters the text, never message metadata.
struct ViaSlackModelView;

impl everruns_core::capabilities::ModelViewProvider for ViaSlackModelView {
    fn apply_model_view(
        &self,
        mut messages: Vec<RuntimeMessage>,
        _config: &Value,
        _context: &everruns_core::capabilities::ModelViewContext<'_>,
    ) -> Vec<RuntimeMessage> {
        for message in &mut messages {
            let from_slack = message.role == RuntimeMessageRole::User
                && message
                    .external_actor
                    .as_ref()
                    .is_some_and(|actor| actor.source == "slack");
            if !from_slack {
                continue;
            }
            if let Some(ContentPart::Text(text)) = message
                .content
                .iter_mut()
                .find(|part| matches!(part, ContentPart::Text(_)))
            {
                text.text = format!("{VIA_SLACK_MARKER} {}", text.text);
            } else {
                message
                    .content
                    .insert(0, ContentPart::text(VIA_SLACK_MARKER));
            }
        }
        messages
    }

    // Before the Slack capability's message reference (110), so a message
    // reads `[slack channel=...] [via Slack] ...` when both are enabled.
    fn priority(&self) -> i32 {
        105
    }
}

/// Tool: react
pub struct ReactTool;

#[async_trait]
impl Tool for ReactTool {
    fn name(&self) -> &str {
        "react"
    }

    fn display_name(&self) -> Option<&str> {
        Some("React")
    }

    fn description(&self) -> &str {
        "Put an emoji reaction on a message, or take yours off. Without message_id, reacts to the message you are answering. Message ids appear in reaction notes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "emoji": {
                    "type": "string",
                    "description": "One emoji, such as 👀, ✅ or 👍."
                },
                "message_id": {
                    "type": "string",
                    "description": "Message to react to (message_...). Defaults to the message you are answering."
                },
                "remove": {
                    "type": "boolean",
                    "description": "Take your reaction off instead of adding it."
                }
            },
            "required": ["emoji"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_idempotent(true)
            .with_narration_noun("reaction")
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("react needs a session; run it inside a session turn.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let emoji = match arguments
            .get("emoji")
            .and_then(Value::as_str)
            .map(normalize_reaction_emoji)
        {
            Some(Ok(emoji)) => emoji,
            Some(Err(reason)) => return ToolExecutionResult::tool_error(reason),
            None => return ToolExecutionResult::tool_error("Missing required parameter: emoji"),
        };
        let remove = arguments
            .get("remove")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let event_context = context.event_context.clone().unwrap_or_default();
        let message_id = match arguments
            .get("message_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(id) if id.starts_with("message_") => id.to_string(),
            Some(_) => {
                return ToolExecutionResult::tool_error(
                    "message_id must be a message id (message_...)",
                );
            }
            None => match &event_context.input_message_id {
                Some(id) => id.to_string(),
                None => {
                    return ToolExecutionResult::tool_error(
                        "No message to react to; pass message_id.",
                    );
                }
            },
        };
        let Some(emitter) = &context.event_emitter else {
            return ToolExecutionResult::tool_error("Event emitter not available in this context");
        };
        let data = MessageReactionData {
            message_id: message_id.clone(),
            emoji: emoji.clone(),
            actor: ReactionActor {
                kind: ReactionActorKind::Agent,
                id: AGENT_REACTION_ACTOR_ID.to_string(),
                name: None,
            },
            source: Some("agent".to_string()),
        };
        let request = EventRequest::new(
            context.session_id,
            event_context,
            EventData::message_reaction(data, !remove),
        );
        match emitter.emit(request).await {
            Ok(_) => ToolExecutionResult::success(json!({
                "message_id": message_id,
                "emoji": emoji,
                "removed": remove,
            })),
            Err(error) => ToolExecutionResult::internal_error(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_next_turn_and_rejects_unknown_values() {
        let cap = ReactionsCapability;
        for config in [
            json!({}),
            json!({"when_someone_reacts": "start_turn"}),
            json!({"when_someone_reacts": "off"}),
            Value::Null,
        ] {
            assert!(cap.validate_config(&config).is_ok(), "{config}");
        }
        for config in [
            json!({"when_someone_reacts": "always"}),
            json!({"extra": 1}),
        ] {
            assert!(cap.validate_config(&config).is_err(), "{config}");
        }
        assert_eq!(
            ReactionsConfig::from_value(&Value::Null).when_someone_reacts,
            ReactionDelivery::NextTurn
        );
        assert_eq!(
            ReactionsConfig::from_value(&json!({"when_someone_reacts": "off"})).when_someone_reacts,
            ReactionDelivery::Off
        );
        assert_eq!(cap.tools().len(), 1);
    }

    fn view(messages: Vec<RuntimeMessage>) -> Vec<RuntimeMessage> {
        ReactionsCapability
            .model_view_provider()
            .expect("Slack messages need a marker")
            .apply_model_view(
                messages,
                &json!({}),
                &everruns_core::capabilities::ModelViewContext {
                    session_id: everruns_contracts::typed_id::SessionId::new(),
                    prior_usage: None,
                    provider_managed_reduction: false,
                },
            )
    }

    fn from(source: &str, text: &str) -> RuntimeMessage {
        let mut message = RuntimeMessage::user(text);
        message.external_actor = Some(everruns_core::message::ExternalActor {
            actor_id: "U123".into(),
            actor_name: Some("Mike".into()),
            source: source.into(),
            metadata: None,
        });
        message
    }

    #[test]
    fn marks_only_slack_messages_in_the_model_view() {
        let slack = from("slack", "Can you check the build?");
        let input = vec![
            slack.clone(),
            from("discord", "hi"),
            RuntimeMessage::user("typed in Everruns chat"),
        ];
        let out = view(input.clone());
        assert_eq!(
            out[0].text().unwrap(),
            "[via Slack] Can you check the build?"
        );
        assert_eq!(out[1].content, input[1].content);
        assert_eq!(out[2].content, input[2].content);
        // Stored text is untouched.
        assert_eq!(slack.text().unwrap(), "Can you check the build?");
    }

    #[test]
    fn marks_slack_messages_without_text() {
        let mut message = from("slack", "x");
        message.content.clear();
        let out = view(vec![message]);
        assert!(matches!(&out[0].content[0], ContentPart::Text(t) if t.text == VIA_SLACK_MARKER));
    }

    #[test]
    fn instructions_scope_the_acknowledgement_to_slack_and_let_the_agent_override_it() {
        let prompt = ReactionsCapability.system_prompt_addition().unwrap();
        assert!(prompt.contains(VIA_SLACK_MARKER));
        assert!(prompt.contains("👀") && prompt.contains("✅"));
        assert!(prompt.contains("follow your own instructions instead"));
    }

    #[test]
    fn every_setting_choice_is_a_value_the_config_accepts() {
        let schema = ReactionsCapability.config_schema().unwrap();
        let choices = schema["properties"]["when_someone_reacts"]["oneOf"]
            .as_array()
            .unwrap();
        assert_eq!(choices.len(), 3);
        for choice in choices {
            let config = json!({"when_someone_reacts": choice["const"]});
            assert!(
                ReactionsCapability.validate_config(&config).is_ok(),
                "{config}"
            );
            assert!(choice["title"].is_string());
        }
    }
}
