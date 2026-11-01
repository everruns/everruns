// Message reactions: emoji on user and agent messages, from people and from
// the agent, and how people's reactions reach the agent.
//
// Spec: knowledge/execution/message-reactions.md.
//
// Decisions:
// - The session event log is the record (`message.reaction.added` /
//   `.removed`), not a table. The agent's own reactions are emitted by the
//   worker from inside a turn, and a log entry is what both writers share
//   without a new host port. Removing appends; nothing is deleted.
// - One reaction per (message, actor, emoji): adding twice or removing what is
//   not there succeeds without writing anything.
// - Only reactions on the agent's own replies are told to the agent, and never
//   the agent's own. They are told once: a reaction note records the last
//   reaction event it covers (`through_sequence`), and the next note starts
//   after it. An add and a remove between two notes cancel out.
// - When to tell is the agent's Reactions capability setting (default: with
//   the next turn). A reaction made while a turn runs is told right away, as
//   a reaction message that joins the running turn.
// - Rights: reacting takes SESSION_MANAGE, like sending a message; reading
//   takes SESSION_VIEW. Execution API callers react in their own sessions.

use crate::domains::common::*;
use crate::domains::messages::service::{CreateMessageContext, MessageService};
use crate::storage::{Database, ReactedMessageRow, ReactionEventRow, SessionRow, StorageBackend};
use chrono::{DateTime, Utc};
use everruns_capabilities::capabilities::reactions::{
    AGENT_REACTION_ACTOR_ID, REACTIONS_CAPABILITY_ID, ReactionDelivery, ReactionsConfig,
};
use everruns_contracts::message_reactions::{ReactionNote, ReactionNoteItem, reaction_excerpt};
use everruns_core::{
    EventContext, EventData, EventRequest, MESSAGE_REACTION_ADDED, MessageReactionData,
    ReactionActor, ReactionActorKind, normalize_reaction_emoji,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

/// Most distinct emoji on one message.
const MAX_EMOJI_PER_MESSAGE: usize = 20;
/// Name used when a reaction carries none.
const UNNAMED_ACTOR: &str = "Someone";

// ============================================================================
// Folding the log
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReactionKey {
    message_id: String,
    actor_kind: ReactionActorKind,
    actor_id: String,
    emoji: String,
}

#[derive(Debug, Clone)]
struct ReactionState {
    active: bool,
    actor: ReactionActor,
    /// When the reaction was last added.
    reacted_at: DateTime<Utc>,
    /// Sequence of the event that last changed it.
    sequence: i64,
    /// Sequence of the event that first added it, for display order.
    first_sequence: i64,
}

fn parse(row: &ReactionEventRow) -> Option<(bool, MessageReactionData)> {
    let data = serde_json::from_value::<MessageReactionData>(row.data.clone()).ok()?;
    Some((row.event_type == MESSAGE_REACTION_ADDED, data))
}

/// Reaction state after every event up to `through` (all events when `None`).
fn fold(events: &[ReactionEventRow], through: Option<i64>) -> HashMap<ReactionKey, ReactionState> {
    let mut state: HashMap<ReactionKey, ReactionState> = HashMap::new();
    for row in events {
        if through.is_some_and(|through| row.sequence > through) {
            break;
        }
        let Some((added, data)) = parse(row) else {
            continue;
        };
        let key = ReactionKey {
            message_id: data.message_id,
            actor_kind: data.actor.kind,
            actor_id: data.actor.id.clone(),
            emoji: data.emoji,
        };
        let entry = state.entry(key).or_insert_with(|| ReactionState {
            active: false,
            actor: data.actor.clone(),
            reacted_at: row.created_at,
            sequence: row.sequence,
            first_sequence: row.sequence,
        });
        if added && !entry.active {
            entry.reacted_at = row.created_at;
        }
        entry.active = added;
        entry.sequence = row.sequence;
        if data.actor.name.is_some() {
            entry.actor = data.actor;
        }
    }
    state
}

/// The changes since `told_through` that the agent has not been told, as
/// note items, in the order they happened. `agent_messages` is the set of
/// message ids that are the agent's replies, with their text.
fn untold_items(
    events: &[ReactionEventRow],
    told_through: Option<i64>,
    agent_messages: &HashMap<String, String>,
) -> Vec<ReactionNoteItem> {
    let people: Vec<ReactionEventRow> = events
        .iter()
        .filter(|row| {
            parse(row).is_some_and(|(_, data)| data.actor.kind != ReactionActorKind::Agent)
        })
        .cloned()
        .collect();
    let before = match told_through {
        Some(through) => fold(&people, Some(through)),
        None => HashMap::new(),
    };
    let after = fold(&people, None);
    let mut changed: Vec<(&ReactionKey, &ReactionState)> = after
        .iter()
        .filter(|(key, state)| {
            let was = before.get(*key).is_some_and(|before| before.active);
            was != state.active
                && told_through.is_none_or(|through| state.sequence > through)
                && agent_messages.contains_key(&key.message_id)
        })
        .collect();
    changed.sort_by_key(|(_, state)| state.sequence);
    changed
        .into_iter()
        .map(|(key, state)| ReactionNoteItem {
            message_id: key.message_id.clone(),
            emoji: key.emoji.clone(),
            actor_name: state
                .actor
                .name
                .clone()
                .unwrap_or_else(|| UNNAMED_ACTOR.to_string()),
            removed: !state.active,
            on_agent_message: true,
            excerpt: agent_messages
                .get(&key.message_id)
                .and_then(|text| reaction_excerpt(text)),
        })
        .collect()
}

fn message_text(message: &serde_json::Value) -> String {
    message
        .get("content")
        .and_then(|content| content.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn is_agent_reply(row: &ReactedMessageRow) -> bool {
    row.event_type == everruns_core::OUTPUT_MESSAGE_COMPLETED
}

// ============================================================================
// Telling the agent
// ============================================================================

/// The agent's "when someone reacts" setting.
async fn delivery_setting(
    db: &Database,
    agent_id: Option<Uuid>,
) -> anyhow::Result<ReactionDelivery> {
    let Some(agent_id) = agent_id else {
        return Ok(ReactionDelivery::default());
    };
    Ok(db
        .get_agent_capabilities(agent_id)
        .await?
        .into_iter()
        .find(|row| row.capability_id == REACTIONS_CAPABILITY_ID)
        .map(|row| ReactionsConfig::from_value(&row.config).when_someone_reacts)
        .unwrap_or_default())
}

/// The reaction note the next message to the agent should carry, if any
/// reactions are waiting. One index probe when the session has no reactions.
pub(crate) async fn pending_reaction_note(
    db: &Database,
    session_id: Uuid,
    agent_id: Option<Uuid>,
) -> anyhow::Result<Option<ReactionNote>> {
    let events = db.list_reaction_events(session_id).await?;
    let Some(latest) = events
        .iter()
        .filter(|row| {
            parse(row).is_some_and(|(_, data)| data.actor.kind != ReactionActorKind::Agent)
        })
        .map(|row| row.sequence)
        .max()
    else {
        return Ok(None);
    };
    let told_through = db.reactions_told_through(session_id).await?;
    if told_through.is_some_and(|through| through >= latest) {
        return Ok(None);
    }
    if delivery_setting(db, agent_id).await? == ReactionDelivery::Off {
        return Ok(None);
    }
    let mut ids: Vec<String> = events
        .iter()
        .filter(|row| told_through.is_none_or(|through| row.sequence > through))
        .filter_map(|row| parse(row).map(|(_, data)| data.message_id))
        .collect();
    ids.sort();
    ids.dedup();
    let agent_messages: HashMap<String, String> = db
        .reacted_messages(session_id, &ids)
        .await?
        .into_iter()
        .filter(is_agent_reply)
        .map(|row| (row.message_id.clone(), message_text(&row.message)))
        .collect();
    let items = untold_items(&events, told_through, &agent_messages);
    if items.is_empty() {
        return Ok(None);
    }
    Ok(Some(ReactionNote {
        through_sequence: latest,
        items,
    }))
}

/// Who reacted, and the authority a reaction message they cause runs under.
#[derive(Debug, Clone)]
pub struct Reactor {
    pub actor: ReactionActor,
    /// Signed-in person, for reactions made through the console or `/v1`.
    pub user_id: Option<Uuid>,
    /// Channel consumer, for Execution API and Work Chat reactions.
    pub principal_id: Option<everruns_contracts::typed_id::PrincipalId>,
    /// Where the reaction was made (`console`, `api`, `work_chat`, ...).
    pub source: &'static str,
}

/// Tell the agent now: write a reaction message, which starts a turn or joins
/// the running one. Best-effort; a failure leaves the reactions for the next
/// turn.
async fn tell_now(
    messages: &MessageService,
    session: &SessionRow,
    reactor: &Reactor,
) -> anyhow::Result<()> {
    let session_uuid = session.id.uuid();
    let agent_uuid = session.agent_id.map(|id| id.uuid());
    let Some(note) = pending_reaction_note(messages.db(), session_uuid, agent_uuid).await? else {
        return Ok(());
    };
    let Some(harness_id) = session.harness_id else {
        return Ok(());
    };
    let ctx = CreateMessageContext {
        runtime_subject_principal_id: reactor.principal_id,
        org_id: session.org_id,
        user_id: reactor.user_id,
        harness_id: harness_id.uuid(),
        agent_id: agent_uuid,
        session_id: session_uuid,
        event_metadata: None,
        request_id: None,
    };
    messages.create_reaction_message(ctx, note).await?;
    Ok(())
}

// ============================================================================
// Adding, removing, listing
// ============================================================================

/// Result of adding or removing a reaction.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReactionChange {
    /// Message the reaction is on (`message_...`).
    #[schema(example = "message_01933b5a00007000800000000000001")]
    pub message_id: String,
    #[schema(example = "👍")]
    pub emoji: String,
    /// Whether the reaction is on the message now.
    pub reacted: bool,
    /// False when the message already was in that state.
    pub changed: bool,
}

/// Add (`add = true`) or remove a reaction, and tell the agent when its
/// setting says so.
pub async fn set_reaction(
    messages: &MessageService,
    session: &SessionRow,
    message_id: &str,
    emoji: &str,
    reactor: Reactor,
    add: bool,
) -> Result<ReactionChange, CommandError> {
    let db = messages.db();
    let emoji = normalize_reaction_emoji(emoji).map_err(CommandError::bad_request)?;
    let session_uuid = session.id.uuid();
    let message = db
        .reacted_messages(session_uuid, &[message_id.to_string()])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| CommandError::not_found("Message"))?;
    let events = db.list_reaction_events(session_uuid).await?;
    let state = fold(&events, None);
    let key = ReactionKey {
        message_id: message_id.to_string(),
        actor_kind: reactor.actor.kind,
        actor_id: reactor.actor.id.clone(),
        emoji: emoji.clone(),
    };
    let active = state.get(&key).is_some_and(|state| state.active);
    if active == add {
        return Ok(ReactionChange {
            message_id: message_id.to_string(),
            emoji,
            reacted: add,
            changed: false,
        });
    }
    if add {
        let mut on_message: Vec<&str> = state
            .iter()
            .filter(|(key, state)| key.message_id == message_id && state.active)
            .map(|(key, _)| key.emoji.as_str())
            .collect();
        on_message.sort();
        on_message.dedup();
        if !on_message.contains(&emoji.as_str()) && on_message.len() >= MAX_EMOJI_PER_MESSAGE {
            return Err(CommandError::bad_request(format!(
                "A message can carry at most {MAX_EMOJI_PER_MESSAGE} different emoji"
            )));
        }
    }
    let data = MessageReactionData {
        message_id: message_id.to_string(),
        emoji: emoji.clone(),
        actor: reactor.actor.clone(),
        source: Some(reactor.source.to_string()),
    };
    messages
        .event_service()
        .emit(EventRequest::new(
            session.id,
            EventContext::empty(),
            EventData::message_reaction(data, add),
        ))
        .await?;

    // Tell the agent now when it is mid-turn or asked to be woken. Everything
    // else waits for the next message (`pending_reaction_note`).
    if reactor.actor.kind != ReactionActorKind::Agent && is_agent_reply(&message) {
        let agent_uuid = session.agent_id.map(|id| id.uuid());
        let setting = delivery_setting(db, agent_uuid).await?;
        let now = match setting {
            ReactionDelivery::Off => false,
            ReactionDelivery::StartTurn => true,
            ReactionDelivery::NextTurn => session.status == "active",
        };
        if now && let Err(error) = tell_now(messages, session, &reactor).await {
            tracing::warn!(
                session_id = %session.id,
                error = %error,
                "Could not tell the agent about a reaction now; it waits for the next turn"
            );
        }
    }
    Ok(ReactionChange {
        message_id: message_id.to_string(),
        emoji,
        reacted: add,
        changed: true,
    })
}

/// One person or agent who reacted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReactionActorView {
    pub kind: ReactionActorKind,
    /// User, principal, agent or platform user id.
    pub id: String,
    pub name: Option<String>,
    pub reacted_at: DateTime<Utc>,
}

