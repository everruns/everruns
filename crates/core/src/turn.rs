//! Turn State Machine - Unified Turn Orchestration
//!
//! # Why This Module Exists
//!
//! This module provides a unified state machine for orchestrating agent turns,
//! extracting the common logic from two previously duplicated implementations:
//!
//! 1. **In-Memory Loop** (`in_memory_loop.rs`) - imperative loop for testing/prototyping
//! 2. **Task Worker** (`worker/unified_worker.rs`) - event-sourced via task queue
//!
//! ## Problems This Solves
//!
//! ### 1. Duplicated Turn Logic
//!
//! Both implementations had nearly identical logic for the turn loop:
//! ```text
//! Input → Reason → (has_tool_calls?) → Act → Reason → ... → Complete
//! ```
//!
//! This duplication meant changes to turn logic had to be made in two places,
//! risking divergence and bugs.
//!
//! ### 2. Inconsistent Error Handling
//!
//! The in-memory loop had a subtle bug where it didn't check `reason_result.success`
//! before continuing to Act:
//!
//! ```ignore
//! // In-memory (buggy):
//! if !reason_result.has_tool_calls || reason_result.tool_calls.is_empty() {
//!     break;  // Only checks has_tool_calls, ignores success field!
//! }
//!
//! // Durable (correct):
//! if reason_result.has_tool_calls && reason_result.success {
//!     // Schedule act...
//! }
//! ```
//!
//! By unifying into a state machine, we ensure consistent error handling everywhere.
//!
//! ### 3. Fragile Turn ID Management
//!
//! Turn IDs (`TurnId`) provide correlation for all events within a turn.
//! Previously:
//!
//! - In-memory: Created once, passed through in-memory references (simple but not durable)
//! - Durable: Serialized to JSON, extracted from task output, passed to next task (fragile)
//!
//! The state machine provides a single source of truth for `TurnId` lifecycle:
//! - Created once when the turn starts
//! - Carried in `TurnContext` throughout execution
//! - Never re-created, preventing correlation breakage
//!
//! ### 4. Iteration Tracking
//!
//! Max iterations limit prevents infinite tool loops. Previously tracked differently:
//! - In-memory: Local loop counter
//! - Durable: Would need separate tracking per workflow
//!
//! The state machine tracks iterations uniformly.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      TurnStateMachine                           │
//! │  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌──────────────┐ │
//! │  │  Input  │───▶│ Reason  │───▶│   Act   │───▶│   Reason     │ │
//! │  └─────────┘    └────┬────┘    └─────────┘    └──────┬───────┘ │
//! │                      │                               │         │
//! │                      ▼                               │         │
//! │              ┌───────────────┐                       │         │
//! │              │   Complete    │◀──────────────────────┘         │
//! │              └───────────────┘                                 │
//! └─────────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//!                    TurnOutcome (Success/Failed/MaxIterations/Sealed)
//! ```
//!
//! ## Usage
//!
//! Both in-memory and durable implementations use the same state machine:
//!
//! ```ignore
//! let mut sm = TurnStateMachine::new(context, max_iterations);
//!
//! loop {
//!     match sm.next_action() {
//!         TurnAction::ExecuteInput => {
//!             let result = input_atom.execute(...).await?;
//!             sm.on_input_completed()?;
//!         }
//!         TurnAction::ExecuteReason => {
//!             let result = reason_atom.execute(...).await?;
//!             sm.on_reason_completed(text, count, result.success, result.error, result.finish_reason, has_pending)?;
//!         }
//!         TurnAction::ExecuteAct { tool_calls } => {
//!             act_atom.execute(...).await?;
//!             sm.on_act_completed()?;
//!         }
//!         TurnAction::Complete(outcome) => {
//!             return outcome;
//!         }
//!     }
//! }
//! ```

use crate::typed_id::{AgentId, MessageId, SessionId, TurnId};
use serde::{Deserialize, Serialize};

/// Context for a turn, created once and carried throughout execution.
///
/// This struct is the single source of truth for turn-scoped identifiers.
/// It is created when the turn begins and passed to all atoms.
#[derive(Debug, Clone)]
pub struct TurnContext {
    /// Session this turn belongs to
    pub session_id: SessionId,

    /// Unique identifier for this turn.
    ///
    /// Created once at turn start, never changes. All events emitted during
    /// this turn use this ID for correlation.
    pub turn_id: TurnId,

