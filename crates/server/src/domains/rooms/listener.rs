//! An agent's `send_message` in a room session becomes its post in the room.
//!
//! Decisions:
//! - The post lands in the thread of the message that last woke the agent in
//!   that conversation, one hop further than it. The session's binding row
//!   remembers both.
//! - The session message id is the post's `source_ref`, so an event delivered
//!   twice posts once.
//! - Posting wakes the agents the post addresses, through the same rules as a
//!   person's post; the hop count is what stops two agents from talking
//!   forever.
//! - Work runs off the event path and is best-effort, like the coordination
//!   listener: a failure is logged, never fails the turn.
//! - The services are bound after construction, because listeners are built
//!   before the session and message services exist.

use super::queries::RoomView;
use super::rules;
use super::wake::{Waker, wake_agents};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::kernel_imports::contracts::typed_id::SessionId;
use crate::storage::{CreateRoomMessageRow, StorageBackend};
use everruns_contracts::runtime::events::CONVERSATION_MESSAGE;
use serde_json::json;
use std::sync::{Arc, OnceLock};

struct Services {
    sessions: Arc<SessionService>,
    messages: Arc<MessageService>,
}

pub struct RoomPostListener {
    db: Arc<StorageBackend>,
    services: OnceLock<Services>,
}

impl RoomPostListener {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            services: OnceLock::new(),
        }
    }

    /// Bind the services that wake the agents an agent's post addresses.
    pub fn bind(&self, sessions: Arc<SessionService>, messages: Arc<MessageService>) {
        let _ = self.services.set(Services { sessions, messages });
    }
}

#[async_trait::async_trait]
impl everruns_core::event_listeners::EventListener for RoomPostListener {
    fn name(&self) -> &'static str {
        "room_posts"
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![CONVERSATION_MESSAGE])
    }

    async fn on_event(&self, event: &everruns_core::Event) {
        let Some(services) = self.services.get() else {
            return;
        };
        let db = self.db.clone();
        let sessions = services.sessions.clone();
        let messages = services.messages.clone();
        let session_id = event.session_id;
        let everruns_contracts::runtime::events::EventData::ConversationMessage(sent) = &event.data
        else {
            return;
        };
        let text = sent.text.clone();
        let message_ref = sent.message_id.to_string();
        tokio::spawn(async move {
            if let Err(error) = post_from_session(
                &db,
                Some((&sessions, &messages)),
                session_id,
                &text,
                &message_ref,
            )
            .await
            {
                tracing::warn!(%session_id, %error, "rooms: could not post an agent's message");
            }
        });
    }
}

/// Post what a room session's agent sent. Does nothing for sessions that are
/// not room sessions. `services` absent (tests) posts without waking anyone.
pub(crate) async fn post_from_session(
    db: &Arc<StorageBackend>,
    services: Option<(&SessionService, &MessageService)>,
    session_id: SessionId,
    text: &str,
    message_ref: &str,
) -> anyhow::Result<Option<crate::storage::RoomMessageRow>> {
    let Some(binding) = db.find_room_agent_session_by_session(session_id).await? else {
        return Ok(None);
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let Some(member) = db
        .get_room_member(binding.room_id, binding.member_id)
        .await?
        .filter(|member| member.left_at.is_none())
    else {
        return Ok(None);
    };
    let Some(room) = db
        .get_room(member.org_id, binding.room_id)
        .await?
        .filter(|room| room.archived_at.is_none())
    else {
        return Ok(None);
    };
    let view = RoomView::load(db, room).await?;
    let thread_id = match binding.last_wake_thread_id {
        Some(thread) => Some(thread),
        None if binding.scope_id != view.room.id.uuid() => Some(
            crate::kernel_imports::contracts::typed_id::RoomMessageId::from_uuid(binding.scope_id),
        ),
        None => None,
    };
    let Some(row) = db
        .post_room_message(CreateRoomMessageRow {
            room_id: view.room.id,
            org_id: view.room.org_id,
            author_member_id: Some(member.id),
            thread_id,
            text: text.to_string(),
            parts: json!([{ "type": "text", "text": text }]),
            mentions: view.mentions_in(text),
            hops: rules::agent_post_hops(binding.last_wake_hops),
            session_id: Some(session_id),
            source_ref: Some(format!("session:{message_ref}")),
        })
        .await?
    else {
        // Already posted.
        return Ok(None);
    };
    if let Some((sessions, messages)) = services {
        wake_agents(
            &Waker {
                db,
                sessions,
                messages,
                // An agent's post is never checked for relevance.
                relevance: None,
            },
            &view,
            &row,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    }
    Ok(Some(row))
}