/// One emoji on a message.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReactionSummary {
    #[schema(example = "👍")]
    pub emoji: String,
    pub count: usize,
    /// The caller is among those who reacted.
    pub me: bool,
    /// The agent is among those who reacted.
    pub agent: bool,
    /// Who reacted, earliest first.
    pub actors: Vec<ReactionActorView>,
}

/// The reactions on one message.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageReactions {
    #[schema(example = "message_01933b5a00007000800000000000001")]
    pub message_id: String,
    pub reactions: Vec<ReactionSummary>,
}

/// Current reactions in a session, by message, emoji in the order first used.
/// `viewer` marks the caller's own reactions; agent reactions are named after
/// the session's agent.
pub async fn list_reactions(
    db: &StorageBackend,
    session: &SessionRow,
    message_id: Option<&str>,
    viewer: Option<(ReactionActorKind, &str)>,
) -> anyhow::Result<Vec<MessageReactions>> {
    let events = db.list_reaction_events(session.id.uuid()).await?;
    let state = fold(&events, None);
    let has_agent = state
        .iter()
        .any(|(key, s)| s.active && key.actor_kind == ReactionActorKind::Agent);
    let agent = match (has_agent, session.agent_id) {
        (true, Some(id)) => db.get_agent_config(session.org_id, id).await?,
        _ => None,
    };
    let mut active: Vec<(&ReactionKey, &ReactionState)> = state
        .iter()
        .filter(|(key, s)| s.active && message_id.is_none_or(|id| key.message_id == id))
        .collect();
    active.sort_by_key(|(_, s)| (s.first_sequence, s.reacted_at));
    let mut out: Vec<MessageReactions> = Vec::new();
    for (key, s) in active {
        let mut actor = ReactionActorView {
            kind: key.actor_kind,
            id: key.actor_id.clone(),
            name: s.actor.name.clone(),
            reacted_at: s.reacted_at,
        };
        if key.actor_kind == ReactionActorKind::Agent
            && key.actor_id == AGENT_REACTION_ACTOR_ID
            && let Some(agent) = &agent
        {
            actor.id = agent.public_id.clone();
            actor.name = Some(
                agent
                    .display_name
                    .clone()
                    .unwrap_or_else(|| agent.name.clone()),
            );
        }
        let me = viewer.is_some_and(|(kind, id)| kind == key.actor_kind && id == key.actor_id);
        let message = match out.iter_mut().find(|m| m.message_id == key.message_id) {
            Some(message) => message,
            None => {
                out.push(MessageReactions {
                    message_id: key.message_id.clone(),
                    reactions: Vec::new(),
                });
                out.last_mut().expect("just pushed")
            }
        };
        match message.reactions.iter_mut().find(|r| r.emoji == key.emoji) {
            Some(summary) => {
                summary.count += 1;
                summary.me |= me;
                summary.agent |= key.actor_kind == ReactionActorKind::Agent;
                summary.actors.push(actor);
            }
            None => message.reactions.push(ReactionSummary {
                emoji: key.emoji.clone(),
                count: 1,
                me,
                agent: key.actor_kind == ReactionActorKind::Agent,
                actors: vec![actor],
            }),
        }
    }
    Ok(out)
}