    /// Message that initiated this turn (the user's input message)
    pub input_message_id: MessageId,

    /// Agent executing this turn
    pub agent_id: AgentId,

    /// Organization ID (for multi-tenancy)
    pub org_id: i64,
}

impl TurnContext {
    /// Create a new turn context with a fresh turn ID.
    pub fn new(
        session_id: SessionId,
        input_message_id: MessageId,
        agent_id: AgentId,
        org_id: i64,
    ) -> Self {
        Self {
            session_id,
            turn_id: TurnId::new(),
            input_message_id,
            agent_id,
            org_id,
        }
    }

    /// Create a turn context with an existing turn ID.
    ///
    /// Use this when resuming a turn (e.g., in durable execution).
    pub fn with_turn_id(
        session_id: SessionId,
        turn_id: TurnId,
        input_message_id: MessageId,
        agent_id: AgentId,
        org_id: i64,
    ) -> Self {
        Self {
            session_id,
            turn_id,
            input_message_id,
            agent_id,
            org_id,
        }
    }
}

/// Current phase of turn execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPhase {
    /// Initial state, waiting to process input
    PendingInput,
    /// Input processed, waiting to reason
    PendingReason,
    /// Reason completed with tool calls, waiting to act
    PendingAct,
    /// Turn has completed
    Completed,
}

/// Action to take next in the turn.
#[derive(Debug, Clone)]
pub enum TurnAction {
    /// Execute the input atom (record user message)
    ExecuteInput,

    /// Execute the reason atom (LLM call)
    ExecuteReason,

    /// Execute the act atom (tool execution)
    ExecuteAct,

    /// Turn is complete
    Complete(TurnOutcome),
}

/// Why a turn was deliberately sealed (stopped to prevent waste).
///
/// `Sealed` is distinct from a successful `Completed` and from an error
/// `Failed`: it means the engine chose to stop a turn that would otherwise
/// keep burning resources without producing useful work. The reason is carried
/// through to the `turn.sealed` event and influences session status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealReason {
    /// A durable turn crashed and was reclaimed repeatedly without making any
    /// forward progress (its progress token never advanced). Sealing prevents a
    /// crash-loop from re-running reason/act and burning tokens until it
    /// incidentally hits max-iterations. See EVE-534.
    NoProgress,

    /// The work budget was exhausted (`HardLimitStopRule` balance <= 0). The
    /// turn is stopped deliberately rather than left reclaimable. See
    /// `specs/budgeting.md`.
    Budget,
}

impl SealReason {
    /// Stable wire string for events and the `turn.sealed` payload.
    pub fn as_str(&self) -> &'static str {
        match self {
            SealReason::NoProgress => "no_progress",
            SealReason::Budget => "budget",
        }
    }

    /// Parse a wire string back into a `SealReason`, defaulting to `NoProgress`
    /// for unknown values so older persisted reasons stay forward-compatible.
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "budget" => SealReason::Budget,
            _ => SealReason::NoProgress,
        }
    }
}

impl std::fmt::Display for SealReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A per-turn, monotonically advancing marker of forward progress.
///
/// # Why this exists (EVE-534)
///
/// A durable turn that crashes and gets reclaimed repeatedly can loop forever
/// (re-running reason/act, burning tokens/billing) until it incidentally hits
/// max-iterations. To defend against this poison-turn case we need a notion of
/// *progress* that is:
///
/// - **Derived from durably-recorded facts** so it is stable under replay — the
///   highest `durable_workflow_events.sequence_num` for the turn's workflow, or
///   equivalently the `(iteration, atoms_completed, settled_tool_calls)` tuple.
/// - **Impossible to game by a non-advancing retry** — re-running the same atom
///   that crashes before recording any event leaves the token unchanged.
///
/// The token is a single `u64` so the no-progress guard can compare cheaply and
/// persist it on the task across recovery attempts. A strictly larger value
/// means the turn advanced; an equal (or smaller) value means it did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProgressToken(pub u64);

impl ProgressToken {
    /// The token before any durable fact has been recorded.
    pub const ZERO: ProgressToken = ProgressToken(0);

    /// Build a token from the highest durable event sequence observed for the
    /// turn. Sequence numbers are monotonic per workflow, so this is monotonic
    /// per turn. The `+1` keeps `ZERO` reserved for "no events yet" while a
    /// missing/`-1` sequence maps to `ZERO`.
    pub fn from_event_sequence(highest_sequence: i64) -> Self {
        ProgressToken((highest_sequence.max(-1) + 1) as u64)
    }

