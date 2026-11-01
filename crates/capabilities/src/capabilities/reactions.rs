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

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
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
            "You can react to messages with `react`. Use it sparingly, as a person would: 👀 when you start on a longer request, ✅ when it is done. People's reactions to your messages arrive as notes labeled \"[Reactions, not typed messages]\"; they are feedback, not new requests.",
        )
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "when_someone_reacts": {
                    "type": "string",
                    "enum": ["next_turn", "start_turn", "off"],
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
}
