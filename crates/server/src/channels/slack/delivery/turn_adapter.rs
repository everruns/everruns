//! The Slack adapter as one turn sees it: replies, streams and status from the
//! shared adapter, plus what only a registered turn can draw.
//!
//! Decisions:
//! - Approval cards bind to the turn's requester, which only registration
//!   knows, so they live here rather than on the shared adapter. Without the
//!   session's approval hint, or without a requester, the ask is posted as
//!   prose: the model asked, the thread shows the question, the person
//!   answers by replying (EVE-1025).
//! - Task progress is one message posted with its `ts` returned and then
//!   rewritten in place (EVE-1026).

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
use everruns_core::channel::{
    ApprovalPrompt, ChannelAgentSurface, ChannelApprovalPrompt, ChannelDeliveryAdapter,
    ChannelProgressSurface, ChannelStreamDelivery, DeliveryContext, DeliveryResult,
    OutboundChannelMessage,
};
use tracing::error;

use crate::channels::slack::api::{
    post_slack_blocks, post_slack_message_returning_ts, update_slack_message_text,
};
use crate::channels::slack::api_error::SlackApiError;
use crate::channels::slack::approvals::{ApprovalBinding, build_approval_blocks};

pub(super) struct SlackTurnAdapter {
    pub inner: Arc<dyn ChannelDeliveryAdapter>,
    /// The session declared the approval hint.
    pub approval_cards: bool,
    /// The Slack user the turn answers, who alone may click its cards.
    pub requester: Option<String>,
}

#[async_trait]
impl ChannelDeliveryAdapter for SlackTurnAdapter {
    fn platform(&self) -> &str {
        self.inner.platform()
    }

    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        self.inner.deliver(message, context).await
    }

    async fn send_ack(
        &self,
        thread_ref: &str,
        text: &str,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        self.inner.send_ack(thread_ref, text, context).await
    }

    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        self.inner.streaming()
    }

    fn agent_surface(&self) -> Option<&dyn ChannelAgentSurface> {
        self.inner.agent_surface()
    }

    fn approvals(&self) -> Option<&dyn ChannelApprovalPrompt> {
        Some(self)
    }

    fn progress(&self) -> Option<&dyn ChannelProgressSurface> {
        Some(self)
    }
}

fn result(outcome: Result<(), SlackApiError>) -> DeliveryResult {
    match outcome {
        Ok(()) => DeliveryResult::Ok,
        Err(SlackApiError::Permanent(error)) => DeliveryResult::PermanentError(error),
        Err(error) => DeliveryResult::TransientError(error.to_string()),
    }
}

#[async_trait]
impl ChannelApprovalPrompt for SlackTurnAdapter {
    async fn prompt(
        &self,
        session_id: SessionId,
        prompt: &ApprovalPrompt,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        let blocks = self
            .requester
            .as_deref()
            .filter(|_| self.approval_cards)
            .and_then(|requester| {
                build_approval_blocks(
                    prompt,
                    &ApprovalBinding::for_new_card(
                        session_id.uuid(),
                        requester,
                        prompt.turn_id.clone(),
                        prompt.action.clone(),
                    ),
                )
            });
        let text = prompt.text();
        let posted = match blocks {
            Some(blocks) => {
                post_slack_blocks(
                    &context.auth_token,
                    &context.channel_id,
                    &context.thread_ref,
                    &text,
                    &blocks,
                )
                .await
            }
            None => super::post_to_slack(
                &context.auth_token,
                &context.channel_id,
                &context.thread_ref,
                &text,
            )
            .await
            .map_err(|error| SlackApiError::Transient(error.to_string())),
        };
        if let Err(error) = &posted {
            error!(%session_id, %error, "Failed to post the Slack approval ask");
        }
        result(posted)
    }
}

#[async_trait]
impl ChannelProgressSurface for SlackTurnAdapter {
    async fn post(&self, text: &str, context: &DeliveryContext) -> Result<String, String> {
        post_slack_message_returning_ts(
            &context.auth_token,
            &context.channel_id,
            &context.thread_ref,
            text,
        )
        .await
        .map_err(|error| error.to_string())
    }

    async fn update(&self, handle: &str, text: &str, context: &DeliveryContext) -> DeliveryResult {
        result(
            update_slack_message_text(&context.auth_token, &context.channel_id, handle, text).await,
        )
    }
}