// ============================================================================
// Commands
// ============================================================================

/// The session, when the caller may see it.
async fn visible_session(ctx: &Ctx, session_id: &str) -> Result<SessionRow, CommandError> {
    let session_id = super::queries::parse_session_id(session_id)?;
    let session = super::queries::session_service(ctx)?
        .get(&ctx.caller, session_id.uuid(), None)
        .await?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    if !crate::domains::sessions::platform_chat_owner_matches_session(
        &ctx.db,
        &ctx.caller,
        &session,
    )
    .await?
    {
        return Err(CommandError::not_found("Session"));
    }
    ctx.db
        .get_session(ctx.org_id(), session_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Session"))
}

/// The signed-in person behind a console or `/v1` call.
async fn console_reactor(ctx: &Ctx) -> Result<Reactor, CommandError> {
    let user_id = ctx
        .caller
        .user_id
        .ok_or_else(|| CommandError::forbidden("Reactions require a signed-in user"))?;
    let name = ctx.db.get_user(user_id).await?.map(|user| user.name);
    Ok(Reactor {
        actor: ReactionActor {
            kind: ReactionActorKind::User,
            id: user_id.to_string(),
            name,
        },
        user_id: Some(user_id),
        principal_id: None,
        source: "console",
    })
}

/// Put an emoji reaction on a message.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct AddMessageReaction {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Message to react to (`message_...`), a user or agent message of the session.
    pub message_id: String,
    /// One emoji, or a custom emoji as `:name:`.
    pub emoji: String,
}

