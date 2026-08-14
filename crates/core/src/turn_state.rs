//! The turn state machine as a serializable value with pure transitions.
//!
//! Neutral state values used by the engine-owned execution machine. Production
//! in-process and durable drivers both advance this representation through
//! `everruns-engine`; core contains the portable data contract only.
//!
//! [`TurnState`] is the representation both can share: a value that serializes,
//! with transitions that consume it and return the next one. It holds exactly
//! what the mutable machine holds and behaves identically — the conformance
//! test in this module drives the same sequences through both and asserts the
//! same actions and outcomes.
//!
//! No I/O belongs here, ever. That is the property the later stages depend on:
//! a transition that loads or emits is a transition a durable host cannot
//! replay.

use serde::{Deserialize, Serialize};

use crate::turn::{SealReason, TurnAction, TurnOutcome, TurnPhase, TurnStopReason};
use crate::typed_id::{AgentId, MessageId, SessionId, TurnId};

/// Turn-scoped identifiers, in a form that survives a round trip through
/// storage.
///
/// Mirrors [`TurnContext`](crate::turn::TurnContext), which is not
/// serializable. Stage 2 collapses the two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnIds {
    /// Session this turn belongs to.
    pub session_id: SessionId,
    /// This turn's identifier, fixed at creation and used to correlate events.
    pub turn_id: TurnId,
    /// The user message that started the turn.
    pub input_message_id: MessageId,
    /// Agent executing the turn.
    pub agent_id: AgentId,
    /// Owning organization.
    pub org_id: i64,
}

impl From<&crate::turn::TurnContext> for TurnIds {
    fn from(context: &crate::turn::TurnContext) -> Self {
        Self {
            session_id: context.session_id,
            turn_id: context.turn_id,
            input_message_id: context.input_message_id,
            agent_id: context.agent_id,
            org_id: context.org_id,
        }
    }
}

/// What the model reported when a reason step finished.
///
/// A struct rather than six positional arguments because every caller of the
/// mutable machine's `on_reason_completed` has to get the order right, and two
/// of the six are `Option<String>`.
#[derive(Debug, Clone, Default)]
pub struct ReasonReport {
    /// Text the model produced; empty is normal for a tool-calling step.
    pub response: String,
    /// How many tool calls the model requested.
    pub tool_call_count: usize,
    /// Whether the call itself succeeded.
    pub success: bool,
    /// Failure message when `success` is false.
    pub error: Option<String>,
    /// Raw provider finish reason, normalized on the way in.
    pub finish_reason: Option<String>,
    /// Whether new user messages arrived mid-turn (in-turn steering). When the
    /// step would otherwise end the turn, this sends it back to reason so the
    /// next iteration picks them up.
    pub has_pending_user_messages: bool,
}

impl ReasonReport {
    /// A successful step that produced `response` and requested no tools.
    pub fn text(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            success: true,
            ..Default::default()
        }
    }

    /// A successful step that requested `tool_call_count` tools.
    pub fn tool_calls(response: impl Into<String>, tool_call_count: usize) -> Self {
        Self {
            response: response.into(),
            tool_call_count,
            success: true,
            ..Default::default()
        }
    }

    /// A failed step.
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(error.into()),
            ..Default::default()
        }
    }

    /// Set the provider finish reason.
    pub fn with_finish_reason(mut self, finish_reason: impl Into<String>) -> Self {
        self.finish_reason = Some(finish_reason.into());
        self
    }

    /// Mark that user messages arrived during the turn.
    pub fn with_pending_user_messages(mut self) -> Self {
        self.has_pending_user_messages = true;
        self
    }
}

/// A turn's orchestration state: phase plus the bookkeeping needed to decide
/// what happens next and what the turn's outcome is.
///
/// Transitions consume `self` and return the next state, so a caller cannot
/// half-apply one or hold a stale copy by accident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnState {
    /// Turn-scoped identifiers.
    pub ids: TurnIds,
    phase: TurnPhase,
    max_iterations: usize,
    current_iteration: usize,
    total_tool_calls: usize,
    last_response: String,
    pending_error: Option<String>,
    pending_stop_reason: TurnStopReason,
    has_pending_tool_calls: bool,
    pending_seal: Option<SealReason>,
}

