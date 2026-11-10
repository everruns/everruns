//! Room commands: the `/v1/rooms` API (knowledge/runtime-resources/rooms.md).
//!
//! Access: every org member can see the org's rooms and read them. People post
//! as themselves and join a room by posting in it. Org admins (and API keys
//! with that role) may act as any member with `as_member`, which is how a
//! builder's product posts for its guests and outside agents. A room's owners
//! and org admins manage its members and settings.

use super::queries::{self as q, RoomView};
use super::rules::{self, LoopLimits, SessionScope, WakeMode};
use super::types::*;
use super::wake::{Waker, wake_agents};
use crate::domains::common::*;
use crate::storage::{
    CreateRoomMemberRow, CreateRoomMessageRow, CreateRoomRow, RoomMemberRow, RoomMessageRow,
    RoomRow, UpdateRoomMemberRow, UpdateRoomRow,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Longest message, in characters.
const MAX_TEXT_CHARS: usize = 40_000;
const MAX_NAME_CHARS: usize = 120;
const MAX_TOPIC_CHARS: usize = 1_000;
const MAX_INSTRUCTIONS_CHARS: usize = 8_000;
const MAX_METADATA_BYTES: usize = 16 * 1024;

fn clean_name(name: &str) -> Result<String, CommandError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(CommandError::bad_request(format!(
            "Room name must be 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name.to_string())
}

fn clean_optional(
    value: Option<String>,
    max: usize,
    field: &str,
) -> Result<Option<String>, CommandError> {
    let Some(value) = value else { return Ok(None) };
    let value = value.trim().to_string();
    if value.chars().count() > max {
        return Err(CommandError::bad_request(format!(
            "{field} is longer than {max} characters"
        )));
    }
    Ok((!value.is_empty()).then_some(value))
}

fn clean_text(text: &str) -> Result<String, CommandError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(CommandError::bad_request("Message text is empty"));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(CommandError::bad_request(format!(
            "Message text is longer than {MAX_TEXT_CHARS} characters"
        )));
    }
    Ok(text.to_string())
}

fn check_metadata(metadata: &serde_json::Value) -> Result<(), CommandError> {
    if !metadata.is_object() {
        return Err(CommandError::bad_request("metadata must be an object"));
    }
    if serde_json::to_vec(metadata).map_or(0, |bytes| bytes.len()) > MAX_METADATA_BYTES {
        return Err(CommandError::bad_request("metadata is larger than 16 KB"));
    }
    Ok(())
}

fn check_link_template(template: &Option<String>) -> Result<(), CommandError> {
    if let Some(template) = template
        && !rules::is_valid_link_template(template)
    {
        return Err(CommandError::bad_request(
            "link_template must be an https URL",
        ));
    }
    Ok(())
}

fn settings_with_limits(
    mut settings: serde_json::Value,
    limits: Option<LoopLimits>,
) -> serde_json::Value {
    if let Some(limits) = limits {
        let limits = limits.clamped();
        if !settings.is_object() {
            settings = json!({});
        }
        settings["loop_limits"] = json!(limits);
    }
    settings
}

/// A member to add, by kind.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct RoomMemberInput {
    pub kind: RoomMemberKind,
    /// For a person: their user id.
    pub user_id: Option<String>,
    /// For an Everruns agent: its id.
    pub agent_id: Option<String>,
    /// Mention name; derived from the display name when absent.
    pub handle: Option<String>,
    /// Shown name; the person's or agent's own name when absent. Required for
    /// outside agents and guests.
    pub display_name: Option<String>,
    pub role: Option<RoomMemberRole>,
    /// When an agent member is woken. Default `mentions`.
    pub wake: Option<WakeMode>,
    /// Which conversation an agent member keeps. Default `thread`.
    pub session_scope: Option<SessionScope>,
    /// Extra instructions an agent member gets in this room.
    pub instructions: Option<String>,
}

