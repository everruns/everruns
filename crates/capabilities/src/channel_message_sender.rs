//! Slack implements the neutral message-sender contract through the existing
//! endpoint action service. The same adapter works for direct and remote hosts.

use crate::slack_action::{SlackAction, SlackActionInvoker, SlackActionOutcome};
use async_trait::async_trait;
use everruns_contracts::typed_id::MessageId;
use everruns_core::channel_messaging::{ChannelMessageReceipt, ChannelMessageSender};
use everruns_core::tools::ToolExecutionResult;
use std::sync::Arc;

/// Session-bound Slack adapter for the channel-neutral posting tool.
pub struct SlackChannelMessageSender(pub Arc<dyn SlackActionInvoker>);

#[async_trait]
impl ChannelMessageSender for SlackChannelMessageSender {
    async fn post_message(
        &self,
        text: &str,
        input_message_id: MessageId,
        tool_call_id: &str,
    ) -> Result<ChannelMessageReceipt, ToolExecutionResult> {
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
                Ok(ChannelMessageReceipt {
                    platform: "slack".into(),
                    channel,
                    message_ref: timestamp,
                })
            }
            Ok(_) => Err(ToolExecutionResult::internal_error_msg(
                "Channel posting returned an unexpected action result",
            )),
            Err(error) if error.is_tool_error() => {
                Err(ToolExecutionResult::tool_error(error.to_string()))
            }
            Err(error) => Err(ToolExecutionResult::internal_error_msg(format!(
                "Message delivery is uncertain: {error}. Do not blindly resend; the message may already be visible."
            ))),
        }
    }
}

/// Install neutral posting and native actions with the same session-bound identity.
pub fn install(
    extensions: &mut everruns_core::tool_context::ToolContextExtensions,
    invoker: Arc<dyn SlackActionInvoker>,
) {
    extensions.insert(Arc::new(
        everruns_core::channel_messaging::ChannelMessageSenderExt(Arc::new(
            SlackChannelMessageSender(invoker.clone()),
        )),
    ));
    extensions.insert(Arc::new(crate::slack_action::SlackActionInvokerExt(
        invoker,
    )));
}