impl TurnState {
    /// Start a turn, waiting on its input step.
    pub fn new(ids: TurnIds, max_iterations: usize) -> Self {
        Self {
            ids,
            phase: TurnPhase::PendingInput,
            max_iterations,
            current_iteration: 0,
            total_tool_calls: 0,
            last_response: String::new(),
            pending_error: None,
            pending_stop_reason: TurnStopReason::EndTurn,
            has_pending_tool_calls: false,
            pending_seal: None,
        }
    }

    /// Current phase.
    pub fn phase(&self) -> TurnPhase {
        self.phase
    }

    /// Reason→act cycles completed so far.
    pub fn current_iteration(&self) -> usize {
        self.current_iteration
    }

    /// Tool calls executed across the turn.
    pub fn total_tool_calls(&self) -> usize {
        self.total_tool_calls
    }

    /// Whether the last reason step left tool calls to run.
    pub fn has_pending_tool_calls(&self) -> bool {
        self.has_pending_tool_calls
    }

    /// Whether the turn has reached a terminal phase.
    pub fn is_completed(&self) -> bool {
        self.phase == TurnPhase::Completed
    }

    /// What the host should do next.
    ///
    /// Pure: calling it twice on the same state yields the same action, and it
    /// never advances anything.
    pub fn next_action(&self) -> TurnAction {
        match self.phase {
            TurnPhase::PendingInput => TurnAction::ExecuteInput,
            TurnPhase::PendingReason => TurnAction::ExecuteReason,
            TurnPhase::PendingAct => TurnAction::ExecuteAct,
            TurnPhase::Completed => TurnAction::Complete(self.outcome()),
        }
    }

    /// The outcome a completed turn resolves to.
    ///
    /// A deliberate seal wins over everything else: budget exhaustion must
    /// resolve to `Sealed` rather than looking like a failure or leaving the
    /// turn reclaimable.
    fn outcome(&self) -> TurnOutcome {
        if let Some(reason) = self.pending_seal {
            return TurnOutcome::Sealed {
                reason,
                response: self.last_response.clone(),
                iterations: self.current_iteration,
                tool_calls_count: self.total_tool_calls,
            };
        }
        if let Some(error) = &self.pending_error {
            return TurnOutcome::Failed {
                error: error.clone(),
                iterations: self.current_iteration,
                stop_reason: self.pending_stop_reason,
            };
        }
        // Reaching the ceiling is the terminal, whether or not tool calls were
        // left unrun. (Mirrors `TurnStateMachine`; the conformance test fails if
        // this drifts, which is how the extra condition tried here was caught.)
        if self.current_iteration >= self.max_iterations {
            return TurnOutcome::MaxIterationsReached {
                response: self.last_response.clone(),
                iterations: self.current_iteration,
                tool_calls_count: self.total_tool_calls,
            };
        }
        TurnOutcome::Success {
            response: self.last_response.clone(),
            iterations: self.current_iteration,
            tool_calls_count: self.total_tool_calls,
            stop_reason: self.pending_stop_reason,
        }
    }

    /// The input step recorded the user message.
    pub fn on_input_completed(mut self) -> Self {
        debug_assert_eq!(self.phase, TurnPhase::PendingInput);
        self.phase = TurnPhase::PendingReason;
        self
    }

    /// A reason step finished.
    pub fn on_reason_completed(mut self, report: ReasonReport) -> Self {
        debug_assert_eq!(self.phase, TurnPhase::PendingReason);

        self.current_iteration += 1;

        if !report.response.is_empty() {
            self.last_response = report.response;
        }

        if !report.success {
            // A refusal is a distinct terminal; anything else that failed is an
            // error, whatever the provider called it.
            self.pending_stop_reason = match TurnStopReason::from_provider_finish_reason(
                report.finish_reason.as_deref(),
            ) {
                TurnStopReason::Refusal => TurnStopReason::Refusal,
                _ => TurnStopReason::Error,
            };
            self.pending_error = report.error;
            self.phase = TurnPhase::Completed;
            return self;
        }

        self.pending_stop_reason =
            TurnStopReason::from_provider_finish_reason(report.finish_reason.as_deref());

        if report.tool_call_count > 0 {
            // The ceiling is checked before anything is recorded: calls that
            // will never run are not counted, and the turn ends with them
            // unrun rather than running one more batch. `total_tool_calls` is
            // therefore a count of executed calls, not requested ones.
            if self.current_iteration >= self.max_iterations {
                self.phase = TurnPhase::Completed;
                return self;
            }
            self.has_pending_tool_calls = true;
            self.total_tool_calls += report.tool_call_count;
            self.phase = TurnPhase::PendingAct;
        } else if report.has_pending_user_messages {
            // Steering: keep reasoning so the next iteration sees the new
            // messages — but the ceiling still applies, or a steady stream of
            // user messages loops forever.
            self.phase = if self.current_iteration >= self.max_iterations {
                TurnPhase::Completed
            } else {
                TurnPhase::PendingReason
            };
        } else {
            self.phase = TurnPhase::Completed;
        }
        self
    }

