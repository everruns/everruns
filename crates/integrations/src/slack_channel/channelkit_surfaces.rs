//! Slack provider primitives for the shared, host-owned delivery projection.
use super::{
    bridge::*,
    markdown::neutralize_broadcasts,
    surfaces::{SLACK_RECIPIENT_TEAM_ID, SLACK_RECIPIENT_USER_ID},
};
use async_trait::async_trait;
use channelkit::{Adapter, Content, Receipt};
use everruns_contracts::runtime::channel::{
    ChannelAgentSurface, ChannelProgressSurface, ChannelStreamDelivery, DeliveryContext,
    DeliveryResult,
};
use std::collections::BTreeMap;

pub struct ChannelkitSurfaces<'a> {
    pub api_base: &'a str,
    pub factory: &'a SlackAdapterFactory,
}

fn handle(reference: &channelkit::MessageRef) -> String {
    reference.id.clone()
}
fn handle_outcome(error: &channelkit::Error) -> DeliveryResult {
    crate::channelkit_bridge::delivery_outcome(
        error,
        |code| super::web_api::PERMANENT_SLACK_ERRORS.contains(&code),
        handle,
    )
}

impl ChannelkitSurfaces<'_> {
    /// Cards make at most one write. Persist its accepted reference before any
    /// later output, including an ephemeral receipt that must remain private.
    pub async fn post_card(
        &self,
        card: &channelkit_slack::SlackCard,
        requester: Option<&str>,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        let card = channelkit_slack::SlackCard {
            title: card
                .title
                .as_deref()
                .map(|title| neutralize_broadcasts(title).into_owned()),
            text: neutralize_broadcasts(&card.text).into_owned(),
            actions: card.actions.clone(),
        };
        let result = async {
            let client = self.factory.for_context(context, self.api_base).await?;
            let target = target(&client, context)?;
            let refs = if let Some(requester) = requester {
                vec![format!(
                    "ephemeral:{}:{}",
                    context.channel_id,
                    client.post_ephemeral(&target, requester, &card).await?
                )]
            } else {
                client
                    .post_card(&target, &card, &Default::default())
                    .await?
                    .messages
                    .iter()
                    .map(message_key)
                    .collect()
            };
            if let Some(sink) = &context.receipts {
                for reference in &refs {
                    if sink.confirmed(reference).await.is_err() {
                        return Ok(DeliveryResult::Unknown { confirmed: refs });
                    }
                }
            }
            Ok(DeliveryResult::Ok)
        }
        .await;
        result.unwrap_or_else(|error| outcome(&error))
    }

    pub async fn post_reply(
        &self,
        text: &str,
        context: &DeliveryContext,
        correlation: Option<(&str, &str)>,
    ) -> Result<Receipt, DeliveryResult> {
        let client = self
            .factory
            .for_context(context, self.api_base)
            .await
            .map_err(|e| outcome(&e))?;
        let target = target(&client, context).map_err(|e| outcome(&e))?;
        let observer =
            context
                .receipts
                .as_ref()
                .map(|sink| crate::channelkit_bridge::ReceiptObserver {
                    sink: sink.as_ref(),
                    target: &target,
                    encode: message_key,
                });
        let options = channelkit_slack::SlackPostOptions {
            metadata: correlation.map(|(session, input)| channelkit_slack::SlackMessageMetadata {
                event_type: "everruns_agent_reply".into(),
                event_payload: BTreeMap::from([
                    ("session_id".into(), session.into()),
                    ("input_message_id".into(), input.into()),
                ]),
            }),
        };
        client
            .post_markdown(
                &target,
                &neutralize_broadcasts(text),
                &options,
                observer
                    .as_ref()
                    .map(|o| o as &dyn channelkit::ReceiptObserver),
            )
            .await
            .map_err(|e| outcome(&e))
    }
}

