//! [`DurableRunner`] as a [`TurnBackend`]: the server's durable turns through
//! the framework's turn execution seam.
//!
//! Decisions (see `knowledge/framework/execution-backends.md`):
//! - The logic that starts, resumes and cancels a durable turn lives here, in
//!   the `TurnBackend` implementation. `AgentRunner` is a shim over it
//!   (`crate::runner`) until the server calls the seam directly.
//! - Only [`TurnInput::Persisted`] is served: the server persists the message
//!   or the tool resolution before it starts the turn, and the turn task
//!   reads it from the session store. Unpersisted input (`Message`,
//!   `ToolResults`) and `ResumeInterrupted` fail with a configuration error;
//!   a framework session's messages go through
//!   [`DurableBackend`](crate::DurableBackend), which persists them itself.
//! - A persisted message for a session whose workflow still runs joins that
//!   turn as a `USER_MESSAGE` signal, the server's steering, instead of
//!   failing as the trait describes for a second turn. The returned ticket
//!   then follows the running workflow.
//! - `start_turn` creates or claims the workflow and enqueues its first task
//!   before it returns. The ticket only observes, so a caller that drops it
//!   (the `AgentRunner` shim always does) changes nothing.
//! - The ticket polls the workflow status every [`TICKET_POLL_INTERVAL`]. A
//!   push notification can replace the poll once more than one process waits
//!   on tickets.
//! - The request's steering handle is closed at start: durable turns take
//!   mid-turn input only as persisted messages plus wake signals.
//!
//! Ticket mapping, from the workflow's terminal status:
//! - `Completed`: `Ok`, with the stop reason and response text the
//!   completion event recorded (`EndTurn` when it recorded none, which is
//!   also how a turn parked on client-side tool results ends, as in
//!   process); a workflow error makes it `success: false`.
//! - `Failed`: `Ok` with `success: false` and `TurnStopReason::Error`; the
//!   driver already recorded the failure in the session log.
//! - `Cancelled`: `Err(AgentLoopError::Cancelled)`, as the trait requires.
//! - `ContinuedAsNew`: an error; turn workflows never roll over.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::host::{PersistedTurn, TurnBackend, TurnInput, TurnRequest, TurnTicket};
use everruns_core::turn::TurnStopReason;
use everruns_durable::{EventLog, WorkflowEvent, WorkflowSignal, WorkflowStatus};
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

use crate::durable_runner::{
    DurableRunner, DurableStoreBackend, DurableTurnInput, DurableTurnOutput,
};
use crate::host::TurnResult;

/// How often a [`DurableRunner`] turn ticket re-reads its workflow status.
///
/// Short enough that an awaited turn reports back promptly next to the
/// LLM latency it waits on; long enough that a waiting ticket costs one
/// status read per interval, not a busy loop on the store.
pub const TICKET_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How many trailing workflow events to search for the latest
/// `WorkflowCompleted`. It is the last event a turn appends, so a short tail
/// finds it without loading a long session's whole history.
const COMPLETION_EVENT_TAIL: i32 = 8;

