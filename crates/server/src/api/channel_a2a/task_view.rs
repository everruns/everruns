// The A2A `Task` as a caller sees it: state, interruption, and outputs.
//
// Design Decision: a task's outputs are A2A Artifacts (spec §3.7: "Results
// SHOULD be returned using Artifacts"), one per final assistant message of the
// task's latest turn, keyed by the message id so `tasks/get` and the
// `artifact-update` frames of `message/stream` name the same artifact. Without
// them a completed task carried no answer at all, and every client that waits
// for the result (a2a CLI, a2a-python, our own `a2a_delegation`) read nothing.
// Only final, text-only content is projected: commentary-phase messages, tool
// calls and tool results never leave the session (TM-A2A-012, mirroring
// TM-APIKEY-004 on the API endpoint).
//
// `message/send` is blocking by default in A2A 1.0 (§3.2.2), so this module
// also owns waiting for a task to settle: reach a terminal state or park on an
// `ask_user` question.

use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::execution_phase::ExecutionPhase;
use everruns_contracts::typed_id::SessionId;
use everruns_core::ContentPart;
use everruns_core::events::{
    EventData, OUTPUT_MESSAGE_COMPLETED, OutputMessageCompletedData, TURN_CANCELLED,
    TURN_COMPLETED, TURN_FAILED, TURN_STARTED,
};
use serde_json::{Value, json};

use super::{AuthorizedA2a, ChannelA2aState, ask_user};
use crate::storage::{EventRow, SessionRow, StorageBackend};

/// Upper bound on how long a blocking `message/send` holds the request open.
/// A turn that runs longer is returned in its current state, and the caller
/// keeps polling `tasks/get` (or uses `message/stream`); proxies in front of
/// the API cut idle requests well before an agent turn is guaranteed to end.
pub(super) const BLOCKING_SEND_TIMEOUT: Duration = Duration::from_secs(120);

/// Event tail read to derive state and outputs. Turn and message events are a
/// small fraction of a session's events, and only the latest turn matters.
const TASK_EVENT_TAIL: i32 = 200;

/// The latest turn of a session, projected for A2A.
#[derive(Debug, PartialEq)]
pub(super) struct TurnProjection {
    /// A2A 0.3 task state label.
    pub state: &'static str,
    /// `(message_id, text)` for each final assistant message of the turn.
    pub outputs: Vec<(String, String)>,
}

/// Pure projection of a session's filtered event tail (ascending) into the
/// latest turn's state and outputs.
pub(super) fn project_latest_turn(events: &[EventRow]) -> TurnProjection {
    let mut state = "submitted";
    let mut outputs = Vec::new();
    for event in events {
        match event.event_type.as_str() {
            TURN_STARTED => {
                state = "working";
                outputs.clear();
            }
            TURN_COMPLETED => state = "completed",
            TURN_FAILED => state = "failed",
            TURN_CANCELLED => state = "canceled",
            OUTPUT_MESSAGE_COMPLETED => {
                if let Ok(data) =
                    serde_json::from_value::<OutputMessageCompletedData>(event.data.clone())
                    && let Some(output) = final_output(&data)
                {
                    outputs.push(output);
                }
            }
            _ => {}
        }
    }
    TurnProjection { state, outputs }
}

/// The `(message_id, text)` an assistant message contributes to the task
/// result, or `None` when it is commentary or carries no text.
pub(super) fn final_output(data: &OutputMessageCompletedData) -> Option<(String, String)> {
    if matches!(data.message.phase, Some(ExecutionPhase::Commentary)) {
        return None;
    }
    let text = data
        .message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) if !text.text.is_empty() => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then(|| (data.message.id.to_string(), text))
}

/// An A2A `Artifact` carrying one agent output (internal 0.3 shape).
pub(super) fn output_artifact(message_id: &str, text: &str) -> Value {
    json!({
        "artifactId": message_id,
        "name": "response",
        "parts": [{ "kind": "text", "text": text }],
    })
}

pub(super) async fn read_latest_turn(
    db: &Arc<StorageBackend>,
    session_id: SessionId,
) -> anyhow::Result<TurnProjection> {
    let filter_types = [
        TURN_STARTED,
        TURN_COMPLETED,
        TURN_FAILED,
        TURN_CANCELLED,
        OUTPUT_MESSAGE_COMPLETED,
    ]
    .map(str::to_string);
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &filter_types,
            &[],
            None,
            Some(TASK_EVENT_TAIL),
        )
        .await?;
    Ok(project_latest_turn(&events))
}

/// Build the full task for a session already fenced to the calling channel
/// (TM-A2A-012): derived state, the `ask_user` interruption if parked, the
/// latest turn's outputs, and the structured `result.json` artifact (EVE-728).
pub(super) async fn load_task(
    state: &ChannelA2aState,
    auth: &AuthorizedA2a,
    session: &SessionRow,
) -> anyhow::Result<Value> {
    load_task_for(&state.db, &state.frontend_url, auth.org_id, session).await
}

