//! Row to API shape, and the lookups every command shares.

use super::rules::{self, LoopLimits, SessionScope, WakeMode};
use super::types::*;
use crate::domains::common::{CommandError, Ctx};
use crate::kernel_imports::OrgRole;
use crate::kernel_imports::contracts::typed_id::{AgentId, RoomId, RoomMemberId, RoomMessageId};
use crate::storage::{
    CreateRoomMemberRow, RoomEventRow, RoomMemberRow, RoomMessageRow, RoomReactionRow, RoomRow,
    StorageBackend,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

pub fn parse_room_id(raw: &str) -> Result<RoomId, CommandError> {
    raw.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid room ID: {e}")))
}

pub fn parse_member_id(raw: &str) -> Result<RoomMemberId, CommandError> {
    raw.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid room member ID: {e}")))
}

pub fn parse_message_id(raw: &str) -> Result<RoomMessageId, CommandError> {
    raw.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid room message ID: {e}")))
}

pub fn row_to_room(row: &RoomRow) -> Room {
    Room {
        id: row.id.to_string(),
        name: row.name.clone(),
        topic: row.topic.clone(),
        external_ref: row.external_ref.clone(),
        metadata: row.metadata.clone(),
        link_template: row.link_template.clone(),
        loop_limits: LoopLimits::from_settings(&row.settings),
        archived: row.archived_at.is_some(),
        last_message_at: row.last_message_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub fn row_to_member(row: &RoomMemberRow, agent_public_ids: &HashMap<Uuid, String>) -> RoomMember {
    RoomMember {
        id: row.id.to_string(),
        room_id: row.room_id.to_string(),
        kind: RoomMemberKind::parse(&row.kind),
        handle: row.handle.clone(),
        display_name: row.display_name.clone(),
        role: RoomMemberRole::parse(&row.role),
        user_id: row.user_id.map(|id| id.to_string()),
        agent_id: row
            .agent_id
            .and_then(|id| agent_public_ids.get(&id.uuid()).cloned()),
        wake: WakeMode::parse(&row.wake),
        session_scope: SessionScope::parse(&row.session_scope),
        instructions: row.instructions.clone(),
        left: row.left_at.is_some(),
        joined_at: row.created_at,
    }
}

pub fn row_to_event(row: &RoomEventRow) -> RoomEvent {
    RoomEvent {
        seq: row.seq,
        room_id: row.room_id.to_string(),
        event_type: row.event_type.clone(),
        message_id: row.message_id.map(|id| id.to_string()),
        member_id: row.member_id.map(|id| id.to_string()),
        data: row.data.clone(),
        created_at: row.created_at,
    }
}

pub fn author(row: &RoomMemberRow) -> RoomAuthor {
    RoomAuthor {
        id: row.id.to_string(),
        handle: row.handle.clone(),
        display_name: row.display_name.clone(),
        kind: RoomMemberKind::parse(&row.kind),
    }
}

/// Everything needed to render a room's messages: its members, past and
/// present, by id.
pub struct RoomView {
    pub room: RoomRow,
    pub members: HashMap<Uuid, RoomMemberRow>,
}

impl RoomView {
    pub async fn load(db: &StorageBackend, room: RoomRow) -> Result<Self, CommandError> {
        let members = db
            .list_room_members(room.id, true)
            .await?
            .into_iter()
            .map(|member| (member.id.uuid(), member))
            .collect();
        Ok(Self { room, members })
    }

    /// Active members by handle.
    pub fn active_by_handle(&self) -> HashMap<&str, &RoomMemberRow> {
        self.members
            .values()
            .filter(|member| member.left_at.is_none())
            .map(|member| (member.handle.as_str(), member))
            .collect()
    }

    /// The active members `text` mentions.
    pub fn mentions_in(&self, text: &str) -> Vec<Uuid> {
        let by_handle = self.active_by_handle();
        rules::mentioned_handles(text)
            .iter()
            .filter_map(|handle| by_handle.get(handle.as_str()))
            .map(|member| member.id.uuid())
            .collect()
    }

    pub async fn render(
        &self,
        db: &StorageBackend,
        rows: &[RoomMessageRow],
    ) -> Result<Vec<RoomMessage>, CommandError> {
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id.uuid()).collect();
        let reactions = db.list_room_reactions(&ids).await?;
        Ok(rows
            .iter()
            .map(|row| self.message(row, &reactions))
            .collect())
    }

    pub async fn render_one(
        &self,
        db: &StorageBackend,
        row: &RoomMessageRow,
    ) -> Result<RoomMessage, CommandError> {
        Ok(self
            .render(db, std::slice::from_ref(row))
            .await?
            .pop()
            .expect("one row renders one message"))
    }

    fn message(&self, row: &RoomMessageRow, reactions: &[RoomReactionRow]) -> RoomMessage {
        let deleted = row.deleted_at.is_some();
        let mut grouped: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        if !deleted {
            for reaction in reactions.iter().filter(|r| r.message_id == row.id) {
                grouped
                    .entry(reaction.emoji.as_str())
                    .or_default()
                    .push(reaction.member_id.to_string());
            }
        }
        let room_id = row.room_id.to_string();
        let message_id = row.id.to_string();
        let thread_id = row.thread_id.map(|id| id.to_string());
        RoomMessage {
            url: rules::render_link(
                self.room.link_template.as_deref(),
                &room_id,
                &message_id,
                thread_id.as_deref(),
            ),
            id: message_id,
            room_id,
            thread_id,
            author: row
                .author_member_id
                .and_then(|id| self.members.get(&id.uuid()))
                .map(author),
            text: if deleted {
                String::new()
            } else {
                row.text.clone()
            },
            mentions: if deleted {
                Vec::new()
            } else {
                row.mentions
                    .iter()
                    .filter_map(|id| self.members.get(id))
                    .map(|member| RoomMention {
                        id: member.id.to_string(),
                        handle: member.handle.clone(),
                    })
                    .collect()
            },
            reactions: grouped
                .into_iter()
                .map(|(emoji, member_ids)| RoomReaction {
                    emoji: emoji.to_string(),
                    count: member_ids.len(),
                    member_ids,
                })
                .collect(),
            reply_count: row.reply_count,
            last_reply_at: row.last_reply_at,
            pinned: row.pinned_at.is_some(),
            deleted,
            edited_at: row.edited_at,
            hops: row.hops,
            session_id: row.session_id.map(|id| id.to_string()),
            created_at: row.created_at,
        }
    }
}

/// Public ids of the agents among `members`.
pub async fn agent_public_ids(
    db: &StorageBackend,
    org_id: i64,
    members: &[RoomMemberRow],
) -> Result<HashMap<Uuid, String>, CommandError> {
    let mut ids = HashMap::new();
    for agent_id in members.iter().filter_map(|member| member.agent_id) {
        if ids.contains_key(&agent_id.uuid()) {
            continue;
        }
        if let Some(agent) = db.get_agent(org_id, agent_id).await? {
            ids.insert(agent_id.uuid(), agent.public_id);
        }
    }
    Ok(ids)
}

/// The room, or not found. Rooms are visible to every member of their org.
pub async fn load_room(ctx: &Ctx, room_id: &str) -> Result<RoomRow, CommandError> {
    let room_id = parse_room_id(room_id)?;
    ctx.db
        .get_room(ctx.org_id(), room_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Room"))
}

pub fn require_open(room: &RoomRow) -> Result<(), CommandError> {
    if room.archived_at.is_some() {
        return Err(CommandError::conflict(
            "Room is archived; unarchive it to change it".to_string(),
        ));
    }
    Ok(())
}

/// Whether the caller may act for other members (`as_member`): org admins,
/// owners and internal callers. A builder's product posts for its guests and
/// outside agents this way.
pub fn can_act_for_others(ctx: &Ctx) -> bool {
    ctx.caller.is_internal || ctx.caller.role.has_permission(OrgRole::Admin)
}

/// Whether the caller manages the room: an org admin, or one of its owners.
pub async fn can_manage(ctx: &Ctx, room: &RoomRow) -> Result<bool, CommandError> {
    if can_act_for_others(ctx) {
        return Ok(true);
    }
    let Some(user_id) = ctx.caller.user_id else {
        return Ok(false);
    };
    Ok(ctx
        .db
        .find_room_member_by_user(room.id, user_id)
        .await?
        .is_some_and(|member| member.role == "owner"))
}

pub async fn require_manage(ctx: &Ctx, room: &RoomRow) -> Result<(), CommandError> {
    if can_manage(ctx, room).await? {
        Ok(())
    } else {
        Err(CommandError::forbidden(
            "Only the room's owners and org admins can change it".to_string(),
        ))
    }
}

/// The member the caller acts as: `as_member` when given (admins only),
/// otherwise the caller's own person member. With `join`, a person who is
/// not a member yet joins first, as people do when they first post in an
/// open room.
pub async fn acting_member(
    ctx: &Ctx,
    room: &RoomRow,
    as_member: Option<&str>,
    join: bool,
) -> Result<RoomMemberRow, CommandError> {
    if let Some(raw) = as_member {
        let member_id = parse_member_id(raw)?;
        let member = ctx
            .db
            .get_room_member(room.id, member_id)
            .await?
            .filter(|member| member.left_at.is_none())
            .ok_or_else(|| CommandError::not_found("Room member"))?;
        let own = ctx.caller.user_id.is_some() && member.user_id == ctx.caller.user_id;
        if !own && !can_act_for_others(ctx) {
            return Err(CommandError::forbidden(
                "Acting as another member needs an org admin".to_string(),
            ));
        }
        return Ok(member);
    }
    let Some(user_id) = ctx.caller.user_id else {
        return Err(CommandError::bad_request(
            "This credential has no user; pass as_member to say which member acts",
        ));
    };
    if let Some(member) = ctx.db.find_room_member_by_user(room.id, user_id).await? {
        return Ok(member);
    }
    if !join {
        return Err(CommandError::forbidden(
            "You are not a member of this room".to_string(),
        ));
    }
    require_open(room)?;
    add_person(ctx, room, user_id, "member").await
}

/// Add an org user to the room under a free handle derived from their name.
pub async fn add_person(
    ctx: &Ctx,
    room: &RoomRow,
    user_id: Uuid,
    role: &str,
) -> Result<RoomMemberRow, CommandError> {
    if !ctx.db.is_organization_member(ctx.org_id(), user_id).await? {
        return Err(CommandError::not_found("User"));
    }
    let user = ctx
        .db
        .get_user(user_id)
        .await?
        .ok_or_else(|| CommandError::not_found("User"))?;
    let display_name = if user.name.trim().is_empty() {
        user.email.split('@').next().unwrap_or("member").to_string()
    } else {
        user.name.clone()
    };
    let base = rules::handle_from_name(&display_name)
        .or_else(|| rules::handle_from_name(user.email.split('@').next().unwrap_or("")))
        .unwrap_or_else(|| "member".to_string());
    insert_member(
        ctx,
        room,
        CreateRoomMemberRow {
            room_id: room.id,
            org_id: room.org_id,
            kind: RoomMemberKind::Person.as_str().into(),
            user_id: Some(user_id),
            agent_id: None,
            handle: base,
            display_name,
            role: role.into(),
            wake: WakeMode::Never.as_str().into(),
            session_scope: SessionScope::Thread.as_str().into(),
            instructions: None,
        },
        true,
    )
    .await
}

/// Insert a member. With `pick_free_handle`, a taken handle gets a number;
/// otherwise a taken handle is a conflict.
pub async fn insert_member(
    ctx: &Ctx,
    room: &RoomRow,
    mut row: CreateRoomMemberRow,
    pick_free_handle: bool,
) -> Result<RoomMemberRow, CommandError> {
    let taken: HashSet<String> = ctx
        .db
        .list_room_members(room.id, false)
        .await?
        .into_iter()
        .map(|member| member.handle)
        .collect();
    if taken.contains(&row.handle) {
        if !pick_free_handle {
            return Err(CommandError::conflict(format!(
                "@{} is already taken in this room",
                row.handle
            )));
        }
        row.handle = rules::unique_handle(&row.handle, &taken);
    }
    ctx.db.add_room_member(row).await.map_err(|error| {
        if crate::errors::is_database_unique_violation(&error) {
            CommandError::conflict("That member is already in the room".to_string())
        } else {
            error.into()
        }
    })
}

/// The agent behind a public agent id, in the caller's org.
pub async fn load_agent(
    ctx: &Ctx,
    public_id: &str,
) -> Result<crate::storage::AgentRow, CommandError> {
    let _: AgentId = public_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))?;
    ctx.db
        .get_agent_by_public_id(ctx.org_id(), public_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))
}
