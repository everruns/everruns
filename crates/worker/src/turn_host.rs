//! The worker's side of the durable turn driver.
//!
//! Decision: the turn driver lives in durable-engine (`TurnTaskDriver`) and
//! is generic over the host it runs steps on. The worker supplies
//! `WorkerRuntimeHost` over its adapters, built fresh per step so each step
//! starts its own setup reads (`phase_reads`) and flushes its own write-behind
//! events, and runs the activities that are not turn steps: leased-resource
//! cleanup, the session task reaper and scheduled invocations.

use crate::durable::ClaimedTask;
use crate::engine::{ActInput, ReasonInput};
use crate::phase_reads::PhaseIds;
use crate::runtime_host::WorkerRuntimeHost;
use crate::task_heartbeat::CancelSignals;
use crate::turn_driver::TurnTaskHost;
use crate::worker_adapters::WorkerAdapters;
use crate::{
    activities::ScheduledAgentTriggerInput, activities::ScheduledChannelInput,
    activities::activity_types,
};
use anyhow::Result;
use async_trait::async_trait;

/// Worker adapters as the turn driver's host source.
#[derive(Clone)]
pub struct WorkerTurnHost<A: WorkerAdapters>(pub A);

#[async_trait]
impl<A: WorkerAdapters> TurnTaskHost for WorkerTurnHost<A> {
    type Host = WorkerRuntimeHost<A>;

    fn host(&self) -> Self::Host {
        WorkerRuntimeHost::new(self.0.clone())
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
        WorkerRuntimeHost::with_event_metadata(self.0.clone(), event_metadata)
            .with_turn_cancellation(cancellation, cancel_requested)
            .prefetching(PhaseIds::reason(input))
    }

    fn act_host(&self, input: &ActInput) -> Self::Host {
        WorkerRuntimeHost::new(self.0.clone()).prefetching(PhaseIds::act(input))
    }

    async fn phase_finished(&self, host: &Self::Host) {
        host.flush_events().await;
    }

    async fn execute_activity(&self, task: &ClaimedTask) -> Result<serde_json::Value> {
        let adapters = &self.0;
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
