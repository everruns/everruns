//! The durable turn driver: runs one claimed turn task and schedules the next.
//!
//! Decision: the driver moved here from the worker so a turn can run against
//! any `TurnStore` (the in-memory or PostgreSQL durable store, or the worker's
//! gRPC store) with any runtime host. The worker keeps its poll loop, config,
//! registration and metrics, and hands every claimed task to
//! [`TurnTaskDriver::execute_task`]. Activity ids, checkpoints, wake-signal
//! drains, failure sealing and log lines are unchanged by the move.
//!
//! Model: queue plus per-step checkpoint. Each turn step (`process_input`,
//! `reason`, `act`) is a queued task; after its activity ran, the engine plans
//! the next step from the `DurableTurnInput` checkpoint and the steering wakes
//! pending at that boundary, and the driver hands off: one atomic store write
//! completes the task, consumes the wakes it counted and enqueues the next
//! step or completes the workflow ([`TurnStore::complete_task_and_hand_off`]).
//!
//! Decision: plan first, then commit the completion and the hand-off together.
//! As two writes (complete and drain, then enqueue), a process exit or a lost
//! reply between them left the workflow running with no task and its drained
//! wakes lost. Planning runs the turn's lifecycle effects (turn completed,
//! session idle), so a step whose hand-off never commits runs again and may
//! repeat them: at least once, never lost. A planning failure fails the task
//! the same way, so it is retried rather than left completed with no successor.
//!
//! Decision: with [`TurnTaskDriver::chain_steps`], the driver enqueues the
//! next step already claimed by its own worker and runs it at once, so a
//! turn's steps run back to back on one warm worker instead of each waiting
//! for a wakeup and a claim (a reason→act→reason hand-off cost a queue hop
//! per step). Each step is still its own task row, so heartbeats, retries,
//! stale reclaim, cancellation and history are unchanged; a store that
//! cannot claim on enqueue (or a draining worker) leaves the step queued.

use crate::durable::{ClaimedTask, SignalDrain, TaskFailureOutcome, WorkflowStatus};
use crate::durable_runner::DurableTurnInput;
use crate::engine::{ActInput, ActPlan, ReasonInput, ReasonResult, TurnExecution, TurnPlan};
use crate::host::{
    RuntimeHostAdapter, RuntimeSessionLifecycle, advance_host_execution,
    execute_act_activity as runtime_execute_act_activity,
};
use crate::task_error::{is_non_retryable_task_error, summarize_task_failure, user_facing_failure};
use crate::task_heartbeat::{CancelSignals, spawn_task_heartbeat};
use crate::turn_start;
use crate::turn_store::{TurnHandOff, TurnNext, TurnStore};
use anyhow::Result;
use async_trait::async_trait;
use everruns_contracts::typed_id::MessageId;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// What a host process supplies to the turn driver: the runtime host each
/// turn step runs on, and the activities that are not turn steps.
///
/// Hosts are built per step, so a host may carry step-scoped state (setup
/// reads started early, write-behind event buffers, cancellation).
#[async_trait]
pub trait TurnTaskHost: Clone + Send + Sync + 'static {
    /// The runtime host the turn activities run on.
    type Host: RuntimeHostAdapter;

    /// A host for the input step, step planning and turn lifecycle effects.
    fn host(&self) -> Self::Host;

    /// A host for a reason step. `cancel` carries the task cancellation and
    /// the explicit turn cancel from the task heartbeat; the host should
    /// expose them as its turn cancellation signals.
    fn reason_host(&self, input: &ReasonInput, cancel: CancelSignals) -> Self::Host;

    /// A host for an act step.
    fn act_host(&self, input: &ActInput) -> Self::Host {
        let _ = input;
        self.host()
    }

    /// Called after a reason or act step ran on `host`, before its result is
    /// used, so effects the host buffered (events) land first.
    async fn phase_finished(&self, host: &Self::Host) {
        let _ = host;
    }

    /// Steering: deliver input that joined the running turn, before a reason
    /// step runs. Returns the ids of the messages it persisted, which cross
    /// the `user_prompt_submit` boundary with that reason (after the turn's
    /// own input on its first iteration).
    ///
    /// The default delivers nothing: the platform worker's steering arrives
    /// as persisted messages plus `USER_MESSAGE` wake signals instead.
    async fn before_reason(&self, input: &DurableTurnInput) -> Result<Vec<MessageId>> {
        let _ = input;
        Ok(Vec::new())
    }

    /// Steering: called once a reason step ran, before the turn's next step
    /// is planned, with the `USER_MESSAGE` wakes this boundary counted (the
    /// hand-off consumes them with the planned step). Returns how many further user messages joined the turn; they
    /// count with the wakes when the engine decides whether the turn
    /// continues. `input` is the turn state the reason ran from.
    ///
    /// The default adds none.
    async fn after_reason(
        &self,
        input: &DurableTurnInput,
        reason: &ReasonResult,
        drained_wakes: usize,
    ) -> Result<usize> {
        let _ = (input, reason, drained_wakes);
        Ok(0)
    }

    /// Called with the turn's next step once the engine planned it, before
    /// the driver enqueues that step or ends the workflow. A host that
    /// accepts mid-turn input closes its ingress here when the turn ends.
    /// `output` is the output of the step the plan follows (an act's carries
    /// the client-side calls a parked turn waits on).
    async fn turn_planned(
        &self,
        checkpoint: &DurableTurnInput,
        plan: &TurnPlan,
        output: &serde_json::Value,
    ) -> Result<()> {
        let _ = (checkpoint, plan, output);
        Ok(())
    }

    /// Run a claimed task whose activity type is not a turn step
    /// (`process_input`, `reason`, `act`). The default rejects it.
    async fn execute_activity(&self, task: &ClaimedTask) -> Result<serde_json::Value> {
        Err(anyhow::anyhow!(
            "Unknown activity type: {}",
            task.activity_type
        ))
    }
}

