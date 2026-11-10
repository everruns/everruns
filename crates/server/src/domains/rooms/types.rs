//! Public shapes of the rooms API.

use super::rules::{LoopLimits, SessionScope, WakeMode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An org-owned conversation space for people and agents.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Room {
    /// Prefixed room id (`room_...`).
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// The builder's own id for this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
    /// Free-form data the builder keeps with the room.
    pub metadata: serde_json::Value,
    /// Where a message opens in the builder's product; `{room_id}`,
    /// `{message_id}` and `{thread_id}` are filled in to make each message's
    /// `url`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_template: Option<String>,
    /// Loop limits in force.
    pub loop_limits: LoopLimits,
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Kind of room member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomMemberKind {
    /// A person in the organization.
    Person,
    /// An Everruns agent; the room wakes it.
    Agent,
    /// An agent elsewhere that posts through the API.
    OutsideAgent,
    /// Someone the builder's product speaks for, such as an end user.
    Guest,
}

impl RoomMemberKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent => "agent",
            Self::OutsideAgent => "outside_agent",
            Self::Guest => "guest",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "person" => Self::Person,
            "agent" => Self::Agent,
            "outside_agent" => Self::OutsideAgent,
            _ => Self::Guest,
        }
    }
}

/// Role of a member in its room.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomMemberRole {
    /// Manages the room: members, settings, archive.
    Owner,
    #[default]
    Member,
}

impl RoomMemberRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "owner" => Self::Owner,
            _ => Self::Member,
        }
    }
}

/// A member of a room.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMember {
    /// Prefixed member id (`rmem_...`).
    pub id: String,
    pub room_id: String,
    pub kind: RoomMemberKind,
    /// Mention name, written `@handle`.
    pub handle: String,
    pub display_name: String,
    pub role: RoomMemberRole,
    /// The person's user id, for a person.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// The agent's id, for an Everruns agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// When an agent member is woken.
    pub wake: WakeMode,
    /// Which conversation an agent member keeps.
    pub session_scope: SessionScope,
    /// Extra instructions an agent member gets in this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Whether the member left. Left members still name their old messages.
    pub left: bool,
    pub joined_at: DateTime<Utc>,
}

/// Who wrote a message, as the message shows it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomAuthor {
    pub id: String,
    pub handle: String,
    pub display_name: String,
    pub kind: RoomMemberKind,
}

/// A member a message mentions.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMention {
    pub id: String,
    pub handle: String,
}

/// One emoji on a message, with who added it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomReaction {
    pub emoji: String,
    pub count: usize,
    pub member_ids: Vec<String>,
}

/// A message in a room.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMessage {
    /// Prefixed message id (`rmsg_...`).
    pub id: String,
    pub room_id: String,
    /// The thread it is a reply in (its root message); absent at the top level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Absent for a room notice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<RoomAuthor>,
    /// Markdown. Empty once deleted.
    pub text: String,
    pub mentions: Vec<RoomMention>,
    pub reactions: Vec<RoomReaction>,
    pub reply_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reply_at: Option<DateTime<Utc>>,
    pub pinned: bool,
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<DateTime<Utc>>,
    /// Agent posts in a row up to this message (0 when a person wrote it).
    pub hops: i32,
    /// The agent session that wrote it, for agent posts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Where it opens in the builder's product (the room's `link_template`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// One entry of a room's change feed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomEvent {
    /// Position in the org's room feed; read on with `after`.
    pub seq: i64,
    pub room_id: String,
    /// `room.created`, `room.updated`, `room.archived`, `room.unarchived`,
    /// `member.joined`, `member.left`, `message.posted`, `message.edited`,
    /// `message.deleted`, `message.pinned`, `message.unpinned`,
    /// `reaction.added`, `reaction.removed`, `agent.woken`, `agent.limited`.
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_id: Option<String>,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomListResponse {
    pub data: Vec<Room>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMemberListResponse {
    pub data: Vec<RoomMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMessageListResponse {
    pub data: Vec<RoomMessage>,
    /// Whether older messages exist before the first one returned.
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomEventListResponse {
    pub data: Vec<RoomEvent>,
    /// Pass as `after` to read on.
    pub next_after: i64,
}

/// An agent member a message woke, or did not wake because of a limit.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomWake {
    pub member_id: String,
    pub handle: String,
    /// `woken`, or the loop limit that stopped it (`hops`, `agent_hourly`,
    /// `room_hourly`), or `failed`.
    pub outcome: String,
    /// The agent's session for this conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A posted message and the agents it woke.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PostRoomMessageResponse {
    pub message: RoomMessage,
    pub woke: Vec<RoomWake>,
}

/// Messages a member has not read yet, each with its thread root for
/// context, already marked read.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomUnreadResponse {
    pub data: Vec<RoomMessage>,
    /// Root messages of the threads the unread replies belong to.
    pub threads: Vec<RoomMessage>,
    /// Whether more unread messages remain after this page.
    pub has_more: bool,
}

/// One reader of a scope.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomReader {
    pub member_id: String,
    pub handle: String,
    pub read_through: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomReadersResponse {
    /// Members who have read up to the message.
    pub read_by: Vec<RoomReader>,
    /// Members who have not, the author left out.
    pub not_read_by: Vec<RoomAuthor>,
}

/// A reader's position in one scope.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomReadMark {
    /// The room id for the top level, or a thread's root message id.
    pub scope_id: String,
    pub read_through: DateTime<Utc>,
}