/// [`load_task`] without a request: push delivery builds the same task from
/// the event that settled it.
pub(super) async fn load_task_for(
    db: &Arc<StorageBackend>,
    frontend_url: &str,
    org_id: i64,
    session: &SessionRow,
) -> anyhow::Result<Value> {
    let turn = read_latest_turn(db, session.id).await?;
    let mut state_label = turn.state;

    // EVE-1062: a session parked on `ask_user` is `input-required`, and the
    // question rides `TaskStatus.message`; the turn that asked is still open,
    // so the lifecycle events above report `working` on their own.
    let mut status_message = None;
    if let Some(pending) = ask_user::pending_ask_user(db, session).await? {
        let projection =
            ask_user::project_ask_user(&pending, &session.id.to_string(), frontend_url);
        state_label = projection.state;
        status_message = Some(projection.message);
    }

    let structured_result =
        crate::domains::session_tasks::read_structured_task_result(db, org_id, session.id).await?;

    let mut artifacts: Vec<Value> = turn
        .outputs
        .iter()
        .map(|(id, text)| output_artifact(id, text))
        .collect();
    if let Some(result) = structured_result {
        artifacts.push(super::a2a_result_artifact(result));
    }

    let mut task = ask_user::with_status_message(
        super::build_task_json(session.id, state_label, None),
        status_message,
    );
    // The session's last change is the task's status timestamp, the key
    // `ListTasks` orders and filters on.
    if let Some(status) = task.get_mut("status").and_then(Value::as_object_mut) {
        status.insert(
            "timestamp".to_string(),
            Value::String(session.updated_at.to_rfc3339()),
        );
    }
    if !artifacts.is_empty()
        && let Some(obj) = task.as_object_mut()
    {
        obj.insert("artifacts".to_string(), Value::Array(artifacts));
    }
    Ok(task)
}

/// Whether a session event settles a task for a blocking `message/send`:
/// the turn ended, or it parked on an `ask_user` question.
pub(super) fn settles_task(data: &EventData) -> bool {
    match data {
        EventData::TurnCompleted(_) | EventData::TurnFailed(_) | EventData::TurnCancelled(_) => {
            true
        }
        EventData::ToolCallRequested(requested) => {
            ask_user::pending_ask_user_from_request(requested).is_some()
        }
        _ => false,
    }
}

/// Wait until the subscribed session settles, or `timeout` passes.
pub(super) async fn wait_until_settled(
    subscription: &mut crate::event_delivery::EventSubscription,
    session_id: SessionId,
    timeout: Duration,
) {
    let wait = async {
        while let Some(event) = subscription.recv().await {
            if event.session_id == session_id && settles_task(&event.data) {
                return;
            }
        }
    };
    let _ = tokio::time::timeout(timeout, wait).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::EventId;
    use everruns_core::message::RuntimeMessage;

    fn row(event_type: &str, data: Value) -> EventRow {
        EventRow {
            id: EventId::new(),
            session_id: SessionId::new(),
            sequence: 0,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data,
            metadata: None,
            tags: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn output(text: &str, phase: Option<ExecutionPhase>) -> EventRow {
        let mut message = RuntimeMessage::assistant(text);
        message.phase = phase;
        let data = OutputMessageCompletedData::new(message);
        row(
            OUTPUT_MESSAGE_COMPLETED,
            serde_json::to_value(data).unwrap(),
        )
    }

    #[test]
    fn no_turn_events_is_submitted_with_no_outputs() {
        let projection = project_latest_turn(&[]);
        assert_eq!(projection.state, "submitted");
        assert!(projection.outputs.is_empty());
    }

    #[test]
    fn completed_turn_carries_its_final_text_only() {
        let events = [
            row(TURN_STARTED, json!({})),
            output("thinking out loud", Some(ExecutionPhase::Commentary)),
            output("the answer", None),
            row(TURN_COMPLETED, json!({})),
        ];
        let projection = project_latest_turn(&events);
        assert_eq!(projection.state, "completed");
        let texts: Vec<_> = projection.outputs.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, ["the answer"]);
    }

    #[test]
    fn a_new_turn_drops_the_previous_turns_outputs() {
        let events = [
            row(TURN_STARTED, json!({})),
            output("first answer", None),
            row(TURN_COMPLETED, json!({})),
            row(TURN_STARTED, json!({})),
        ];
        let projection = project_latest_turn(&events);
        assert_eq!(projection.state, "working");
        assert!(projection.outputs.is_empty());
    }

    #[test]
    fn non_text_content_is_not_projected() {
        let data = OutputMessageCompletedData::new(RuntimeMessage::assistant("  "));
        assert!(final_output(&data).is_none());
    }
}
