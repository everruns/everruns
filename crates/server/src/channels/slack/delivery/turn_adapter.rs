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
//! - An MCP App view becomes a picture (rendered through the session owner's
//!   Browserless connection) and an "Open interactive view" button to the
//!   session. Without a picture the button goes alone; with views turned off
//!   for Slack nothing is posted and the agent's reply speaks for the view.

use std::sync::Arc;

use crate::domains::session_commands::mcp_apps::{McpAppSnapshot, McpAppSnapshots};
use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
use everruns_core::channel::{
    ApprovalPrompt, ChannelAgentSurface, ChannelAppView, ChannelAppViewSurface,
    ChannelApprovalPrompt, ChannelDeliveryAdapter, ChannelProgressSurface, ChannelReactionSurface,
    ChannelStreamDelivery, DeliveryContext, DeliveryResult, OutboundChannelMessage,
};

use crate::channels::slack::approvals::{ApprovalBinding, build_approval_card};
use everruns_integrations::slack_channel::web_api::SLACK_API_BASE;
use everruns_integrations::slack_channel::{bridge, channelkit_surfaces::ChannelkitSurfaces};

fn surfaces() -> Result<ChannelkitSurfaces<'static>, DeliveryResult> {
    Ok(ChannelkitSurfaces {
        api_base: SLACK_API_BASE,
        factory: bridge::direct_factory().map_err(|error| bridge::outcome(&error))?,
    })
}

pub(crate) struct SlackTurnAdapter {
    pub inner: Arc<dyn ChannelDeliveryAdapter>,
    /// The session declared the approval hint.
    pub approval_cards: bool,
    pub private_approvals: bool,
    /// The Slack user the turn answers, who alone may click its cards.
    pub requester: Option<String>,
    /// Draws MCP App views; `None` leaves views to the agent's reply.
    pub app_views: Option<SlackAppViews>,
}

/// What a turn needs to post MCP App views.
#[derive(Clone)]
pub(crate) struct SlackAppViews {
    pub snapshots: Arc<dyn McpAppSnapshots>,
    pub org_id: i64,
    /// Link to the session in the console, for the button.
    pub session_link: Option<String>,
}

#[async_trait]
impl ChannelDeliveryAdapter for SlackTurnAdapter {
    fn requires_checkpoint(&self) -> bool {
        self.inner.requires_checkpoint()
    }

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

    fn app_views(&self) -> Option<&dyn ChannelAppViewSurface> {
        self.app_views
            .as_ref()
            .map(|_| self as &dyn ChannelAppViewSurface)
    }
}

/// Label for a view's source, from its sanitized server prefix.
fn server_label(server: &str) -> String {
    let mut chars = server.replace('_', " ").chars().collect::<Vec<_>>();
    if let Some(first) = chars.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    chars.into_iter().collect()
}

/// The button message under a view's picture (or in its place).
pub(crate) fn app_view_card(
    view: &ChannelAppView,
    session_link: Option<&str>,
    pictured: bool,
) -> Option<channelkit_slack::SlackCard> {
    let link = session_link?;
    let label = server_label(&view.app.server);
    let note = if pictured {
        format!("Picture rendered by Everruns from the {label} view")
    } else {
        format!("{label} sent an interactive view ({})", view.app.tool)
    };
    Some(channelkit_slack::SlackCard {
        title: None,
        text: note,
        actions: vec![channelkit_slack::SlackButton {
            action_id: "open_mcp_app_view".into(),
            label: "Open interactive view".into(),
            value: None,
            url: Some(link.into()),
        }],
    })
}

#[async_trait]
impl ChannelAppViewSurface for SlackTurnAdapter {
    async fn show(
        &self,
        session_id: SessionId,
        view: &ChannelAppView,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        let Some(views) = &self.app_views else {
            return DeliveryResult::Ok;
        };
        let surface = match surfaces() {
            Ok(surface) => surface,
            Err(error) => return error,
        };
        let snapshot = views
            .snapshots
            .snapshot(views.org_id, session_id, &view.app, &view.text)
            .await;
        let pictured = match snapshot {
            McpAppSnapshot::Off => return DeliveryResult::Ok,
            McpAppSnapshot::LinkOnly => false,
            McpAppSnapshot::Picture(png) if png.is_empty() || png.len() > 8 * 1024 * 1024 => false,
            McpAppSnapshot::Picture(png) => {
                let upload = async {
                    let client = surface
                        .factory
                        .for_context(context, surface.api_base)
                        .await?;
                    let target = bridge::target(&client, context)?;
                    client
                        .upload_with_options(
                            &target,
                            &format!("{}-view.png", view.app.server),
                            &png,
                            &channelkit_slack::SlackUploadOptions {
                                markdown_comment: Some(everruns_integrations::slack_channel::markdown::neutralize_broadcasts(&format!(
                                    "**{}** · {}", server_label(&view.app.server), view.app.tool
                                )).into_owned()),
                                ..Default::default()
                            },
                        )
                        .await
                }
                .await;
                match upload {
                    Ok(file) => {
                        let reference = format!("file:{}", file.id);
                        if let Some(sink) = &context.receipts
                            && sink.confirmed(&reference).await.is_err()
                        {
                            return DeliveryResult::Unknown {
                                confirmed: vec![reference],
                            };
                        }
                        true
                    }
                    Err(error) => {
                        let outcome = bridge::outcome(&error);
                        // Only a definite refusal permits a link-only fallback.
                        if outcome.is_uncertain() || !outcome.confirmed().is_empty() {
                            return outcome;
                        }
                        false
                    }
                }
            }
        };
        let Some(card) = app_view_card(view, views.session_link.as_deref(), pictured) else {
            return DeliveryResult::Ok;
        };
        surface.post_card(&card, None, context).await
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
        let surface = match surfaces() {
            Ok(surface) => surface,
            Err(error) => return error,
        };
        match blocks {
            Some(card) => {
                surface
                    .post_card(
                        &card,
                        self.private_approvals
                            .then_some(self.requester.as_deref())
                            .flatten(),
                        context,
                    )
                    .await
            }
            None => surface
                .post_reply(&prompt.text(), context, None)
                .await
                .map_or_else(|error| error, |_| DeliveryResult::Ok),
        }
    }
}

#[async_trait]
impl ChannelProgressSurface for SlackTurnAdapter {
    async fn post(&self, text: &str, context: &DeliveryContext) -> Result<String, String> {
        self.post_checked(text, context)
            .await
            .map_err(|error| error.to_string())
    }
    async fn post_checked(
        &self,
        text: &str,
        context: &DeliveryContext,
    ) -> Result<String, DeliveryResult> {
        surfaces()?.post_checked(text, context).await
    }
    async fn update(&self, handle: &str, text: &str, context: &DeliveryContext) -> DeliveryResult {
        match surfaces() {
            Ok(surface) => surface.update(handle, text, context).await,
            Err(error) => error,
        }
    }
}

fn approval_blocks(
    session_id: SessionId,
    prompt: &ApprovalPrompt,
    requester: Option<&str>,
    enabled: bool,
    private: bool,
    destination: (&str, &str),
) -> Result<Option<channelkit_slack::SlackCard>, &'static str> {
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
            build_approval_card(prompt, &binding)
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
            serde_json::from_str(blocks.actions[0].value.as_deref().unwrap()).unwrap();
        assert!(binding.private);
        assert_eq!(binding.requester, "U1");
        assert_eq!(binding.turn_id.as_deref(), Some("turn"));
        assert_eq!(binding.session_id, session.to_string());
    }
}