#[async_trait]
impl ChannelStreamDelivery for ChannelkitSurfaces<'_> {
    async fn start(&self, context: &DeliveryContext) -> Result<String, String> {
        self.start_checked(context)
            .await
            .map_err(|error| error.to_string())
    }
    async fn start_checked(&self, context: &DeliveryContext) -> Result<String, DeliveryResult> {
        let client = self
            .factory
            .for_context(context, self.api_base)
            .await
            .map_err(|e| handle_outcome(&e))?;
        let target = target(&client, context).map_err(|e| handle_outcome(&e))?;
        let recipient = context
            .extra
            .get(SLACK_RECIPIENT_USER_ID)
            .filter(|user| !user.is_empty())
            .map(|user| channelkit_slack::StreamRecipient {
                user_id: user.clone(),
                team_id: context
                    .extra
                    .get(SLACK_RECIPIENT_TEAM_ID)
                    .filter(|team| !team.is_empty())
                    .cloned(),
            });
        let reference = client
            .start_stream(&target, recipient.as_ref())
            .await
            .map_err(|e| handle_outcome(&e))?;
        if let Some(sink) = &context.receipts
            && sink.confirmed(&reference.id).await.is_err()
        {
            return Err(DeliveryResult::Unknown {
                confirmed: vec![reference.id],
            });
        }
        Ok(reference.id)
    }
    async fn append(&self, id: &str, text: &str, context: &DeliveryContext) -> DeliveryResult {
        let result = async {
            let client = self.factory.for_context(context, self.api_base).await?;
            client
                .append_stream(
                    &reference(&client, context, id)?,
                    &neutralize_broadcasts(text),
                )
                .await
        }
        .await;
        result.map_or_else(|e| handle_outcome(&e), |_| DeliveryResult::Ok)
    }
    async fn stop(&self, id: &str, context: &DeliveryContext) -> DeliveryResult {
        let result = async {
            let client = self.factory.for_context(context, self.api_base).await?;
            match client.stop_stream(&reference(&client, context, id)?).await {
                // Replaying known-handle cleanup is harmless when the stream is
                // already closed or its message was removed.
                Err(channelkit::Error::Rejected { code })
                    if matches!(
                        code.as_str(),
                        "message_not_in_streaming_state" | "message_not_found"
                    ) =>
                {
                    Ok(())
                }
                result => result,
            }
        }
        .await;
        result.map_or_else(|e| handle_outcome(&e), |_| DeliveryResult::Ok)
    }
    async fn replace(&self, id: &str, text: &str, context: &DeliveryContext) -> DeliveryResult {
        let stopped = self.stop(id, context).await;
        if stopped != DeliveryResult::Ok {
            return stopped;
        }
        let result = async {
            let client = self.factory.for_context(context, self.api_base).await?;
            let reference = reference(&client, context, id)?;
            let text = neutralize_broadcasts(text);
            let text = if text.trim().is_empty() {
                "Response retracted."
            } else {
                &text
            };
            match client
                .edit(&reference, Content::Markdown(text.into()))
                .await
            {
                Err(
                    error @ (channelkit::Error::InvalidInput(_) | channelkit::Error::Capacity(_)),
                ) => {
                    client
                        .edit(
                            &reference,
                            Content::Text(
                                "Response retracted; the replacement could not be displayed."
                                    .into(),
                            ),
                        )
                        .await?;
                    Err(error)
                }
                Err(channelkit::Error::Rejected { code }) if code == "invalid_blocks" => {
                    client
                        .edit(
                            &reference,
                            Content::Text(
                                "Response retracted; the replacement could not be displayed."
                                    .into(),
                            ),
                        )
                        .await?;
                    Err(channelkit::Error::Rejected { code })
                }
                result => result,
            }
        }
        .await;
        result.map_or_else(|e| handle_outcome(&e), |_| DeliveryResult::Ok)
    }
}

#[async_trait]
impl ChannelAgentSurface for ChannelkitSurfaces<'_> {
    async fn set_status(&self, text: &str, context: &DeliveryContext) -> DeliveryResult {
        if context.thread_ref.is_empty() {
            return DeliveryResult::Ok;
        }
        let result = async {
            let client = self.factory.for_context(context, self.api_base).await?;
            client
                .set_status(&target(&client, context)?, &neutralize_broadcasts(text))
                .await
        }
        .await;
        result.map_or_else(|e| outcome(&e), |_| DeliveryResult::Ok)
    }
    async fn set_title(&self, text: &str, context: &DeliveryContext) -> DeliveryResult {
        if context.thread_ref.is_empty() {
            return DeliveryResult::Ok;
        }
        let result = async {
            let client = self.factory.for_context(context, self.api_base).await?;
            let title = neutralize_broadcasts(text)
                .chars()
                .take(250)
                .collect::<String>();
            client.set_title(&target(&client, context)?, &title).await
        }
        .await;
        result.map_or_else(|e| outcome(&e), |_| DeliveryResult::Ok)
    }
}

#[async_trait]
impl ChannelProgressSurface for ChannelkitSurfaces<'_> {
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
        let client = self
            .factory
            .for_context(context, self.api_base)
            .await
            .map_err(|e| handle_outcome(&e))?;
        let target = target(&client, context).map_err(|e| handle_outcome(&e))?;
        let receipt = client
            .post_markdown_single(
                &target,
                &neutralize_broadcasts(text),
                &Default::default(),
                None,
            )
            .await
            .map_err(|e| handle_outcome(&e))?;
        let id = receipt
            .messages
            .first()
            .map(|reference| reference.id.clone())
            .ok_or(DeliveryResult::Unknown { confirmed: vec![] })?;
        if let Some(sink) = &context.receipts
            && sink.confirmed(&id).await.is_err()
        {
            return Err(DeliveryResult::Unknown {
                confirmed: vec![id],
            });
        }
        Ok(id)
    }
    async fn update(&self, id: &str, text: &str, context: &DeliveryContext) -> DeliveryResult {
        let result = async {
            let client = self.factory.for_context(context, self.api_base).await?;
            client
                .edit(
                    &reference(&client, context, id)?,
                    Content::Markdown(neutralize_broadcasts(text).into_owned()),
                )
                .await
        }
        .await;
        result.map_or_else(|e| handle_outcome(&e), |_| DeliveryResult::Ok)
    }
}