async fn add_member(
    ctx: &Ctx,
    room: &RoomRow,
    input: RoomMemberInput,
) -> Result<RoomMemberRow, CommandError> {
    let role = input.role.unwrap_or_default().as_str().to_string();
    let instructions = clean_optional(input.instructions, MAX_INSTRUCTIONS_CHARS, "instructions")?;
    if let Some(handle) = &input.handle
        && !rules::is_valid_handle(handle)
    {
        return Err(CommandError::bad_request(
            "handle must be lowercase letters, digits, '_', '-' or '.', starting with a letter or digit, at most 40 characters",
        ));
    }
    match input.kind {
        RoomMemberKind::Person => {
            let user_id: Uuid = input
                .user_id
                .as_deref()
                .ok_or_else(|| CommandError::bad_request("A person needs user_id"))?
                .parse()
                .map_err(|_| CommandError::bad_request("Invalid user_id"))?;
            // People keep the handle derived from their name, so the same
            // person reads the same in every room; the shown name can differ.
            let member = q::add_person(ctx, room, user_id, &role).await?;
            match clean_optional(input.display_name, MAX_NAME_CHARS, "display_name")? {
                Some(display_name) => Ok(ctx
                    .db
                    .update_room_member(
                        room.id,
                        member.id,
                        UpdateRoomMemberRow {
                            display_name: Some(display_name),
                            ..Default::default()
                        },
                    )
                    .await?
                    .unwrap_or(member)),
                None => Ok(member),
            }
        }
        RoomMemberKind::Agent => {
            let agent = q::load_agent(
                ctx,
                input
                    .agent_id
                    .as_deref()
                    .ok_or_else(|| CommandError::bad_request("An agent needs agent_id"))?,
            )
            .await?;
            let display_name = input.display_name.unwrap_or_else(|| agent.name.clone());
            let derived = input.handle.is_none();
            let handle = match input.handle {
                Some(handle) => handle,
                None => rules::handle_from_name(&display_name).unwrap_or_else(|| "agent".into()),
            };
            q::insert_member(
                ctx,
                room,
                CreateRoomMemberRow {
                    room_id: room.id,
                    org_id: room.org_id,
                    kind: RoomMemberKind::Agent.as_str().into(),
                    user_id: None,
                    agent_id: Some(agent.id),
                    handle,
                    display_name,
                    role,
                    wake: input.wake.unwrap_or_default().as_str().into(),
                    session_scope: input.session_scope.unwrap_or_default().as_str().into(),
                    instructions,
                },
                derived,
            )
            .await
        }
        RoomMemberKind::OutsideAgent | RoomMemberKind::Guest => {
            let display_name = clean_optional(input.display_name, MAX_NAME_CHARS, "display_name")?
                .ok_or_else(|| {
                    CommandError::bad_request("Outside agents and guests need display_name")
                })?;
            let derived = input.handle.is_none();
            let handle = match input.handle {
                Some(handle) => handle,
                None => rules::handle_from_name(&display_name).unwrap_or_else(|| "guest".into()),
            };
            q::insert_member(
                ctx,
                room,
                CreateRoomMemberRow {
                    room_id: room.id,
                    org_id: room.org_id,
                    kind: input.kind.as_str().into(),
                    user_id: None,
                    agent_id: None,
                    handle,
                    display_name,
                    role,
                    wake: WakeMode::Never.as_str().into(),
                    session_scope: SessionScope::Thread.as_str().into(),
                    instructions: None,
                },
                derived,
            )
            .await
        }
    }
}

async fn member_out(ctx: &Ctx, row: &RoomMemberRow) -> Result<RoomMember, CommandError> {
    let ids = q::agent_public_ids(&ctx.db, ctx.org_id(), std::slice::from_ref(row)).await?;
    Ok(q::row_to_member(row, &ids))
}

// ============================================================================
// Rooms
// ============================================================================

/// Create a room. The person creating it becomes its first owner.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateRoom {
    pub name: String,
    pub topic: Option<String>,
    /// Your own id for this room (a ticket, a project). Unique in the org.
    pub external_ref: Option<String>,
    /// Free-form data kept with the room, up to 16 KB.
    pub metadata: Option<serde_json::Value>,
    /// Where a message opens in your product: `{room_id}`, `{message_id}`
    /// and `{thread_id}` are filled in.
    pub link_template: Option<String>,
    /// Loop limits; defaults apply when absent.
    pub loop_limits: Option<LoopLimits>,
    /// Members to add right away.
    #[serde(default)]
    pub members: Vec<RoomMemberInput>,
}

#[command(
    name = "create_room",
    category = "rooms",
    description = "Create a room where people and agents talk. The caller becomes its first owner; members can be added in the same call.",
    method = "POST",
    path = "/v1/rooms",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "create").with_examples(&[CliExample::new("Open a room where a support agent and your people talk", "everruns rooms create --name support-desk --topic 'Customer escalations' --members '[{\"kind\":\"agent\",\"agent_id\":\"agent_01h9\"}]'")]),
    http = plain,
    responses((status = 409, description = "external_ref is already used by another room")),
)]
impl Command for CreateRoom {
    type Output = Room;

    async fn execute(self, ctx: &Ctx) -> Result<Room, CommandError> {
        let metadata = self.metadata.unwrap_or_else(|| json!({}));
        check_metadata(&metadata)?;
        check_link_template(&self.link_template)?;
        let external_ref = clean_optional(self.external_ref, 200, "external_ref")?;
        if let Some(external_ref) = &external_ref
            && ctx
                .db
                .get_room_by_external_ref(ctx.org_id(), external_ref)
                .await?
                .is_some()
        {
            return Err(CommandError::conflict(
                "Another room already uses this external_ref".to_string(),
            ));
        }
        let room = ctx
            .db
            .create_room(CreateRoomRow {
                org_id: ctx.org_id(),
                name: clean_name(&self.name)?,
                topic: clean_optional(self.topic, MAX_TOPIC_CHARS, "topic")?,
                external_ref,
                metadata,
                link_template: self.link_template.map(|t| t.trim().to_string()),
                settings: settings_with_limits(json!({}), self.loop_limits),
                created_by_user_id: ctx.caller.user_id,
            })
            .await?;
        if let Some(user_id) = ctx.caller.user_id {
            q::add_person(ctx, &room, user_id, "owner").await?;
        }
        for member in self.members {
            add_member(ctx, &room, member).await?;
        }
        Ok(q::row_to_room(&room))
    }
}

#[derive(Debug, Default, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRooms {
    /// Include archived rooms.
    pub include_archived: Option<bool>,
    /// Only rooms the caller is a member of.
    pub mine: Option<bool>,
    /// The room with this external_ref.
    pub external_ref: Option<String>,
    pub limit: Option<i64>,
}