/// Runs claimed durable tasks of an agent turn against a [`TurnStore`].
///
/// Each call runs one task: it checks the workflow is not cancelled, records
/// the activity, heartbeats the task, runs the step, then completes or fails
/// the task and schedules the turn's next step.
pub struct TurnTaskDriver<S: TurnStore + ?Sized, H: TurnTaskHost> {
    store: Arc<S>,
    hosts: H,
    worker_id: String,
    heartbeat_interval: Duration,
    chain_steps: bool,
}

impl<S: TurnStore + ?Sized, H: TurnTaskHost> Clone for TurnTaskDriver<S, H> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            hosts: self.hosts.clone(),
            worker_id: self.worker_id.clone(),
            heartbeat_interval: self.heartbeat_interval,
            chain_steps: self.chain_steps,
        }
    }
}

impl<S: TurnStore + ?Sized, H: TurnTaskHost> TurnTaskDriver<S, H> {
    /// A driver that claims ownership as `worker_id` and heartbeats each task
    /// every `heartbeat_interval`.
    pub fn new(
        store: Arc<S>,
        hosts: H,
        worker_id: impl Into<String>,
        heartbeat_interval: Duration,
    ) -> Self {
        Self {
            store,
            hosts,
            worker_id: worker_id.into(),
            heartbeat_interval,
            chain_steps: false,
        }
    }

    /// Run each turn step's next step on this driver right away, claimed on
    /// enqueue, instead of handing it to the queue (see the module notes).
    /// Off by default.
    #[must_use]
    pub fn chain_steps(mut self, chain: bool) -> Self {
        self.chain_steps = chain;
        self
    }

    /// Execute one claimed task to completion or failure, then, with
    /// [`chain_steps`](Self::chain_steps), every step of its turn this
    /// driver got claimed after it.
    ///
    /// # Errors
    ///
    /// Returns the failing task's error after recording the failure in the
    /// store.
    pub async fn execute_task(&self, task: &ClaimedTask) -> Result<()> {
        let claim_for = self.chain_steps.then_some(self.worker_id.as_str());
        let mut next = self.execute_one(task, claim_for).await?;
        while let Some(task) = next {
            debug!(
                task_id = %task.id,
                workflow_id = ?task.workflow_id,
                activity_type = %task.activity_type,
                "Running chained turn step"
            );
            next = self.execute_one(&task, claim_for).await?;
        }
        Ok(())
    }

    async fn execute_one(
        &self,
        task: &ClaimedTask,
        claim_for: Option<&str>,
    ) -> Result<Option<ClaimedTask>> {
        execute_task(
            &self.store,
            &self.hosts,
            &self.worker_id,
            self.heartbeat_interval,
            task,
            claim_for,
        )
        .await
    }
}

