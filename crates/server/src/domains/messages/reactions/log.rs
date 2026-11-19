// Where a session's reactions are read and written.
//
// Decisions (data plane v2, step 11.6):
// - A session the platform's workers run keeps its reactions in the control
//   plane's event index, as it always has. A v2 session keeps them in its
//   own log on its data plane: the control plane reads them back through the
//   session's event read, and records a person's reaction there as an event
//   the cell takes from the control plane (`record`). One fold, one set of
//   rules, for both.
// - Telling the agent at once sends a reaction message to the cell, like any
//   message the control plane admits. A reaction left for the agent's next
//   turn is told with the next message the control plane admits; a message a
//   person sends straight to a v2 session's data plane does not carry it.

use super::{ReactedMessageRow, ReactionEventRow};
use crate::domains::data_plane::session_plane::SessionPlane;
use crate::domains::messages::service::{CreateMessageContext, MessageService};
use crate::storage::Database;
use everruns_contracts::dataplane::SessionEventRow;
use everruns_contracts::message_reactions::ReactionNote;
use everruns_contracts::session_commands;
use everruns_contracts::typed_id::{EventId, MessageId};
use everruns_core::{
    EventRequest, INPUT_MESSAGE, MESSAGE_REACTION_ADDED, MESSAGE_REACTION_REMOVED,
};
use uuid::Uuid;

/// Newest reaction events read from a data plane.
const MAX_DATA_PLANE_REACTIONS: usize = 1_000;
/// Newest input messages searched for the last reaction note.
const NOTE_SEARCH: usize = 200;

/// A session's reaction log.
pub(super) enum ReactionLog<'a> {
    /// The control plane's event index (a session the workers run).
    Indexed(&'a Database),
    /// The session's own log on its data plane.
    DataPlane(SessionPlane),
}

impl ReactionLog<'_> {
    /// Every reaction event, oldest first.
    pub(super) async fn reaction_events(
        &self,
        session: Uuid,
    ) -> anyhow::Result<Vec<ReactionEventRow>> {
        match self {
            Self::Indexed(db) => db.list_reaction_events(session).await,
            Self::DataPlane(plane) => Ok(plane
                .events(
                    1,
                    i32::MAX,
                    &[MESSAGE_REACTION_ADDED, MESSAGE_REACTION_REMOVED],
                    Some(MAX_DATA_PLANE_REACTIONS),
                )
                .await?
                .into_iter()
                .map(reaction_row)
                .collect()),
        }
    }

    /// The last reaction event the agent was told about.
    pub(super) async fn told_through(&self, session: Uuid) -> anyhow::Result<Option<i64>> {
        match self {
            Self::Indexed(db) => db.reactions_told_through(session).await,
            Self::DataPlane(plane) => Ok(told_through(
                &plane
                    .events(1, i32::MAX, &[INPUT_MESSAGE], Some(NOTE_SEARCH))
                    .await?,
            )),
        }
    }

    /// The user and agent messages with these ids.
    pub(super) async fn reacted_messages(
        &self,
        session: Uuid,
        ids: &[String],
    ) -> anyhow::Result<Vec<ReactedMessageRow>> {
        match self {
            Self::Indexed(db) => db.reacted_messages(session, ids).await,
            Self::DataPlane(plane) => Ok(plane
                .messages(ids)
                .await?
                .into_iter()
                .filter_map(message_row)
                .collect()),
        }
    }

    /// Append a reaction event.
    pub(super) async fn record(
        &self,
        messages: &MessageService,
        request: EventRequest,
    ) -> anyhow::Result<()> {
        match self {
            Self::Indexed(_) => {
                messages.event_service().emit(request).await?;
            }
            Self::DataPlane(plane) => {
                let body = serde_json::json!({ "event_id": EventId::new(), "event": request });
                plane.command(&session_commands::RECORD, &body).await?;
            }
        }
        Ok(())
    }

    /// Tell the agent `note` now, as a reaction message.
    pub(super) async fn tell(
        &self,
        messages: &MessageService,
        ctx: CreateMessageContext,
        note: ReactionNote,
    ) -> anyhow::Result<()> {
        match self {
            Self::Indexed(_) => {
                messages.create_reaction_message(ctx, note).await?;
            }
            Self::DataPlane(plane) => {
                let mut metadata = std::collections::HashMap::new();
                note.write(&mut metadata);
                let origin = everruns_core::message::MessageOrigin::new(
                    everruns_core::message::MessageOriginKind::Reaction,
                );
                // Retry-safe: the same note is the same message.
                let id = MessageId::from_uuid(Uuid::new_v5(
                    &ctx.session_id,
                    format!("everruns.reactions.{}", note.through_sequence).as_bytes(),
                ));
                let body = serde_json::json!({
                    "id": id,
                    "text": note.render(),
                    "origin": origin,
                    "metadata": metadata,
                });
                plane.command(&session_commands::MESSAGES, &body).await?;
            }
        }
        Ok(())
    }
}

fn reaction_row(row: SessionEventRow) -> ReactionEventRow {
    ReactionEventRow {
        sequence: i64::from(row.sequence),
        event_type: row.event_type,
        data: row.data,
        created_at: row.ts,
    }
}

fn message_row(row: SessionEventRow) -> Option<ReactedMessageRow> {
    let message = row.data.get("message")?.clone();
    Some(ReactedMessageRow {
        message_id: message.get("id")?.as_str()?.to_owned(),
        event_type: row.event_type,
        message,
        sequence: i64::from(row.sequence),
    })
}

/// The newest `through_sequence` a reaction note in `inputs` carries.
fn told_through(inputs: &[SessionEventRow]) -> Option<i64> {
    inputs.iter().rev().find_map(|row| {
        row.data["message"]["metadata"]["everruns_reactions"]["through_sequence"].as_i64()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(sequence: i32, event_type: &str, data: serde_json::Value) -> SessionEventRow {
        SessionEventRow {
            id: format!("event_{sequence}"),
            sequence,
            event_type: event_type.to_owned(),
            ts: chrono::Utc::now(),
            context: serde_json::Value::Null,
            data,
        }
    }

    #[test]
    fn the_newest_note_says_what_was_told() {
        let note = |through: i64| json!({ "message": { "metadata": { "everruns_reactions": { "through_sequence": through } } } });
        let inputs = [
            row(1, INPUT_MESSAGE, note(3)),
            row(5, INPUT_MESSAGE, note(4)),
            row(7, INPUT_MESSAGE, json!({ "message": { "content": [] } })),
        ];
        assert_eq!(told_through(&inputs), Some(4));
        assert_eq!(told_through(&inputs[2..]), None);
    }

    #[test]
    fn a_message_event_becomes_a_reacted_message() {
        let message = json!({ "message": { "id": "message_1", "content": [] } });
        let row = message_row(row(9, "output.message.completed", message)).unwrap();
        assert_eq!(row.message_id, "message_1");
        assert_eq!(row.sequence, 9);
        assert!(message_row(super::tests::row(2, "input.message", json!({}))).is_none());
    }
}