#[command(
    name = "list_rooms",
    category = "rooms",
    description = "List the org's rooms, most recently active first.",
    method = "GET",
    path = "/v1/rooms",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "list").with_examples(&[CliExample::new("Find the room your product made for a ticket", "everruns rooms list --external-ref ticket-4521")]),
    http = plain,
    params(ListRooms),
)]
impl Command for ListRooms {
    type Output = RoomListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomListResponse, CommandError> {
        if let Some(external_ref) = self.external_ref.as_deref() {
            let room = ctx
                .db
                .get_room_by_external_ref(ctx.org_id(), external_ref)
                .await?;
            return Ok(RoomListResponse {
                data: room.iter().map(q::row_to_room).collect(),
            });
        }
        let member_user_id = if self.mine.unwrap_or(false) {
            ctx.caller.user_id
        } else {
            None
        };
        let rows = ctx
            .db
            .list_rooms(
                ctx.org_id(),
                self.include_archived.unwrap_or(false),
                member_user_id,
                self.limit.unwrap_or(50).clamp(1, 200),
            )
            .await?;
        Ok(RoomListResponse {
            data: rows.iter().map(q::row_to_room).collect(),
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetRoom {
    /// Prefixed room id.
    pub room_id: String,
}

#[command(
    name = "get_room",
    category = "rooms",
    description = "Get a room.",
    method = "GET",
    path = "/v1/rooms/{room_id}",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "get").with_args(&[CliArg::new("room_id").at(1)]).with_examples(&[CliExample::new("Check a room's topic, settings and last activity", "everruns rooms get room_01h9")]),
    positional = "room_id",
    http = plain,
    responses((status = 404, description = "Room not found")),
)]
impl Command for GetRoom {
    type Output = Room;

    async fn execute(self, ctx: &Ctx) -> Result<Room, CommandError> {
        Ok(q::row_to_room(&q::load_room(ctx, &self.room_id).await?))
    }
}

/// Change a room. Only provided fields change; send `""` to clear the topic
/// or the link template.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateRoom {
    pub room_id: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub link_template: Option<String>,
    pub loop_limits: Option<LoopLimits>,
}

#[command(
    name = "update_room",
    category = "rooms",
    description = "Change a room's name, topic, metadata, link template or loop limits. Owners and org admins only.",
    method = "PATCH",
    path = "/v1/rooms/{room_id}",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "update").with_examples(&[CliExample::new("Change what a room is about", "everruns rooms update --room-id room_01h9 --topic 'Release 0.52 coordination'")]),
    http = plain,
)]
impl Command for UpdateRoom {
    type Output = Room;

    async fn execute(self, ctx: &Ctx) -> Result<Room, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        q::require_manage(ctx, &room).await?;
        if let Some(metadata) = &self.metadata {
            check_metadata(metadata)?;
        }
        let link_template = self.link_template.map(|t| {
            let t = t.trim().to_string();
            (!t.is_empty()).then_some(t)
        });
        if let Some(Some(template)) = &link_template {
            check_link_template(&Some(template.clone()))?;
        }
        let settings = self
            .loop_limits
            .map(|limits| settings_with_limits(room.settings.clone(), Some(limits)));
        let row = ctx
            .db
            .update_room(
                ctx.org_id(),
                room.id,
                UpdateRoomRow {
                    name: self.name.as_deref().map(clean_name).transpose()?,
                    topic: match self.topic {
                        Some(topic) => Some(clean_optional(Some(topic), MAX_TOPIC_CHARS, "topic")?),
                        None => None,
                    },
                    metadata: self.metadata,
                    link_template,
                    settings,
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Room"))?;
        Ok(q::row_to_room(&row))
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ArchiveRoom {
    pub room_id: String,
    /// `false` brings an archived room back.
    #[serde(default = "default_true")]
    pub archived: bool,
}

fn default_true() -> bool {
    true
}

#[command(
    name = "archive_room",
    category = "rooms",
    description = "Archive a room (or bring it back with archived=false). An archived room keeps everything but takes no posts and wakes no agents.",
    method = "POST",
    path = "/v1/rooms/{room_id}/archive",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "archive").with_examples(&[CliExample::new("Close a room once its work is done", "everruns rooms archive --room-id room_01h9")]),
    http = plain,
)]
impl Command for ArchiveRoom {
    type Output = Room;

    async fn execute(self, ctx: &Ctx) -> Result<Room, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        q::require_manage(ctx, &room).await?;
        let row = ctx
            .db
            .set_room_archived(ctx.org_id(), room.id, self.archived)
            .await?
            .ok_or_else(|| CommandError::not_found("Room"))?;
        Ok(q::row_to_room(&row))
    }
}

// ============================================================================
// Members
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AddRoomMember {
    pub room_id: String,
    #[serde(flatten)]
    pub member: RoomMemberInput,
}

