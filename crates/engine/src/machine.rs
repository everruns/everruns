//! Stateful execution over the shared turn planner.
//!
//! [`TurnExecution`] is the abstract execution kernel. Hosts decide how an
//! execution is driven and stored, while this type owns the current
//! [`TurnState`] and the rules for advancing it after every phase.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ActivityOutcome, HostFacts, TurnLifecycleEffect, TurnPlan, TurnState, plan_next_turn};

/// One engine decision and the ordered lifecycle effects it requires.
#[derive(Debug, Clone)]
pub struct ExecutionTransition {
    /// The next semantic step for the execution driver.
    pub plan: TurnPlan,
    /// Effects the driver must apply in order before scheduling the plan.
    pub effects: Vec<TurnLifecycleEffect>,
}

/// Common contract implemented by immediate and durable turn executions.
///
/// The contract is deliberately synchronous and sans I/O. An implementation
/// may keep the state in process, checkpoint it between durable activities, or
/// project it into another scheduler. Stores, queues, clocks, and effect
/// application remain execution-driver concerns.
pub trait Execution {
    /// Current engine-owned state.
    fn state(&self) -> &TurnState;

    /// Advance after one completed Input/Reason/Act phase.
    fn advance(
        &mut self,
        outcome: ActivityOutcome,
        pending_user_message_count: usize,
        now: DateTime<Utc>,
        facts: HostFacts,
    ) -> ExecutionTransition;
}

/// Default stateful implementation of the shared turn execution contract.
///
/// Immediate hosts keep this value in memory for the turn lifetime. Durable
/// hosts serialize its state between activities and restore it before the next
/// transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnExecution {
    state: TurnState,
}

impl TurnExecution {
    /// Start or restore an execution from its engine state.
    pub fn new(state: TurnState) -> Self {
        Self { state }
    }

    /// Consume the execution and return its checkpoint value.
    pub fn into_state(self) -> TurnState {
        self.state
    }

    fn apply_plan(&mut self, plan: &TurnPlan) {
        match plan {
            TurnPlan::ScheduleReason(next) => self.state = next.clone(),
            TurnPlan::ScheduleAct(plan) => {
                let mut next = (*plan.resume_state).clone();
                next.previous_response_id = plan.previous_response_id.clone();
                next.iteration = plan.iteration;
                next.request_id = plan.request_id.clone();
                self.state = next;
            }
            TurnPlan::WaitForToolResults { resume } => self.state = resume.clone(),
            TurnPlan::Complete { .. } => {}
        }
    }
}

impl Execution for TurnExecution {
    fn state(&self) -> &TurnState {
        &self.state
    }

    fn advance(
        &mut self,
        outcome: ActivityOutcome,
        pending_user_message_count: usize,
        now: DateTime<Utc>,
        facts: HostFacts,
    ) -> ExecutionTransition {
        // A terminal reason plan carries no resume state because the driver has
        // nothing left to schedule. Preserve the reason summary in the owned
        // execution nevertheless, so in-memory inspection and durable terminal
        // checkpoints observe the same final counters and output metadata.
        let terminal_reason_state = match &outcome {
            ActivityOutcome::Reason(reason) => Some(self.state.with_reason_summary(reason)),
            _ => None,
        };
        let (plan, effects) =
            plan_next_turn(&self.state, outcome, pending_user_message_count, now, facts);
        self.apply_plan(&plan);
        if matches!(plan, TurnPlan::Complete { .. })
            && let Some(terminal_reason_state) = terminal_reason_state
        {
            self.state = terminal_reason_state;
        }
        ExecutionTransition { plan, effects }
    }
}

#[cfg(test)]
mod tests {
    use everruns_provider::typed_id::{HarnessId, MessageId, SessionId, TurnId};

    use super::*;

    fn state() -> TurnState {
        TurnState {
            org_id: 1,
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
        }
    }

    #[test]
    fn process_input_advances_the_owned_state() {
        let turn_id = TurnId::new();
        let mut execution = TurnExecution::new(state());

        let transition = execution.advance(
            ActivityOutcome::ProcessInput {
                turn_id: Some(turn_id),
            },
            0,
            Utc::now(),
            HostFacts::default(),
        );

        assert!(matches!(transition.plan, TurnPlan::ScheduleReason(_)));
        assert_eq!(execution.state().turn_id, Some(turn_id));
    }

    #[test]
    fn execution_checkpoint_round_trips() {
        let execution = TurnExecution::new(state());
        let bytes = serde_json::to_vec(&execution).expect("serialize execution");
        let restored: TurnExecution =
            serde_json::from_slice(&bytes).expect("restore execution checkpoint");

        assert_eq!(restored.state().session_id, execution.state().session_id);
        assert_eq!(
            restored.state().input_message_id,
            execution.state().input_message_id
        );
    }
}