/// Execute a single task
async fn execute_task<S, H>(
    store: &Arc<S>,
    hosts: &H,
    worker_id: &str,
    heartbeat_interval: Duration,
    task: &ClaimedTask,
    claim_for: Option<&str>,
) -> Result<Option<ClaimedTask>>
where
    S: TurnStore + ?Sized,
    H: TurnTaskHost,
{
    info!(
        task_id = %task.id,
        workflow_id = ?task.workflow_id,
        activity_type = %task.activity_type,
        attempt = task.attempt,
        "Executing task"
    );

    // Check if workflow is cancelled (only for workflow-bound tasks). The
    // claim usually reports the status; ask the store only when it did not.
    if let Some(wf_id) = task.workflow_id {
        let workflow_status = match task.workflow_status {
            Some(status) => Ok(status),
            None => store
                .get_workflow(wf_id)
                .await
                .map(|workflow| workflow.status),
        };
        if let Ok(status) = workflow_status
            && status == WorkflowStatus::Cancelled
        {
            info!(
                task_id = %task.id,
                workflow_id = %wf_id,
                "Workflow cancelled, skipping task"
            );
            let _ = store
                .fail_task_and_record(task, "Workflow cancelled", false)
                .await;
            return Ok(None);
        }
    }

    store.record_activity_started(task, worker_id).await;

    let (heartbeat_cancel_tx, heartbeat_handle, task_cancellation) = spawn_task_heartbeat(
        store.clone(),
        task.id,
        worker_id.to_string(),
        heartbeat_interval,
    );

    // Execute based on activity type. Keep fallible parsing inside this result so
    // cleanup below runs before malformed tasks are failed.
    let execution = async {
        // A `process_input` task runs the turn's first reason too, and is then
        // completed and scheduled as that `reason` (see `turn_start`).
        let mut activity = task.activity_type.as_str();
        let (result, turn_input_opt) = match activity {
            "process_input" | "reason" => {
                let turn_input: DurableTurnInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse task input: {}", e))?;
                let cancel = task_cancellation.clone();
                if activity == "process_input" {
                    activity = "reason";
                    let (res, checkpoint) =
                        turn_start::execute_turn_start(hosts, &turn_input, task.id, cancel).await;
                    (res, Some(checkpoint))
                } else {
                    let res = turn_start::execute_reason_activity(hosts, &turn_input, cancel);
                    (res.await, Some(turn_input))
                }
            }
            "act" => {
                let act_input: ActInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse ActInput: {}", e))?;

                let resume_state = parse_resume_state(&task.input)?;

                // Create DurableTurnInput from ActInput context
                let turn_input = resume_state.unwrap_or(DurableTurnInput {
                    org_id: act_input.org_id.ok_or_else(|| {
                        anyhow::anyhow!("ActInput.org_id must be set for durable turns")
                    })?,
                    session_id: act_input.context.session_id,
                    harness_id: act_input.harness_id,
                    agent_id: act_input.agent_id,
                    input_message_id: act_input.context.input_message_id,
                    turn_id: Some(act_input.context.turn_id),
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
                });
                let res = execute_act_activity(hosts, &act_input).await;
                (res, Some(turn_input))
            }
            // Activities that are not turn steps belong to the host process
            // (the worker's cleanup, reaper and scheduled invocations). They
            // share this task lifecycle: cancellation check, heartbeat,
            // completion and failure recording.
            _ => (hosts.execute_activity(task).await, None),
        };
        Ok::<_, anyhow::Error>((result, turn_input_opt, activity))
    }
    .await;

    let _ = heartbeat_cancel_tx.send(());
    let _ = heartbeat_handle.await;

    let (result, turn_input_opt, activity) = match execution {
        Ok(execution) => execution,
        Err(e) => {
            fail_activity_task(store, hosts, task, None, &e).await?;

            return Err(e);
        }
    };

    let output = match result {
        Ok(output) => output,
        Err(e) => {
            fail_activity_task(store, hosts, task, turn_input_opt.as_ref(), &e).await?;
            return Err(e);
        }
    };

    // A turn step plans its successor, then completes and hands off in one
    // store write; any other task just completes.
    let (Some(turn_input), Some(workflow_id)) = (turn_input_opt, task.workflow_id) else {
        match store
            .complete_task_and_record(task, worker_id, output)
            .await
        {
            Ok(()) => info!(
                task_id = %task.id,
                activity_type = %task.activity_type,
                "Task completed successfully"
            ),
            Err(e) => warn!(task_id = %task.id, error = %e, "Task completion rejected"),
        }
        return Ok(None);
    };

    let hand_off = match plan_next_step(
        store,
        hosts,
        workflow_id,
        activity,
        &turn_input,
        &output,
        claim_for,
    )
    .await
    {
        Ok(hand_off) => hand_off,
        Err(e) => {
            // Nothing was handed off: the step runs again, as after a crash
            // before its completion.
            fail_activity_task(store, hosts, task, Some(&turn_input), &e).await?;
            return Err(e);
        }
    };

    match store
        .complete_task_and_hand_off(task, worker_id, output, hand_off)
        .await
    {
        Ok(next) => {
            info!(
                task_id = %task.id,
                activity_type = %task.activity_type,
                "Task completed successfully"
            );
            Ok(next)
        }
        Err(e) => {
            // Rejected (the task was reclaimed) or unknown (the reply was
            // lost): either way the store holds a claimed task to reclaim or
            // the committed hand-off, never a run with no task.
            warn!(
                task_id = %task.id,
                error = %e,
                "Task completion rejected - skipping next activity"
            );
            Ok(None)
        }
    }
}

