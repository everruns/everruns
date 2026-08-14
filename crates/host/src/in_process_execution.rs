//! Immediate in-process implementation of the engine execution contract.

use chrono::{DateTime, Utc};
use everruns_engine::{
    ActivityOutcome, Execution, ExecutionTransition, HostFacts, TurnExecution, TurnState,
};

/// Turn execution whose state lives in the current process.
///
/// The host performs each planned phase immediately. All semantic state
/// advancement remains delegated to [`TurnExecution`] in `everruns-engine`.
#[derive(Debug, Clone)]
pub struct InProcessExecution {
    inner: TurnExecution,
}

impl InProcessExecution {
    /// Start an in-process execution from the common engine state.
    pub fn new(state: TurnState) -> Self {
        Self {
            inner: TurnExecution::new(state),
        }
    }

    /// Consume this driver and return the common checkpoint value.
    pub fn into_state(self) -> TurnState {
        self.inner.into_state()
    }
}

impl Execution for InProcessExecution {
    fn state(&self) -> &TurnState {
        self.inner.state()
    }

    fn advance(
        &mut self,
        outcome: ActivityOutcome,
        pending_user_message_count: usize,
        now: DateTime<Utc>,
        facts: HostFacts,
    ) -> ExecutionTransition {
        self.inner
            .advance(outcome, pending_user_message_count, now, facts)
    }
}
