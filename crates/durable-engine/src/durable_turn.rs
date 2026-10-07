//! Agent-turn conventions layered on the generic `everruns-durable` engine.
//!
//! Decision: `everruns-durable` knows workflows, activities, tasks and signals
//! and nothing about sessions or turns. The names, ids and options that give a
//! durable task its turn meaning live here, in durable-engine, and the server's
//! durable gRPC service applies the same rules to tasks a worker enqueues
//! remotely. Every string here is persisted or on the wire: keep values stable.

use everruns_core::engine::TurnState;
use everruns_durable::ActivityOptions;
use uuid::Uuid;

/// Workflow type of a session's turn workflow (one per session, a run per
/// turn). The server's stale-task reaper resumes runs of this type stranded
/// between two steps.
pub const TURN_WORKFLOW_TYPE: &str = "turn_workflow";

/// Signal type for a user message that arrived while a turn is already
/// running (steering, or a task wake picked up at an iteration boundary).
///
/// Payload:
/// - Required: `input_message_id` (`<MessageId>`)
/// - Optional (used for follow-on turn, with fallbacks if omitted):
///   `org_id`, `harness_id`, `agent_id`
///
/// Example: `{ "input_message_id": "<id>", "org_id": 1, "harness_id": "<id>", "agent_id": "<id>" }`
pub const USER_MESSAGE: &str = "user_message";

/// The [`USER_MESSAGE`] payload announcing the stored message `input`
/// starts its turn with.
pub(crate) fn steering_payload(input: &TurnState) -> serde_json::Value {
    serde_json::json!({
        "input_message_id": input.input_message_id.to_string(),
        "org_id": input.org_id,
        "harness_id": input.harness_id.to_string(),
        "agent_id": input.agent_id.map(|id| id.to_string()),
    })
}

/// The input of a turn started for the [`USER_MESSAGE`] `payload` on the
/// session `previous` ran a turn of, the payload's fields taking over the
/// previous turn's. `None` when the payload names no message.
pub(crate) fn turn_input_for_steering(
    previous: &TurnState,
    payload: &serde_json::Value,
) -> Option<TurnState> {
    fn field<T: std::str::FromStr>(payload: &serde_json::Value, key: &str) -> Option<T> {
        payload.get(key)?.as_str()?.parse().ok()
    }
    Some(TurnState {
        org_id: payload
            .get("org_id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(previous.org_id),
        session_id: previous.session_id,
        harness_id: field(payload, "harness_id").unwrap_or(previous.harness_id),
        agent_id: field(payload, "agent_id").or(previous.agent_id),
        input_message_id: field(payload, "input_message_id")?,
        turn_id: None,
        previous_response_id: None,
        iteration: 1,
        request_id: None,
        started_at: Some(chrono::Utc::now()),
        cumulative_usage: None,
        tool_call_count: 0,
        llm_call_count: 0,
        time_to_first_token_ms: None,
        final_message_id: None,
        final_answer_preview: None,
    })
}

/// Activity-id prefix of the `reason` task that resumes a parked turn once its
/// waiting resolution (tool results, approvals, answers) is claimed.
///
/// Several paths may race to resume the same resolution, so these tasks
/// enqueue idempotently per `(workflow_id, activity_id)`. A unique index backs this
/// over exactly this prefix, so the prefix is part of the data contract.
pub const WAITING_TURN_RESOLUTION_ACTIVITY_PREFIX: &str = "waiting_turn_resolution_";

/// Activity id of the task that resumes the parked turn for `resolution_id`.
pub fn waiting_turn_resolution_activity_id(resolution_id: Uuid) -> String {
    format!("{WAITING_TURN_RESOLUTION_ACTIVITY_PREFIX}{resolution_id}")
}

/// Durable options for a turn task identified only by its activity id.
///
/// The worker's durable store backends and the server's gRPC enqueue both take
/// a bare activity id, so the turn semantics that need engine options are
/// derived from it here: a waiting-turn resolution enqueues idempotently.
pub fn activity_options_for(activity_id: &str) -> ActivityOptions {
    let options = ActivityOptions::default();
    if activity_id.starts_with(WAITING_TURN_RESOLUTION_ACTIVITY_PREFIX) {
        options.with_dedupe_by_activity_id()
    } else {
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_turn_resolution_tasks_enqueue_idempotently() {
        let id = waiting_turn_resolution_activity_id(Uuid::nil());
        assert_eq!(
            id,
            "waiting_turn_resolution_00000000-0000-0000-0000-000000000000"
        );
        assert!(activity_options_for(&id).dedupe_by_activity_id);
    }

    #[test]
    fn other_turn_tasks_keep_default_options() {
        for id in ["input_0192", "reason-1", "act-1", "waiting_turn"] {
            assert_eq!(activity_options_for(id), ActivityOptions::default(), "{id}");
        }
    }

    #[test]
    fn signal_wire_value_is_stable() {
        assert_eq!(USER_MESSAGE, "user_message");
    }
}
