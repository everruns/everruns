// Turn scheduling decisions after activity completion
//
// Decision: Extract the "what to do next after reason completes" logic into
// a shared module so neither durable_worker nor unified_worker knows about
// steering signals directly. Both workers call `after_reason_completed` and
// act on the returned `ReasonDecision`.
//
// This mirrors how TurnStateMachine (core) owns phase transitions while
// this module owns the scheduling policy that wraps those transitions.

use everruns_core::ReasonResult;
use everruns_durable::WorkflowSignal;
use tracing::info;

/// Decision returned after reason activity completes.
///
/// The caller (worker) acts on this without needing to understand
/// steering signals, session.idled timing, or turn continuation logic.
#[derive(Debug)]
pub enum ReasonDecision {
    /// Schedule act activity — reason returned tool calls.
    ScheduleAct,

    /// Continue the same turn with another reason iteration.
    ///
    /// A user message arrived during the turn. The message is already
    /// in the DB and will be picked up by the message retriever on the
    /// next reason call. No new turn is needed — this is "in-turn steering"
    /// matching Claude Code / Codex behavior.
    ContinueTurn {
        /// Number of user messages that were queued during the turn.
        queued_message_count: usize,
    },

    /// Turn is truly complete — no tool calls, no pending messages.
    /// The caller should emit session.idled and mark the workflow done.
    Complete,

    /// Turn failed — reason reported an error.
    /// The caller should emit session.idled (already emitted turn.failed
    /// from the activity) and mark the workflow done.
    Failed,
}

/// Determine what to do after reason activity completes.
///
/// This is the single source of truth for the scheduling decision.
/// Both durable_worker and unified_worker call this and act on the result.
///
/// # Arguments
/// * `reason_result` - The result from the reason activity
/// * `pending_signals` - Consumed steering signals from the workflow store
pub fn after_reason_completed(
    reason_result: &ReasonResult,
    pending_signals: &[WorkflowSignal],
) -> ReasonDecision {
    // Failure is terminal — don't check signals
    if !reason_result.success {
        return ReasonDecision::Failed;
    }

    // Has tool calls → schedule act (signals wait until after act→reason loop)
    if reason_result.has_tool_calls {
        return ReasonDecision::ScheduleAct;
    }

    // No tool calls, success → check for pending user messages
    let user_message_count = pending_signals
        .iter()
        .filter(|sig| sig.signal_type == everruns_durable::signal_types::USER_MESSAGE)
        .count();

    if user_message_count > 0 {
        if user_message_count > 1 {
            info!(
                queued_count = user_message_count,
                "Multiple user messages arrived during turn — all in \
                 conversation history, continuing with next reason iteration"
            );
        }
        ReasonDecision::ContinueTurn {
            queued_message_count: user_message_count,
        }
    } else {
        ReasonDecision::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::ReasonResult;

    fn success_result(has_tool_calls: bool) -> ReasonResult {
        ReasonResult {
            success: true,
            text: "test".to_string(),
            tool_calls: vec![],
            has_tool_calls,
            tool_definitions: vec![],
            max_iterations: 100,
            error: None,
            usage: None,
            response_id: None,
            locale: None,
        }
    }

    fn failed_result() -> ReasonResult {
        ReasonResult {
            success: false,
            text: String::new(),
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: vec![],
            max_iterations: 100,
            error: Some("error".to_string()),
            usage: None,
            response_id: None,
            locale: None,
        }
    }

    fn user_message_signal() -> WorkflowSignal {
        WorkflowSignal::new(
            everruns_durable::signal_types::USER_MESSAGE,
            serde_json::json!({"input_message_id": "msg_test"}),
        )
    }

    #[test]
    fn test_schedule_act_when_tool_calls() {
        let result = success_result(true);
        let decision = after_reason_completed(&result, &[]);
        assert!(matches!(decision, ReasonDecision::ScheduleAct));
    }

    #[test]
    fn test_complete_when_no_tools_no_signals() {
        let result = success_result(false);
        let decision = after_reason_completed(&result, &[]);
        assert!(matches!(decision, ReasonDecision::Complete));
    }

    #[test]
    fn test_continue_turn_when_user_message_signal() {
        let result = success_result(false);
        let signals = vec![user_message_signal()];
        let decision = after_reason_completed(&result, &signals);
        match decision {
            ReasonDecision::ContinueTurn {
                queued_message_count,
            } => assert_eq!(queued_message_count, 1),
            other => panic!("Expected ContinueTurn, got {:?}", other),
        }
    }

    #[test]
    fn test_continue_turn_with_multiple_signals() {
        let result = success_result(false);
        let signals = vec![user_message_signal(), user_message_signal()];
        let decision = after_reason_completed(&result, &signals);
        match decision {
            ReasonDecision::ContinueTurn {
                queued_message_count,
            } => assert_eq!(queued_message_count, 2),
            other => panic!("Expected ContinueTurn, got {:?}", other),
        }
    }

    #[test]
    fn test_failed_ignores_signals() {
        let result = failed_result();
        let signals = vec![user_message_signal()];
        let decision = after_reason_completed(&result, &signals);
        assert!(matches!(decision, ReasonDecision::Failed));
    }

    #[test]
    fn test_tool_calls_ignores_signals() {
        let result = success_result(true);
        let signals = vec![user_message_signal()];
        let decision = after_reason_completed(&result, &signals);
        // Signals are deferred until after the act→reason loop
        assert!(matches!(decision, ReasonDecision::ScheduleAct));
    }
}