async fn fail_activity_task<S: TurnStore + ?Sized, H: TurnTaskHost>(
    store: &Arc<S>,
    hosts: &H,
    task: &ClaimedTask,
    turn_input: Option<&DurableTurnInput>,
    error: &anyhow::Error,
) -> Result<TaskFailureOutcome> {
    let failure = summarize_task_failure(
        task.id,
        task.workflow_id,
        &task.activity_type,
        task.attempt,
        Some(task.max_attempts),
        &task.input,
        error,
    );
    let retryable = !is_non_retryable_task_error(error);
    let outcome = store
        .fail_task_and_record(task, &failure.persisted_message, retryable)
        .await
        .map_err(|store_error| anyhow::anyhow!("Failed to persist task failure: {store_error}"))?;

    if matches!(outcome, TaskFailureOutcome::ExhaustedRetries) {
        terminalize_failed_turn(hosts, task, turn_input, &failure.persisted_message).await?;
    }

    Ok(outcome)
}

async fn terminalize_failed_turn<H: TurnTaskHost>(
    hosts: &H,
    task: &ClaimedTask,
    turn_input: Option<&DurableTurnInput>,
    persisted_error: &str,
) -> Result<()> {
    let parsed_input = if turn_input.is_none() {
        serde_json::from_value::<DurableTurnInput>(task.input.clone()).ok()
    } else {
        None
    };
    let Some(input) = turn_input.or(parsed_input.as_ref()) else {
        return Ok(());
    };
    let Some(turn_id) = input.turn_id else {
        warn!(
            task_id = %task.id,
            workflow_id = ?task.workflow_id,
            "Cannot emit terminal turn events without a turn_id"
        );
        return Ok(());
    };

    let user_error = user_facing_failure(persisted_error);
    let message = user_error.fallback_message();
    let lifecycle = RuntimeSessionLifecycle::new(hosts.host(), input.org_id, input.session_id);
    lifecycle
        .turn_failed(turn_id, input.input_message_id, &message, Some(&user_error))
        .await
        .map_err(anyhow::Error::from)?;
    lifecycle
        .fire_turn_end_hooks(input.harness_id, input.agent_id, turn_id, false)
        .await;
    Ok(())
}

fn parse_resume_state(input: &serde_json::Value) -> Result<Option<DurableTurnInput>> {
    match input.get("resume_state") {
        Some(value) if value.is_null() => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| anyhow::anyhow!("Failed to parse resume_state: {}", e)),
        None => Ok(None),
    }
}

// =============================================================================
// Activity Implementations
// =============================================================================

/// Execute act activity (tool execution)
async fn execute_act_activity<H: TurnTaskHost>(
    hosts: &H,
    input: &ActInput,
) -> Result<serde_json::Value> {
    debug!(
        session_id = %input.context.session_id,
        tool_count = input.tool_calls.len(),
        "Executing act activity"
    );

    let host = hosts.act_host(input);
    let result = runtime_execute_act_activity(&host, input.clone()).await;
    hosts.phase_finished(&host).await;
    let result = result?;

    Ok(serde_json::to_value(&result)?)
}

// =============================================================================
// Activity Scheduling
// =============================================================================

/// Plan the step after `completed_activity` from the turn checkpoint and
/// the steering wakes pending at this boundary, as the hand-off that commits
/// it.
async fn plan_next_step<S: TurnStore + ?Sized, H: TurnTaskHost>(
    store: &Arc<S>,
    hosts: &H,
    workflow_id: Uuid,
    completed_activity: &str,
    input: &DurableTurnInput,
    output: &serde_json::Value,
    claim_for: Option<&str>,
) -> Result<TurnHandOff> {
    let reason_final_answer = reason_final_answer(completed_activity, output)?;

    // Count queued USER_MESSAGE steering signals (task wakes) at the
    // boundaries that precede another reason iteration. The already-persisted
    // wake message is picked up by that reason (it re-reads full history); the
    // count governs turn continuation, and the hand-off consumes exactly the
    // wakes counted, with the step it plans: exactly-once delivery, and a wake
    // that arrives meanwhile stays for the next boundary. See
    // `drains_wake_signals_after`.
    let drains = drains_wake_signals_after(completed_activity, reason_final_answer);
    let drained = if drains {
        store
            .count_pending_signals(workflow_id, crate::durable_turn::USER_MESSAGE)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to read workflow wake signals: {}", error))?
    } else {
        0
    };
    let mut pending_user_message_count = drained;
    if completed_activity == "reason" {
        let reason: ReasonResult = serde_json::from_value(output.clone())
            .map_err(|error| anyhow::anyhow!("Invalid reason output payload: {}", error))?;
        pending_user_message_count += hosts.after_reason(input, &reason, drained).await?;
    }

    if completed_activity == "act" && pending_user_message_count > 0 {
        debug!(
            %workflow_id,
            pending_user_message_count,
            "delivering mid-turn task wake(s) at the act→reason boundary"
        );
    }

    let mut execution = TurnExecution::new(input.clone());
    let plan = advance_host_execution(
        &hosts.host(),
        &mut execution,
        completed_activity,
        output,
        pending_user_message_count,
    )
    .await?;
    let checkpoint = execution.into_state();
    hosts.turn_planned(&checkpoint, &plan, output).await?;
    let claim_for = claim_for.map(str::to_owned);
    let next = match plan {
        TurnPlan::ScheduleReason(_) => TurnNext::Step {
            activity_id: format!("reason_{}", Uuid::now_v7()),
            activity_type: "reason".to_string(),
            input: serde_json::to_value(&checkpoint)?,
            claim_for,
        },
        TurnPlan::ScheduleAct(plan) => TurnNext::Step {
            activity_id: format!("act_{}", Uuid::now_v7()),
            activity_type: "act".to_string(),
            input: act_task_input(&plan, &checkpoint)?,
            claim_for,
        },
        TurnPlan::Complete { stop_reason, error } => TurnNext::Complete {
            event_output: turn_output_with_stop_reason(output.clone(), stop_reason),
            stored_output: Some(serde_json::to_value(&checkpoint)?),
            error: error.map(crate::durable::WorkflowError::new),
        },
        TurnPlan::WaitForToolResults { .. } => TurnNext::Complete {
            event_output: output.clone(),
            stored_output: Some(serde_json::to_value(&checkpoint)?),
            error: None,
        },
    };
    Ok(TurnHandOff {
        workflow_id,
        drain: drains.then(|| SignalDrain {
            signal_type: crate::durable_turn::USER_MESSAGE.to_string(),
            limit: drained,
        }),
        next,
    })
}