#[command(
    name = "add_room_member",
    category = "rooms",
    description = "Add a person, an Everruns agent, an outside agent or a guest to a room. Owners and org admins only.",
    method = "POST",
    path = "/v1/rooms/{room_id}/members",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "members"], "add").with_examples(&[CliExample::new("Add an agent that answers only when mentioned", "everruns rooms members add --room-id room_01h9 --kind agent --agent-id agent_01h9 --wake mentions")]),
    http = plain,
    responses((status = 409, description = "Handle taken, or already a member")),
)]
impl Command for AddRoomMember {
    type Output = RoomMember;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMember, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        q::require_manage(ctx, &room).await?;
        q::require_open(&room)?;
        let row = add_member(ctx, &room, self.member).await?;
        member_out(ctx, &row).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRoomMembers {
    #[param(ignore)]
    pub room_id: String,
    /// Include members who left.
    pub include_left: Option<bool>,
}

#[command(
    name = "list_room_members",
    category = "rooms",
    description = "List a room's members.",
    method = "GET",
    path = "/v1/rooms/{room_id}/members",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "members"], "list").with_args(&[CliArg::new("room_id").at(1)]).with_examples(&[CliExample::new("See who is in a room and how each agent is woken", "everruns rooms members list room_01h9")]),
    positional = "room_id",
    http = plain,
    params(ListRoomMembers),
)]
impl Command for ListRoomMembers {
    type Output = RoomMemberListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMemberListResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let rows = ctx
            .db
            .list_room_members(room.id, self.include_left.unwrap_or(false))
            .await?;
        let ids = q::agent_public_ids(&ctx.db, ctx.org_id(), &rows).await?;
        Ok(RoomMemberListResponse {
            data: rows.iter().map(|row| q::row_to_member(row, &ids)).collect(),
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateRoomMember {
    pub room_id: String,
    pub member_id: String,
    pub display_name: Option<String>,
    pub role: Option<RoomMemberRole>,
    pub wake: Option<WakeMode>,
    pub session_scope: Option<SessionScope>,
    /// Send `""` to clear.
    pub instructions: Option<String>,
}

#[command(
    name = "update_room_member",
    category = "rooms",
    description = "Change a room member's settings. Name, role, wake setting, session scope or instructions; owners and org admins only.",
    method = "PATCH",
    path = "/v1/rooms/{room_id}/members/{member_id}",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "members"], "update").with_examples(&[CliExample::new("Let an agent read every message in the room", "everruns rooms members update --room-id room_01h9 --member-id rmem_01h9 --wake everything")]),
    http = plain,
)]
impl Command for UpdateRoomMember {
    type Output = RoomMember;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMember, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        q::require_manage(ctx, &room).await?;
        let member_id = q::parse_member_id(&self.member_id)?;
        let instructions = match self.instructions {
            Some(text) => Some(clean_optional(
                Some(text),
                MAX_INSTRUCTIONS_CHARS,
                "instructions",
            )?),
            None => None,
        };
        let row = ctx
            .db
            .update_room_member(
                room.id,
                member_id,
                UpdateRoomMemberRow {
                    display_name: clean_optional(
                        self.display_name,
                        MAX_NAME_CHARS,
                        "display_name",
                    )?,
                    role: self.role.map(|role| role.as_str().to_string()),
                    wake: self.wake.map(|wake| wake.as_str().to_string()),
                    session_scope: self.session_scope.map(|scope| scope.as_str().to_string()),
                    instructions,
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Room member"))?;
        member_out(ctx, &row).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RemoveRoomMember {
    pub room_id: String,
    pub member_id: String,
}

#[command(
    name = "remove_room_member",
    category = "rooms",
    description = "Remove a member from a room. Their messages stay. Owners and org admins can remove anyone; a person can leave on their own.",
    method = "DELETE",
    path = "/v1/rooms/{room_id}/members/{member_id}",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "members"], "remove").with_examples(&[CliExample::new("Take an agent out of a room", "everruns rooms members remove --room-id room_01h9 --member-id rmem_01h9")]),
    http = plain,
)]
impl Command for RemoveRoomMember {
    type Output = RoomMember;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMember, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let member_id = q::parse_member_id(&self.member_id)?;
        let member = ctx
            .db
            .get_room_member(room.id, member_id)
            .await?
            .filter(|member| member.left_at.is_none())
            .ok_or_else(|| CommandError::not_found("Room member"))?;
        let leaving_self = ctx.caller.user_id.is_some() && member.user_id == ctx.caller.user_id;
        if !leaving_self {
            q::require_manage(ctx, &room).await?;
        }
        let row = ctx
            .db
            .remove_room_member(room.id, member_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Room member"))?;
        member_out(ctx, &row).await
    }
}

// ============================================================================
// Messages
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PostRoomMessage {
    pub room_id: String,
    /// Markdown. `@handle` mentions a member; mentioning an agent wakes it.
    pub text: String,
    /// Reply in the thread of this message (replies are one level deep: a
    /// reply to a reply lands in the same thread).
    pub reply_to: Option<String>,
    /// Post as this member (org admins: a builder's product posting for a
    /// guest or an outside agent).
    pub as_member: Option<String>,
}

#[command(
    name = "post_room_message",
    category = "rooms",
    description = "Post a message in a room, at the top level or as a reply. Returns the message and the agents it woke.",
    method = "POST",
    path = "/v1/rooms/{room_id}/messages",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "post").with_examples(&[CliExample::new("Ask an agent in the room to look at something", "everruns rooms messages post --room-id room_01h9 --text '@triage can you look at ticket 4521?'")]),
    http = plain,
)]
impl Command for PostRoomMessage {
    type Output = PostRoomMessageResponse;