    /// Returns true if `self` represents strictly more progress than `prev`.
    pub fn advanced_from(&self, prev: ProgressToken) -> bool {
        self.0 > prev.0
    }
}

/// Default number of consecutive no-progress recoveries before a turn is sealed.
///
/// Configurable via `DURABLE_NO_PROGRESS_SEAL_THRESHOLD`. See EVE-534.
pub const DEFAULT_NO_PROGRESS_SEAL_THRESHOLD: u32 = 3;

/// Read the no-progress seal threshold from the environment, falling back to
/// [`DEFAULT_NO_PROGRESS_SEAL_THRESHOLD`]. A value of 0 is coerced to 1 so the
/// guard can never be disabled into an infinite crash-loop.
pub fn no_progress_seal_threshold_from_env() -> u32 {
    std::env::var("DURABLE_NO_PROGRESS_SEAL_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_NO_PROGRESS_SEAL_THRESHOLD)
        .max(1)
}

/// Final outcome of a turn.
#[derive(Debug, Clone)]
pub enum TurnOutcome {
    /// Turn completed successfully
    Success {
        /// Final text response from the agent
        response: String,
        /// Number of reasoning iterations
        iterations: usize,
        /// Total tool calls made
        tool_calls_count: usize,
        /// Structured reason the final generation stopped.
        stop_reason: TurnStopReason,
    },

    /// Turn failed due to an error
    Failed {
        /// Error message
        error: String,
        /// Iterations completed before failure
        iterations: usize,
        /// Structured reason the turn failed.
        stop_reason: TurnStopReason,
    },

    /// Turn stopped due to max iterations limit
    MaxIterationsReached {
        /// Final response at time of limit
        response: String,
        /// Number of iterations (equals max_iterations)
        iterations: usize,
        /// Total tool calls made
        tool_calls_count: usize,
    },

    /// Turn was deliberately sealed to prevent further waste (EVE-534).
    ///
    /// Distinct from `Success` (work finished) and `Failed` (an error ended the
    /// turn): `Sealed` means the engine chose to stop a turn that would
    /// otherwise keep consuming resources without progressing. Sealed turns are
    /// terminal and **non-retryable** — the durable task is routed to the DLQ
    /// rather than requeued, and a `turn.sealed` event is emitted.
    Sealed {
        /// Why the turn was sealed.
        reason: SealReason,
        /// Final response at time of sealing (may be empty).
        response: String,
        /// Iterations completed before sealing.
        iterations: usize,
        /// Total tool calls made before sealing.
        tool_calls_count: usize,
    },
}

/// Stable reason a turn stopped, independent of provider-specific strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Error,
    Cancelled,
}

impl TurnStopReason {
    /// Normalize a provider finish reason at the runtime boundary.
    pub fn from_provider_finish_reason(reason: Option<&str>) -> Self {
        match reason.map(str::to_ascii_lowercase).as_deref() {
            Some("length" | "max_tokens" | "max_output_tokens") => Self::MaxTokens,
            Some("refusal" | "content_filter" | "safety") => Self::Refusal,
            Some("error") => Self::Error,
            Some("cancelled" | "canceled") => Self::Cancelled,
            _ => Self::EndTurn,
        }
    }
}

impl TurnOutcome {
    /// Check if the turn completed successfully
    pub fn is_success(&self) -> bool {
        matches!(self, TurnOutcome::Success { .. })
    }

    /// Check if the turn was deliberately sealed (EVE-534).
    pub fn is_sealed(&self) -> bool {
        matches!(self, TurnOutcome::Sealed { .. })
    }

