//! The durable turn driver: runs one claimed turn task and schedules the next.
//!
//! Decision: the driver moved here from the worker so a turn can run against
//! any `TaskStore` (the in-memory or PostgreSQL durable store, or the worker's
//! gRPC store) with any runtime host. The worker keeps its poll loop, config,
//! registration and metrics, and hands every claimed task to
//! [`TurnTaskDriver::execute_task`]. Activity ids, checkpoints, wake-signal
//! drains, failure sealing and log lines are unchanged by the move.
//!
//! Model: queue plus per-step checkpoint. Each turn step (`process_input`,
//! `reason`, `act`) is a queued task; after it completes, the engine plans
//! the next step from the `DurableTurnInput` checkpoint and the driver enqueues
//! it, or completes the workflow.
//!
//! Decision: with [`TurnTaskDriver::chain_steps`], the driver enqueues the
//! next step already claimed by its own worker and runs it at once, so a
//! turn's steps run back to back on one warm worker instead of each waiting
//! for a wakeup and a claim (a reason→act→reason hand-off cost a queue hop
//! per step). Each step is still its own task row, so heartbeats, retries,
//! stale reclaim, cancellation and history are unchanged; a store that
//! cannot claim on enqueue (or a draining worker) leaves the step queued.

use crate::durable::{ClaimedTask, TaskFailureOutcome, WorkflowStatus};
use crate::durable_runner::DurableTurnInput;
use crate::engine::{ActInput, ActPlan, ReasonInput, ReasonResult, TurnPlan};
use crate::host::{
    RuntimeHostAdapter, RuntimeSessionLifecycle, advance_host_execution,
    execute_act_activity as runtime_execute_act_activity,
};
use crate::task_error::{is_non_retryable_task_error, summarize_task_failure, user_facing_failure};
use crate::task_heartbeat::{CancelSignals, spawn_task_heartbeat};
use crate::task_store::TaskStore;
use crate::turn_start;
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

    /// Steering: called once a reason step completed, before the turn's next
    /// step is planned, with the `USER_MESSAGE` wakes this boundary already
    /// drained. Returns how many further user messages joined the turn; they
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

/// Runs claimed durable tasks of an agent turn against a [`TaskStore`].
///
/// Each call runs one task: it checks the workflow is not cancelled, records
/// the activity, heartbeats the task, runs the step, then completes or fails
/// the task and schedules the turn's next step.
pub struct TurnTaskDriver<S: TaskStore, H: TurnTaskHost> {
    store: Arc<S>,
    hosts: H,
    worker_id: String,
    heartbeat_interval: Duration,
    chain_steps: bool,
}

impl<S: TaskStore, H: TurnTaskHost> Clone for TurnTaskDriver<S, H> {
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

impl<S: TaskStore, H: TurnTaskHost> TurnTaskDriver<S, H> {
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
    S: TaskStore,
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
            None => store.get_workflow_status(wf_id).await,
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

    match result {
        Ok(output) => {
            // Complete the task (verifying ownership), draining wake signals
            // in the same call when this boundary is a drain point.
            let schedules = turn_input_opt.is_some() && task.workflow_id.is_some();
            let final_answer = reason_final_answer(activity, &output).unwrap_or(false);
            let drain = (schedules && drains_wake_signals_after(activity, final_answer))
                .then_some(crate::durable_turn::USER_MESSAGE);
            let complete_result = store
                .complete_task_and_drain(task, worker_id, output.clone(), drain)
                .await;

            match complete_result {
                Ok(drained) => {
                    info!(
                        task_id = %task.id,
                        activity_type = %task.activity_type,
                        "Task completed successfully"
                    );

                    // Schedule next activity if needed (only for workflow-bound tasks)
                    if let (Some(turn_input), Some(wf_id)) = (turn_input_opt, task.workflow_id) {
                        return schedule_next_activity(
                            store,
                            hosts,
                            wf_id,
                            activity,
                            &turn_input,
                            &output,
                            drained,
                            claim_for,
                        )
                        .await;
                    }
                }
                Err(e) => {
                    warn!(
                        task_id = %task.id,
                        error = %e,
                        "Task completion rejected - skipping next activity"
                    );
                }
            }
        }
        Err(e) => {
            fail_activity_task(store, hosts, task, turn_input_opt.as_ref(), &e).await?;

            return Err(e);
        }
    }