    async fn execute(self, ctx: &Ctx) -> Result<PostRoomMessageResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        q::require_open(&room)?;
        let text = clean_text(&self.text)?;
        let author = q::acting_member(ctx, &room, self.as_member.as_deref(), true).await?;
        let thread_id = match self.reply_to.as_deref() {
            Some(raw) => {
                let parent = ctx
                    .db
                    .get_room_message(room.id, q::parse_message_id(raw)?)
                    .await?
                    .ok_or_else(|| CommandError::not_found("Message"))?;
                Some(parent.thread_id.unwrap_or(parent.id))
            }
            None => None,
        };
        let view = RoomView::load(&ctx.db, room).await?;
        let row = ctx
            .db
            .post_room_message(CreateRoomMessageRow {
                room_id: view.room.id,
                org_id: view.room.org_id,
                author_member_id: Some(author.id),
                thread_id,
                parts: json!([{ "type": "text", "text": text }]),
                mentions: view.mentions_in(&text),
                text,
                // A person, guest or outside agent writing resets the chain.
                hops: 0,
                session_id: None,
                source_ref: None,
            })
            .await?
            .ok_or_else(|| CommandError::internal(anyhow::anyhow!("message was not stored")))?;
        // Your own post is read.
        ctx.db
            .mark_room_read(
                view.room.id,
                author.id,
                row.thread_id.map_or(view.room.id.uuid(), |id| id.uuid()),
                row.created_at,
            )
            .await?;
        let woke = match (&ctx.session_service, &ctx.message_service) {
            (Some(sessions), Some(messages)) => {
                wake_agents(
                    &Waker {
                        db: &ctx.db,
                        sessions,
                        messages,
                    },
                    &view,
                    &row,
                )
                .await?
            }
            _ => Vec::new(),
        };
        Ok(PostRoomMessageResponse {
            message: view.render_one(&ctx.db, &row).await?,
            woke,
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRoomMessages {
    #[param(ignore)]
    pub room_id: String,
    /// Messages before this time (page backwards).
    pub before: Option<DateTime<Utc>>,
    /// Messages after this time (read forward).
    pub after: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

#[command(
    name = "list_room_messages",
    category = "rooms",
    description = "Read a room's top-level messages, oldest first. Without a position, the newest page.",
    method = "GET",
    path = "/v1/rooms/{room_id}/messages",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "list").with_args(&[CliArg::new("room_id").at(1)]).with_examples(&[CliExample::new("Read the latest messages at a room's top level", "everruns rooms messages list room_01h9 --limit 20")]),
    positional = "room_id",
    http = plain,
    params(ListRoomMessages),
)]
impl Command for ListRoomMessages {
    type Output = RoomMessageListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessageListResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        page(ctx, room, None, self.before, self.after, self.limit).await
    }
}

async fn page(
    ctx: &Ctx,
    room: RoomRow,
    thread_id: Option<crate::kernel_imports::contracts::typed_id::RoomMessageId>,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limit: Option<i64>,
) -> Result<RoomMessageListResponse, CommandError> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let mut rows = ctx
        .db
        .list_room_messages(room.id, thread_id, before, after, limit + 1)
        .await?;
    let forward = after.is_some() && before.is_none();
    let has_more = rows.len() as i64 > limit;
    if has_more {
        if forward {
            rows.pop();
        } else {
            rows.remove(0);
        }
    }
    let view = RoomView::load(&ctx.db, room).await?;
    Ok(RoomMessageListResponse {
        data: view.render(&ctx.db, &rows).await?,
        has_more,
    })
}

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRoomReplies {
    #[param(ignore)]
    pub room_id: String,
    #[param(ignore)]
    pub message_id: String,
    pub before: Option<DateTime<Utc>>,
    pub after: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

#[command(
    name = "list_room_replies",
    category = "rooms",
    description = "Read the replies in a message's thread, oldest first.",
    method = "GET",
    path = "/v1/rooms/{room_id}/messages/{message_id}/replies",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "replies").with_examples(&[CliExample::new("Read the replies in one thread", "everruns rooms messages replies --room-id room_01h9 --message-id rmsg_01h9")]),
    http = plain,
    params(ListRoomReplies),
)]
impl Command for ListRoomReplies {
    type Output = RoomMessageListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessageListResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let root = ctx
            .db
            .get_room_message(room.id, q::parse_message_id(&self.message_id)?)
            .await?
            .ok_or_else(|| CommandError::not_found("Message"))?;
        let thread = root.thread_id.unwrap_or(root.id);
        page(ctx, room, Some(thread), self.before, self.after, self.limit).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetRoomMessage {
    pub room_id: String,
    pub message_id: String,
}

#[command(
    name = "get_room_message",
    category = "rooms",
    description = "Get one message.",
    method = "GET",
    path = "/v1/rooms/{room_id}/messages/{message_id}",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "get").with_examples(&[CliExample::new("Open one message with its reactions", "everruns rooms messages get --room-id room_01h9 --message-id rmsg_01h9")]),
    http = plain,
)]
impl Command for GetRoomMessage {
    type Output = RoomMessage;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessage, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let row = ctx
            .db
            .get_room_message(room.id, q::parse_message_id(&self.message_id)?)
            .await?
            .ok_or_else(|| CommandError::not_found("Message"))?;
        let view = RoomView::load(&ctx.db, room).await?;
        view.render_one(&ctx.db, &row).await
    }
}