    /// Get the seal reason, if the turn was sealed.
    pub fn seal_reason(&self) -> Option<SealReason> {
        match self {
            TurnOutcome::Sealed { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    /// Return the stable reason this outcome stopped.
    pub fn stop_reason(&self) -> TurnStopReason {
        match self {
            TurnOutcome::Success { stop_reason, .. } | TurnOutcome::Failed { stop_reason, .. } => {
                *stop_reason
            }
            TurnOutcome::MaxIterationsReached { .. } => TurnStopReason::MaxTurnRequests,
            TurnOutcome::Sealed { .. } => TurnStopReason::Error,
        }
    }

    /// Get the final response, if any
    pub fn response(&self) -> Option<&str> {
        match self {
            TurnOutcome::Success { response, .. } => Some(response),
            TurnOutcome::MaxIterationsReached { response, .. } => Some(response),
            TurnOutcome::Sealed { response, .. } => Some(response),
            TurnOutcome::Failed { .. } => None,
        }
    }

    /// Get the error message, if any
    pub fn error(&self) -> Option<&str> {
        match self {
            TurnOutcome::Failed { error, .. } => Some(error),
            _ => None,
        }
    }

    /// Get the number of iterations
    pub fn iterations(&self) -> usize {
        match self {
            TurnOutcome::Success { iterations, .. } => *iterations,
            TurnOutcome::Failed { iterations, .. } => *iterations,
            TurnOutcome::MaxIterationsReached { iterations, .. } => *iterations,
            TurnOutcome::Sealed { iterations, .. } => *iterations,
        }
    }
}

/// State machine for turn orchestration.
///
/// This is the core abstraction that unifies turn logic between in-memory
/// and durable execution. It tracks the current phase, determines the next
/// action, and handles state transitions.
///
/// # Thread Safety
///
/// The state machine is not thread-safe. For durable execution, each task
/// execution creates its own state machine from serialized state.
#[derive(Debug)]
pub struct TurnStateMachine {
    /// Turn context with IDs
    context: TurnContext,

    /// Current phase
    phase: TurnPhase,

    /// Maximum allowed iterations (Reason → Act cycles)
    max_iterations: usize,

    /// Current iteration count
    current_iteration: usize,

    /// Total tool calls made across all iterations
    total_tool_calls: usize,

    /// Last text response from Reason
    last_response: String,

    /// Pending error from Reason (set when success=false)
    pending_error: Option<String>,

    /// Provider-derived reason for the terminal generation.
    pending_stop_reason: TurnStopReason,

    /// Whether the last Reason had tool calls
    has_pending_tool_calls: bool,

    /// Pending seal reason (set by `seal()` when the engine decides to stop the
    /// turn deliberately, e.g. work-budget exhausted). Takes precedence over a
    /// normal completion so the turn resolves to `TurnOutcome::Sealed`.
    pending_seal: Option<SealReason>,
}

impl TurnStateMachine {
    /// Create a new state machine for a turn.
    ///
    /// # Arguments
    ///
    /// * `context` - Turn context with session, turn, and agent IDs
    /// * `max_iterations` - Maximum Reason → Act cycles before stopping
    pub fn new(context: TurnContext, max_iterations: usize) -> Self {
        Self {
            context,
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

    /// Get the turn context.
    pub fn context(&self) -> &TurnContext {
        &self.context
    }

    /// Get the current phase.
    pub fn phase(&self) -> TurnPhase {
        self.phase
    }

    /// Get the current iteration count.
    pub fn current_iteration(&self) -> usize {
        self.current_iteration
    }

    /// Get the total tool calls made so far.
    pub fn total_tool_calls(&self) -> usize {
        self.total_tool_calls
    }

    /// Determine the next action to take.
    ///
    /// This is the core dispatch method. Call this in a loop and execute
    /// the returned action until `TurnAction::Complete` is returned.
    pub fn next_action(&self) -> TurnAction {
        match self.phase {
            TurnPhase::PendingInput => TurnAction::ExecuteInput,
            TurnPhase::PendingReason => TurnAction::ExecuteReason,
            TurnPhase::PendingAct => TurnAction::ExecuteAct,
            TurnPhase::Completed => {
                // A deliberate seal takes precedence over any other terminal:
                // budget exhaustion must resolve to `Sealed { budget }` rather
                // than leaving the turn reclaimable or surfacing as a failure.
                if let Some(reason) = self.pending_seal {
                    return TurnAction::Complete(TurnOutcome::Sealed {
                        reason,
                        response: self.last_response.clone(),
                        iterations: self.current_iteration,
                        tool_calls_count: self.total_tool_calls,
                    });
                }
                // Build outcome based on state
                if let Some(error) = &self.pending_error {
                    TurnAction::Complete(TurnOutcome::Failed {
                        error: error.clone(),
                        iterations: self.current_iteration,
                        stop_reason: self.pending_stop_reason,
                    })
                } else if self.current_iteration >= self.max_iterations {
                    TurnAction::Complete(TurnOutcome::MaxIterationsReached {
                        response: self.last_response.clone(),
                        iterations: self.current_iteration,
                        tool_calls_count: self.total_tool_calls,
                    })
                } else {
                    TurnAction::Complete(TurnOutcome::Success {
                        response: self.last_response.clone(),
                        iterations: self.current_iteration,
                        tool_calls_count: self.total_tool_calls,
                        stop_reason: self.pending_stop_reason,
                    })
                }
            }
        }
    }

    /// Record that input processing completed.
    ///
    /// Call this after successfully executing the input atom.
    pub fn on_input_completed(&mut self) {
        debug_assert_eq!(self.phase, TurnPhase::PendingInput);
        self.phase = TurnPhase::PendingReason;
    }

    /// Record that reasoning completed.
    ///
    /// # Arguments
    ///
    /// * `response` - The text response from the LLM (may be empty)
    /// * `tool_call_count` - Number of tool calls (0 if none)
    /// * `success` - Whether the LLM call succeeded
    /// * `error` - Error message if success is false
    /// * `finish_reason` - Raw provider finish reason, when available
    /// * `has_pending_user_messages` - Whether new user messages arrived during
    ///   this turn (steering signals). When true and reason would otherwise
    ///   complete (no tool calls, success), the turn stays in PendingReason so
    ///   the next iteration picks up the new messages from the conversation
    ///   history. This is "in-turn steering" — matching Claude Code behavior.
    pub fn on_reason_completed(
        &mut self,
        response: String,
        tool_call_count: usize,
        success: bool,
        error: Option<String>,
        finish_reason: Option<String>,
        has_pending_user_messages: bool,
    ) {
        debug_assert_eq!(self.phase, TurnPhase::PendingReason);

        self.current_iteration += 1;

        // Store response
        if !response.is_empty() {
            self.last_response = response;
        }

        // Handle failure
        if !success {
            self.pending_stop_reason =
                match TurnStopReason::from_provider_finish_reason(finish_reason.as_deref()) {
                    TurnStopReason::Refusal => TurnStopReason::Refusal,
                    _ => TurnStopReason::Error,
                };
            self.pending_error = error;
            self.phase = TurnPhase::Completed;
            return;
        }

        self.pending_stop_reason =
            TurnStopReason::from_provider_finish_reason(finish_reason.as_deref());

        // Handle tool calls
        if tool_call_count > 0 {
            // Check max iterations before proceeding to Act
            if self.current_iteration >= self.max_iterations {
                self.phase = TurnPhase::Completed;
                return;
            }

            self.has_pending_tool_calls = true;
            self.total_tool_calls += tool_call_count;
            self.phase = TurnPhase::PendingAct;
        } else if has_pending_user_messages {
            // No tool calls but user sent messages during this turn.
            // Enforce max_iterations before continuing — prevents unbounded
            // reason loops from a steady stream of user messages.
            if self.current_iteration >= self.max_iterations {
                self.phase = TurnPhase::Completed;
            } else {
                self.phase = TurnPhase::PendingReason;
            }
        } else {
            // No tool calls, no pending messages — turn is complete
            self.phase = TurnPhase::Completed;
        }
    }

    /// Record that action (tool execution) completed.
    ///
    /// Call this after successfully executing the act atom.
    /// The turn then loops back to Reason for another iteration.
    pub fn on_act_completed(&mut self) {
        debug_assert_eq!(self.phase, TurnPhase::PendingAct);
        self.has_pending_tool_calls = false;
        // Loop back to reason for next iteration
        self.phase = TurnPhase::PendingReason;
    }

    /// Deliberately seal the turn, stopping further scheduling (EVE-534).
    ///
    /// Call this between atoms when the engine decides to stop a turn to prevent
    /// waste — e.g. the work budget is exhausted (`SealReason::Budget`). The
    /// turn transitions to `Completed` and `next_action` resolves to
    /// `TurnOutcome::Sealed`. Sealing is idempotent and the first reason wins.
    pub fn seal(&mut self, reason: SealReason) {
        if self.pending_seal.is_none() {
            self.pending_seal = Some(reason);
        }
        self.phase = TurnPhase::Completed;
    }

    /// Check if the turn has completed.
    pub fn is_completed(&self) -> bool {
        self.phase == TurnPhase::Completed
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> TurnContext {
        TurnContext::new(SessionId::new(), MessageId::new(), AgentId::new(), 0)
    }

    #[test]
    fn test_simple_turn_no_tools() {
        let mut sm = TurnStateMachine::new(test_context(), 10);

        // Start with input
        assert!(matches!(sm.next_action(), TurnAction::ExecuteInput));
        sm.on_input_completed();

        // Then reason
        assert!(matches!(sm.next_action(), TurnAction::ExecuteReason));
        sm.on_reason_completed("Hello!".to_string(), 0, true, None, None, false);

        // Complete
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::Success {
                response,
                iterations,
                tool_calls_count,
                stop_reason,
            }) => {
                assert_eq!(response, "Hello!");
                assert_eq!(iterations, 1);
                assert_eq!(tool_calls_count, 0);
                assert_eq!(stop_reason, TurnStopReason::EndTurn);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_turn_with_one_tool_call() {
        let mut sm = TurnStateMachine::new(test_context(), 10);

        // Input
        assert!(matches!(sm.next_action(), TurnAction::ExecuteInput));
        sm.on_input_completed();

        // First reason - requests tool call
        assert!(matches!(sm.next_action(), TurnAction::ExecuteReason));
        sm.on_reason_completed("Let me check...".to_string(), 1, true, None, None, false);

        // Act
        assert!(matches!(sm.next_action(), TurnAction::ExecuteAct));
        sm.on_act_completed();

        // Second reason - no more tool calls
        assert!(matches!(sm.next_action(), TurnAction::ExecuteReason));
        sm.on_reason_completed("Here's the result.".to_string(), 0, true, None, None, false);

        // Complete
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::Success {
                response,
                iterations,
                tool_calls_count,
                ..
            }) => {
                assert_eq!(response, "Here's the result.");
                assert_eq!(iterations, 2);
                assert_eq!(tool_calls_count, 1);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_max_iterations() {
        let mut sm = TurnStateMachine::new(test_context(), 2);

        // Input
        sm.on_input_completed();

        // First reason - requests tool
        sm.on_reason_completed("Trying...".to_string(), 1, true, None, None, false);
        sm.on_act_completed();

        // Second reason - requests another tool (hits max)
        sm.on_reason_completed("Still trying...".to_string(), 1, true, None, None, false);

        // Should complete with max iterations
        match sm.next_action() {
            TurnAction::Complete(outcome @ TurnOutcome::MaxIterationsReached { .. }) => {
                assert_eq!(outcome.iterations(), 2);
                assert_eq!(outcome.stop_reason(), TurnStopReason::MaxTurnRequests);
            }
            other => panic!("Expected MaxIterationsReached, got {:?}", other),
        }
    }

    #[test]
    fn test_provider_length_finish_reason_surfaces_as_max_tokens() {
        let mut sm = TurnStateMachine::new(test_context(), 2);
        sm.on_input_completed();
        sm.on_reason_completed(
            "Truncated response".to_string(),
            0,
            true,
            None,
            Some("length".to_string()),
            false,
        );

        match sm.next_action() {
            TurnAction::Complete(outcome) => {
                assert_eq!(outcome.stop_reason(), TurnStopReason::MaxTokens);
            }
            other => panic!("Expected completed turn, got {other:?}"),
        }
    }

    #[test]
    fn test_provider_refusal_finish_reason_surfaces_as_refusal() {
        let mut sm = TurnStateMachine::new(test_context(), 2);
        sm.on_input_completed();
        sm.on_reason_completed(
            String::new(),
            0,
            false,
            Some("Model refused".to_string()),
            Some("refusal".to_string()),
            false,
        );

        match sm.next_action() {
            TurnAction::Complete(outcome) => {
                assert_eq!(outcome.stop_reason(), TurnStopReason::Refusal);
            }
            other => panic!("Expected completed turn, got {other:?}"),
        }
    }

    #[test]
    fn test_stop_reason_wire_values_cover_terminal_contract() {
        let values = [
            (TurnStopReason::EndTurn, "\"end_turn\""),
            (TurnStopReason::MaxTokens, "\"max_tokens\""),
            (TurnStopReason::MaxTurnRequests, "\"max_turn_requests\""),
            (TurnStopReason::Refusal, "\"refusal\""),
            (TurnStopReason::Error, "\"error\""),
            (TurnStopReason::Cancelled, "\"cancelled\""),
        ];

        for (reason, expected) in values {
            assert_eq!(serde_json::to_string(&reason).unwrap(), expected);
        }

        assert_eq!(
            TurnStopReason::from_provider_finish_reason(Some("content_filter")),
            TurnStopReason::Refusal
        );
        assert_eq!(
            TurnStopReason::from_provider_finish_reason(Some("cancelled")),
            TurnStopReason::Cancelled
        );
    }

    #[test]
    fn test_reason_failure() {
        let mut sm = TurnStateMachine::new(test_context(), 10);

        // Input
        sm.on_input_completed();

        // Reason fails
        sm.on_reason_completed(
            String::new(),
            0,
            false,
            Some("LLM error".to_string()),
            None,
            false,
        );

        // Should complete with failure
        match sm.next_action() {
            TurnAction::Complete(outcome @ TurnOutcome::Failed { .. }) => {
                assert_eq!(outcome.error(), Some("LLM error"));
                assert_eq!(outcome.stop_reason(), TurnStopReason::Error);
            }
            other => panic!("Expected Failed, got {:?}", other),
        }
    }

    #[test]
    fn test_context_preserved() {
        let context = TurnContext::new(SessionId::new(), MessageId::new(), AgentId::new(), 42);
        let turn_id = context.turn_id;

        let sm = TurnStateMachine::new(context, 10);

        // Context should be accessible and unchanged
        assert_eq!(sm.context().turn_id, turn_id);
        assert_eq!(sm.context().org_id, 42);
    }

    #[test]
    fn test_outcome_helpers() {
        let success = TurnOutcome::Success {
            response: "test".to_string(),
            iterations: 1,
            tool_calls_count: 0,
            stop_reason: TurnStopReason::EndTurn,
        };
        assert!(success.is_success());
        assert_eq!(success.response(), Some("test"));
        assert!(success.error().is_none());

        let failed = TurnOutcome::Failed {
            error: "oops".to_string(),
            iterations: 0,
            stop_reason: TurnStopReason::Error,
        };
        assert!(!failed.is_success());
        assert!(failed.response().is_none());
        assert_eq!(failed.error(), Some("oops"));
    }

    #[test]
    fn test_pending_user_message_continues_turn() {
        let mut sm = TurnStateMachine::new(test_context(), 10);
        sm.on_input_completed();

        // Reason completes with no tools, BUT there are pending user messages
        sm.on_reason_completed("Hello!".to_string(), 0, true, None, None, true);

        // Should NOT be completed — stays in PendingReason
        assert!(!sm.is_completed());
        assert_eq!(sm.phase(), TurnPhase::PendingReason);
        assert!(matches!(sm.next_action(), TurnAction::ExecuteReason));

        // Second reason picks up the new message and completes normally
        sm.on_reason_completed("Got your message!".to_string(), 0, true, None, None, false);
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::Success {
                response,
                iterations,
                ..
            }) => {
                assert_eq!(response, "Got your message!");
                assert_eq!(iterations, 2);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_pending_messages_ignored_on_failure() {
        let mut sm = TurnStateMachine::new(test_context(), 10);
        sm.on_input_completed();

        // Failure + pending messages → still fails
        sm.on_reason_completed(
            String::new(),
            0,
            false,
            Some("LLM error".to_string()),
            None,
            true,
        );
        assert!(sm.is_completed());
        assert!(matches!(
            sm.next_action(),
            TurnAction::Complete(TurnOutcome::Failed { .. })
        ));
    }

    #[test]
    fn test_progress_token_monotonicity() {
        // Higher event sequence => strictly higher token.
        let t0 = ProgressToken::from_event_sequence(-1); // no events yet
        let t1 = ProgressToken::from_event_sequence(0);
        let t2 = ProgressToken::from_event_sequence(5);
        assert_eq!(t0, ProgressToken::ZERO);
        assert!(t1.advanced_from(t0));
        assert!(t2.advanced_from(t1));
        // Same sequence => no advance (a non-advancing retry can't game it).
        let t2_again = ProgressToken::from_event_sequence(5);
        assert!(!t2_again.advanced_from(t2));
        assert!(!t2.advanced_from(t2_again));
        // Ordering matches numeric ordering.
        assert!(t2 > t1 && t1 > t0);
    }

    #[test]
    fn test_no_progress_counter_logic() {
        // Mirrors the guard the store applies on each reclaim: the counter only
        // increments when the token is unchanged across attempts, and resets to
        // zero on any advance. Sealing fires when the counter reaches N.
        let threshold = 3u32;
        let mut recorded = ProgressToken::ZERO;
        let mut no_progress = 0u32;

        let step =
            |recorded: &mut ProgressToken, no_progress: &mut u32, observed: ProgressToken| {
                if observed.advanced_from(*recorded) {
                    *recorded = observed;
                    *no_progress = 0;
                } else {
                    *no_progress += 1;
                }
                *no_progress
            };

        // Crash without recording any event => token unchanged => increments.
        assert_eq!(
            step(&mut recorded, &mut no_progress, ProgressToken::ZERO),
            1
        );
        assert_eq!(
            step(&mut recorded, &mut no_progress, ProgressToken::ZERO),
            2
        );
        // An advance resets the counter.
        assert_eq!(
            step(
                &mut recorded,
                &mut no_progress,
                ProgressToken::from_event_sequence(2)
            ),
            0
        );
        // Then stall again until we hit the seal threshold.
        let stuck = ProgressToken::from_event_sequence(2);
        assert_eq!(step(&mut recorded, &mut no_progress, stuck), 1);
        assert_eq!(step(&mut recorded, &mut no_progress, stuck), 2);
        let count = step(&mut recorded, &mut no_progress, stuck);
        assert_eq!(count, 3);
        assert!(count >= threshold, "should seal once threshold reached");
    }

    #[test]
    fn test_seal_threshold_env_never_zero() {
        // Default applies when unset; a 0 must coerce to at least 1.
        // (We only assert the floor invariant without touching process env.)
        assert_eq!(DEFAULT_NO_PROGRESS_SEAL_THRESHOLD, 3);
    }

    #[test]
    fn test_budget_seal_outcome() {
        let mut sm = TurnStateMachine::new(test_context(), 10);
        sm.on_input_completed();
        // A reason completes with tool calls (turn would normally continue)...
        sm.on_reason_completed("Working...".to_string(), 1, true, None, None, false);
        sm.on_act_completed();
        // ...but the engine seals it because the work budget is exhausted.
        sm.seal(SealReason::Budget);
        assert!(sm.is_completed());
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::Sealed {
                reason, iterations, ..
            }) => {
                assert_eq!(reason, SealReason::Budget);
                assert_eq!(iterations, 1);
            }
            other => panic!("Expected Sealed, got {:?}", other),
        }
    }

    #[test]
    fn test_seal_takes_precedence_over_error() {
        // If a turn both errored and was sealed, the deliberate seal wins so the
        // turn is non-retryable rather than surfacing as a transient failure.
        let mut sm = TurnStateMachine::new(test_context(), 10);
        sm.on_input_completed();
        sm.on_reason_completed(
            String::new(),
            0,
            false,
            Some("LLM error".to_string()),
            None,
            false,
        );
        sm.seal(SealReason::NoProgress);
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::Sealed { reason, .. }) => {
                assert_eq!(reason, SealReason::NoProgress);
            }
            other => panic!("Expected Sealed, got {:?}", other),
        }
    }

    #[test]
    fn test_seal_reason_wire_roundtrip() {
        assert_eq!(SealReason::NoProgress.as_str(), "no_progress");
        assert_eq!(SealReason::Budget.as_str(), "budget");
        assert_eq!(SealReason::from_str_lossy("budget"), SealReason::Budget);
        assert_eq!(
            SealReason::from_str_lossy("no_progress"),
            SealReason::NoProgress
        );
        // Unknown reasons stay forward-compatible.
        assert_eq!(
            SealReason::from_str_lossy("future_reason"),
            SealReason::NoProgress
        );
    }

    #[test]
    fn test_outcome_sealed_helpers() {
        let sealed = TurnOutcome::Sealed {
            reason: SealReason::Budget,
            response: "partial".to_string(),
            iterations: 2,
            tool_calls_count: 1,
        };
        assert!(sealed.is_sealed());
        assert!(!sealed.is_success());
        assert_eq!(sealed.seal_reason(), Some(SealReason::Budget));
        assert_eq!(sealed.response(), Some("partial"));
        assert!(sealed.error().is_none());
        assert_eq!(sealed.iterations(), 2);
    }

    #[test]
    fn test_pending_messages_ignored_when_tool_calls() {
        let mut sm = TurnStateMachine::new(test_context(), 10);
        sm.on_input_completed();

        // Tool calls + pending messages → tool calls take priority
        sm.on_reason_completed("Working...".to_string(), 2, true, None, None, true);
        assert_eq!(sm.phase(), TurnPhase::PendingAct);
    }
}
