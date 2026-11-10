//! Public shapes of the rooms API.

use super::rules::{LoopLimits, SessionScope, WakeMode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An org-owned conversation space for people and agents.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Room {
    /// Prefixed room id (`room_...`).
    #[schema(example = "room_01933b5a000070008000000000000001")]
    pub id: String,
    /// Room name.
    #[schema(example = "Support desk")]
    pub name: String,
    /// What the room is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Customer escalations")]
    pub topic: Option<String>,
    /// The builder's own id for this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "ticket-4521")]
    pub external_ref: Option<String>,
    /// Free-form data the builder keeps with the room.
    #[schema(example = json!({"product": "helpdesk"}))]
    pub metadata: serde_json::Value,
    /// Where a message opens in the builder's product; `{room_id}`,
    /// `{message_id}` and `{thread_id}` are filled in to make each message's
    /// `url`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://desk.example.com/rooms/{room_id}/m/{message_id}")]
    pub link_template: Option<String>,
    /// Loop limits in force.
    pub loop_limits: LoopLimits,
    /// Whether the room is archived: it keeps everything but takes no posts and wakes no agents.
    #[schema(example = false)]
    pub archived: bool,
    /// When the last message was posted.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub last_message_at: Option<DateTime<Utc>>,
    /// When the room was created.
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub created_at: DateTime<Utc>,
    /// When the room last changed.
    #[schema(example = "2026-11-10T15:04:05Z")]
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
    #[schema(example = "rmem_01933b5a000070008000000000000003")]
    pub id: String,
    /// The room this member belongs to.
    #[schema(example = "room_01933b5a000070008000000000000001")]
    pub room_id: String,
    /// Person, Everruns agent, outside agent or guest.
    pub kind: RoomMemberKind,
    /// Mention name, written `@handle`.
    #[schema(example = "triage")]
    pub handle: String,
    /// Name shown with the member's messages.
    #[schema(example = "Triage agent")]
    pub display_name: String,
    /// Owner or member.
    pub role: RoomMemberRole,
    /// The person's user id, for a person.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "user_01933b5a000070008000000000000004")]
    pub user_id: Option<String>,
    /// The agent's id, for an Everruns agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "agent_01933b5a000070008000000000000005")]
    pub agent_id: Option<String>,
    /// When an agent member is woken.
    pub wake: WakeMode,
    /// Which conversation an agent member keeps.
    pub session_scope: SessionScope,
    /// Extra instructions an agent member gets in this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Answer billing questions only.")]
    pub instructions: Option<String>,
    /// Whether the member left. Left members still name their old messages.
    #[schema(example = false)]
    pub left: bool,
    /// When the member joined.
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub joined_at: DateTime<Utc>,
}

/// Who wrote a message, as the message shows it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomAuthor {
    /// Member id of the author.
    #[schema(example = "rmem_01933b5a000070008000000000000003")]
    pub id: String,
    /// Mention name, written `@handle`.
    #[schema(example = "triage")]
    pub handle: String,
    /// Name shown with the message.
    #[schema(example = "Triage agent")]
    pub display_name: String,
    /// Person, Everruns agent, outside agent or guest.
    pub kind: RoomMemberKind,
}

/// A member a message mentions.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMention {
    /// Member id of the mentioned member.
    #[schema(example = "rmem_01933b5a000070008000000000000003")]
    pub id: String,
    /// Mention name as written.
    #[schema(example = "triage")]
    pub handle: String,
}

/// One emoji on a message, with who added it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomReaction {
    /// The emoji, as text.
    #[schema(example = "👀")]
    pub emoji: String,
    /// How many members added it.
    #[schema(example = 2)]
    pub count: usize,
    /// Members who added it.
    #[schema(example = json!(["rmem_01933b5a000070008000000000000003"]))]
    pub member_ids: Vec<String>,
}