/// Load a message the caller may change: its author, or a room manager.
async fn own_message(
    ctx: &Ctx,
    room: &RoomRow,
    message_id: &str,
    as_member: Option<&str>,
) -> Result<RoomMessageRow, CommandError> {
    let row = ctx
        .db
        .get_room_message(room.id, q::parse_message_id(message_id)?)
        .await?
        .filter(|row| row.deleted_at.is_none())
        .ok_or_else(|| CommandError::not_found("Message"))?;
    if q::can_manage(ctx, room).await? && as_member.is_none() {
        return Ok(row);
    }
    let actor = q::acting_member(ctx, room, as_member, false).await?;
    if row.author_member_id != Some(actor.id) {
        return Err(CommandError::forbidden(
            "Only the author or a room owner can change this message".to_string(),
        ));
    }
    Ok(row)
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct EditRoomMessage {
    pub room_id: String,
    pub message_id: String,
    pub text: String,
    pub as_member: Option<String>,
}

#[command(
    name = "edit_room_message",
    category = "rooms",
    description = "Change a message's text. Its author, or a room owner. Editing never wakes agents again.",
    method = "PATCH",
    path = "/v1/rooms/{room_id}/messages/{message_id}",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "edit").with_examples(&[CliExample::new("Fix a typo in your message", "everruns rooms messages edit --room-id room_01h9 --message-id rmsg_01h9 --text 'Deploy is at 15:00 CT'")]),
    http = plain,
)]
impl Command for EditRoomMessage {
    type Output = RoomMessage;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessage, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        q::require_open(&room)?;
        let row = own_message(ctx, &room, &self.message_id, self.as_member.as_deref()).await?;
        let text = clean_text(&self.text)?;
        let view = RoomView::load(&ctx.db, room).await?;
        let mentions = view.mentions_in(&text);
        let row = ctx
            .db
            .edit_room_message(
                view.room.id,
                row.id,
                &text,
                &json!([{ "type": "text", "text": text }]),
                &mentions,
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Message"))?;
        view.render_one(&ctx.db, &row).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeleteRoomMessage {
    #[param(ignore)]
    pub room_id: String,
    #[param(ignore)]
    pub message_id: String,
    pub as_member: Option<String>,
}

#[command(
    name = "delete_room_message",
    category = "rooms",
    description = "Delete a message. Its author, or a room owner. A marker stays in its place and replies stay.",
    method = "DELETE",
    path = "/v1/rooms/{room_id}/messages/{message_id}",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "delete").with_examples(&[CliExample::new("Remove a message posted by mistake", "everruns rooms messages delete --room-id room_01h9 --message-id rmsg_01h9")]),
    http = plain,
    params(DeleteRoomMessage),
)]
impl Command for DeleteRoomMessage {
    type Output = RoomMessage;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessage, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let row = own_message(ctx, &room, &self.message_id, self.as_member.as_deref()).await?;
        let row = ctx
            .db
            .delete_room_message(room.id, row.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Message"))?;
        let view = RoomView::load(&ctx.db, room).await?;
        view.render_one(&ctx.db, &row).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReactToRoomMessage {
    pub room_id: String,
    pub message_id: String,
    /// An emoji, or a short name such as `+1`.
    pub emoji: String,
    /// `false` removes the reaction.
    #[serde(default = "default_true")]
    pub add: bool,
    pub as_member: Option<String>,
}

#[command(
    name = "react_to_room_message",
    category = "rooms",
    description = "Add a reaction to a message, or remove it with add=false. Reactions never wake agents.",
    method = "POST",
    path = "/v1/rooms/{room_id}/messages/{message_id}/reactions",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "react").with_examples(&[CliExample::new("Show you are looking at a message", "everruns rooms messages react --room-id room_01h9 --message-id rmsg_01h9 --emoji eyes")]),
    http = plain,
)]
impl Command for ReactToRoomMessage {
    type Output = RoomMessage;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessage, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        q::require_open(&room)?;
        let emoji = self.emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 64 {
            return Err(CommandError::bad_request(
                "emoji must be 1 to 64 characters",
            ));
        }
        let row = ctx
            .db
            .get_room_message(room.id, q::parse_message_id(&self.message_id)?)
            .await?
            .filter(|row| row.deleted_at.is_none())
            .ok_or_else(|| CommandError::not_found("Message"))?;
        let actor = q::acting_member(ctx, &room, self.as_member.as_deref(), true).await?;
        if self.add {
            ctx.db
                .add_room_reaction(&room, row.id, actor.id, emoji)
                .await?;
        } else {
            ctx.db
                .remove_room_reaction(&room, row.id, actor.id, emoji)
                .await?;
        }
        let view = RoomView::load(&ctx.db, room).await?;
        view.render_one(&ctx.db, &row).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PinRoomMessage {
    pub room_id: String,
    pub message_id: String,
    /// `false` unpins.
    #[serde(default = "default_true")]
    pub pinned: bool,
    pub as_member: Option<String>,
}

#[command(
    name = "pin_room_message",
    category = "rooms",
    description = "Pin a message to the room, or unpin it with pinned=false. Any member can pin.",
    method = "POST",
    path = "/v1/rooms/{room_id}/messages/{message_id}/pin",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "pin").with_examples(&[CliExample::new("Pin the decision so everyone finds it", "everruns rooms messages pin --room-id room_01h9 --message-id rmsg_01h9")]),
    http = plain,
)]
impl Command for PinRoomMessage {
    type Output = RoomMessage;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessage, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        q::require_open(&room)?;
        let actor = q::acting_member(ctx, &room, self.as_member.as_deref(), true).await?;
        let row = ctx
            .db
            .set_room_message_pinned(
                room.id,
                q::parse_message_id(&self.message_id)?,
                self.pinned,
                Some(actor.id),
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Message"))?;
        let view = RoomView::load(&ctx.db, room).await?;
        view.render_one(&ctx.db, &row).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListRoomPins {
    pub room_id: String,
}

#[command(
    name = "list_room_pins",
    category = "rooms",
    description = "List a room's pinned messages, most recently pinned first.",
    method = "GET",
    path = "/v1/rooms/{room_id}/pins",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "pinned").with_args(&[CliArg::new("room_id").at(1)]).with_examples(&[CliExample::new("List what a room has pinned", "everruns rooms messages pinned room_01h9")]),
    positional = "room_id",
    http = plain,
)]
impl Command for ListRoomPins {
    type Output = RoomMessageListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessageListResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let rows = ctx.db.list_pinned_room_messages(room.id).await?;
        let view = RoomView::load(&ctx.db, room).await?;
        Ok(RoomMessageListResponse {
            data: view.render(&ctx.db, &rows).await?,
            has_more: false,
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchRoomMessages {
    #[param(ignore)]
    pub room_id: String,
    /// Text to find, case-insensitive.
    pub q: String,
    pub limit: Option<i64>,
}

#[command(
    name = "search_room_messages",
    category = "rooms",
    description = "Find messages in a room by text, newest first.",
    method = "GET",
    path = "/v1/rooms/{room_id}/search",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "search").with_examples(&[CliExample::new("Find where a room talked about a deploy", "everruns rooms messages search --room-id room_01h9 --q deploy")]),
    http = plain,
    params(SearchRoomMessages),
)]
impl Command for SearchRoomMessages {
    type Output = RoomMessageListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomMessageListResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let query = self.q.trim();
        if query.is_empty() || query.chars().count() > 200 {
            return Err(CommandError::bad_request("q must be 1 to 200 characters"));
        }
        let rows = ctx
            .db
            .search_room_messages(room.id, query, self.limit.unwrap_or(20).clamp(1, 100))
            .await?;
        let view = RoomView::load(&ctx.db, room).await?;
        Ok(RoomMessageListResponse {
            data: view.render(&ctx.db, &rows).await?,
            has_more: false,
        })
    }
}