    /// The act step finished; loop back to reason.
    pub fn on_act_completed(mut self) -> Self {
        debug_assert_eq!(self.phase, TurnPhase::PendingAct);
        self.has_pending_tool_calls = false;
        self.phase = TurnPhase::PendingReason;
        self
    }

    /// Deliberately stop the turn to prevent waste.
    ///
    /// Idempotent, and the first reason wins: a turn sealed for no-progress
    /// that later trips the budget check is still a no-progress seal.
    pub fn seal(mut self, reason: SealReason) -> Self {
        if self.pending_seal.is_none() {
            self.pending_seal = Some(reason);
        }
        self.phase = TurnPhase::Completed;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::{TurnContext, TurnStateMachine};

    fn ids() -> TurnIds {
        TurnIds::from(&TurnContext::new(
            SessionId::new(),
            MessageId::new(),
            AgentId::new(),
            0,
        ))
    }

    fn state(max_iterations: usize) -> TurnState {
        TurnState::new(ids(), max_iterations)
    }

    /// One step in a turn, applied to either representation.
    #[derive(Clone, Debug)]
    enum Step {
        Input,
        Reason(ReasonReport),
        Act,
        Seal(SealReason),
    }

    /// Drive both representations through the same steps and assert they agree
    /// at every point. This is the whole claim of stage 1: the value can
    /// replace the mutable machine because it already behaves like it.
    fn assert_equivalent(max_iterations: usize, steps: Vec<Step>) {
        let context = TurnContext::new(SessionId::new(), MessageId::new(), AgentId::new(), 0);
        let mut machine = TurnStateMachine::new(context.clone(), max_iterations);
        let mut value = TurnState::new(TurnIds::from(&context), max_iterations);

        assert_actions_match(&machine, &value, "before any step");

        for (index, step) in steps.into_iter().enumerate() {
            match step {
                Step::Input => {
                    machine.on_input_completed();
                    value = value.on_input_completed();
                }
                Step::Reason(report) => {
                    machine.on_reason_completed(
                        report.response.clone(),
                        report.tool_call_count,
                        report.success,
                        report.error.clone(),
                        report.finish_reason.clone(),
                        report.has_pending_user_messages,
                    );
                    value = value.on_reason_completed(report);
                }
                Step::Act => {
                    machine.on_act_completed();
                    value = value.on_act_completed();
                }
                Step::Seal(reason) => {
                    machine.seal(reason);
                    value = value.seal(reason);
                }
            }
            assert_actions_match(&machine, &value, &format!("after step {index}"));
        }
    }

    fn assert_actions_match(machine: &TurnStateMachine, value: &TurnState, at: &str) {
        assert_eq!(machine.phase(), value.phase(), "phase diverged {at}");
        assert_eq!(
            machine.current_iteration(),
            value.current_iteration(),
            "iteration diverged {at}"
        );
        assert_eq!(
            machine.total_tool_calls(),
            value.total_tool_calls(),
            "tool-call count diverged {at}"
        );
        assert_eq!(
            format!("{:?}", machine.next_action()),
            format!("{:?}", value.next_action()),
            "next action diverged {at}"
        );
    }

    #[test]
    fn equivalent_on_a_plain_answer() {
        assert_equivalent(
            10,
            vec![Step::Input, Step::Reason(ReasonReport::text("Hi"))],
        );
    }

    #[test]
    fn equivalent_on_a_tool_call_round_trip() {
        assert_equivalent(
            10,
            vec![
                Step::Input,
                Step::Reason(ReasonReport::tool_calls("Checking…", 2)),
                Step::Act,
                Step::Reason(ReasonReport::text("Done.")),
            ],
        );
    }

    #[test]
    fn equivalent_when_the_iteration_ceiling_is_hit() {
        assert_equivalent(
            2,
            vec![
                Step::Input,
                Step::Reason(ReasonReport::tool_calls("one", 1)),
                Step::Act,
                Step::Reason(ReasonReport::tool_calls("two", 1)),
            ],
        );
    }

    #[test]
    fn equivalent_on_failure_and_refusal() {
        assert_equivalent(
            10,
            vec![Step::Input, Step::Reason(ReasonReport::failed("boom"))],
        );
        assert_equivalent(
            10,
            vec![
                Step::Input,
                Step::Reason(ReasonReport::failed("refused").with_finish_reason("content_filter")),
            ],
        );
    }

    #[test]
    fn equivalent_on_in_turn_steering() {
        assert_equivalent(
            10,
            vec![
                Step::Input,
                Step::Reason(ReasonReport::text("partial").with_pending_user_messages()),
                Step::Reason(ReasonReport::text("final")),
            ],
        );
    }

    #[test]
    fn equivalent_when_steering_hits_the_ceiling() {
        assert_equivalent(
            1,
            vec![
                Step::Input,
                Step::Reason(ReasonReport::text("partial").with_pending_user_messages()),
            ],
        );
    }

    #[test]
    fn equivalent_on_a_seal_mid_turn() {
        assert_equivalent(
            10,
            vec![
                Step::Input,
                Step::Reason(ReasonReport::tool_calls("working", 1)),
                Step::Seal(SealReason::Budget),
            ],
        );
    }

    #[test]
    fn a_seal_keeps_its_first_reason() {
        let sealed = state(10)
            .on_input_completed()
            .seal(SealReason::NoProgress)
            .seal(SealReason::Budget);

        match sealed.next_action() {
            TurnAction::Complete(TurnOutcome::Sealed { reason, .. }) => {
                assert_eq!(reason, SealReason::NoProgress);
            }
            other => panic!("expected a seal, got {other:?}"),
        }
    }

    #[test]
    fn next_action_does_not_advance_the_state() {
        let value = state(10).on_input_completed();
        let before = value.clone();
        let _ = value.next_action();
        let _ = value.next_action();
        assert_eq!(value, before);
    }

    #[test]
    fn state_survives_a_round_trip_through_storage() {
        // The property the durable host needs: persist mid-turn, resume, and
        // continue as if nothing happened.
        let mid_turn = state(10)
            .on_input_completed()
            .on_reason_completed(ReasonReport::tool_calls("checking", 3));

        let json = serde_json::to_string(&mid_turn).expect("serialize");
        let resumed: TurnState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(resumed, mid_turn);
        assert!(matches!(resumed.next_action(), TurnAction::ExecuteAct));

        let finished = resumed
            .on_act_completed()
            .on_reason_completed(ReasonReport::text("done"));
        match finished.next_action() {
            TurnAction::Complete(TurnOutcome::Success {
                response,
                iterations,
                tool_calls_count,
                ..
            }) => {
                assert_eq!(response, "done");
                assert_eq!(iterations, 2);
                assert_eq!(tool_calls_count, 3);
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn replaying_from_storage_at_every_step_matches_holding_it_in_memory() {
        // The durable path in miniature: throw the value away after each
        // transition and rebuild it from bytes. Any hidden state that does not
        // serialize shows up here as a divergence.
        let steps = vec![
            Step::Input,
            Step::Reason(ReasonReport::tool_calls("a", 1)),
            Step::Act,
            Step::Reason(ReasonReport::tool_calls("b", 2)),
            Step::Act,
            Step::Reason(ReasonReport::text("done")),
        ];

        let mut in_memory = state(10);
        let mut round_tripped = state(10);
        // Same ids on both sides so the comparison is about phase/bookkeeping.
        round_tripped.ids = in_memory.ids.clone();

        for step in steps {
            let apply = |value: TurnState, step: &Step| match step {
                Step::Input => value.on_input_completed(),
                Step::Reason(report) => value.on_reason_completed(report.clone()),
                Step::Act => value.on_act_completed(),
                Step::Seal(reason) => value.seal(*reason),
            };
            in_memory = apply(in_memory, &step);
            round_tripped = apply(round_tripped, &step);
            let bytes = serde_json::to_vec(&round_tripped).expect("serialize");
            round_tripped = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(round_tripped, in_memory);
        }

        assert!(matches!(
            in_memory.next_action(),
            TurnAction::Complete(TurnOutcome::Success { .. })
        ));
    }
}