/// A message in a room.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMessage {
    /// Prefixed message id (`rmsg_...`).
    #[schema(example = "rmsg_01933b5a000070008000000000000002")]
    pub id: String,
    /// The room it was posted in.
    #[schema(example = "room_01933b5a000070008000000000000001")]
    pub room_id: String,
    /// The thread it is a reply in (its root message); absent at the top level.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "rmsg_01933b5a000070008000000000000002")]
    pub thread_id: Option<String>,
    /// Absent for a room notice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<RoomAuthor>,
    /// Markdown. Empty once deleted.
    #[schema(example = "@triage can you check ticket 4521?")]
    pub text: String,
    /// Members the text mentions, resolved when it was posted.
    pub mentions: Vec<RoomMention>,
    /// Reactions, one entry per emoji.
    pub reactions: Vec<RoomReaction>,
    /// Replies in its thread, for a top-level message.
    #[schema(example = 3)]
    pub reply_count: i32,
    /// When its thread last got a reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub last_reply_at: Option<DateTime<Utc>>,
    /// Whether it is pinned to the room.
    #[schema(example = false)]
    pub pinned: bool,
    /// Whether it was deleted; a marker stays in its place.
    #[schema(example = false)]
    pub deleted: bool,
    /// When it was last edited.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub edited_at: Option<DateTime<Utc>>,
    /// Agent posts in a row up to this message (0 when a person wrote it).
    #[schema(example = 0)]
    pub hops: i32,
    /// The agent session that wrote it, for agent posts.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "session_01933b5a000070008000000000000006")]
    pub session_id: Option<String>,
    /// Where it opens in the builder's product (the room's `link_template`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(
        example = "https://desk.example.com/rooms/room_01933b5a000070008000000000000001/m/rmsg_01933b5a000070008000000000000002"
    )]
    pub url: Option<String>,
    /// When it was posted.
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub created_at: DateTime<Utc>,
}

/// One entry of a room's change feed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomEvent {
    /// Position in the org's room feed; read on with `after`.
    #[schema(example = 120)]
    pub seq: i64,
    /// The room it happened in.
    #[schema(example = "room_01933b5a000070008000000000000001")]
    pub room_id: String,
    /// `room.created`, `room.updated`, `room.archived`, `room.unarchived`,
    /// `member.joined`, `member.left`, `message.posted`, `message.edited`,
    /// `message.deleted`, `message.pinned`, `message.unpinned`,
    /// `reaction.added`, `reaction.removed`, `agent.woken`, `agent.limited`.
    #[serde(rename = "type")]
    #[schema(example = "message.posted")]
    pub event_type: String,
    /// The message it is about, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "rmsg_01933b5a000070008000000000000002")]
    pub message_id: Option<String>,
    /// The member it is about or by, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "rmem_01933b5a000070008000000000000003")]
    pub member_id: Option<String>,
    /// Details of the change; the shape depends on `type`.
    #[schema(example = json!({"hops": 0}))]
    pub data: serde_json::Value,
    /// When it happened.
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub created_at: DateTime<Utc>,
}

/// A page of rooms.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomListResponse {
    /// Rooms, most recently active first.
    pub data: Vec<Room>,
}

/// A room's members.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMemberListResponse {
    /// Members, in the order they joined.
    pub data: Vec<RoomMember>,
}

/// A page of messages.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomMessageListResponse {
    /// Messages, oldest first.
    pub data: Vec<RoomMessage>,
    /// Whether older messages exist before the first one returned.
    #[schema(example = false)]
    pub has_more: bool,
}

/// A page of a room change feed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomEventListResponse {
    /// Events, in order.
    pub data: Vec<RoomEvent>,
    /// Pass as `after` to read on.
    #[schema(example = 120)]
    pub next_after: i64,
}

/// An agent member a message woke, or did not wake because of a limit.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomWake {
    /// The agent member.
    #[schema(example = "rmem_01933b5a000070008000000000000003")]
    pub member_id: String,
    /// Its mention name.
    #[schema(example = "triage")]
    pub handle: String,
    /// `woken`, or the loop limit that stopped it (`hops`, `agent_hourly`,
    /// `room_hourly`), or `failed`.
    #[schema(example = "woken")]
    pub outcome: String,
    /// The agent's session for this conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "session_01933b5a000070008000000000000006")]
    pub session_id: Option<String>,
}

/// A posted message and the agents it woke.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PostRoomMessageResponse {
    /// The posted message.
    pub message: RoomMessage,
    /// Agent members the message addressed, and what happened to each.
    pub woke: Vec<RoomWake>,
}

/// Messages a member has not read yet, each with its thread root for
/// context, already marked read.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomUnreadResponse {
    /// Unread messages, oldest first.
    pub data: Vec<RoomMessage>,
    /// Root messages of the threads the unread replies belong to.
    pub threads: Vec<RoomMessage>,
    /// Whether more unread messages remain after this page.
    #[schema(example = false)]
    pub has_more: bool,
}

/// One reader of a scope.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomReader {
    /// The reader.
    #[schema(example = "rmem_01933b5a000070008000000000000003")]
    pub member_id: String,
    /// Its mention name.
    #[schema(example = "ana-lopez")]
    pub handle: String,
    /// How far it has read.
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub read_through: DateTime<Utc>,
}

/// Who has read a message, and who has not.
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
    #[schema(example = "room_01933b5a000070008000000000000001")]
    pub scope_id: String,
    /// How far the reader has read.
    #[schema(example = "2026-11-10T15:04:05Z")]
    pub read_through: DateTime<Utc>,
}