// ============================================================================
// Reads
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MarkRoomRead {
    pub room_id: String,
    /// Mark read up to and including this message, in its scope (the top
    /// level, or its thread). Without it, the whole top level up to now.
    pub message_id: Option<String>,
    pub as_member: Option<String>,
}

#[command(
    name = "mark_room_read",
    category = "rooms",
    description = "Mark a room's top level, or a thread, read up to a message. Marks only move forward.",
    method = "POST",
    path = "/v1/rooms/{room_id}/read",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "mark-read").with_examples(&[CliExample::new("Mark everything in a room as read", "everruns rooms mark-read --room-id room_01h9")]),
    http = plain,
)]
impl Command for MarkRoomRead {
    type Output = RoomReadMark;

    async fn execute(self, ctx: &Ctx) -> Result<RoomReadMark, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let reader = q::acting_member(ctx, &room, self.as_member.as_deref(), false).await?;
        let (scope_id, through) = match self.message_id.as_deref() {
            Some(raw) => {
                let row = ctx
                    .db
                    .get_room_message(room.id, q::parse_message_id(raw)?)
                    .await?
                    .ok_or_else(|| CommandError::not_found("Message"))?;
                (
                    row.thread_id.map_or(room.id.uuid(), |id| id.uuid()),
                    row.created_at,
                )
            }
            None => (room.id.uuid(), Utc::now()),
        };
        let mark = ctx
            .db
            .mark_room_read(room.id, reader.id, scope_id, through)
            .await?;
        Ok(RoomReadMark {
            scope_id: scope_label(&room, mark.scope_id),
            read_through: mark.read_through,
        })
    }
}

