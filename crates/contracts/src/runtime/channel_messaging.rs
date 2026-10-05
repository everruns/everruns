//! Explicit, channel-neutral communication. Hosts bind a sender to one session;
//! the tool chooses content, never credentials or an arbitrary destination.

use std::sync::Arc;

use crate::{tool_types::ToolHints, typed_id::MessageId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::runtime::tools::{Tool, ToolExecutionResult};
use crate::runtime::{RuntimeAgent, channel::ChannelReplyMode, tool_context::ToolContext};

/// Agent-facing tool name shared by every messaging transport.
pub const CHANNEL_POST_MESSAGE_TOOL_NAME: &str = "channel_post_message";
/// Maximum Unicode characters in a single explicit message.
pub const MAX_CHANNEL_MESSAGE_CHARS: usize = 12_000;
/// Generic reply-mode tag namespace.
pub const CHANNEL_REPLY_MODE_TAG_PREFIX: &str = "channel:reply_mode:";
/// Mode where assistant text stays in the session and tools publish messages.
pub const CHANNEL_TOOL_ONLY_TAG: &str = "channel:reply_mode:tool_only";
/// Slack's persisted reply-mode tag namespace.
pub const SLACK_REPLY_MODE_TAG_PREFIX: &str = "slack:reply_mode:";
/// Slack alias for the generic tool-only mode.
pub const SLACK_TOOL_ONLY_TAG: &str = "slack:reply_mode:tool_only";

const PROMPT_MARKER: &str = "# Channel Communication";
const SYSTEM_PROMPT: &str = r#"# Channel Communication

This session is attached to an external conversation. Normal assistant messages stay in Everruns and are not sent to that conversation.

Use `channel_post_message` to send user-facing updates, questions, and final answers. You choose the wording and when to communicate; the runtime selects the conversation that triggered this turn.
- Post concise, meaningful updates during longer work. Avoid low-level tool chatter or repeated acknowledgements.
- Send questions and final answers through `channel_post_message` before ending the turn.
- A successful result confirms the channel accepted the message and includes its message reference. Do not post the same content again as an assistant reply.
- If delivery fails, use the error to decide what to do. An uncertain result may already have been posted; do not blindly resend it.
- Platform reactions, edits, uploads, and approval tools remain available when enabled."#;

/// A message accepted by the destination platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMessageReceipt {
    /// Destination platform, such as Slack.
    pub platform: String,
    /// Platform channel or conversation identifier.
    pub channel: String,
    /// Platform message identifier, usable by platform-specific editing tools.
    pub message_ref: String,
}

/// Host service bound to one session's channel identity.
#[async_trait]
pub trait ChannelMessageSender: Send + Sync {
    /// Post to the trusted conversation of this input invocation. Identifiers
    /// come from the executor, not model arguments. Return only after acceptance.
    async fn post_message(
        &self,
        text: &str,
        input_message_id: MessageId,
        tool_call_id: &str,
    ) -> Result<ChannelMessageReceipt, ToolExecutionResult>;
}

/// Sender installed by a host into the tool-context extension bag.
#[derive(Clone)]
pub struct ChannelMessageSenderExt(pub Arc<dyn ChannelMessageSender>);

/// Recognize existing stored progress-only sessions without retaining the old tool.
pub fn session_uses_channel_tools(tags: &[String]) -> bool {
    tags.iter().any(|tag| {
        matches!(
            tag.as_str(),
            CHANNEL_TOOL_ONLY_TAG
                | SLACK_TOOL_ONLY_TAG
                | "channel:reply_mode:report_progress_only"
                | "slack:reply_mode:report_progress_only"
        )
    })
}

/// Replace stale reply-mode tags while preserving unrelated session metadata.
pub fn sync_channel_reply_mode_tags(tags: &mut Vec<String>, mode: ChannelReplyMode) {
    tags.retain(|tag| !tag.starts_with(CHANNEL_REPLY_MODE_TAG_PREFIX));
    if mode == ChannelReplyMode::ToolOnly {
        tags.push(CHANNEL_TOOL_ONLY_TAG.into());
    }
}

