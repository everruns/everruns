//! Slack delivers an explicit agent's sent messages through the existing
//! endpoint action service. The same adapter works for direct and remote hosts.

use crate::slack_action::{SlackAction, SlackActionError, SlackActionInvoker, SlackActionOutcome};
use async_trait::async_trait;
use everruns_contracts::typed_id::MessageId;
use everruns_core::conversation::{ConversationSender, ConversationSenderExt};
use everruns_core::events::ConversationDelivery;
use everruns_core::tools::ToolExecutionResult;
use std::sync::Arc;

/// Session-bound Slack delivery for `send_message`.
pub struct SlackConversationSender(pub Arc<dyn SlackActionInvoker>);

#[async_trait]
impl ConversationSender for SlackConversationSender {
    async fn send(
        &self,
        text: &str,
        input_message_id: MessageId,
        tool_call_id: &str,
    ) -> Result<Option<ConversationDelivery>, ToolExecutionResult> {
        match self
            .0
            .invoke(SlackAction::PostMessage {
                text: text.into(),
                input_message_id: input_message_id.to_string(),
                tool_call_id: tool_call_id.into(),
            })
            .await
        {
            Ok(SlackActionOutcome::MessagePosted { channel, timestamp }) => {
                Ok(Some(ConversationDelivery {
                    platform: "slack".into(),
                    channel,
                    message_ref: timestamp,
                }))
            }
            Ok(_) => Err(ToolExecutionResult::internal_error_msg(
                "Message delivery returned an unexpected action result",
            )),
            // The input did not come from Slack (it was typed in the web app,
            // or the session has no Slack thread): the session itself is the
            // conversation, and the message is already recorded there.
            Err(SlackActionError::NoSlackSession) => Ok(None),
            Err(error) if error.is_tool_error() => {
                Err(ToolExecutionResult::tool_error(error.to_string()))
            }
            Err(error) => Err(ToolExecutionResult::internal_error_msg(format!(
                "Message delivery is uncertain: {error}. Do not blindly resend; the message may already be visible."
            ))),
        }
    }
}

/// Install Slack delivery and native Slack actions with the same
/// session-bound identity.
pub fn install(
    extensions: &mut everruns_core::tool_context::ToolContextExtensions,
    invoker: Arc<dyn SlackActionInvoker>,
) {
    extensions.insert(Arc::new(ConversationSenderExt(Arc::new(
        SlackConversationSender(invoker.clone()),
    ))));
    extensions.insert(Arc::new(crate::slack_action::SlackActionInvokerExt(
        invoker,
    )));
}