fn scope_label(room: &RoomRow, scope_id: Uuid) -> String {
    if scope_id == room.id.uuid() {
        room.id.to_string()
    } else {
        crate::kernel_imports::contracts::typed_id::RoomMessageId::from_uuid(scope_id).to_string()
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReadRoomUnread {
    pub room_id: String,
    pub limit: Option<i64>,
    /// Look without marking read.
    #[serde(default)]
    pub peek: bool,
    pub as_member: Option<String>,
}

#[command(
    name = "read_room_unread",
    category = "rooms",
    description = "Read what the caller has not read yet, and mark it read. Returns new top-level messages and new replies in threads they are in, with each thread's root for context.",
    method = "POST",
    path = "/v1/rooms/{room_id}/unread",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "unread").with_examples(&[CliExample::new("Catch up on what you have not read in a room", "everruns rooms unread --room-id room_01h9 --limit 50")]),
    http = plain,
)]
impl Command for ReadRoomUnread {
    type Output = RoomUnreadResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomUnreadResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let reader = q::acting_member(ctx, &room, self.as_member.as_deref(), false).await?;
        let limit = self.limit.unwrap_or(50).clamp(1, 200);
        let mut rows = ctx.db.list_room_unread(room.id, &reader, limit + 1).await?;
        let has_more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        let thread_ids: Vec<Uuid> = {
            let mut ids: Vec<Uuid> = rows
                .iter()
                .filter_map(|row| row.thread_id.map(|id| id.uuid()))
                .collect();
            ids.sort();
            ids.dedup();
            ids
        };
        let threads = ctx
            .db
            .list_room_messages_by_ids(room.id, &thread_ids)
            .await?;
        if !self.peek {
            // Each scope is read up to the newest message returned in it.
            let mut newest: std::collections::HashMap<Uuid, DateTime<Utc>> =
                std::collections::HashMap::new();
            for row in &rows {
                let scope = row.thread_id.map_or(room.id.uuid(), |id| id.uuid());
                let at = newest.entry(scope).or_insert(row.created_at);
                if row.created_at > *at {
                    *at = row.created_at;
                }
            }
            for (scope, at) in newest {
                ctx.db.mark_room_read(room.id, reader.id, scope, at).await?;
            }
        }
        let view = RoomView::load(&ctx.db, room).await?;
        Ok(RoomUnreadResponse {
            data: view.render(&ctx.db, &rows).await?,
            threads: view.render(&ctx.db, &threads).await?,
            has_more,
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListRoomMessageReaders {
    pub room_id: String,
    pub message_id: String,
}

#[command(
    name = "list_room_message_readers",
    category = "rooms",
    description = "Who has read a message, and who has not.",
    method = "GET",
    path = "/v1/rooms/{room_id}/messages/{message_id}/readers",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "messages"], "readers").with_examples(&[CliExample::new("See who has read a message and who has not", "everruns rooms messages readers --room-id room_01h9 --message-id rmsg_01h9")]),
    http = plain,
)]
impl Command for ListRoomMessageReaders {
    type Output = RoomReadersResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomReadersResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let row = ctx
            .db
            .get_room_message(room.id, q::parse_message_id(&self.message_id)?)
            .await?
            .ok_or_else(|| CommandError::not_found("Message"))?;
        let scope = row.thread_id.map_or(room.id.uuid(), |id| id.uuid());
        let marks = ctx.db.list_room_readers(scope, row.created_at).await?;
        let view = RoomView::load(&ctx.db, room).await?;
        let read: std::collections::HashSet<Uuid> =
            marks.iter().map(|mark| mark.member_id.uuid()).collect();
        let read_by = marks
            .iter()
            .filter_map(|mark| {
                let member = view.members.get(&mark.member_id.uuid())?;
                // The author is neither a reader nor someone who has yet to read.
                (member.left_at.is_none() && Some(member.id) != row.author_member_id).then(|| {
                    RoomReader {
                        member_id: member.id.to_string(),
                        handle: member.handle.clone(),
                        read_through: mark.read_through,
                    }
                })
            })
            .collect();
        let mut not_read_by: Vec<&RoomMemberRow> = view
            .members
            .values()
            .filter(|member| {
                member.left_at.is_none()
                    && Some(member.id) != row.author_member_id
                    && !read.contains(&member.id.uuid())
            })
            .collect();
        not_read_by.sort_by_key(|member| member.created_at);
        Ok(RoomReadersResponse {
            read_by,
            not_read_by: not_read_by.into_iter().map(q::author).collect(),
        })
    }
}

// ============================================================================
// Events
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRoomEvents {
    #[param(ignore)]
    pub room_id: String,
    /// Read events after this position (`next_after` of the last page).
    pub after: Option<i64>,
    pub limit: Option<i64>,
}

#[command(
    name = "list_room_events",
    category = "rooms",
    description = "Read a room's change feed in order. It covers posts, edits, reactions, members and agent wakes; poll with after=next_after.",
    method = "GET",
    path = "/v1/rooms/{room_id}/events",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "events").with_examples(&[CliExample::new("Follow one room's changes from where you stopped", "everruns rooms events --room-id room_01h9 --after 120")]),
    http = plain,
    params(ListRoomEvents),
)]
impl Command for ListRoomEvents {
    type Output = RoomEventListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomEventListResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        events(ctx, Some(room.id), self.after, self.limit).await
    }
}

#[derive(Debug, Default, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListOrgRoomEvents {
    /// Read events after this position (`next_after` of the last page).
    pub after: Option<i64>,
    pub limit: Option<i64>,
}

#[command(
    name = "list_org_room_events",
    category = "rooms",
    description = "Read the change feed of every room in the org, in order. Poll with after=next_after.",
    method = "GET",
    path = "/v1/rooms/events",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms"], "feed").with_examples(&[CliExample::new("Follow every room in the organization from where you stopped", "everruns rooms feed --after 120")]),
    http = plain,
    params(ListOrgRoomEvents),
)]
impl Command for ListOrgRoomEvents {
    type Output = RoomEventListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomEventListResponse, CommandError> {
        events(ctx, None, self.after, self.limit).await
    }
}

async fn events(
    ctx: &Ctx,
    room_id: Option<crate::kernel_imports::contracts::typed_id::RoomId>,
    after: Option<i64>,
    limit: Option<i64>,
) -> Result<RoomEventListResponse, CommandError> {
    let after = after.unwrap_or(0).max(0);
    let rows = ctx
        .db
        .list_room_events(
            ctx.org_id(),
            room_id,
            after,
            limit.unwrap_or(100).clamp(1, 500),
        )
        .await?;
    Ok(RoomEventListResponse {
        next_after: rows.last().map_or(after, |row| row.seq),
        data: rows.iter().map(q::row_to_event).collect(),
    })
}
