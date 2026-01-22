//! Turn State Machine - Unified Turn Orchestration
//!
//! # Why This Module Exists
//!
//! This module provides a unified state machine for orchestrating agent turns,
//! extracting the common logic from two previously duplicated implementations:
//!
//! 1. **In-Memory Loop** (`in_memory_loop.rs`) - imperative loop for testing/prototyping
//! 2. **Durable Worker** (`worker/durable_worker.rs`) - event-sourced via task queue
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
//!                    TurnOutcome (Success/Failed/MaxIterations)
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
//!             sm.on_reason_completed(result.has_tool_calls, result.success, result.error)?;
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
    },

    /// Turn failed due to an error
    Failed {
        /// Error message
        error: String,
        /// Iterations completed before failure
        iterations: usize,
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
}

impl TurnOutcome {
    /// Check if the turn completed successfully
    pub fn is_success(&self) -> bool {
        matches!(self, TurnOutcome::Success { .. })
    }

    /// Get the final response, if any
    pub fn response(&self) -> Option<&str> {
        match self {
            TurnOutcome::Success { response, .. } => Some(response),
            TurnOutcome::MaxIterationsReached { response, .. } => Some(response),
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

    /// Whether the last Reason had tool calls
    has_pending_tool_calls: bool,
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
            has_pending_tool_calls: false,
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
                // Build outcome based on state
                if let Some(error) = &self.pending_error {
                    TurnAction::Complete(TurnOutcome::Failed {
                        error: error.clone(),
                        iterations: self.current_iteration,
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
    /// * `has_tool_calls` - Whether the LLM requested tool calls
    /// * `tool_call_count` - Number of tool calls (0 if none)
    /// * `success` - Whether the LLM call succeeded
    /// * `error` - Error message if success is false
    pub fn on_reason_completed(
        &mut self,
        response: String,
        has_tool_calls: bool,
        tool_call_count: usize,
        success: bool,
        error: Option<String>,
    ) {
        debug_assert_eq!(self.phase, TurnPhase::PendingReason);

        self.current_iteration += 1;

        // Store response
        if !response.is_empty() {
            self.last_response = response;
        }

        // Handle failure
        if !success {
            self.pending_error = error;
            self.phase = TurnPhase::Completed;
            return;
        }

        // Handle tool calls
        if has_tool_calls && tool_call_count > 0 {
            // Check max iterations before proceeding to Act
            if self.current_iteration >= self.max_iterations {
                self.phase = TurnPhase::Completed;
                return;
            }

            self.has_pending_tool_calls = true;
            self.total_tool_calls += tool_call_count;
            self.phase = TurnPhase::PendingAct;
        } else {
            // No tool calls, turn is complete
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
        sm.on_reason_completed("Hello!".to_string(), false, 0, true, None);

        // Complete
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::Success {
                response,
                iterations,
                tool_calls_count,
            }) => {
                assert_eq!(response, "Hello!");
                assert_eq!(iterations, 1);
                assert_eq!(tool_calls_count, 0);
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
        sm.on_reason_completed("Let me check...".to_string(), true, 1, true, None);

        // Act
        assert!(matches!(sm.next_action(), TurnAction::ExecuteAct));
        sm.on_act_completed();

        // Second reason - no more tool calls
        assert!(matches!(sm.next_action(), TurnAction::ExecuteReason));
        sm.on_reason_completed("Here's the result.".to_string(), false, 0, true, None);

        // Complete
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::Success {
                response,
                iterations,
                tool_calls_count,
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
        sm.on_reason_completed("Trying...".to_string(), true, 1, true, None);
        sm.on_act_completed();

        // Second reason - requests another tool (hits max)
        sm.on_reason_completed("Still trying...".to_string(), true, 1, true, None);

        // Should complete with max iterations
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::MaxIterationsReached { iterations, .. }) => {
                assert_eq!(iterations, 2);
            }
            other => panic!("Expected MaxIterationsReached, got {:?}", other),
        }
    }

    #[test]
    fn test_reason_failure() {
        let mut sm = TurnStateMachine::new(test_context(), 10);

        // Input
        sm.on_input_completed();

        // Reason fails
        sm.on_reason_completed(
            String::new(),
            false,
            0,
            false,
            Some("LLM error".to_string()),
        );

        // Should complete with failure
        match sm.next_action() {
            TurnAction::Complete(TurnOutcome::Failed { error, .. }) => {
                assert_eq!(error, "LLM error");
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
        };
        assert!(success.is_success());
        assert_eq!(success.response(), Some("test"));
        assert!(success.error().is_none());

        let failed = TurnOutcome::Failed {
            error: "oops".to_string(),
            iterations: 0,
        };
        assert!(!failed.is_success());
        assert!(failed.response().is_none());
        assert_eq!(failed.error(), Some("oops"));
    }
}
