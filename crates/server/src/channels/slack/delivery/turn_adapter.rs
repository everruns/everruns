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
    ChannelProgressSurface, ChannelReactionSurface, ChannelStreamDelivery, DeliveryContext,
    DeliveryResult, OutboundChannelMessage,
};
use tracing::error;

use crate::channels::slack::approvals::{ApprovalBinding, build_approval_blocks};
use everruns_integrations::slack_channel::web_api::SlackApiError;
use everruns_integrations::slack_channel::web_api::{
    post_slack_blocks, post_slack_ephemeral_blocks, post_slack_message_returning_ts,
    update_slack_message_text,
};

pub(crate) struct SlackTurnAdapter {
    pub inner: Arc<dyn ChannelDeliveryAdapter>,
    /// The session declared the approval hint.
    pub approval_cards: bool,
    pub private_approvals: bool,
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

    fn reactions(&self) -> Option<&dyn ChannelReactionSurface> {
        self.inner.reactions()
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
        // Private mode must never degrade to a public question, including when
        // the requester is missing or a card exceeds Slack's size limit.
        let blocks = approval_blocks(
            session_id,
            prompt,
            self.requester.as_deref(),
            self.approval_cards,
            self.private_approvals,
            (&context.channel_id, &context.thread_ref),
        );
        let blocks = match blocks {
            Ok(blocks) => blocks,
            Err(error) => return DeliveryResult::PermanentError(error.into()),
        };
        let text = prompt.text();
        let posted = match blocks {
            Some(blocks) if self.private_approvals => {
                post_slack_ephemeral_blocks(
                    &context.auth_token,
                    &context.channel_id,
                    &context.thread_ref,
                    self.requester
                        .as_deref()
                        .expect("private card requires requester"),
                    &text,
                    &blocks,
                )
                .await
            }
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

fn approval_blocks(
    session_id: SessionId,
    prompt: &ApprovalPrompt,
    requester: Option<&str>,
    enabled: bool,
    private: bool,
    destination: (&str, &str),
) -> Result<Option<serde_json::Value>, &'static str> {
    let blocks = requester
        .filter(|user| enabled && !user.is_empty())
        .and_then(|requester| {
            let mut binding = ApprovalBinding::for_new_card(
                session_id.uuid(),
                requester,
                prompt.turn_id.clone(),
                prompt.action.clone(),
            );
            binding.private = private;
            if private {
                binding.private_destination = Some((destination.0.into(), destination.1.into()));
            }
            build_approval_blocks(prompt, &binding)
        });
    if private && (blocks.is_none() || destination.0.is_empty() || destination.1.is_empty()) {
        return Err("Private approval card unavailable; answer the pending request in Everruns.");
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_approval_never_falls_back_to_public_prose() {
        let session = SessionId::from_uuid(uuid::Uuid::now_v7());
        let prompt = ApprovalPrompt {
            action: "private action".into(),
            question: None,
            turn_id: Some("turn".into()),
        };
        for requester in [None, Some("")] {
            assert!(
                approval_blocks(session, &prompt, requester, true, true, ("C1", "1.0")).is_err()
            );
        }
        assert!(approval_blocks(session, &prompt, Some("U1"), false, true, ("C1", "1.0")).is_err());
        let oversized = ApprovalPrompt {
            action: "x".repeat(3000),
            ..prompt.clone()
        };
        assert!(
            approval_blocks(session, &oversized, Some("U1"), true, true, ("C1", "1.0")).is_err()
        );
        assert!(
            approval_blocks(session, &prompt, None, true, false, ("C1", "1.0"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn private_card_retains_requester_and_turn_binding() {
        let session = SessionId::from_uuid(uuid::Uuid::now_v7());
        let prompt = ApprovalPrompt {
            action: "private action".into(),
            question: None,
            turn_id: Some("turn".into()),
        };
        let blocks = approval_blocks(session, &prompt, Some("U1"), true, true, ("C1", "1.0"))
            .unwrap()
            .unwrap();
        let binding: ApprovalBinding =
            serde_json::from_str(blocks[1]["elements"][0]["value"].as_str().unwrap()).unwrap();
        assert!(binding.private);
        assert_eq!(binding.requester, "U1");
        assert_eq!(binding.turn_id.as_deref(), Some("turn"));
        assert_eq!(binding.session_id, session.to_string());
    }
}
