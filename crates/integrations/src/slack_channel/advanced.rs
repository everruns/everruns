//! Optional Slack conversation affordances. No control-plane state is required.
use super::{Slack, bridge, strip_mentions, surfaces::*, unix_now};
use async_trait::async_trait;
use channelkit::Adapter;
use everruns_contracts::{runtime::channel::*, typed_id::SessionId};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
};

/// Application-owned relevance policy. Called before creating a session or starting a turn.
#[async_trait]
pub trait Participation: Send + Sync {
    /// Decide whether this message merits a reply. Errors reject the message.
    async fn admit(
        &self,
        message: &InboundMessage,
        session_id: Option<&str>,
    ) -> Result<bool, ChannelError>;
}
/// Built-in inexpensive participation modes.
#[derive(Debug, Clone, Copy)]
pub enum SlackAdmission {
    /// Always participate.
    Always,
    /// Only direct messages or explicit mentions.
    Direct,
}
#[async_trait]
impl Participation for SlackAdmission {
    async fn admit(&self, m: &InboundMessage, _: Option<&str>) -> Result<bool, ChannelError> {
        Ok(matches!(self, Self::Always)
            || m.reply_to.channel_id.starts_with('D')
            || m.event
                .message_metadata
                .get("slack_mention")
                .and_then(Value::as_bool)
                == Some(true))
    }
}
#[derive(Clone, Default)]
pub(super) struct Advanced {
    pub pane: bool,
    pub progress: bool,
    pub approvals: bool,
    pub history: usize,
    pub admission: Option<Arc<dyn Participation>>,
    cards: Arc<Mutex<HashMap<String, Card>>>,
}
impl fmt::Debug for Advanced {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Advanced")
            .field("pane", &self.pane)
            .field("history", &self.history)
            .finish_non_exhaustive()
    }
}
#[derive(Clone)]
struct Card {
    channel: String,
    thread: String,
    user: String,
    expires: i64,
    action: String,
    turn: Option<String>,
}
pub(super) fn target(channel: &str, thread: &str, user: &str, team: &str) -> DeliveryTarget {
    let mut t = DeliveryTarget::new(channel, thread);
    t.extra.insert(SLACK_RECIPIENT_USER_ID.into(), user.into());
    t.extra.insert(SLACK_RECIPIENT_TEAM_ID.into(), team.into());
    t
}
impl Slack {
    /// Enable progressive replies and Slack agent-pane status/title operations.
    /// Slack must have the Agents feature enabled and appropriate scopes installed.
    pub fn agent_pane(mut self) -> Self {
        self.advanced.pane = true;
        self
    }
    /// Show task progress as one edited message per turn.
    pub fn task_progress(mut self) -> Self {
        self.advanced.progress = true;
        self
    }
    /// Render request-approval questions with one-use buttons bound to the requesting person.
    /// Outstanding cards live in this driver instance and expire after fifteen minutes.
    pub fn approval_buttons(mut self) -> Self {
        self.advanced.approvals = true;
        self
    }
    /// Seed new conversations with at most 100 preceding thread messages.
    pub fn thread_history(mut self, limit: usize) -> Self {
        self.advanced.history = limit.min(100);
        self
    }
    /// Select messages before a turn is created, using an application-owned relevance policy.
    pub fn participation(mut self, policy: impl Participation + 'static) -> Self {
        self.advanced.admission = Some(Arc::new(policy));
        self
    }
    pub(super) fn clear_approval_cards(&self, channel: &str, thread: &str) {
        if let Ok(mut cards) = self.advanced.cards.lock() {
            cards.retain(|_, card| card.channel != channel || card.thread != thread);
        }
    }
    pub(super) fn interactive_action(
        &self,
        action: &channelkit::ActionEvent,
    ) -> Result<Inbound, ChannelError> {
        if !self.advanced.approvals {
            return Ok(Inbound::Ignore);
        }
        let key = &action.value;
        let mut cards = self
            .advanced
            .cards
            .lock()
            .map_err(|_| ChannelError::Session("Approval storage unavailable".into()))?;
        cards.retain(|_, c| c.expires >= unix_now());
        let Some(card) = cards.get(key).cloned() else {
            return Ok(Inbound::Ignore);
        };
        if action.actor.id != card.user
            || action.conversation.channel != card.channel
            || action.conversation.thread.as_deref() != Some(card.thread.as_str())
        {
            return Err(ChannelError::Unauthorized(
                "This approval belongs to another person or conversation".into(),
            ));
        }
        let text = match action.id.as_str() {
            "everruns_approve" => "Approved",
            "everruns_deny" => "Denied",
            _ => return Ok(Inbound::Ignore),
        };
        cards.remove(key);
        Ok(Inbound::Message(Box::new(InboundMessage {
            event: InboundChannelEvent {
                actor: ExternalActor {
                    actor_id: card.user.clone(),
                    actor_name: None,
                    source: "slack".into(),
                    metadata: None,
                },
                text: format!(
                    "{text} the approval request{}: {}",
                    card.turn
                        .as_ref()
                        .map(|t| format!(" from turn {t}"))
                        .unwrap_or_default(),
                    card.action
                ),
                attachments: vec![],
                dedup_key: format!("slack-approval:{key}"),
                thread_ref: Some(format!("{}:{}", card.channel, card.thread)),
                routing_metadata: HashMap::from([
                    ("channel_id".into(), card.channel.clone()),
                    ("user_id".into(), card.user.clone()),
                ]),
                message_metadata: Default::default(),
            },
            reply_to: target(&card.channel, &card.thread, &card.user, ""),
        })))
    }
    pub(super) async fn history(
        &self,
        message: &InboundMessage,
        context: &DeliveryContext,
    ) -> Result<Vec<InboundChannelEvent>, ChannelError> {
        let limit = self.advanced.history;
        if limit == 0 {
            return Ok(vec![]);
        }
        let factory = self.factory().map_err(super::ingress::ingress_error)?;
        let client = factory
            .for_context(context, &self.api_base)
            .await
            .map_err(super::ingress::ingress_error)?;
        let target = bridge::target(&client, context).map_err(super::ingress::ingress_error)?;
        let current = message
            .event
            .message_metadata
            .get("slack_ts")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut events = vec![];
        let mut cursor = None;
        // Bound both pages and retained messages. History is informational;
        // it cannot authorize an action or choose another conversation.
        for _ in 0..4 {
            let page = client
                .history(&target, cursor.as_deref(), (limit + 1).min(100))
                .await
                .map_err(|_| ChannelError::Session("Slack history unavailable".into()))?;
            for reply in page.messages {
                let ts = &reply.reference.id;
                if ts.is_empty() || ts.as_str() >= current || reply.author.is_bot {
                    continue;
                }
                let mut event = message.event.clone();
                event
                    .message_metadata
                    .insert("slack_user".into(), json!(&reply.author.id));
                event
                    .message_metadata
                    .insert("slack_mention".into(), json!(reply.mentioned));
                event
                    .routing_metadata
                    .insert("user_id".into(), reply.author.id.clone());
                event.actor.actor_id = reply.author.id;
                event.actor.actor_name = reply.author.display_name;
                event.text = strip_mentions(&reply.text).chars().take(12000).collect();
                event.attachments.clear();
                event.dedup_key = format!("{}:{ts}", message.reply_to.channel_id);
                event.message_metadata.insert("slack_ts".into(), json!(ts));
                events.push(event);
            }
            cursor = page.next_cursor;
            if cursor.is_none() || events.len() >= limit {
                break;
            }
        }
        events.sort_by(|a, b| a.dedup_key.cmp(&b.dedup_key));
        events.truncate(limit);
        Ok(events)
    }
}
#[async_trait]
impl ChannelStreamDelivery for Slack {
    async fn start(&self, c: &DeliveryContext) -> Result<String, String> {
        self.start_checked(c)
            .await
            .map_err(|error| error.to_string())
    }
    async fn start_checked(&self, c: &DeliveryContext) -> Result<String, DeliveryResult> {
        let c = self.scoped_context(c)?;
        self.provider_surfaces()
            .map_err(|error| bridge::outcome(&error))?
            .start_checked(&c)
            .await
    }
    async fn append(&self, h: &str, t: &str, c: &DeliveryContext) -> DeliveryResult {
        let c = match self.scoped_context(c) {
            Ok(c) => c,
            Err(error) => return error,
        };
        match self.provider_surfaces() {
            Ok(provider) => provider.append(h, t, &c).await,
            Err(error) => bridge::outcome(&error),
        }
    }
    async fn replace(&self, h: &str, t: &str, c: &DeliveryContext) -> DeliveryResult {
        let c = match self.scoped_context(c) {
            Ok(c) => c,
            Err(error) => return error,
        };
        match self.provider_surfaces() {
            Ok(provider) => provider.replace(h, t, &c).await,
            Err(error) => bridge::outcome(&error),
        }
    }
    async fn stop(&self, h: &str, c: &DeliveryContext) -> DeliveryResult {
        let c = match self.scoped_context(c) {
            Ok(c) => c,
            Err(error) => return error,
        };
        match self.provider_surfaces() {
            Ok(provider) => provider.stop(h, &c).await,
            Err(error) => bridge::outcome(&error),
        }
    }
}
#[async_trait]
impl ChannelAgentSurface for Slack {
    async fn set_status(&self, t: &str, c: &DeliveryContext) -> DeliveryResult {
        let c = match self.scoped_context(c) {
            Ok(c) => c,
            Err(error) => return error,
        };
        match self.provider_surfaces() {
            Ok(provider) => provider.set_status(t, &c).await,
            Err(error) => bridge::outcome(&error),
        }
    }
    async fn set_title(&self, t: &str, c: &DeliveryContext) -> DeliveryResult {
        let c = match self.scoped_context(c) {
            Ok(c) => c,
            Err(error) => return error,
        };
        match self.provider_surfaces() {
            Ok(provider) => provider.set_title(t, &c).await,
            Err(error) => bridge::outcome(&error),
        }
    }
}
#[async_trait]
impl ChannelProgressSurface for Slack {
    async fn post(&self, t: &str, c: &DeliveryContext) -> Result<String, String> {
        self.post_checked(t, c)
            .await
            .map_err(|error| error.to_string())
    }
    async fn post_checked(&self, t: &str, c: &DeliveryContext) -> Result<String, DeliveryResult> {
        let c = self.scoped_context(c)?;
        self.provider_surfaces()
            .map_err(|error| bridge::outcome(&error))?
            .post_checked(t, &c)
            .await
    }
    async fn update(&self, h: &str, t: &str, c: &DeliveryContext) -> DeliveryResult {
        let c = match self.scoped_context(c) {
            Ok(c) => c,
            Err(error) => return error,
        };
        match self.provider_surfaces() {
            Ok(provider) => provider.update(h, t, &c).await,
            Err(error) => bridge::outcome(&error),
        }
    }
}
#[async_trait]
impl ChannelApprovalPrompt for Slack {
    async fn prompt(
        &self,
        _session: SessionId,
        p: &ApprovalPrompt,
        c: &DeliveryContext,
    ) -> DeliveryResult {
        let c = match self.scoped_context(c) {
            Ok(c) => c,
            Err(error) => return error,
        };
        let c = &c;
        let user = c
            .extra
            .get(SLACK_RECIPIENT_USER_ID)
            .cloned()
            .unwrap_or_default();
        if user.is_empty() {
            return DeliveryResult::PermanentError(
                "Slack approval has no requesting person".into(),
            );
        }
        let key = uuid::Uuid::new_v4().to_string();
        {
            let Ok(mut cards) = self.advanced.cards.lock() else {
                return DeliveryResult::TransientError("Approval storage unavailable".into());
            };
            cards.retain(|_, v| v.expires >= unix_now());
            if cards.len() >= 10000 {
                return DeliveryResult::TransientError("Too many outstanding approvals".into());
            }
            cards.retain(|_, card| card.channel != c.channel_id || card.thread != c.thread_ref);
            cards.insert(
                key.clone(),
                Card {
                    channel: c.channel_id.clone(),
                    thread: c.thread_ref.clone(),
                    user,
                    expires: unix_now() + 900,
                    action: p.action.clone(),
                    turn: p.turn_id.clone(),
                },
            );
        }
        let result = async {
            let factory = self.factory()?;
            let client = factory.for_context(c, &self.api_base).await?;
            let target = bridge::target(&client, c)?;
            let card = channelkit_slack::SlackCard {
                title: None,
                text: super::markdown::neutralize_broadcasts(&p.text()).into_owned(),
                actions: vec![
                    channelkit_slack::SlackButton {
                        action_id: "everruns_approve".into(),
                        label: "Approve".into(),
                        value: Some(key.clone()),
                        url: None,
                    },
                    channelkit_slack::SlackButton {
                        action_id: "everruns_deny".into(),
                        label: "Deny".into(),
                        value: Some(key.clone()),
                        url: None,
                    },
                ],
            };
            let observer =
                c.receipts
                    .as_ref()
                    .map(|sink| crate::channelkit_bridge::ReceiptObserver {
                        sink: sink.as_ref(),
                        target: &target,
                        encode: bridge::message_key,
                    });
            let receipt = client
                .post_card(&target, &card, &Default::default())
                .await?;
            if let Some(observer) = observer {
                for reference in &receipt.messages {
                    if let Err(error) =
                        channelkit::ReceiptObserver::confirmed(&observer, reference).await
                    {
                        return Err(channelkit::Error::PartialDelivery {
                            confirmed: receipt.messages.clone(),
                            source: Box::new(error),
                        });
                    }
                }
            }
            Ok(receipt)
        }
        .await;
        match result {
            Ok(_) => DeliveryResult::Ok,
            Err(error) => {
                // An unknown/partial send may already show its card. Keep its
                // authorization binding until expiry instead of disabling it.
                let outcome = bridge::outcome(&error);
                if !outcome.is_uncertain()
                    && let Ok(mut cards) = self.advanced.cards.lock()
                {
                    cards.remove(&key);
                }
                outcome
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_sdk_card_cannot_resume_the_conversation() {
        let slack = Slack::new("fixture-token", "fixture-secret").approval_buttons();
        slack.advanced.cards.lock().unwrap().insert(
            "expired".into(),
            Card {
                channel: "C1".into(),
                thread: "1.0".into(),
                user: "U7".into(),
                expires: unix_now() - 1,
                action: "fixture action".into(),
                turn: None,
            },
        );
        let action = channelkit::ActionEvent {
            conversation: channelkit::ConversationKey::new(
                channelkit::Provider::Slack,
                "everruns-sdk-slack",
                "C1",
                Some("1.0".into()),
            )
            .unwrap(),
            actor: channelkit::User {
                id: "U7".into(),
                display_name: None,
                is_bot: false,
            },
            id: "everruns_approve".into(),
            value: "expired".into(),
            message_id: Some("2.0".into()),
        };
        assert!(matches!(
            slack.interactive_action(&action).unwrap(),
            Inbound::Ignore
        ));
    }
}
