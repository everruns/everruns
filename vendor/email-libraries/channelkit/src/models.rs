use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Official providers are a closed allowlist.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Agentmail,
    Slack,
    Whatsapp,
    Discord,
    Teams,
    Github,
    Linear,
    X,
    Web,
}
impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agentmail => "agentmail",
            Self::Slack => "slack",
            Self::Whatsapp => "whatsapp",
            Self::Discord => "discord",
            Self::Teams => "teams",
            Self::Github => "github",
            Self::Linear => "linear",
            Self::X => "x",
            Self::Web => "web",
        }
    }
}
impl std::str::FromStr for Provider {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "agentmail" => Ok(Self::Agentmail),
            "slack" => Ok(Self::Slack),
            "whatsapp" => Ok(Self::Whatsapp),
            "discord" => Ok(Self::Discord),
            "teams" => Ok(Self::Teams),
            "github" => Ok(Self::Github),
            "linear" => Ok(Self::Linear),
            "x" => Ok(Self::X),
            "web" => Ok(Self::Web),
            _ => Err(Error::Unsupported("provider")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ConversationKey {
    pub provider: Provider,
    pub installation: String,
    pub channel: String,
    pub thread: Option<String>,
}
impl ConversationKey {
    pub fn new(
        provider: Provider,
        installation: impl Into<String>,
        channel: impl Into<String>,
        thread: Option<String>,
    ) -> Result<Self> {
        let key = Self {
            provider,
            installation: installation.into(),
            channel: channel.into(),
            thread,
        };
        key.validate()?;
        Ok(key)
    }
    pub fn validate(&self) -> Result<()> {
        for value in [&self.installation, &self.channel] {
            if value.is_empty() || value.len() > 256 {
                return Err(Error::InvalidInput("conversation identifier"));
            }
        }
        if self
            .thread
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 256)
        {
            return Err(Error::InvalidInput("thread identifier"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MessageRef {
    pub conversation: ConversationKey,
    pub id: String,
}
impl MessageRef {
    /// Compare provider message identity when a reaction/action omits its root thread.
    pub fn same_message(&self, other: &Self) -> bool {
        self.id == other.id
            && self.conversation.provider == other.conversation.provider
            && self.conversation.installation == other.conversation.installation
            && self.conversation.channel == other.conversation.channel
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub display_name: Option<String>,
    pub is_bot: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub name: Option<String>,
    pub media_type: Option<String>,
    pub size: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub reference: MessageRef,
    pub author: User,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub direct: bool,
    pub mentioned: bool,
    pub reply_to: Option<String>,
    pub timestamp: Option<u64>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReactionChange {
    Added(String),
    Removed(String),
    Set(Option<String>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReactionEvent {
    pub target: MessageRef,
    pub actor: User,
    pub change: ReactionChange,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionEvent {
    pub conversation: ConversationKey,
    pub actor: User,
    pub id: String,
    pub value: String,
    pub message_id: Option<String>,
}
/// Feedback is satisfaction data, never an authorization or approval grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FeedbackValue {
    Positive,
    Negative,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FeedbackSource {
    Reaction(ReactionEvent),
    Action(ActionEvent),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackEvent {
    pub target: MessageRef,
    /// None represents a clear with an unknown previous value.
    pub value: Option<FeedbackValue>,
    pub source: FeedbackSource,
}
impl FeedbackEvent {
    pub fn actor(&self) -> &User {
        match &self.source {
            FeedbackSource::Reaction(event) => &event.actor,
            FeedbackSource::Action(event) => &event.actor,
        }
    }
}
impl ReactionEvent {
    /// Opt-in projection onto a host-correlated reply. Retains removal/set semantics.
    pub fn feedback_for(&self, reply: &MessageRef) -> Option<FeedbackEvent> {
        if !self.target.same_message(reply) {
            return None;
        }
        let value = match &self.change {
            ReactionChange::Added(emoji)
            | ReactionChange::Removed(emoji)
            | ReactionChange::Set(Some(emoji)) => Some(match emoji.as_str() {
                "👍" | "thumbsup" | "+1" => FeedbackValue::Positive,
                "👎" | "thumbsdown" | "-1" => FeedbackValue::Negative,
                _ => return None,
            }),
            ReactionChange::Set(None) => None,
        };
        Some(FeedbackEvent {
            target: reply.clone(),
            value,
            source: FeedbackSource::Reaction(self.clone()),
        })
    }
}
impl ActionEvent {
    /// Project only explicitly configured action IDs or selection values on a known reply.
    pub fn feedback_for(
        &self,
        reply: &MessageRef,
        positive: &str,
        negative: &str,
    ) -> Option<FeedbackEvent> {
        if positive.is_empty() || negative.is_empty() || positive == negative {
            return None;
        }
        let target = MessageRef {
            conversation: self.conversation.clone(),
            id: self.message_id.clone()?,
        };
        if !target.same_message(reply) {
            return None;
        }
        let is_positive = self.id == positive || self.value == positive;
        let is_negative = self.id == negative || self.value == negative;
        if is_positive == is_negative {
            return None;
        }
        let value = if is_positive {
            FeedbackValue::Positive
        } else {
            FeedbackValue::Negative
        };
        Some(FeedbackEvent {
            target: reply.clone(),
            value: Some(value),
            source: FeedbackSource::Action(self.clone()),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandEvent {
    pub conversation: ConversationKey,
    pub actor: User,
    pub command: String,
    pub text: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Accepted,
    Sent,
    Delivered,
    Read,
    Failed,
}
impl DeliveryStatus {
    /// A failure is not an ordering relation; hosts keep its error separately.
    pub fn reconcile(self, next: Self) -> Self {
        if next == Self::Failed {
            return next;
        }
        if self == Self::Failed {
            return self;
        }
        if next as u8 > self as u8 { next } else { self }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusEvent {
    pub target: MessageRef,
    pub status: DeliveryStatus,
    pub error_code: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Control {
    Stop,
    Title(String),
    View(BTreeMap<String, String>),
    IdentityChanged { old: String, new: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Message(Message),
    Reaction(ReactionEvent),
    Action(ActionEvent),
    Command(CommandEvent),
    Status(StatusEvent),
    Updated(Message),
    Deleted(MessageRef),
    Control {
        conversation: ConversationKey,
        /// Missing actors are informational only; hosts must not authorize a command from them.
        actor: Option<User>,
        action: Control,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub kind: EventKind,
}
impl Event {
    pub fn conversation(&self) -> &ConversationKey {
        match &self.kind {
            EventKind::Message(m) | EventKind::Updated(m) => &m.reference.conversation,
            EventKind::Reaction(r) => &r.target.conversation,
            EventKind::Status(s) => &s.target.conversation,
            EventKind::Action(a) => &a.conversation,
            EventKind::Command(c) => &c.conversation,
            EventKind::Deleted(r) => &r.conversation,
            EventKind::Control { conversation, .. } => conversation,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub messages: Vec<MessageRef>,
    pub status: DeliveryStatus,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    pub next_cursor: Option<String>,
    pub complete: bool,
    pub native: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Support {
    Native,
    Emulated,
    Conditional,
    Unsupported,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub reactions: Support,
    pub edit: Support,
    pub delete: Support,
    pub history: Support,
    pub streaming: Support,
    pub typing: Support,
    pub media: Support,
}
