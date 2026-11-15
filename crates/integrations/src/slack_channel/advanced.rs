//! Optional Slack conversation affordances. No control-plane state is required.
use super::{
    Slack,
    markdown::{markdown_block, neutralize_broadcasts},
    strip_mentions,
    surfaces::*,
    unix_now,
    web_api::slack_api_call,
};
use async_trait::async_trait;
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
    pub(super) fn interactive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        if !self.advanced.approvals {
            return Ok(Inbound::Ignore);
        }
        let payload = url::form_urlencoded::parse(&request.body)
            .find(|(k, _)| k == "payload")
            .map(|(_, v)| v.into_owned())
            .ok_or_else(|| ChannelError::BadRequest("Slack interaction has no payload".into()))?;
        let body: Value = serde_json::from_str(&payload)
            .map_err(|_| ChannelError::BadRequest("Invalid Slack interaction".into()))?;
        if self
            .workspace
            .as_ref()
            .is_some_and(|team| body["team"]["id"].as_str() != Some(team.as_str()))
        {
            return Err(ChannelError::Unauthorized(
                "Slack interaction belongs to another workspace".into(),
            ));
        }
        let action = &body["actions"][0];
        let key = action["value"].as_str().unwrap_or("");
        let mut cards = self
            .advanced
            .cards
            .lock()
            .map_err(|_| ChannelError::Session("Approval storage unavailable".into()))?;
        cards.retain(|_, c| c.expires >= unix_now());
        let Some(card) = cards.get(key).cloned() else {
            return Ok(Inbound::Ignore);
        };
        // A valid Slack signature authenticates Slack, not the person allowed to answer this card.
        if body["user"]["id"].as_str() != Some(&card.user)
            || body["channel"]["id"].as_str() != Some(&card.channel)
        {
            return Err(ChannelError::Unauthorized(
                "This approval belongs to another person or channel".into(),
            ));
        }
        let text = match action["action_id"].as_str() {
            Some("everruns_approve") => "Approved",
            Some("everruns_deny") => "Denied",
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
    pub(super) fn pane_control(&self, event: &Value) -> Option<Inbound> {
        if !self.advanced.pane {
            return None;
        }
        let kind = event["type"].as_str()?;
        let thread = &event["assistant_thread"];
        let action = match kind {
            "app_context_changed" | "assistant_thread_context_changed" => ControlAction::Context {
                key: "channel_view_context".into(),
                value: thread["context"].clone(),
            },
            "agent_session_stopped" => ControlAction::Cancel,
            "agent_session_title_changed" => ControlAction::Rename {
                title: thread["title"]
                    .as_str()
                    .or(event["title"].as_str())?
                    .chars()
                    .take(250)
                    .collect(),
            },
            "assistant_thread_started" => return Some(Inbound::Ignore),
            _ => return None,
        };
        let channel = thread["channel_id"]
            .as_str()
            .or(event["channel"].as_str())?;
        let ts = thread["thread_ts"]
            .as_str()
            .or(event["thread_ts"].as_str())?;
        let user = event["user"]
            .as_str()
            .or(thread["user_id"].as_str())
            .unwrap_or("");
        Some(Inbound::Control(Box::new(InboundControl {
            route: InboundRoute {
                actor: ExternalActor {
                    actor_id: user.into(),
                    actor_name: None,
                    source: "slack".into(),
                    metadata: None,
                },
                thread_ref: Some(format!("{channel}:{ts}")),
                routing_metadata: HashMap::from([
                    ("channel_id".into(), channel.into()),
                    ("user_id".into(), user.into()),
                ]),
                reply_to: target(channel, ts, user, ""),
            },
            action,
        })))
    }
    pub(super) async fn history(
        &self,
        message: &InboundMessage,
    ) -> Result<Vec<InboundChannelEvent>, ChannelError> {
        let limit = self.advanced.history;
        if limit == 0 {
            return Ok(vec![]);
        }
        let token = self.token.value().unwrap_or_default();
        if token.is_empty() {
            return Err(ChannelError::Unauthorized(
                "Slack history needs a bot token".into(),
            ));
        }
        let body=slack_api_call(&self.api_base,&token,"conversations.replies",json!({"channel":message.reply_to.channel_id,"ts":message.reply_to.thread_ref,"limit":limit+1})).await.map_err(|_|ChannelError::Session("Slack history unavailable".into()))?;
        let current = message
            .event
            .message_metadata
            .get("slack_ts")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut events = vec![];
        for reply in body["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .take(limit + 1)
        {
            let ts = reply["ts"].as_str().unwrap_or("");
            if ts.is_empty() || ts >= current || reply.get("bot_id").is_some() {
                continue;
            }
            let mut event = message.event.clone();
            event.actor.actor_id = reply["user"].as_str().unwrap_or("").into();
            event.text = strip_mentions(reply["text"].as_str().unwrap_or(""))
                .chars()
                .take(12000)
                .collect();
            event.attachments.clear();
            event.dedup_key = format!("{}:{ts}", message.reply_to.channel_id);
            event.message_metadata.insert("slack_ts".into(), json!(ts));
            events.push(event);
        }
        events.sort_by(|a, b| a.dedup_key.cmp(&b.dedup_key));
        events.truncate(limit);
        Ok(events)
    }
}
#[async_trait]
impl ChannelStreamDelivery for Slack {
    async fn start(&self, c: &DeliveryContext) -> Result<String, String> {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .start(c)
        .await
    }
    async fn append(&self, h: &str, t: &str, c: &DeliveryContext) -> DeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .append(h, t, c)
        .await
    }
    async fn replace(&self, h: &str, t: &str, c: &DeliveryContext) -> DeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .replace(h, t, c)
        .await
    }
    async fn stop(&self, h: &str, c: &DeliveryContext) -> DeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .stop(h, c)
        .await
    }
}
#[async_trait]
impl ChannelAgentSurface for Slack {
    async fn set_status(&self, s: &str, c: &DeliveryContext) -> DeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .set_status(s, c)
        .await
    }
    async fn set_title(&self, t: &str, c: &DeliveryContext) -> DeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .set_title(t, c)
        .await
    }
}
#[async_trait]
impl ChannelProgressSurface for Slack {
    async fn post(&self, t: &str, c: &DeliveryContext) -> Result<String, String> {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .post(t, c)
        .await
    }
    async fn update(&self, h: &str, t: &str, c: &DeliveryContext) -> DeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .update(h, t, c)
        .await
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
        let body = json!({"channel":c.channel_id,"thread_ts":c.thread_ref,"text":neutralize_broadcasts(&p.text()),"blocks":[markdown_block(&p.text()),{"type":"actions","elements":[{"type":"button","text":{"type":"plain_text","text":"Approve"},"action_id":"everruns_approve","value":key},{"type":"button","text":{"type":"plain_text","text":"Deny"},"action_id":"everruns_deny","value":key}]}]});
        match slack_api_call(&self.api_base, &c.auth_token, "chat.postMessage", body).await {
            Ok(_) => DeliveryResult::Ok,
            Err(e) => {
                if let Ok(mut cards) = self.advanced.cards.lock() {
                    cards.remove(&key);
                }
                classify_slack_failure(e)
            }
        }
    }
}