/// Whether a completed `reason` produced a final answer (no tool calls, no
/// pause), winding the turn down. Always false for other activities.
fn reason_final_answer(completed_activity: &str, output: &serde_json::Value) -> Result<bool> {
    if completed_activity != "reason" {
        return Ok(false);
    }
    let reason_result: ReasonResult = serde_json::from_value(output.clone())
        .map_err(|error| anyhow::anyhow!("Invalid reason output payload: {}", error))?;
    let continues = reason_result.has_tool_calls || reason_result.waiting_for_tool_results;
    Ok(reason_result.success && !continues)
}

fn turn_output_with_stop_reason(
    mut output: serde_json::Value,
    stop_reason: crate::core::turn::TurnStopReason,
) -> serde_json::Value {
    if let Some(object) = output.as_object_mut() {
        object.insert("stop_reason".to_string(), serde_json::json!(stop_reason));
    }
    output
}

/// Iteration boundaries at which queued `USER_MESSAGE` steering signals (task
/// wakes) are drained.
///
/// A wake is delivered as a persisted user message plus a durable
/// `USER_MESSAGE` signal (see `SessionTaskWaker`). The message is picked up by
/// the next reason iteration (which re-reads full history); the signal governs
/// whether the loop runs that next iteration. Draining it:
/// - at the `act`→`reason` boundary delivers a wake that arrived **mid-turn**
///   at the very next reason, so an active parent reacts without ending its
///   turn (EVE-681); and
/// - at a final-answer `reason` boundary decides continue-vs-idle for a wake
///   that arrived as the turn wound down.
///
/// Because the hand-off consumes the wakes it counted, a wake drained at the
/// act boundary is not seen again by the end-of-turn drain — mid-turn XOR
/// next-turn, never both.
fn drains_wake_signals_after(completed_activity: &str, reason_final_answer: bool) -> bool {
    match completed_activity {
        "act" => true,
        "reason" => reason_final_answer,
        _ => false,
    }
}

