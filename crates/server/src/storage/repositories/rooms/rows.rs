// Room rows (knowledge/runtime-resources/rooms.md).

use crate::kernel_imports::contracts::typed_id::{
    AgentId, RoomId, RoomMemberId, RoomMessageId, SessionId,
};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomRow {
    pub id: RoomId,
    pub org_id: i64,
    pub name: String,
    pub topic: Option<String>,
    pub external_ref: Option<String>,
    pub metadata: serde_json::Value,
    pub link_template: Option<String>,
    pub settings: serde_json::Value,
    pub created_by_user_id: Option<Uuid>,
    pub archived_at: Option<DateTime<Utc>>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateRoomRow {
    pub org_id: i64,
    pub name: String,
    pub topic: Option<String>,
    pub external_ref: Option<String>,
    pub metadata: serde_json::Value,
    pub link_template: Option<String>,
    pub settings: serde_json::Value,
    pub created_by_user_id: Option<Uuid>,
}

/// Fields a room update may change; `None` keeps the stored value.
#[derive(Debug, Clone, Default)]
pub struct UpdateRoomRow {
    pub name: Option<String>,
    /// `Some(None)` clears the topic.
    pub topic: Option<Option<String>>,
    pub metadata: Option<serde_json::Value>,
    pub link_template: Option<Option<String>>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomMemberRow {
    pub id: RoomMemberId,
    pub room_id: RoomId,
    pub org_id: i64,
    pub kind: String,
    pub user_id: Option<Uuid>,
    pub agent_id: Option<AgentId>,
    pub handle: String,
    pub display_name: String,
    pub role: String,
    pub wake: String,
    pub session_scope: String,
    pub instructions: Option<String>,
    pub left_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateRoomMemberRow {
    pub room_id: RoomId,
    pub org_id: i64,
    pub kind: String,
    pub user_id: Option<Uuid>,
    pub agent_id: Option<AgentId>,
    pub handle: String,
    pub display_name: String,
    pub role: String,
    pub wake: String,
    pub session_scope: String,
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateRoomMemberRow {
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub wake: Option<String>,
    pub session_scope: Option<String>,
    pub instructions: Option<Option<String>>,
}

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomMessageRow {
    pub id: RoomMessageId,
    pub room_id: RoomId,
    pub org_id: i64,
    pub author_member_id: Option<RoomMemberId>,
    pub thread_id: Option<RoomMessageId>,
    pub text: String,
    pub parts: serde_json::Value,
    pub mentions: Vec<Uuid>,
    pub hops: i32,
    pub session_id: Option<SessionId>,
    pub source_ref: Option<String>,
    pub reply_count: i32,
    pub last_reply_at: Option<DateTime<Utc>>,
    pub pinned_at: Option<DateTime<Utc>>,
    pub edited_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateRoomMessageRow {
    pub room_id: RoomId,
    pub org_id: i64,
    pub author_member_id: Option<RoomMemberId>,
    pub thread_id: Option<RoomMessageId>,
    pub text: String,
    pub parts: serde_json::Value,
    pub mentions: Vec<Uuid>,
    pub hops: i32,
    pub session_id: Option<SessionId>,
    pub source_ref: Option<String>,
}

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomReactionRow {
    pub message_id: RoomMessageId,
    pub member_id: RoomMemberId,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomReadMarkRow {
    pub member_id: RoomMemberId,
    pub scope_id: Uuid,
    pub room_id: RoomId,
    pub read_through: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomEventRow {
    pub seq: i64,
    pub room_id: RoomId,
    pub org_id: i64,
    pub event_type: String,
    pub message_id: Option<RoomMessageId>,
    pub member_id: Option<RoomMemberId>,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateRoomEventRow {
    pub room_id: RoomId,
    pub org_id: i64,
    pub event_type: String,
    pub message_id: Option<RoomMessageId>,
    pub member_id: Option<RoomMemberId>,
    pub data: serde_json::Value,
}

/// The room conversation an agent session belongs to.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomAgentSessionRow {
    pub member_id: RoomMemberId,
    pub scope_id: Uuid,
    pub room_id: RoomId,
    pub session_id: SessionId,
    pub last_wake_message_id: Option<RoomMessageId>,
    pub last_wake_thread_id: Option<RoomMessageId>,
    pub last_wake_hops: i32,
    pub created_at: DateTime<Utc>,
}

/// One version of a room file, without its content.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomFileVersionRow {
    pub id: Uuid,
    pub room_id: RoomId,
    pub org_id: i64,
    pub file_id: Uuid,
    pub path: String,
    pub version: i32,
    pub action: String,
    pub size_bytes: i64,
    pub moved_from: Option<String>,
    pub author_member_id: Option<RoomMemberId>,
    pub session_id: Option<SessionId>,
    pub created_at: DateTime<Utc>,
}

/// A room file as it is now: the file and its latest version.
#[derive(Debug, Clone, FromRow)]
pub struct RoomFileRow {
    pub file_id: Uuid,
    pub path: String,
    pub size_bytes: i64,
    pub version: i32,
    pub author_member_id: Option<RoomMemberId>,
    pub updated_at: DateTime<Utc>,
}

/// Where a room's files live: the room and the Memory that holds them.
#[derive(Debug, Clone, Copy)]
pub struct RoomFileStore {
    pub room_id: RoomId,
    pub org_id: i64,
    pub memory_id: Uuid,
}

/// Who changed a room file.
#[derive(Debug, Clone, Copy, Default)]
pub struct RoomFileAuthor {
    pub member_id: Option<RoomMemberId>,
    /// The agent session, for an agent writing through `/room`.
    pub session_id: Option<SessionId>,
}

/// What a room file change did.
#[derive(Debug, Clone)]
pub enum RoomFileChange {
    Done(RoomFileVersionRow),
    /// The file is not at the version the caller read: `current` is the
    /// version it is at (`None` when there is no file there).
    Conflict {
        current: Option<i32>,
    },
    NotFound,
    /// The path, or one of its parents, is the wrong kind of entry.
    WrongKind(String),
}