/// Normalize Slack and generic mode tags together when ingress reuses a session.
pub fn sync_slack_reply_mode_tags(tags: &mut Vec<String>, mode: ChannelReplyMode) {
    tags.retain(|tag| !tag.starts_with(SLACK_REPLY_MODE_TAG_PREFIX));
    if mode == ChannelReplyMode::ToolOnly {
        tags.push(SLACK_TOOL_ONLY_TAG.into());
    }
    sync_channel_reply_mode_tags(tags, mode);
}

/// Expose explicit communication and instructions only for tool-only sessions.
pub fn apply_channel_message_mode(mut agent: RuntimeAgent) -> RuntimeAgent {
    if !agent
        .tools
        .iter()
        .any(|tool| tool.name() == CHANNEL_POST_MESSAGE_TOOL_NAME)
    {
        agent.tools.push(ChannelPostMessageTool.to_definition());
    }
    if !agent.system_prompt.contains(PROMPT_MARKER) {
        agent.system_prompt = format!("{SYSTEM_PROMPT}\n\n{}", agent.system_prompt);
    }
    agent
}

/// Post an agent-composed message to the current external conversation.
pub struct ChannelPostMessageTool;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageArguments {
    text: String,
}

#[async_trait]
impl Tool for ChannelPostMessageTool {
    fn name(&self) -> &str {
        CHANNEL_POST_MESSAGE_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("Post to conversation")
    }

    fn description(&self) -> &str {
        "Post a message to the external conversation that triggered this turn. Use for meaningful updates, questions, and final answers. Write the complete user-facing message in Markdown; no status heading is added. The runtime selects the destination and channel identity. Success confirms delivery and returns a message reference for later edits. Normal assistant replies are not published in agent-controlled mode."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type":"object", "properties": {"text": {
            "type":"string", "minLength":1, "maxLength":MAX_CHANNEL_MESSAGE_CHARS,
            "description":"Complete message to send to the user. Supports Markdown. Use concise updates, clear questions, or the final answer."
        }}, "required":["text"], "additionalProperties":false})
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn hints(&self) -> ToolHints {
        // A network failure after acceptance is ambiguous. Durable Act must
        // never replay this side effect as if it were a pure reporting payload.
        ToolHints::default()
            .with_readonly(false)
            .with_idempotent(false)
    }

    fn narrate(
        &self,
        _call: &crate::tool_types::ToolCall,
        phase: crate::runtime::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: crate::runtime::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        use crate::runtime::tool_narration::ToolNarrationPhase;
        Some(
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                    "Posting to conversation"
                }
                ToolNarrationPhase::Completed => "Posted to conversation",
                ToolNarrationPhase::Failed => "Could not post to conversation",
            }
            .into(),
        )
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "Posting requires an external conversation and invocation context",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let args = match serde_json::from_value::<MessageArguments>(arguments) {
            Ok(args) => args,
            Err(error) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid channel_post_message arguments: {error}"
                ));
            }
        };
        if args.text.trim().is_empty() {
            return ToolExecutionResult::tool_error("Message text must not be empty");
        }
        if args.text.chars().count() > MAX_CHANNEL_MESSAGE_CHARS {
            return ToolExecutionResult::tool_error(
                "Message text must not exceed 12000 characters; share a file for longer content",
            );
        }
        let Some(sender) = context.extensions.get::<ChannelMessageSenderExt>() else {
            return self.execute(Value::Null).await;
        };
        let Some(input_id) = context
            .event_context
            .as_ref()
            .and_then(|ctx| ctx.input_message_id)
        else {
            return self.execute(Value::Null).await;
        };
        let Some(call_id) = context.tool_call_id.as_deref().filter(|id| !id.is_empty()) else {
            return self.execute(Value::Null).await;
        };
        match sender.0.post_message(&args.text, input_id, call_id).await {
            Ok(receipt) => ToolExecutionResult::success(json!({
                "delivered":true, "platform":receipt.platform,
                "channel":receipt.channel, "message_ref":receipt.message_ref
            })),
            Err(error) => error,
        }
    }
}

#[cfg(test)]
mod tests;
