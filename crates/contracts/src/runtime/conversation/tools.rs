//! The tools an agent that talks explicitly uses to talk.
//!
//! `send_message` says something to the conversation; `no_reply` records a
//! decision to stay quiet. The destination is always the conversation of the
//! input that started the turn, chosen by the host, never by the model.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::runtime::events::{ConversationDelivery, ConversationMessageData};
use crate::runtime::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
use crate::runtime::tools::{Tool, ToolExecutionResult};
use crate::runtime::{RuntimeAgent, tool_context::ToolContext};
use crate::{tool_types::ToolHints, typed_id::MessageId};

use super::Communication;

/// Tool an explicit agent talks with.
pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";
/// Tool an explicit agent uses to end a turn without saying anything.
pub const NO_REPLY_TOOL_NAME: &str = "no_reply";
/// Maximum Unicode characters in one sent message.
pub const MAX_MESSAGE_CHARS: usize = 12_000;

const PROMPT_MARKER: &str = "# How you talk";
const SYSTEM_PROMPT: &str = r#"# How you talk

People do not see what you write as an assistant. That text is your private working notes: think, plan and keep track there.

To say something to the person, call `send_message`. Everything they see from you comes through it.
- Send a message when you have something worth saying: an answer, a question, a short progress update during longer work, or a result. Write it complete, in Markdown, for the person reading it.
- Do not narrate tool calls or repeat yourself. One clear message beats several small ones.
- A successful result means the message was delivered. Do not send the same content again.
- If sending fails, read the error. An uncertain failure may already have been delivered, so do not blindly resend.
- When a message needs no answer from you (an acknowledgement, people talking among themselves), call `no_reply` instead of sending something empty."#;

/// Delivers sent messages to an external conversation, such as a Slack thread.
///
/// A host installs one when a session's conversation lives on another
/// platform. Without one, the session itself is the conversation and the
/// `conversation.message` event is the delivery.
#[async_trait]
pub trait ConversationSender: Send + Sync {
    /// Deliver `text` to the conversation that `input_message_id` came from.
    ///
    /// Identifiers come from the executor, never from model arguments. Return
    /// only after the platform accepted the message, or `Ok(None)` when that
    /// input has no external conversation (it was typed in the web app, for
    /// example), in which case the session is the conversation.
    async fn send(
        &self,
        text: &str,
        input_message_id: MessageId,
        tool_call_id: &str,
    ) -> Result<Option<ConversationDelivery>, ToolExecutionResult>;
}

/// Sender installed by a host into the tool-context extension bag.
#[derive(Clone)]
pub struct ConversationSenderExt(pub Arc<dyn ConversationSender>);

/// Turn an agent into one that talks explicitly: add the two tools and the
/// instructions, and mark the agent so its assistant text is recorded as
/// working notes. Idempotent.
pub fn apply_explicit_communication(mut agent: RuntimeAgent) -> RuntimeAgent {
    for definition in [SendMessageTool.to_definition(), NoReplyTool.to_definition()] {
        if !agent
            .tools
            .iter()
            .any(|tool| tool.name() == definition.name())
        {
            agent.tools.push(definition);
        }
    }
    if !agent.system_prompt.contains(PROMPT_MARKER) {
        agent.system_prompt = format!("{SYSTEM_PROMPT}\n\n{}", agent.system_prompt);
    }
    agent.communication = Communication::Explicit;
    agent
}

/// The `conversation.message` a settled tool call produced, if it was a
/// delivered `send_message`. The engine records it next to the tool result, so
/// every host emits the same event no matter which sender delivered it.
pub fn sent_message(
    tool_name: &str,
    tool_call_id: &str,
    arguments: &Value,
    result: Option<&Value>,
) -> Option<ConversationMessageData> {
    if tool_name != SEND_MESSAGE_TOOL_NAME {
        return None;
    }
    let result = result?;
    if result.get("sent").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let message_id = serde_json::from_value(result.get("message_id")?.clone()).ok()?;
    let text = arguments.get("text")?.as_str()?.to_owned();
    let delivery = result
        .get("delivery")
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    Some(ConversationMessageData {
        message_id,
        text,
        tool_call_id: tool_call_id.to_owned(),
        delivery,
    })
}