impl DurableRunner {
    /// Start a turn from a persisted message, or steer the running one.
    async fn start_persisted_message(
        &self,
        session_id: SessionId,
        org_id: i64,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        input_message_id: MessageId,
        request_id: Option<String>,
    ) -> anyhow::Result<()> {
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
            if let Err(error) = self
                .store
                .lock()
                .await
                .send_signal(workflow_id, signal)
                .await
            {
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
        let input_json = serde_json::to_value(input)?;
        {
            let mut store = self.store.lock().await;

            match store.try_claim_workflow_for_new_turn(workflow_id).await {
                Ok(true) => {
                    if let Err(error) = store
                        .enqueue_task(
                            workflow_id,
                            format!("input_{}", Uuid::now_v7()),
                            "process_input".to_string(),
                            input_json,
                        )
                        .await
                    {
                        let _ = store
                            .update_workflow_status(
                                workflow_id,
                                WorkflowStatus::Completed,
                                None,
                                None,
                            )
                            .await;
                        return Err(anyhow::anyhow!("Failed to enqueue task: {error}"));
                    }
                }
                Ok(false) => return Ok(false),
                Err(error) => {
                    let err = error.to_string();
                    if !err.contains("not found") && !err.contains("NOT_FOUND") {
                        return Err(anyhow::anyhow!("Failed to check workflow status: {error}"));
                    }

                    let activity_id = format!("input_{}", Uuid::now_v7());
                    store
                        .start_workflow_with_task(
                            workflow_id,
                            "turn_workflow",
                            input_json,
                            activity_id.clone(),
                            "process_input".to_string(),
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to start workflow: {e}"))?;
                }
            }
        }

        self.notify_task_available("process_input").await;
        Ok(true)
    }

    /// Continue the turn that parked on client-side tool results, from the
    /// checkpoint it saved, once the resolution `resolution_id` is stored.
    async fn resume_persisted_resolution(
        &self,
        session_id: SessionId,
        resolution_id: Uuid,
    ) -> anyhow::Result<()> {
        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;

        let (status, result_json, _) = store
            .get_workflow_status(workflow_id)
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
            .enqueue_task(
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
        drop(store);
        self.notify_task_available("reason").await;

        Ok(())
    }

    /// A ticket that resolves once the session's workflow reaches a
    /// terminal status.
    pub(crate) fn ticket(&self, session_id: SessionId, turn_id: TurnId) -> TurnTicket {
        let store = Arc::clone(&self.store);
        TurnTicket::new(
            session_id,
            turn_id,
            await_workflow(store, session_id.uuid(), turn_id),
        )
    }
}

#[async_trait]
impl TurnBackend for DurableRunner {
    /// Start the turn on the durable queue and return a ticket that polls
    /// for its end. Serves only [`TurnInput::Persisted`]; see the module
    /// notes for the steering and mapping rules.
    async fn start_turn(&self, request: TurnRequest) -> Result<TurnTicket> {
        let TurnRequest {
            session_id,
            turn_id,
            input,
            steering,
            ..
        } = request;
        steering.close();
        let persisted = match input {
            TurnInput::Persisted(persisted) => *persisted,
            other => return Err(unsupported_input(&other)),
        };
        match persisted {
            PersistedTurn::Message {
                org_id,
                harness_id,
                agent_id,
                input_message_id,
                request_id,
            } => {
                self.start_persisted_message(
                    session_id,
                    org_id,
                    harness_id,
                    agent_id,
                    input_message_id,
                    request_id,
                )
                .await
            }
            PersistedTurn::ToolResolution { resolution_id } => {
                self.resume_persisted_resolution(session_id, resolution_id)
                    .await
            }
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
        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;
        let was_running = matches!(
            store.get_workflow_status(workflow_id).await,
            Ok((status, _, _)) if !status.is_terminal()
        );
        store.cancel_pending_tasks(workflow_id).await.map_err(|e| {
            AgentLoopError::store(format!("Failed to cancel pending workflow tasks: {e}"))
        })?;
        let message = "User requested cancellation".to_string();
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
                Some(message),
            )
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to cancel workflow: {e}")))?;
        Ok(was_running)
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;
        match store.get_workflow_status(workflow_id).await {
            Ok((status, _, _)) => !status.is_terminal(),
            Err(_) => false,
        }
    }

    async fn active_count(&self) -> usize {
        let mut store = self.store.lock().await;
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
        "the durable runner cannot run {name} yet; it serves only server-persisted input \
         (TurnInput::Persisted)"
    ))
}

/// Carry a store failure's full message. The `AgentRunner` shim unwraps it
/// back into the message the server always logged.
fn store_error(error: anyhow::Error) -> AgentLoopError {
    AgentLoopError::store(format!("{error:#}"))
}

/// The output of the workflow's latest `WorkflowCompleted` event, read from
/// the tail of its event log.
pub(crate) async fn latest_completion_output<S: EventLog + ?Sized>(
    store: &S,
    workflow_id: Uuid,
) -> anyhow::Result<Option<serde_json::Value>> {
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

/// What a ticket saw when its workflow ended.
struct EndedWorkflow {
    status: WorkflowStatus,
    /// The stored workflow result: the turn checkpoint.
    checkpoint: Option<serde_json::Value>,
    error: Option<String>,
    /// The latest completion event's output, for a completed workflow.
    completion: Option<serde_json::Value>,
}

/// Poll `workflow_id` until it ends, then map the end to a turn result.
async fn await_workflow(
    store: Arc<Mutex<dyn DurableStoreBackend>>,
    workflow_id: Uuid,
    turn_id: TurnId,
) -> Result<TurnResult> {
    loop {
        let ended = {
            let mut store = store.lock().await;
            let (status, checkpoint, error) = store
                .get_workflow_status(workflow_id)
                .await
                .map_err(store_error)?;
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
                Some(EndedWorkflow {
                    status,
                    checkpoint,
                    error,
                    completion,
                })
            } else {
                None
            }
        };
        if let Some(ended) = ended {
            return turn_result(ended, turn_id);
        }
        tokio::time::sleep(TICKET_POLL_INTERVAL).await;
    }
}

/// Map an ended workflow to the ticket's result; see the module notes.
fn turn_result(ended: EndedWorkflow, ticket_turn_id: TurnId) -> Result<TurnResult> {
    let checkpoint = ended
        .checkpoint
        .and_then(|value| serde_json::from_value::<DurableTurnInput>(value).ok());
    let turn_id = checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.turn_id)
        .unwrap_or(ticket_turn_id);
    let iterations = checkpoint
        .as_ref()
        .map_or(0, |checkpoint| checkpoint.iteration as usize);
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
                tool_calls_count: checkpoint
                    .as_ref()
                    .map_or(0, |checkpoint| checkpoint.tool_call_count as usize),
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
