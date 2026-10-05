//! The start of a turn: input, then the first reason, in one task.
//!
//! Decision: the `process_input` task runs the turn's first reason itself
//! instead of completing, enqueueing a `reason` task and waiting for a worker
//! to claim it. That hand-off cost a completion, an enqueue, a claim and a
//! second round of setup reads before the first model call (about 230 ms in
//! production). The reason's setup reads start before the input runs, so the
//! two overlap.
//!
//! The task completes with the reason's output and is scheduled exactly as a
//! completed `reason` would be, so every later step is unchanged. The turn id
//! is derived from the task id, so a retried task restarts the same turn
//! rather than opening a second one. Servers keep enqueueing `process_input`;
//! workers without this change run it the old way.

use crate::core::ExecutionContext;
use crate::durable_runner::DurableTurnInput;
use crate::engine::{InputAtomInput, ReasonInput, TurnPlan};
use crate::host::{
    advance_host_execution, execute_input_activity as runtime_execute_input_activity,
    execute_reason_activity_with_prompt_messages as runtime_execute_reason_activity_with_prompt_messages,
};
use crate::task_heartbeat::CancelSignals;
use crate::turn_driver::TurnTaskHost;
use anyhow::Result;
use everruns_contracts::typed_id::{ExecId, TurnId};
use tracing::debug;
use uuid::Uuid;

/// Run a turn's input and its first reason.
///
/// Returns the reason's output with the turn state it ran from. That state
/// carries the turn id once the input succeeded, so a failure in the reason
/// can still fail the turn by id.
pub(crate) async fn execute_turn_start<H: TurnTaskHost>(
    hosts: &H,
    input: &DurableTurnInput,
    task_id: Uuid,
    cancel: CancelSignals,
) -> (Result<serde_json::Value>, DurableTurnInput) {
    let turn_id = turn_id_for(input, task_id);
    // Start the reason's setup reads now so they overlap the input.
    let reason_host = reason_host(hosts, input, cancel, turn_id);

    let checkpoint = match plan_first_reason(hosts, input, turn_id).await {
        Ok(checkpoint) => checkpoint,
        Err(error) => return (Err(error), input.clone()),
    };
    let output = run_reason(hosts, reason_host, &checkpoint).await;
    (output, checkpoint)
}

/// The turn a `process_input` task opens. Keyed by the task, which keeps its id
/// across retries, so a retry restarts the same turn.
fn turn_id_for(input: &DurableTurnInput, task_id: Uuid) -> TurnId {
    input.turn_id.unwrap_or_else(|| TurnId::from_uuid(task_id))
}

/// Run the input step and plan the reason that follows it.
async fn plan_first_reason<H: TurnTaskHost>(
    hosts: &H,
    input: &DurableTurnInput,
    turn_id: TurnId,
) -> Result<DurableTurnInput> {
    let input_output = execute_input_activity(hosts, input, turn_id).await?;
    let mut execution = crate::DurableExecution::new(input.clone());
    let plan = advance_host_execution(
        &hosts.host(),
        &mut execution,
        "process_input",
        &input_output,
        0,
    )
    .await?;
    // The engine always follows the input with a reason.
    anyhow::ensure!(
        matches!(plan, TurnPlan::ScheduleReason(_)),
        "process_input planned no reason step"
    );
    Ok(execution.checkpoint())
}

/// Execute the input step of a turn.
async fn execute_input_activity<H: TurnTaskHost>(
    hosts: &H,
    input: &DurableTurnInput,
    turn_id: TurnId,
) -> Result<serde_json::Value> {
    debug!(session_id = %input.session_id, "Executing input activity");

    let context = ExecutionContext {
        // Input/Reason atoms do not key file I/O by ExecutionContext; the act
        // path receives its workspace-set context from the orchestration.
        workspace_id: None,
        session_id: input.session_id,
        turn_id,
        input_message_id: input.input_message_id,
        exec_id: ExecId::new(),
    };
    let result =
        runtime_execute_input_activity(&hosts.host(), input.org_id, InputAtomInput { context })
            .await?;

    // Include turn_id in output for propagation
    let mut output = serde_json::to_value(&result)?;
    if let serde_json::Value::Object(ref mut map) = output {
        map.insert(
            "turn_id".to_string(),
            serde_json::json!(turn_id.to_string()),
        );
    }
    Ok(output)
}