/// Say something to the conversation.
pub struct SendMessageTool;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendArguments {
    text: String,
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        SEND_MESSAGE_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("Send message")
    }

    fn description(&self) -> &str {
        "Send a message to the person you are talking with. This is the only way they see anything from you; your assistant text is private notes. Use it for answers, questions, results and meaningful progress updates. Write the complete message in Markdown. The runtime picks the destination: the conversation this turn came from. Success means it was delivered."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type":"object", "properties": {"text": {
            "type":"string", "minLength":1, "maxLength":MAX_MESSAGE_CHARS,
            "description":"The complete message for the person. Markdown."
        }}, "required":["text"], "additionalProperties":false})
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn hints(&self) -> ToolHints {
        // A failure after the platform accepted the message is ambiguous.
        // Durable replay must never send it twice.
        ToolHints::default()
            .with_readonly(false)
            .with_idempotent(false)
    }

    fn narrate(
        &self,
        call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        // The text is already meant for the person, unlike prompts or file bodies.
        Some(crate::runtime::tool_narration::narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            ("Sending message", "Sent message", "Could not send message"),
            (
                "Надсилаю повідомлення",
                "Надіслав повідомлення",
                "Не вдалося надіслати повідомлення",
            ),
            &["text"],
        ))
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        self.execute_with_context(
            arguments,
            &ToolContext::new(crate::runtime::typed_id::SessionId::new()),
        )
        .await
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let args = match serde_json::from_value::<SendArguments>(arguments) {
            Ok(args) => args,
            Err(error) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid send_message arguments: {error}"
                ));
            }
        };
        if args.text.trim().is_empty() {
            return ToolExecutionResult::tool_error("Message text must not be empty");
        }
        if args.text.chars().count() > MAX_MESSAGE_CHARS {
            return ToolExecutionResult::tool_error(format!(
                "Message text must not exceed {MAX_MESSAGE_CHARS} characters; share a file for longer content"
            ));
        }
        let delivery = match external_target(context) {
            Some((sender, input_id, call_id)) => {
                match sender.0.send(&args.text, input_id, call_id).await {
                    Ok(delivery) => delivery,
                    Err(error) => return error,
                }
            }
            None => None,
        };
        let mut result = json!({"sent": true, "message_id": MessageId::new()});
        if let Some(delivery) = delivery {
            result["delivery"] = json!(delivery);
        }
        ToolExecutionResult::success(result)
    }
}

/// The installed sender plus the trusted identifiers it needs, when this call
/// can reach an external conversation at all.
fn external_target(context: &ToolContext) -> Option<(Arc<ConversationSenderExt>, MessageId, &str)> {
    let sender = context.extensions.get::<ConversationSenderExt>()?;
    let input_id = context.event_context.as_ref()?.input_message_id?;
    let call_id = context
        .tool_call_id
        .as_deref()
        .filter(|id| !id.is_empty())?;
    Some((sender, input_id, call_id))
}

/// End a turn on purpose without saying anything.
pub struct NoReplyTool;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoReplyArguments {
    #[serde(default)]
    #[allow(dead_code)]
    reason: Option<String>,
}

#[async_trait]
impl Tool for NoReplyTool {
    fn name(&self) -> &str {
        NO_REPLY_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("No reply")
    }

    fn description(&self) -> &str {
        "Decide not to answer the latest message, for example an acknowledgement or people talking among themselves. Nothing is sent. Use it instead of sending an empty or filler message."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type":"object", "properties": {"reason": {
            "type":"string", "maxLength": 500,
            "description":"Why no reply is needed. Kept in the transcript, never shown to the person."
        }}, "additionalProperties":false})
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        match serde_json::from_value::<NoReplyArguments>(arguments) {
            Ok(_) => ToolExecutionResult::success(json!({"replied": false})),
            Err(error) => {
                ToolExecutionResult::tool_error(format!("Invalid no_reply arguments: {error}"))
            }
        }
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
