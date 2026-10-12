// A Poppy conversation's events (spec 7.5, 7.7), projected from the backing
// session's stored events.
//
// Design Decisions:
// - Event ids are `evt_{sequence}`, so a cursor is a position in the
//   session's own event log and needs no table. The closing `state` event is
//   `evt_closed`, always last: nothing is read after it.
// - Only final answers become company messages. Commentary, tool calls,
//   reasoning and internal notes (a message carrying `poppy_note`) never
//   reach the personal agent. THREAT[TM-POPPY-006].
// - The personal agent's own messages are echoed as it sent them (its `id`,
//   `sender`, `text`, `data`), not as the text rendered for the model.

use everruns_core::events::{
    INPUT_MESSAGE, InputMessageData, OUTPUT_MESSAGE_COMPLETED, OutputMessageCompletedData,
    TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED,
};
use serde_json::{Value, json};

use crate::storage::EventRow;

/// Metadata keys on a user message the personal agent sent.
pub(super) const MESSAGE_ID: &str = "poppy_message_id";
pub(super) const SENDER: &str = "poppy_sender";
pub(super) const TEXT: &str = "poppy_text";
pub(super) const DATA: &str = "poppy_data";
/// Marks a message the channel wrote for the agent, never shown as a message.
pub(super) const NOTE: &str = "poppy_note";

/// The closing event's id.
pub(super) const CLOSED_EVENT_ID: &str = "evt_closed";

/// Stored event types a conversation is read from.
pub(super) const EVENT_TYPES: [&str; 6] = [
    INPUT_MESSAGE,
    OUTPUT_MESSAGE_COMPLETED,
    TURN_STARTED,
    TURN_COMPLETED,
    TURN_FAILED,
    TURN_CANCELLED,
];

/// Where a read starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cursor {
    /// After the stored event with this sequence; `0` is the beginning.
    After(i32),
    /// After the closing event: nothing follows.
    Closed,
}

/// Pure: parse a cursor. `Err` is `invalid_cursor`.
pub(super) fn parse_cursor(cursor: Option<&str>) -> Result<Cursor, ()> {
    match cursor {
        None => Ok(Cursor::After(0)),
        Some(CLOSED_EVENT_ID) => Ok(Cursor::Closed),
        Some(cursor) => cursor
            .strip_prefix("evt_")
            .and_then(|seq| seq.parse::<i32>().ok())
            .filter(|seq| *seq > 0)
            .map(Cursor::After)
            .ok_or(()),
    }
}

pub(super) fn event_id(sequence: i32) -> String {
    format!("evt_{sequence}")
}

/// Pure: the Poppy event for one stored event, or `None` when it is hidden.
pub(super) fn project(row: &EventRow) -> Option<Value> {
    let created_at = row.ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let id = event_id(row.sequence);
    match row.event_type.as_str() {
        INPUT_MESSAGE => {
            let data: InputMessageData = serde_json::from_value(row.data.clone()).ok()?;
            let metadata = data.message.metadata?;
            if metadata.contains_key(NOTE) {
                return None;
            }
            let mut message = json!({
                "id": metadata.get(MESSAGE_ID)?.as_str()?,
                "role": "user",
                "sender": metadata.get(SENDER).and_then(Value::as_str).unwrap_or("agent"),
            });
            for (key, field) in [(TEXT, "text"), (DATA, "data")] {
                if let Some(value) = metadata.get(key) {
                    message[field] = value.clone();
                }
            }
            Some(
                json!({ "id": id, "type": "message", "created_at": created_at, "message": message }),
            )
        }
        OUTPUT_MESSAGE_COMPLETED => {
            let data: OutputMessageCompletedData = serde_json::from_value(row.data.clone()).ok()?;
            let text = everruns_core::conversation::said_text_with_phase(
                data.message.phase,
                &data.message.content,
            )?;
            Some(json!({
                "id": id,
                "type": "message",
                "created_at": created_at,
                "message": {
                    "id": format!("msg_{}", row.id.uuid().simple()),
                    "role": "company",
                    "sender": "agent",
                    "text": text,
                },
            }))
        }
        TURN_STARTED => Some(state_event(&id, &created_at, "working")),
        TURN_COMPLETED | TURN_FAILED | TURN_CANCELLED => {
            Some(state_event(&id, &created_at, "idle"))
        }
        _ => None,
    }
}

/// Pure: whether a long-poll may return on `event`: anything from the
/// company side except the start of a turn.
pub(super) fn answers(event: &Value) -> bool {
    match event["type"].as_str() {
        Some("message") => event["message"]["role"] == "company",
        Some("state") => event["status"] != "working",
        _ => true,
    }
}

pub(super) fn state_event(id: &str, created_at: &str, status: &str) -> Value {
    json!({
        "id": id,
        "type": "state",
        "created_at": created_at,
        "status": status,
        "responder": "agent",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::{EventId, SessionId};

    fn row(sequence: i32, event_type: &str, data: Value) -> EventRow {
        EventRow {
            id: EventId::new(),
            session_id: SessionId::new(),
            sequence,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data,
            metadata: None,
            tags: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn input(metadata: Value) -> Value {
        let mut message = everruns_core::message::RuntimeMessage::user("rendered for the model");
        message.metadata = serde_json::from_value(metadata).ok();
        serde_json::to_value(InputMessageData::new(message)).unwrap()
    }

    #[test]
    fn cursors_round_trip_and_reject_foreign_values() {
        assert_eq!(parse_cursor(None), Ok(Cursor::After(0)));
        assert_eq!(parse_cursor(Some("evt_42")), Ok(Cursor::After(42)));
        assert_eq!(parse_cursor(Some(CLOSED_EVENT_ID)), Ok(Cursor::Closed));
        for bad in ["42", "evt_", "evt_x", "evt_-1", "evt_0", "msg_1"] {
            assert!(parse_cursor(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn echoes_the_personal_agents_message_as_sent() {
        let event = project(&row(
            3,
            INPUT_MESSAGE,
            input(json!({
                MESSAGE_ID: "msg_a", SENDER: "human", TEXT: "Medium, please.",
                DATA: { "size": "M" },
            })),
        ))
        .unwrap();
        assert_eq!(event["id"], "evt_3");
        assert_eq!(
            event["message"],
            json!({ "id": "msg_a", "role": "user", "sender": "human",
                    "text": "Medium, please.", "data": { "size": "M" } })
        );
    }

    #[test]
    fn hides_notes_and_unknown_messages() {
        assert!(project(&row(1, INPUT_MESSAGE, input(json!({ NOTE: "handoff" })))).is_none());
        assert!(project(&row(1, INPUT_MESSAGE, input(json!({ "source": "ui" })))).is_none());
        assert!(project(&row(1, "tool.started", json!({}))).is_none());
    }

    #[test]
    fn long_polls_wait_past_their_own_echo() {
        let echo = json!({ "type": "message", "message": { "role": "user" } });
        let reply = json!({ "type": "message", "message": { "role": "company" } });
        assert!(!answers(&echo));
        assert!(answers(&reply));
        assert!(!answers(&state_event("evt_1", "t", "working")));
        assert!(answers(&state_event("evt_2", "t", "idle")));
    }

    #[test]
    fn turns_become_state_events() {
        assert_eq!(
            project(&row(5, TURN_STARTED, json!({}))).unwrap()["status"],
            "working"
        );
        assert_eq!(
            project(&row(6, TURN_FAILED, json!({}))).unwrap()["status"],
            "idle"
        );
    }
}