pub(crate) fn act_task_input(
    plan: &ActPlan,
    checkpoint: &DurableTurnInput,
) -> Result<serde_json::Value> {
    let mut input = serde_json::to_value(&plan.input)?;
    input["resume_state"] = serde_json::to_value(checkpoint)?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable::{
        ActivityOptions, DurableAdmin, EventLog, HeartbeatResponse, SignalStore, StoreError,
        TaskDefinition, TaskQueue, WorkerInfo, WorkerRegistry, WorkflowError,
    };
    use everruns_contracts::typed_id::TurnId;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ---- EVE-681: mid-turn task wake drain ----

    fn user_message_signal() -> crate::durable::WorkflowSignal {
        crate::durable::WorkflowSignal::new(
            crate::durable_turn::USER_MESSAGE,
            serde_json::json!({}),
        )
    }

    /// `TurnStore` stub returning a fixed set of pending signals and counting
    /// how many times wake-signal drains are called.
    #[derive(Clone)]
    struct RecordingStore {
        signals: Vec<crate::durable::WorkflowSignal>,
        consume_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl TurnStore for RecordingStore {
        async fn register_worker(&self, _worker: WorkerInfo) -> Result<(), StoreError> {
            Ok(())
        }
        async fn worker_heartbeat(
            &self,
            _worker_id: &str,
            _current_load: usize,
            _accepting_tasks: bool,
        ) -> Result<everruns_durable::WorkerHeartbeat, StoreError> {
            Ok(Default::default())
        }
        async fn drain_worker(&self, _worker_id: &str) -> Result<(), StoreError> {
            Ok(())
        }
        async fn deregister_worker(&self, _worker_id: &str) -> Result<usize, StoreError> {
            Ok(0)
        }
        async fn claim_task(
            &self,
            _worker_id: &str,
            _activity_types: &[String],
            _max_tasks: usize,
        ) -> Result<Vec<ClaimedTask>, StoreError> {
            Ok(vec![])
        }
        async fn heartbeat_task(
            &self,
            _task_id: Uuid,
            _worker_id: &str,
            _details: Option<serde_json::Value>,
        ) -> Result<HeartbeatResponse, StoreError> {
            Ok(HeartbeatResponse {
                accepted: true,
                should_cancel: false,
            })
        }
        async fn get_workflow(
            &self,
            _workflow_id: Uuid,
        ) -> Result<crate::turn_store::WorkflowSnapshot, StoreError> {
            Ok(crate::turn_store::WorkflowSnapshot {
                status: WorkflowStatus::Running,
                output: None,
                error: None,
            })
        }

        async fn start_turn(
            &self,
            _workflow_id: Uuid,
            _workflow_type: &str,
            _input: serde_json::Value,
            _activity_id: String,
            _activity_type: String,
        ) -> Result<crate::durable::RunStart, StoreError> {
            Ok(crate::durable::RunStart::Active)
        }

        async fn cancel_pending_tasks(&self, _workflow_id: Uuid) -> Result<u64, StoreError> {
            Ok(0)
        }

        async fn count_active_workflows(&self) -> Result<usize, StoreError> {
            Ok(0)
        }

        async fn send_signal(
            &self,
            _workflow_id: Uuid,
            _signal: crate::durable::WorkflowSignal,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        async fn record_activity_started(&self, _task: &ClaimedTask, _worker_id: &str) {}
        async fn complete_task_and_record(
            &self,
            _task: &ClaimedTask,
            _worker_id: &str,
            _output: serde_json::Value,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        async fn fail_task_and_record(
            &self,
            _task: &ClaimedTask,
            _error: &str,
            _retryable: bool,
        ) -> Result<TaskFailureOutcome, StoreError> {
            Ok(TaskFailureOutcome::MovedToDlq)
        }
        async fn enqueue_task_and_record(
            &self,
            _workflow_id: Uuid,
            _activity_id: String,
            _activity_type: String,
            _input: serde_json::Value,
        ) -> Result<Uuid, StoreError> {
            Ok(Uuid::now_v7())
        }
        async fn update_workflow_status(
            &self,
            _workflow_id: Uuid,
            _status: WorkflowStatus,
            _output: Option<serde_json::Value>,
            _error: Option<WorkflowError>,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        async fn complete_workflow(
            &self,
            _workflow_id: Uuid,
            _event_output: serde_json::Value,
            _stored_output: Option<serde_json::Value>,
            _error: Option<WorkflowError>,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        async fn consume_pending_signals(
            &self,
            _workflow_id: Uuid,
        ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
            Ok(self.signals.clone())
        }

        async fn count_pending_signals(
            &self,
            _workflow_id: Uuid,
            signal_type: &str,
        ) -> Result<usize, StoreError> {
            Ok(self
                .signals
                .iter()
                .filter(|signal| signal.signal_type == signal_type)
                .count())
        }

        async fn consume_pending_signals_by_type(
            &self,
            _workflow_id: Uuid,
            signal_type: &str,
        ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
            self.consume_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .signals
                .iter()
                .filter(|signal| signal.signal_type == signal_type)
                .cloned()
                .collect())
        }
    }

    #[test]
    fn wake_signals_drain_at_act_and_final_reason_boundaries() {
        // `act` always precedes another reason → the mid-turn delivery point.
        assert!(drains_wake_signals_after("act", false));
        // A final-answer reason drains to decide continue-vs-idle.
        assert!(drains_wake_signals_after("reason", true));
        // A tool-calling reason does not drain; the following `act` will.
        assert!(!drains_wake_signals_after("reason", false));
        // Turn start and unknown activities never drain.
        assert!(!drains_wake_signals_after("process_input", false));
        assert!(!drains_wake_signals_after("input", false));
    }

    #[test]
    fn only_a_successful_reason_without_tool_calls_is_a_final_answer() {
        let reason = |has_tool_calls| {
            serde_json::to_value(ReasonResult {
                success: true,
                has_tool_calls,
                ..ReasonResult::default()
            })
            .unwrap()
        };
        assert!(reason_final_answer("reason", &reason(false)).unwrap());
        assert!(!reason_final_answer("reason", &reason(true)).unwrap());
        assert!(!reason_final_answer("act", &serde_json::json!({})).unwrap());
        assert!(reason_final_answer("reason", &serde_json::json!({})).is_err());
    }

    #[tokio::test]
    async fn a_store_without_an_atomic_hand_off_makes_the_writes_in_steps() {
        // The default hand-off completes, drains only when asked to, then
        // enqueues; a drain the plan counted none for touches no signals.
        let store = RecordingStore {
            signals: vec![user_message_signal()],
            consume_calls: Arc::new(AtomicUsize::new(0)),
        };
        let workflow_id = Uuid::now_v7();
        let task = ClaimedTask {
            id: Uuid::now_v7(),
            workflow_id: Some(workflow_id),
            activity_id: "act-1".into(),
            activity_type: "act".into(),
            input: serde_json::json!({}),
            attempt: 1,
            max_attempts: 1,
            ..Default::default()
        };
        let hand_off = |limit| TurnHandOff {
            workflow_id,
            drain: Some(SignalDrain {
                signal_type: crate::durable_turn::USER_MESSAGE.into(),
                limit,
            }),
            next: TurnNext::Step {
                activity_id: "reason-2".into(),
                activity_type: "reason".into(),
                input: serde_json::json!({}),
                claim_for: None,
            },
        };
        let next = store
            .complete_task_and_hand_off(&task, "w", serde_json::json!({}), hand_off(0))
            .await
            .unwrap();
        assert!(next.is_none());
        assert_eq!(store.consume_calls.load(Ordering::SeqCst), 0);
        store
            .complete_task_and_hand_off(&task, "w", serde_json::json!({}), hand_off(1))
            .await
            .unwrap();
        assert_eq!(store.consume_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn durable_turn_output_surfaces_structured_stop_reason() {
        let output = turn_output_with_stop_reason(
            serde_json::json!({ "success": true, "error": null }),
            crate::core::turn::TurnStopReason::MaxTokens,
        );

        assert_eq!(output["success"], true);
        assert_eq!(output["error"], serde_json::Value::Null);
        assert_eq!(output["stop_reason"], "max_tokens");
    }

    #[test]
    fn act_wire_input_uses_the_engine_checkpoint_as_its_only_resume_state() {
        use crate::engine::{ActSchedulingFacts, TurnState, plan_after_reason};
        use everruns_contracts::typed_id::{HarnessId, MessageId, SessionId};

        let state = TurnState {
            org_id: 7,
            session_id: SessionId::new(),
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id: MessageId::new(),
            turn_id: Some(TurnId::new()),
            previous_response_id: Some("before".into()),
            iteration: 2,
            request_id: Some("request-before".into()),
            started_at: None,
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        let reason = ReasonResult {
            native_counts: None,
            success: true,
            text: String::new(),
            tool_calls: vec![everruns_contracts::tool_types::ToolCall {
                id: "call-1".into(),
                name: "noop".into(),
                arguments: serde_json::json!({}),
            }],
            has_tool_calls: true,
            tool_definitions: vec![],
            max_iterations: 8,
            error: None,
            user_facing_error: None,
            error_disclosure: None,
            usage: None,
            output_message_id: None,
            time_to_first_token_ms: None,
            response_id: Some("response-after".into()),
            finish_reason: Some("tool_calls".into()),
            ..ReasonResult::default()
        };
        let (TurnPlan::ScheduleAct(plan), _) = plan_after_reason(
            &state,
            reason,
            0,
            chrono::Utc::now(),
            Some(ActSchedulingFacts::default()),
        ) else {
            panic!("reason with a tool call must schedule act");
        };
        let mut checkpoint = state;
        checkpoint.previous_response_id = Some("checkpoint-response".into());
        checkpoint.iteration = 9;
        checkpoint.request_id = Some("checkpoint-request".into());

        let input = act_task_input(&plan, &checkpoint).expect("serialize act input");

        assert_eq!(input["resume_state"]["iteration"], 9);
        assert_eq!(
            input["resume_state"]["previous_response_id"],
            "checkpoint-response"
        );
        assert_eq!(input["resume_state"]["request_id"], "checkpoint-request");
        assert!(input.get("iteration").is_none());
        assert!(input.get("previous_response_id").is_none());
        assert!(input.get("request_id").is_none());
    }

    /// A running workflow with one task claimed by `worker`, on the memory
    /// store.
    async fn claimed_turn_step(
        store: &crate::durable::InMemoryWorkflowEventStore,
        activity_type: &str,
    ) -> (Uuid, ClaimedTask) {
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
            .await
            .unwrap();
        EventLog::update_workflow_status(store, workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
        TaskQueue::enqueue_task(
            store,
            TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: format!("{activity_type}-1"),
                activity_type: activity_type.to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            },
        )
        .await
        .unwrap();
        let worker = crate::durable::WorkerInfo::new("worker", [activity_type]);
        WorkerRegistry::register_worker(store, worker)
            .await
            .unwrap();
        let claimed = TaskQueue::claim_task(store, "worker", &[activity_type.into()], 1).await;
        (workflow_id, claimed.unwrap().pop().unwrap())
    }

    #[tokio::test]
    async fn a_hand_off_consumes_only_the_user_message_wakes_it_counted() {
        let store = crate::durable::InMemoryWorkflowEventStore::new();
        let (workflow_id, task) = claimed_turn_step(&store, "act").await;
        // Two wakes and an unrelated signal accrued during the step; the plan
        // counted one wake.
        for signal in [
            user_message_signal(),
            crate::durable::WorkflowSignal::new(
                crate::durable::signal_types::CANCEL,
                serde_json::json!({}),
            ),
            user_message_signal(),
        ] {
            TurnStore::send_signal(&store, workflow_id, signal)
                .await
                .unwrap();
        }
        let counted = TurnStore::count_pending_signals(
            &store,
            workflow_id,
            crate::durable_turn::USER_MESSAGE,
        )
        .await
        .unwrap();
        assert_eq!(counted, 2, "counting consumes nothing");

        let next = TurnStore::complete_task_and_hand_off(
            &store,
            &task,
            "worker",
            serde_json::json!({}),
            TurnHandOff {
                workflow_id,
                drain: Some(SignalDrain {
                    signal_type: crate::durable_turn::USER_MESSAGE.into(),
                    limit: 1,
                }),
                next: TurnNext::Step {
                    activity_id: "reason-2".into(),
                    activity_type: "reason".into(),
                    input: serde_json::json!({}),
                    claim_for: Some("worker".into()),
                },
            },
        )
        .await
        .unwrap()
        .expect("the next step is claimed by the registered worker");
        assert_eq!(next.activity_type, "reason");

        // The uncounted wake and the cancel signal stay for later.
        let left = SignalStore::consume_pending_signals(&store, workflow_id)
            .await
            .unwrap();
        let types: Vec<_> = left.iter().map(|s| s.signal_type.as_str()).collect();
        assert_eq!(
            types,
            [
                crate::durable::signal_types::CANCEL,
                crate::durable_turn::USER_MESSAGE
            ]
        );
    }

    #[tokio::test]
    async fn a_rejected_hand_off_changes_nothing() {
        let store = crate::durable::InMemoryWorkflowEventStore::new();
        let (workflow_id, task) = claimed_turn_step(&store, "reason").await;
        TurnStore::send_signal(&store, workflow_id, user_message_signal())
            .await
            .unwrap();

        let rejected = TurnStore::complete_task_and_hand_off(
            &store,
            &task,
            "another-worker",
            serde_json::json!({}),
            TurnHandOff {
                workflow_id,
                drain: Some(SignalDrain {
                    signal_type: crate::durable_turn::USER_MESSAGE.into(),
                    limit: 1,
                }),
                next: TurnNext::Complete {
                    event_output: serde_json::json!({}),
                    stored_output: None,
                    error: None,
                },
            },
        )
        .await;

        assert!(matches!(rejected, Err(StoreError::TaskNotOwned(_))));
        assert_eq!(
            EventLog::get_workflow_status(&store, workflow_id)
                .await
                .unwrap(),
            WorkflowStatus::Running
        );
        assert_eq!(
            TaskQueue::get_task(&store, task.id).await.unwrap().status,
            crate::durable::TaskStatus::Claimed
        );
        assert_eq!(
            TurnStore::count_pending_signals(
                &store,
                workflow_id,
                crate::durable_turn::USER_MESSAGE
            )
            .await
            .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn direct_store_elects_one_terminal_failure_owner() {
        let store = crate::durable::InMemoryWorkflowEventStore::new();
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
            .await
            .unwrap();
        EventLog::update_workflow_status(&store, workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
        store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "reason".to_string(),
                activity_type: "reason".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();
        let worker = crate::durable::WorkerInfo::new("worker", ["reason"]);
        let registered = WorkerRegistry::register_worker(&store, worker).await;
        registered.unwrap();
        let claimed = TaskQueue::claim_task(&store, "worker", &["reason".into()], 1).await;
        let task = claimed.unwrap().pop().unwrap();

        let outcome = TurnStore::fail_task_and_record(&store, &task, "terminal", false)
            .await
            .unwrap();

        assert!(matches!(outcome, TaskFailureOutcome::ExhaustedRetries));
        assert_eq!(
            EventLog::get_workflow_status(&store, workflow_id)
                .await
                .unwrap(),
            WorkflowStatus::Failed
        );
        let events = store.get_workflow_events(workflow_id).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "workflow_failed")
                .count(),
            1
        );
    }

    #[test]
    fn parse_resume_state_allows_missing_field() {
        let input = serde_json::json!({});

        let parsed = parse_resume_state(&input).expect("missing resume_state is allowed");

        assert!(parsed.is_none());
    }

    #[test]
    fn parse_resume_state_rejects_malformed_state() {
        let input = serde_json::json!({
            "resume_state": {
                "org_id": "not-an-org-id"
            }
        });

        let error = parse_resume_state(&input).expect_err("malformed resume_state must fail");

        assert!(
            error.to_string().contains("Failed to parse resume_state"),
            "unexpected error: {error}"
        );
    }
}
