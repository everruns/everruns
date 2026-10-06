//! [`DurableRunner`] as a [`TurnBackend`]: the platform server's durable
//! turns through the framework's turn entry point.
//!
//! Execution behavior:
//! - The server calls this `TurnBackend` directly; there is no other entry
//!   point for its turns.
//! - Only stored input is served: [`TurnInput::StoredMessage`] and
//!   [`TurnInput::RecordedToolResults`]. The server persists the message or
//!   the tool resolution before it starts the turn, and the turn task reads
//!   it from the session store. The runner holds no session runtime to write
//!   input through, so `Message`, `ToolResults` and `ResumeInterrupted` fail
//!   with a configuration error; a framework session's input goes through
//!   [`DurableBackend`](crate::DurableBackend), which records it itself.
//! - A stored message needs the request's [`TurnScope`]: the runner cannot
//!   look the session's organization, harness and agent up. The turn's id is
//!   minted by its input step, as the platform's turns always were; the
//!   request's id only labels the ticket until the checkpoint names the
//!   turn.
//! - A stored message for a session whose workflow still runs joins that
//!   turn as a `USER_MESSAGE` signal, the server's steering, instead of
//!   failing as the trait describes for a second turn. The returned ticket
//!   then follows the running workflow.
//! - `start_turn` creates or claims the workflow and enqueues its first task
//!   before it returns. The ticket only observes, so a caller that drops it
//!   (the server usually does) changes nothing.
//! - The ticket wakes on the store's workflow-end signal
//!   ([`TurnStore::workflow_end_signal`]) when the store has one,
//!   re-reading the status every [`TICKET_FALLBACK_POLL_INTERVAL`] in case a
//!   wakeup is missed. The memory store has one: every path that ends a
//!   workflow (driver completion, task failure, cancel, dead task) writes the
//!   status through it, so its status writes are the one hook, and a
//!   [`DurableBackend`](crate::DurableBackend) turn reports back as soon as
//!   it ends. The PostgreSQL store has none, since another process may end
//!   the workflow, so its tickets poll every [`TICKET_POLL_INTERVAL`]; the
//!   server mostly drops those tickets.
//! - The request's steering handle is closed at start: durable turns take
//!   mid-turn input only as persisted messages plus wake signals.
//!
//! Ticket mapping, from the workflow's terminal status:
//! - `Completed`: `Ok`, with the stop reason and response text the
//!   completion event recorded (`EndTurn` when it recorded none, which is
//!   also how a turn parked on client-side tool results ends, as in
//!   process); a workflow error makes it `success: false`.
//! - Iterations and tool calls are the checkpoint's, less what a continued
//!   turn had counted before this run of it (a `TurnBaseline`), and a parked
//!   turn's checkpoint counts the reason it resumes with, which it has not
//!   run: the numbers an in-process result reports.
//! - `Failed`: `Ok` with `success: false` and `TurnStopReason::Error`; the
//!   driver already recorded the failure in the session log.
//! - `Cancelled`: `Err(AgentLoopError::Cancelled)`, as the trait requires.
//! - `ContinuedAsNew`: an error; turn workflows never roll over.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::host::{TurnBackend, TurnInput, TurnRequest, TurnScope, TurnTicket};
use everruns_core::turn::TurnStopReason;
use everruns_durable::{
    EventLog, RunStart, StoreError, WorkflowError, WorkflowEvent, WorkflowSignal, WorkflowStatus,
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::durable_runner::{DurableRunner, DurableTurnInput, DurableTurnOutput};
use crate::host::TurnResult;
use crate::turn_store::{TurnStore, WorkflowSnapshot};

/// How often a [`DurableRunner`] turn ticket re-reads its workflow status
/// on a store without a workflow-end signal.
///
/// Short enough that an awaited turn reports back promptly next to the
/// LLM latency it waits on; long enough that a waiting ticket costs one
/// status read per interval, not a busy loop on the store.
pub const TICKET_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How often a ticket re-reads its workflow status while it waits on the
/// store's workflow-end signal: only the fallback for a wakeup that never
/// comes, so long enough to cost nothing while a turn runs.
pub const TICKET_FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How many trailing workflow events to search for the latest
/// `WorkflowCompleted`. It is the last event a turn appends, so a short tail
/// finds it without loading a long session's whole history.
const COMPLETION_EVENT_TAIL: i32 = 8;

impl DurableRunner {
    /// Start a turn from a stored message, or steer the running one.
    async fn start_stored_message(
        &self,
        session_id: SessionId,
        scope: TurnScope,
        input_message_id: MessageId,
        request_id: Option<String>,
    ) -> anyhow::Result<()> {
        let TurnScope {
            org_id,
            harness_id,
            agent_id,
            ..
        } = scope;
        info!(
            org_id,
            session_id = %session_id,
            harness_id = %harness_id,
            ?agent_id,
            input_message_id = %input_message_id,
            "starting durable turn workflow"
        );

        let input = DurableTurnInput {
            org_id,
            session_id,
            harness_id,
            agent_id,
            input_message_id,
            turn_id: None,
            previous_response_id: None,
            iteration: 1,
            request_id,
            started_at: Some(Utc::now()),
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        let workflow_id = session_id.uuid();
        if !self.start_workflow(workflow_id, &input).await? {
            let signal = WorkflowSignal::new(
                crate::durable_turn::USER_MESSAGE,
                serde_json::json!({
                    "input_message_id": input_message_id.to_string(),
                    "org_id": org_id,
                    "harness_id": harness_id.to_string(),
                    "agent_id": agent_id.map(|id| id.to_string()),
                }),
            );
            if let Err(error) = self.store.send_signal(workflow_id, signal).await {
                warn!(
                    session_id = %session_id,
                    %error,
                    "failed to send steering signal"
                );
            }
        }
        Ok(())
    }

    /// Start a new run of `workflow_id` whose first task is the input step
    /// of `input`, and wake the workers. Returns `false`, starting nothing,
    /// when the workflow still runs a turn.
    pub(crate) async fn start_workflow(
        &self,
        workflow_id: Uuid,
        input: &DurableTurnInput,
    ) -> anyhow::Result<bool> {
        self.start_workflow_at(
            workflow_id,
            format!("input_{}", Uuid::now_v7()),
            "process_input",
            serde_json::to_value(input)?,
        )
        .await
    }

    /// Start a new run of `workflow_id` whose first task is `activity_type`
    /// with `input_json`, and wake the workers. Returns `false`, starting
    /// nothing, when the workflow still runs a turn.
    ///
    /// A turn usually starts at its input step; a turn resumed from the
    /// session log after a process exit starts at the act it was cut off in.
    pub(crate) async fn start_workflow_at(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: &str,
        input_json: serde_json::Value,
    ) -> anyhow::Result<bool> {
        // One atomic store call, no runner lock: concurrent sends to a
        // session elect one winner in the store, the rest steer its run.
        let started = self
            .store
            .start_turn(
                workflow_id,
                crate::durable_turn::TURN_WORKFLOW_TYPE,
                input_json,
                activity_id,
                activity_type.to_string(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start turn workflow: {e}"))?;
        if matches!(started, RunStart::Active) {
            return Ok(false);
        }

        self.notify_task_available(activity_type).await;
        Ok(true)
    }

    /// Continue the turn that parked on client-side tool results, from the
    /// checkpoint it saved, once the resolution `resolution_id` is stored.
    pub(crate) async fn resume_persisted_resolution(
        &self,
        session_id: SessionId,
        resolution_id: Uuid,
    ) -> anyhow::Result<()> {
        let workflow_id = session_id.uuid();
        let store = &self.store;

        let WorkflowSnapshot {
            status,
            output: result_json,
            ..
        } = store
            .get_workflow(workflow_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get workflow status: {e}"))?;

        let saved_value = result_json.ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot resume: workflow {workflow_id} has no saved turn input in result"
            )
        })?;
        let turn_input: DurableTurnInput = serde_json::from_value(saved_value)
            .map_err(|e| anyhow::anyhow!("Failed to parse saved turn input: {e}"))?;

        let input_json = serde_json::to_value(&turn_input)?;
        match status {
            WorkflowStatus::Completed => {
                store
                    .update_workflow_status(
                        workflow_id,
                        WorkflowStatus::Pending,
                        Some(serde_json::to_value(&turn_input)?),
                        None,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to reset workflow status: {e}"))?;
            }
            WorkflowStatus::Pending | WorkflowStatus::Running => {}
            _ => {
                return Err(anyhow::anyhow!(
                    "Cannot resume workflow {workflow_id} from status {status:?}"
                ));
            }
        }

        if let Err(error) = store
            .enqueue_task_and_record(
                workflow_id,
                crate::durable_turn::waiting_turn_resolution_activity_id(resolution_id),
                "reason".to_string(),
                input_json,
            )
            .await
        {
            let _ = store
                .update_workflow_status(
                    workflow_id,
                    WorkflowStatus::Completed,
                    Some(serde_json::to_value(&turn_input).unwrap_or_default()),
                    None,
                )
                .await;
            return Err(anyhow::anyhow!("Failed to enqueue reason task: {error}"));
        }
        self.notify_task_available("reason").await;

        Ok(())
    }

    /// Drop the session workflow's pending tasks and mark it cancelled with
    /// `message`; whether it was still running. The body of
    /// [`TurnBackend::cancel`], which a backend also uses to end a workflow
    /// another process left behind.
    pub(crate) async fn cancel_workflow(
        &self,
        session_id: SessionId,
        message: &str,
    ) -> Result<bool> {
        let workflow_id = session_id.uuid();
        let store = &self.store;
        let was_running = matches!(
            store.get_workflow(workflow_id).await,
            Ok(workflow) if !workflow.status.is_terminal()
        );
        store.cancel_pending_tasks(workflow_id).await.map_err(|e| {
            AgentLoopError::store(format!("Failed to cancel pending workflow tasks: {e}"))
        })?;
        let message = message.to_string();
        let output = DurableTurnOutput {
            session_id,
            success: false,
            error: Some(message.clone()),
            stop_reason: TurnStopReason::Cancelled,
        };
        let output = serde_json::to_value(output)
            .map_err(|e| AgentLoopError::store(format!("Failed to encode turn output: {e}")))?;
        store
            .update_workflow_status(
                workflow_id,
                WorkflowStatus::Cancelled,
                Some(output),
                Some(WorkflowError::new(message)),
            )
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to cancel workflow: {e}")))?;
        Ok(was_running)
    }

    /// A ticket that resolves once the session's workflow reaches a
    /// terminal status.
    pub(crate) fn ticket(&self, session_id: SessionId, turn_id: TurnId) -> TurnTicket {
        self.ticket_after(session_id, turn_id, TurnBaseline::default())
    }

    /// A ticket for a turn continued from `baseline`: its result counts only
    /// the steps this run of the turn takes.
    pub(crate) fn ticket_after(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        baseline: TurnBaseline,
    ) -> TurnTicket {
        let store = Arc::clone(&self.store);
        TurnTicket::new(
            session_id,
            turn_id,
            await_workflow(store, session_id.uuid(), turn_id, baseline),
        )
    }
}

#[async_trait]
impl TurnBackend for DurableRunner {
    /// Start the turn on the durable queue and return a ticket that
    /// resolves when it ends. Serves only stored input; see the module notes
    /// for the scope, steering and mapping rules.
    async fn start_turn(&self, request: TurnRequest) -> Result<TurnTicket> {
        let TurnRequest {
            session_id,
            turn_id,
            input,
            steering,
            scope,
            request_id,
            ..
        } = request;
        steering.close();
        match input {
            TurnInput::StoredMessage { message_id } => {
                let scope = scope.ok_or_else(|| {
                    AgentLoopError::config(
                        "the durable runner needs TurnRequest::scope to start a turn from \
                         TurnInput::StoredMessage; it cannot look the session up",
                    )
                })?;
                self.start_stored_message(session_id, scope, message_id, request_id)
                    .await
            }
            TurnInput::RecordedToolResults { resolution_id } => {
                self.resume_persisted_resolution(session_id, resolution_id)
                    .await
            }
            other => return Err(unsupported_input(&other)),
        }
        .map_err(store_error)?;
        Ok(self.ticket(session_id, turn_id))
    }

    /// Cancel the session's workflow: drop its pending tasks and mark it
    /// cancelled. A step already executing finishes, but the driver checks
    /// the status before every task, so the turn takes no further step.
    ///
    /// The workflow is marked cancelled even when it had already ended, as
    /// the server's cancel always did; the returned flag says whether it was
    /// still running.
    async fn cancel(&self, session_id: SessionId) -> Result<bool> {
        self.cancel_workflow(session_id, "User requested cancellation")
            .await
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        let workflow_id = session_id.uuid();
        let store = &self.store;
        match store.get_workflow(workflow_id).await {
            Ok(workflow) => !workflow.status.is_terminal(),
            Err(_) => false,
        }
    }

    async fn active_count(&self) -> usize {
        let store = &self.store;
        store.count_active_workflows().await.unwrap_or_default()
    }
}

fn unsupported_input(input: &TurnInput) -> AgentLoopError {
    let name = match input {
        TurnInput::Message(_) => "TurnInput::Message",
        TurnInput::ResumeInterrupted => "TurnInput::ResumeInterrupted",
        TurnInput::ToolResults(_) => "TurnInput::ToolResults",
        _ => "this turn input",
    };
    AgentLoopError::config(format!(
        "the durable runner cannot run {name}; it has no session runtime to record input \
         through and serves only stored input (TurnInput::StoredMessage, \
         TurnInput::RecordedToolResults)"
    ))
}

/// Carry a store failure's full message.
fn store_error(error: anyhow::Error) -> AgentLoopError {
    AgentLoopError::store(format!("{error:#}"))
}

/// The output of the workflow's latest `WorkflowCompleted` event, read from
/// the tail of its event log.
pub(crate) async fn latest_completion_output<S: EventLog + ?Sized>(
    store: &S,
    workflow_id: Uuid,
) -> std::result::Result<Option<serde_json::Value>, StoreError> {
    // Event sequence numbers count from zero, so `count` is the next one.
    let count = i32::try_from(store.count_events(workflow_id).await?).unwrap_or(i32::MAX);
    let events = store
        .load_events_after(workflow_id, count.saturating_sub(COMPLETION_EVENT_TAIL + 1))
        .await?;
    Ok(events.into_iter().rev().find_map(|(_, event)| match event {
        WorkflowEvent::WorkflowCompleted { result } => Some(result),
        _ => None,
    }))
}

/// What a continued turn had already counted when this run of it began.
///
/// An in-process turn reports the reasons and tool calls of the run that
/// returns its result, so a turn continued after parking or a process exit
/// counts only what it ran since. The checkpoint counts the whole turn; a
/// ticket subtracts this from it to report the same numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TurnBaseline {
    /// Reasons the turn ran before this run.
    pub(crate) iterations: u32,
    /// Tool calls the checkpoint counts that this run's acts do not run.
    pub(crate) tool_calls: u32,
}

/// What a ticket saw when its workflow ended.
struct EndedWorkflow {
    status: WorkflowStatus,
    /// The stored workflow result: the turn checkpoint.
    checkpoint: Option<serde_json::Value>,
    error: Option<String>,
    /// The latest completion event's output, for a completed workflow.
    completion: Option<serde_json::Value>,
}

/// Wait until `workflow_id` ends, then map the end to a turn result.
async fn await_workflow(
    store: Arc<dyn TurnStore>,
    workflow_id: Uuid,
    turn_id: TurnId,
    baseline: TurnBaseline,
) -> Result<TurnResult> {
    loop {
        let (ended, end_signal) = {
            // Subscribe before reading the status, so an end that lands
            // between the read and the wait still wakes this ticket.
            let end_signal = store.workflow_end_signal(workflow_id);
            let WorkflowSnapshot {
                status,
                output: checkpoint,
                error,
            } = store
                .get_workflow(workflow_id)
                .await
                .map_err(|error| store_error(error.into()))?;
            if status.is_terminal() {
                let completion = if status == WorkflowStatus::Completed {
                    store
                        .latest_completion_output(workflow_id)
                        .await
                        .unwrap_or_else(|error| {
                            warn!(%workflow_id, %error, "failed to read turn completion event");
                            None
                        })
                } else {
                    None
                };
                let ended = EndedWorkflow {
                    status,
                    checkpoint,
                    error,
                    completion,
                };
                (Some(ended), None)
            } else {
                (None, end_signal)
            }
        };
        if let Some(ended) = ended {
            return turn_result(ended, turn_id, baseline);
        }
        match end_signal {
            // Woken or timed out, the next status read decides.
            Some(end_signal) => {
                let _ = tokio::time::timeout(TICKET_FALLBACK_POLL_INTERVAL, end_signal).await;
            }
            None => tokio::time::sleep(TICKET_POLL_INTERVAL).await,
        }
    }
}

/// Map an ended workflow to the ticket's result; see the module notes.
fn turn_result(
    ended: EndedWorkflow,
    ticket_turn_id: TurnId,
    baseline: TurnBaseline,
) -> Result<TurnResult> {
    let checkpoint = ended
        .checkpoint
        .and_then(|value| serde_json::from_value::<DurableTurnInput>(value).ok());
    let turn_id = checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.turn_id)
        .unwrap_or(ticket_turn_id);
    // A parked turn's checkpoint is the state its next reason resumes from,
    // one iteration past the reasons it ran; only a pause completes the
    // workflow without a stop reason.
    let parked = ended.status == WorkflowStatus::Completed
        && ended.completion.as_ref().is_some_and(|completion| {
            completion.get("stop_reason").is_none()
                && completion
                    .get("waiting_for_tool_results")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
        });
    let iterations = checkpoint.as_ref().map_or(0, |checkpoint| {
        checkpoint
            .iteration
            .saturating_sub(u32::from(parked))
            .saturating_sub(baseline.iterations) as usize
    });
    let failed = |error: Option<String>, stop_reason| TurnResult {
        response: String::new(),
        iterations,
        tool_calls_count: 0,
        success: false,
        error: Some(error.unwrap_or_else(|| "durable turn failed".to_string())),
        stop_reason,
        turn_id,
    };

    match ended.status {
        WorkflowStatus::Cancelled => Err(AgentLoopError::Cancelled),
        WorkflowStatus::Failed => Ok(failed(ended.error, TurnStopReason::Error)),
        WorkflowStatus::Completed => {
            let completion = ended.completion.unwrap_or_default();
            let stop_reason = completion
                .get("stop_reason")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or(TurnStopReason::EndTurn);
            if let Some(error) = ended.error {
                let message = completion
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .map_or(error, str::to_string);
                return Ok(failed(Some(message), stop_reason));
            }
            let response = completion
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.final_answer_preview.clone())
                })
                .unwrap_or_default();
            Ok(TurnResult {
                response,
                iterations,
                tool_calls_count: checkpoint.as_ref().map_or(0, |checkpoint| {
                    checkpoint
                        .tool_call_count
                        .saturating_sub(baseline.tool_calls) as usize
                }),
                success: true,
                error: None,
                stop_reason,
                turn_id,
            })
        }
        status => Err(AgentLoopError::store(format!(
            "the session's turn workflow ended as {status:?}, which a turn never does"
        ))),
    }
}