    Ok(None)
}

async fn fail_activity_task<S: TaskStore, H: TurnTaskHost>(
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

/// Schedule the next activity based on current activity completion.
///
/// Returns the next step when it was enqueued claimed for `claim_for`.
#[allow(clippy::too_many_arguments)]
async fn schedule_next_activity<S: TaskStore, H: TurnTaskHost>(
    store: &Arc<S>,
    hosts: &H,
    workflow_id: Uuid,
    completed_activity: &str,
    input: &DurableTurnInput,
    output: &serde_json::Value,
    drained: Option<usize>,
    claim_for: Option<&str>,
) -> Result<Option<ClaimedTask>> {
    let reason_final_answer = reason_final_answer(completed_activity, output)?;

    // Drain queued USER_MESSAGE steering signals (task wakes) at the boundaries
    // that precede another reason iteration. The already-persisted wake message
    // is picked up by that reason (it re-reads full history); consuming the
    // signal here is what governs turn continuation and, being destructive,
    // gives exactly-once delivery — see `drains_wake_signals_after`.
    let mut pending_user_message_count = match drained {
        Some(count) => count,
        None => {
            count_drained_wakes(store, workflow_id, completed_activity, reason_final_answer).await?
        }
    };
    if completed_activity == "reason" {
        let reason: ReasonResult = serde_json::from_value(output.clone())
            .map_err(|error| anyhow::anyhow!("Invalid reason output payload: {}", error))?;
        pending_user_message_count += hosts
            .after_reason(input, &reason, pending_user_message_count)
            .await?;
    }

    if completed_activity == "act" && pending_user_message_count > 0 {
        debug!(
            %workflow_id,
            pending_user_message_count,
            "delivering mid-turn task wake(s) at the act→reason boundary"
        );
    }

    let mut execution = crate::DurableExecution::new(input.clone());
    let plan = advance_host_execution(
        &hosts.host(),
        &mut execution,
        completed_activity,
        output,
        pending_user_message_count,
    )
    .await?;
    let checkpoint = execution.checkpoint();
    hosts.turn_planned(&checkpoint, &plan, output).await?;
    match plan {
        TurnPlan::ScheduleReason(_) => {
            return enqueue_reason_task(store, workflow_id, &checkpoint, claim_for).await;
        }
        TurnPlan::ScheduleAct(plan) => {
            return enqueue_act_task(store, workflow_id, &plan, &checkpoint, claim_for).await;
        }
        TurnPlan::Complete { stop_reason, error } => {
            let turn_output = turn_output_with_stop_reason(output.clone(), stop_reason);
            store
                .complete_workflow(
                    workflow_id,
                    turn_output,
                    Some(serde_json::to_value(&checkpoint)?),
                    error.map(crate::durable::WorkflowError::new),
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;
        }
        TurnPlan::WaitForToolResults { .. } => {
            store
                .complete_workflow(
                    workflow_id,
                    output.clone(),
                    Some(serde_json::to_value(&checkpoint)?),
                    None,
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to persist wait-for-tools state: {}", e))?;
        }
    }

    Ok(None)
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
/// Because `consume_pending_signals` is a destructive read, a wake drained at
/// the act boundary is not seen again by the end-of-turn drain — mid-turn XOR
/// next-turn, never both.
fn drains_wake_signals_after(completed_activity: &str, reason_final_answer: bool) -> bool {
    match completed_activity {
        "act" => true,
        "reason" => reason_final_answer,
        _ => false,
    }
}

/// Consume and count queued `USER_MESSAGE` wakes for `workflow_id` when this
/// activity boundary is a drain point (see [`drains_wake_signals_after`]).
/// Returns 0 without touching the store at non-drain boundaries.
async fn count_drained_wakes<S: TaskStore>(
    store: &Arc<S>,
    workflow_id: Uuid,
    completed_activity: &str,
    reason_final_answer: bool,
) -> Result<usize> {
    if !drains_wake_signals_after(completed_activity, reason_final_answer) {
        return Ok(0);
    }
    Ok(store
        .consume_pending_signals_by_type(workflow_id, crate::durable_turn::USER_MESSAGE)
        .await
        .map_err(|error| anyhow::anyhow!("Failed to consume workflow wake signals: {}", error))?
        .len())
}

async fn enqueue_reason_task<S: TaskStore>(
    store: &Arc<S>,
    workflow_id: Uuid,
    input: &DurableTurnInput,
    claim_for: Option<&str>,
) -> Result<Option<ClaimedTask>> {
    let activity_id = format!("reason_{}", Uuid::now_v7());
    let input_json = serde_json::to_value(input)?;
    enqueue_step(
        store,
        workflow_id,
        activity_id,
        "reason",
        input_json,
        claim_for,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))
}

/// Enqueue a turn step, claimed for `claim_for` when set.
async fn enqueue_step<S: TaskStore>(
    store: &Arc<S>,
    workflow_id: Uuid,
    activity_id: String,
    activity_type: &str,
    input: serde_json::Value,
    claim_for: Option<&str>,
) -> Result<Option<ClaimedTask>, crate::durable::StoreError> {
    let activity_type = activity_type.to_string();
    match claim_for {
        Some(worker_id) => {
            store
                .enqueue_claimed_task_and_record(
                    workflow_id,
                    activity_id,
                    activity_type,
                    input,
                    worker_id,
                )
                .await
        }
        None => store
            .enqueue_task_and_record(workflow_id, activity_id, activity_type, input)
            .await
            .map(|_| None),
    }
}

async fn enqueue_act_task<S: TaskStore>(
    store: &Arc<S>,
    workflow_id: Uuid,
    plan: &ActPlan,
    checkpoint: &DurableTurnInput,
    claim_for: Option<&str>,
) -> Result<Option<ClaimedTask>> {
    let act_input_json = act_task_input(plan, checkpoint)?;

    let activity_id = format!("act_{}", Uuid::now_v7());
    enqueue_step(
        store,
        workflow_id,
        activity_id,
        "act",
        act_input_json,
        claim_for,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to enqueue act task: {}", e))
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
        ActivityOptions, DurableAdmin, EventLog, HeartbeatResponse, StoreError, TaskDefinition,
        TaskQueue, WorkerInfo, WorkerRegistry, WorkflowError,
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

    /// `TaskStore` stub returning a fixed set of pending signals and counting
    /// how many times wake-signal drains are called.
    #[derive(Clone)]
    struct RecordingStore {
        signals: Vec<crate::durable::WorkflowSignal>,
        consume_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl TaskStore for RecordingStore {
        async fn register_worker(&self, _worker: WorkerInfo) -> Result<(), StoreError> {
            Ok(())
        }
        async fn worker_heartbeat(
            &self,
            _worker_id: &str,
            _current_load: usize,
            _accepting_tasks: bool,
        ) -> Result<(), StoreError> {
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
        async fn get_workflow_status(
            &self,
            _workflow_id: Uuid,
        ) -> Result<WorkflowStatus, StoreError> {
            Ok(WorkflowStatus::Running)
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

    #[tokio::test]
    async fn completion_drains_only_where_the_store_folds_it_in() {
        // `execute_task` asks the completion to drain at a final-answer reason
        // and after act; a store that cannot fold the drain in returns None so
        // `schedule_next_activity` consumes the wakes itself.
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

        let store = RecordingStore {
            signals: vec![user_message_signal()],
            consume_calls: Arc::new(AtomicUsize::new(0)),
        };
        let task = ClaimedTask {
            id: Uuid::now_v7(),
            workflow_id: Some(Uuid::now_v7()),
            activity_id: "act-1".into(),
            activity_type: "act".into(),
            input: serde_json::json!({}),
            attempt: 1,
            max_attempts: 1,
            ..Default::default()
        };
        let drained = store
            .complete_task_and_drain(&task, "w", serde_json::json!({}), Some("user_message"))
            .await
            .unwrap();
        assert_eq!(drained, None);
        assert_eq!(store.consume_calls.load(Ordering::SeqCst), 0);
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

    #[tokio::test]
    async fn act_boundary_drains_only_user_message_wakes() {
        // Two wakes plus an unrelated signal accrued during the turn.
        let store = Arc::new(RecordingStore {
            signals: vec![
                user_message_signal(),
                crate::durable::WorkflowSignal::new(
                    crate::durable::signal_types::CANCEL,
                    serde_json::json!({}),
                ),
                user_message_signal(),
            ],
            consume_calls: Arc::new(AtomicUsize::new(0)),
        });

        let count = count_drained_wakes(&store, Uuid::now_v7(), "act", false)
            .await
            .expect("act boundary drains");

        // Only USER_MESSAGE wakes are consumed; the cancel signal remains pending
        // for the normal signal dispatcher instead of being dropped.
        assert_eq!(count, 2);
        assert_eq!(store.consume_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn tool_calling_reason_does_not_drain_wakes() {
        let store = Arc::new(RecordingStore {
            signals: vec![user_message_signal()],
            consume_calls: Arc::new(AtomicUsize::new(0)),
        });

        // A reason that emitted tool calls (`reason_final_answer = false`) must
        // not consume signals — the following `act` boundary owns that drain,
        // so the wake is delivered exactly once.
        let count = count_drained_wakes(&store, Uuid::now_v7(), "reason", false)
            .await
            .expect("non-drain boundary");

        assert_eq!(count, 0);
        assert_eq!(
            store.consume_calls.load(Ordering::SeqCst),
            0,
            "must not touch the signal store at a non-drain boundary"
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

        let outcome = TaskStore::fail_task_and_record(&store, &task, "terminal", false)
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