/// Execute a reasoning step (LLM call).
pub(crate) async fn execute_reason_activity<H: TurnTaskHost>(
    hosts: &H,
    input: &DurableTurnInput,
    cancel: CancelSignals,
) -> Result<serde_json::Value> {
    let host = reason_host(hosts, input, cancel, input.turn_id.unwrap_or_default());
    run_reason(hosts, host, input).await
}

/// A reason host with its setup reads already started. The reads depend only
/// on the session, harness, agent and input message, not on the turn state.
fn reason_host<H: TurnTaskHost>(
    hosts: &H,
    input: &DurableTurnInput,
    cancel: CancelSignals,
    turn_id: TurnId,
) -> H::Host {
    hosts.reason_host(&reason_input(input, turn_id), cancel)
}

fn reason_input(input: &DurableTurnInput, turn_id: TurnId) -> ReasonInput {
    ReasonInput {
        context: ExecutionContext {
            // Input/Reason atoms do not key file I/O by ExecutionContext.
            workspace_id: None,
            session_id: input.session_id,
            turn_id,
            input_message_id: input.input_message_id,
            exec_id: ExecId::new(),
        },
        harness_id: input.harness_id,
        agent_id: input.agent_id,
        org_id: input.org_id,
        mcp_tool_definitions: vec![],
        previous_response_id: input.previous_response_id.clone(),
        iteration: input.iteration,
    }
}

async fn run_reason<H: TurnTaskHost>(
    hosts: &H,
    host: H::Host,
    input: &DurableTurnInput,
) -> Result<serde_json::Value> {
    debug!(
        session_id = %input.session_id,
        turn_id = ?input.turn_id,
        "Executing reason activity"
    );
    let reason_input = reason_input(input, input.turn_id.unwrap_or_default());
    // The prompt messages `execute_reason_activity` would pick (the turn's
    // input on its first iteration), then any steering the host delivers at
    // this boundary, as the in-process turn loop orders them.
    let mut prompt_message_ids: Vec<_> = (input.iteration <= 1)
        .then_some(input.input_message_id)
        .into_iter()
        .collect();
    prompt_message_ids.extend(hosts.before_reason(input).await?);
    let result = runtime_execute_reason_activity_with_prompt_messages(
        &host,
        input.org_id,
        reason_input,
        prompt_message_ids,
    )
    .await;
    hosts.phase_finished(&host).await;
    let result = result?;

    // Turn lifecycle events (turn.completed, turn.failed, session.idled) are NOT
    // emitted here. They are deferred to the workflow scheduler which checks for
    // pending steering signals before deciding whether the turn is truly done.
    // This ensures turn.completed is emitted exactly once and prevents the
    // idle→active flicker when steering continues the turn.
    Ok(serde_json::to_value(&result)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::{HarnessId, MessageId, SessionId};

    fn input(turn_id: Option<TurnId>) -> DurableTurnInput {
        DurableTurnInput {
            org_id: 1,
            session_id: SessionId::new(),
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id: MessageId::new(),
            turn_id,
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
    fn a_retried_task_reopens_the_same_turn() {
        let task_id = Uuid::now_v7();
        let first = turn_id_for(&input(None), task_id);
        assert_eq!(first, turn_id_for(&input(None), task_id));
        assert_ne!(first, turn_id_for(&input(None), Uuid::now_v7()));
    }

    #[test]
    fn a_turn_id_already_on_the_input_wins() {
        let existing = TurnId::new();
        assert_eq!(
            turn_id_for(&input(Some(existing)), Uuid::now_v7()),
            existing
        );
    }
}