#[command(
    name = "add_message_reaction",
    category = "messages",
    description = "Put an emoji reaction on a user or agent message. Adding one you already put there changes nothing. Reactions on the agent's replies reach the agent as a short note, by default with its next turn.",
    summary = "React to a message",
    method = "PUT",
    path = "/v1/sessions/{session_id}/messages/{message_id}/reactions/{emoji}",
    policy = crate::domains::sessions::SESSION_MANAGE,
    positional = "session_id",
    http = plain,
    responses((status = 404, description = "Session or message not found")),
    cli = CliRoute::new(&["sessions", "messages"], "react").with_args(&[CliArg::new("session_id").at(1).long("session"), CliArg::new("message_id").at(2), CliArg::new("emoji").at(3)]).with_examples(&[CliExample::new("Give an agent reply a thumbs up", "everruns sessions messages react session_01h9 message_01h9 👍",)]),
)]
impl Command for AddMessageReaction {
    type Output = ReactionChange;

    async fn execute(self, ctx: &Ctx) -> Result<ReactionChange, CommandError> {
        let reactor = console_reactor(ctx).await?;
        let session = visible_session(ctx, &self.session_id).await?;
        let messages = super::queries::message_service(ctx)?;
        set_reaction(
            messages,
            &session,
            &self.message_id,
            &self.emoji,
            reactor,
            true,
        )
        .await
    }
}

