//! The worker's side of the durable turn driver.
//!
//! Decision: the turn driver lives in durable-engine (`TurnTaskDriver`) and
//! is generic over the host it runs steps on. The worker supplies
//! `WorkerRuntimeHost` over its adapters, built fresh per step so each step
//! starts its own setup reads (`phase_reads`) and flushes its own write-behind
//! events, and runs the activities that are not turn steps: leased-resource
//! cleanup, the session task reaper and scheduled invocations. The setup
//! reads of a turn's phases share what earlier phases of the same turn read
//! on this worker (`turn_reads`), until the turn ends.

use crate::durable::ClaimedTask;
use crate::durable_runner::DurableTurnInput;
use crate::engine::{ActInput, ReasonInput, TurnPlan};
use crate::phase_reads::PhaseIds;
use crate::runtime_host::WorkerRuntimeHost;
use crate::task_heartbeat::CancelSignals;
use crate::turn_driver::TurnTaskHost;
use crate::turn_reads::{TurnReads, TurnSlot};
use crate::worker_adapters::WorkerAdapters;
use crate::{
    activities::ScheduledAgentTriggerInput, activities::ScheduledChannelInput,
    activities::activity_types,
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

/// Worker adapters as the turn driver's host source.
#[derive(Clone)]
pub struct WorkerTurnHost<A: WorkerAdapters> {
    adapters: A,
    turns: Arc<TurnReads>,
}

impl<A: WorkerAdapters> WorkerTurnHost<A> {
    pub fn new(adapters: A) -> Self {
        Self {
            adapters,
            turns: Arc::default(),
        }
    }

    fn turn(
        &self,
        org_id: Option<i64>,
        session_id: Uuid,
        input_message_id: Uuid,
    ) -> Option<TurnSlot> {
        let org_id = org_id?;
        (!input_message_id.is_nil()).then(|| self.turns.slot(org_id, session_id, input_message_id))
    }
}

#[async_trait]
impl<A: WorkerAdapters> TurnTaskHost for WorkerTurnHost<A> {
    type Host = WorkerRuntimeHost<A>;

    fn host(&self) -> Self::Host {
        WorkerRuntimeHost::new(self.adapters.clone())
    }

    /// A reason host with its setup reads already started. The reads depend
    /// only on the session, harness, agent and input message, not on the turn
    /// state.
    fn reason_host(
        &self,
        input: &ReasonInput,
        (cancellation, cancel_requested): CancelSignals,
    ) -> Self::Host {
        let event_metadata = input.agent_id.map(|agent_id| {
            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "agent_id".to_string(),
                serde_json::Value::String(agent_id.to_string()),
            );
            metadata
        });
        let mut host =
            WorkerRuntimeHost::with_event_metadata(self.adapters.clone(), event_metadata)
                .with_turn_cancellation(cancellation, cancel_requested);
        if let Some(turn) = self.turn(
            Some(input.org_id),
            input.context.session_id.uuid(),
            input.context.input_message_id.uuid(),
        ) {
            host = host.with_turn_reads(turn);
        }
        host.prefetching(PhaseIds::reason(input))
    }

    fn act_host(&self, input: &ActInput) -> Self::Host {
        let mut host = WorkerRuntimeHost::new(self.adapters.clone());
        if let Some(turn) = self.turn(
            input.org_id,
            input.context.session_id.uuid(),
            input.context.input_message_id.uuid(),
        ) {
            host = host.with_turn_reads(turn);
        }
        host.prefetching(PhaseIds::act(input))
    }

    /// A turn that completed or paused keeps nothing for its next phase.
    async fn turn_planned(
        &self,
        checkpoint: &DurableTurnInput,
        plan: &TurnPlan,
        _output: &serde_json::Value,
    ) -> Result<()> {
        if matches!(
            plan,
            TurnPlan::Complete { .. } | TurnPlan::WaitForToolResults { .. }
        ) {
            self.turns.end(
                checkpoint.org_id,
                checkpoint.session_id.uuid(),
                checkpoint.input_message_id.uuid(),
            );
        }
        Ok(())
    }

    async fn phase_finished(&self, host: &Self::Host) {
        host.flush_events().await;
    }

    async fn execute_activity(&self, task: &ClaimedTask) -> Result<serde_json::Value> {
        let adapters = &self.adapters;
        match task.activity_type.as_str() {
            "leased_resource_cleanup" => {
                let cleanup_input: crate::leased_resource_cleanup::LeasedResourceCleanupInput =
                    serde_json::from_value(task.input.clone())
                        .map_err(|e| anyhow::anyhow!("Failed to parse cleanup input: {}", e))?;
                crate::leased_resource_cleanup::execute_cleanup_activity(adapters, &cleanup_input)
                    .await
            }
            "session_task_reaper" => {
                let reaper_input: crate::session_task_reaper::SessionTaskReaperInput =
                    serde_json::from_value(task.input.clone())
                        .map_err(|e| anyhow::anyhow!("Failed to parse reaper input: {}", e))?;
                crate::session_task_reaper::execute_reaper_activity(adapters, &reaper_input).await
            }
            activity_types::INVOKE_SCHEDULED_CHANNEL => {
                let input: ScheduledChannelInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse scheduled app input: {}", e))?;
                adapters
                    .invoke_scheduled_channel(input.org_id, &input.app_id, &input.channel_id)
                    .await
                    .map_err(anyhow::Error::from)
            }
            activity_types::INVOKE_AGENT_TRIGGER => {
                let input: ScheduledAgentTriggerInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse agent trigger input: {}", e))?;
                adapters
                    .invoke_agent_trigger(input.org_id, &input.agent_id, &input.trigger_id)
                    .await
                    .map_err(anyhow::Error::from)
            }
            _ => Err(anyhow::anyhow!(
                "Unknown activity type: {}",
                task.activity_type
            )),
        }
    }
}
