//! Durable implementation of the shared engine execution contract.

use chrono::{DateTime, Utc};
use everruns_engine::{
    ActivityOutcome, Execution, ExecutionTransition, HostFacts, TurnExecution, TurnState,
};

/// Execution whose common engine state is checkpointed between activities.
///
/// `everruns-durable` owns persistence and retry mechanics; semantic phase and
/// turn behavior remains in `everruns-engine` through [`TurnExecution`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DurableExecution {
    inner: TurnExecution,
}

impl DurableExecution {
    /// Restore a durable execution from its persisted turn state.
    pub fn new(state: TurnState) -> Self {
        Self {
            inner: TurnExecution::new(state),
        }
    }

    /// Return the engine state to persist with the next activity.
    pub fn checkpoint(&self) -> TurnState {
        self.inner.state().clone()
    }

    /// Consume this driver and return its engine state.
    pub fn into_state(self) -> TurnState {
        self.inner.into_state()
    }
}

impl Execution for DurableExecution {
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

#[cfg(test)]
mod tests {
    use everruns_engine::Execution;
    use everruns_provider::typed_id::{HarnessId, MessageId, SessionId};

    use super::*;

    #[test]
    fn durable_checkpoint_round_trips_without_host_state() {
        let state = TurnState {
            org_id: 7,
            session_id: SessionId::new(),
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id: MessageId::new(),
            turn_id: None,
            previous_response_id: None,
            iteration: 1,
            request_id: None,
            started_at: None,
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        let execution = DurableExecution::new(state);
        let encoded = serde_json::to_vec(&execution).expect("serialize durable execution");
        let restored: DurableExecution =
            serde_json::from_slice(&encoded).expect("restore durable execution");

        assert_eq!(restored.state().org_id, 7);
        assert_eq!(
            restored.checkpoint().session_id,
            execution.state().session_id
        );
    }
}