/// Take your emoji reaction off a message.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct RemoveMessageReaction {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Message the reaction is on (`message_...`).
    pub message_id: String,
    /// The emoji to take off.
    pub emoji: String,
}

#[command(
    name = "remove_message_reaction",
    category = "messages",
    description = "Take your emoji reaction off a message. Removing one that is not there changes nothing. The history is kept in the session's events.",
    summary = "Remove your reaction from a message",
    method = "DELETE",
    path = "/v1/sessions/{session_id}/messages/{message_id}/reactions/{emoji}",
    policy = crate::domains::sessions::SESSION_MANAGE,
    positional = "session_id",
    http = plain,
    responses((status = 404, description = "Session or message not found")),
    cli = CliRoute::new(&["sessions", "messages"], "unreact").with_args(&[CliArg::new("session_id").at(1).long("session"), CliArg::new("message_id").at(2), CliArg::new("emoji").at(3)]).with_examples(&[CliExample::new("Take back a thumbs up", "everruns sessions messages unreact session_01h9 message_01h9 👍",)]),
)]
impl Command for RemoveMessageReaction {
    type Output = ReactionChange;

    async fn execute(self, ctx: &Ctx) -> Result<ReactionChange, CommandError> {
        let reactor = console_reactor(ctx).await?;
        let session = visible_session(ctx, &self.session_id).await?;
        let messages = super::queries::message_service(ctx)?;
        set_reaction(
            messages,
            &session,
            &self.message_id,
            &self.emoji,
            reactor,
            false,
        )
        .await
    }
}

/// List the reactions in a session.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListMessageReactions {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Only this message's reactions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

#[command(
    name = "list_message_reactions",
    category = "messages",
    description = "List the emoji reactions on the messages of a session, from people and from the agent, with who reacted and whether you did.",
    summary = "List reactions in a session",
    method = "GET",
    path = "/v1/sessions/{session_id}/reactions",
    policy = crate::domains::sessions::SESSION_VIEW,
    positional = "session_id",
    http = plain,
    responses((status = 404, description = "Session not found")),
    cli = CliRoute::new(&["sessions", "messages"], "reactions").with_args(&[CliArg::new("session_id").at(1).long("session")]).with_examples(&[CliExample::new("See who reacted to what in a session", "everruns sessions messages reactions session_01h9",)]),
)]
impl Command for ListMessageReactions {
    type Output = Vec<MessageReactions>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<MessageReactions>, CommandError> {
        let session = visible_session(ctx, &self.session_id).await?;
        let viewer = ctx.caller.user_id.map(|id| id.to_string());
        Ok(list_reactions(
            &ctx.db,
            &session,
            self.message_id.as_deref(),
            viewer.as_deref().map(|id| (ReactionActorKind::User, id)),
        )
        .await?)
    }
}

#[cfg(test)]
mod tests;
