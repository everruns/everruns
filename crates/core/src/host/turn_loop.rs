//! The loop that drives one turn, from its entry plan to completion or a
//! park, generic over the host.
//!
//! Decision: the loop is host-generic. Every reason-vs-act-vs-complete choice
//! comes from [`crate::engine`]; the loop only runs the step each plan names
//! through the generic activity functions and performs the lifecycle effects
//! the engine returns. What differs between hosts is where a turn's input
//! comes from and where a park is kept, so that is the one port the loop
//! takes: a [`TurnInbox`]. The in-process runtime implements it over its
//! [`TurnSteering`](crate::host::TurnSteering) queue, its session wake queue
//! and its in-memory parked turns; a worker running whole turns on a session
//! lease implements it over its own queue.
//!
//! Decision: only a reason that would otherwise finish the turn may close the
//! inbox, and the check for pending input and the close are one atomic step
//! ([`TurnInbox::after_reason`]). A send is therefore always either steering
//! for this turn or the start of the next one, never lost between the two.

use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::error::Result;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId, WorkspaceId};

use crate::ExecutionContext;
use crate::engine::{
    ActivityOutcome, Execution, HostFacts, ReasonInput, TurnPlan, TurnState, reason_schedules_act,
};
use crate::host::InProcessExecution;
use crate::host::runtime_host::{
    RuntimeHostAdapter, execute_act_activity, execute_reason_activity_with_prompt_messages,
};
use crate::host::turn_strategy::{self, resolve_pause_hints};
use crate::turn::TurnStopReason;

/// Summary of one driven turn, as [`drive_turn`] returns it.
#[derive(Debug, Clone)]
pub struct TurnResult {
    /// Final text response produced by the turn.
    pub response: String,
    /// Number of reason iterations executed.
    pub iterations: usize,
    /// Total number of tool calls executed during the turn.
    pub tool_calls_count: usize,
    /// Whether the turn completed without an unrecoverable failure.
    pub success: bool,
    /// Failure message when `success` is false.
    pub error: Option<String>,
    /// Structured reason the turn stopped.
    pub stop_reason: TurnStopReason,
    /// Turn identifier used to correlate emitted events.
    pub turn_id: TurnId,
}

/// The identity of the turn [`drive_turn`] runs: what its steps are
/// addressed to, fixed by the turn's entry step.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TurnDrive {
    /// The session the turn runs in.
    pub session_id: SessionId,
    /// The host's internal org id the turn's steps run under.
    pub org_id: i64,
    /// The turn's id, carried by every event it emits.
    pub turn_id: TurnId,
    /// The message that started the turn.
    pub input_message_id: MessageId,
    /// The harness the turn resolves its configuration from.
    pub harness_id: HarnessId,
    /// The agent the turn runs, when the session has one.
    pub agent_id: Option<AgentId>,
    /// The workspace the turn's tools run against.
    pub workspace_id: WorkspaceId,
}

impl TurnDrive {
    /// A turn of `session_id` with the given identity.
    pub fn new(
        session_id: SessionId,
        org_id: i64,
        turn_id: TurnId,
        input_message_id: MessageId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        workspace_id: WorkspaceId,
    ) -> Self {
        Self {
            session_id,
            org_id,
            turn_id,
            input_message_id,
            harness_id,
            agent_id,
            workspace_id,
        }
    }

    /// The turn `state` carries (after its input step), running against
    /// `workspace_id`. A state without a turn id gets a fresh one.
    pub fn for_state(state: &TurnState, workspace_id: WorkspaceId) -> Self {
        Self::new(
            state.session_id,
            state.org_id,
            state.turn_id.unwrap_or_default(),
            state.input_message_id,
            state.harness_id,
            state.agent_id,
            workspace_id,
        )
    }
}

/// Input still pending after a reason, as [`TurnInbox::after_reason`]
/// reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PendingInput {
    /// How many inputs are waiting for the turn. A non-zero count keeps a
    /// turn that would otherwise finish going for another reason.
    pub count: usize,
    /// Ids of the inputs already persisted for the next reason, whose
    /// `user_prompt_submit` hooks that reason applies. Inputs counted but
    /// not taken yet arrive through the next [`TurnInbox::take`].
    pub prompt_message_ids: Vec<MessageId>,
}

impl PendingInput {
    /// `count` pending inputs, of which `prompt_message_ids` are persisted.
    pub fn new(count: usize, prompt_message_ids: Vec<MessageId>) -> Self {
        Self {
            count,
            prompt_message_ids,
        }
    }
}

/// Where a running turn's input comes from and where its park is kept: the
/// one host port [`drive_turn`] takes.
///
/// Input reaches a turn only at a reason boundary. The loop calls
/// [`take`](Self::take) before every reason, [`after_reason`](Self::after_reason)
/// after every reason, and ends the turn through [`close`](Self::close)
/// (it completed) or [`park`](Self::park) (it waits for tool results).
#[async_trait]
pub trait TurnInbox: Send + Sync {
    /// Take every input that arrived since the last reason boundary, persist
    /// it in the session's log if the host has not already, and return the
    /// ids of its messages. The next reason runs prompt hooks on them.
    async fn take(&self, session_id: SessionId) -> Result<Vec<MessageId>>;

    /// Report the input pending after a reason.
    ///
    /// `would_finish` is true when the reason would otherwise end the turn:
    /// it scheduled no act and the turn may still continue. Then, and only
    /// then, the inbox takes its pending input or, when there is none,
    /// closes, in one atomic step, so any later input belongs to the next
    /// turn. Otherwise it only counts input it must deliver at the next
    /// boundary; the count still decides the stop reason of a turn that hit
    /// its iteration cap.
    async fn after_reason(&self, session_id: SessionId, would_finish: bool)
    -> Result<PendingInput>;

    /// Stop accepting input for this turn: it is ending. Later input
    /// belongs to the next turn.
    async fn close(&self, session_id: SessionId) -> Result<()>;

    /// The turn parked waiting for the results of `client_tool_calls` (empty
    /// when a reason parked it) and continues from `resume` once they are
    /// recorded. Close the inbox, persist input it accepted that no reason
    /// will see now, and record the park where the host resumes it from.
    async fn park(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        client_tool_calls: Vec<ToolCall>,
        resume: TurnState,
    ) -> Result<()>;
}

/// Drive one turn from the plan its entry step produced (a new input's
/// input step, a resume after tool results, or a resume from the log) until
/// it completes or parks.
///
/// Generic over the host: every step runs through the shared activity
/// functions on `adapter`, and the turn's input arrives through `inbox`.
pub async fn drive_turn<A: RuntimeHostAdapter>(
    adapter: &A,
    inbox: &dyn TurnInbox,
    drive: TurnDrive,
    mut execution: InProcessExecution,
    mut plan: TurnPlan,
) -> Result<TurnResult> {
    let TurnDrive {
        session_id,
        org_id,
        turn_id,
        input_message_id,
        harness_id,
        agent_id,
        workspace_id,
    } = drive;
    let base_context = |exec: bool| {
        let context = ExecutionContext::new(session_id, turn_id, input_message_id)
            .with_workspace_id(workspace_id);
        if exec { context.next_exec() } else { context }
    };

    // Host-side bookkeeping for the returned `TurnResult`. These are
    // summaries of what the host executed, not inputs to any decision.
    let mut iterations: usize = 0;
    let mut tool_calls_count: usize = 0;
    let mut reply = crate::conversation::TurnReply::default();
    let mut pending_prompt_message_ids = Vec::new();
    // The client-side calls the last act left for the caller to run.
    let mut client_tool_calls = Vec::new();

    loop {
        match plan {
            TurnPlan::ScheduleReason(_) => {
                let state = execution.state().clone();
                let mut prompt_message_ids = std::mem::take(&mut pending_prompt_message_ids);
                // Iteration boundary: input that arrived during the previous
                // step (or, for wakes, while the session was idle) joins this
                // reason before the LLM call (EVE-681, part A).
                prompt_message_ids.extend(inbox.take(session_id).await?);
                if state.iteration == 1 {
                    prompt_message_ids.insert(0, input_message_id);
                }
                let reason_result = execute_reason_activity_with_prompt_messages(
                    adapter,
                    org_id,
                    ReasonInput {
                        context: base_context(true),
                        harness_id,
                        agent_id,
                        org_id,
                        mcp_tool_definitions: vec![],
                        previous_response_id: state.previous_response_id.clone(),
                        iteration: state.iteration,
                    },
                    prompt_message_ids,
                )
                .await?;

                iterations += 1;
                reply.push_text(reason_result.said_text(), reason_result.has_tool_calls);

                // Only a reason that would otherwise finish may close the
                // inbox; see the module note.
                let can_continue =
                    reason_result.success && state.iteration < reason_result.max_iterations as u32;
                let would_finish = !reason_schedules_act(&state, &reason_result) && can_continue;
                let pending = inbox.after_reason(session_id, would_finish).await?;
                pending_prompt_message_ids.extend(pending.prompt_message_ids);
                if !can_continue {
                    inbox.close(session_id).await?;
                }

                let act_scheduling =
                    turn_strategy::resolve_act_scheduling(adapter, &state, &reason_result).await?;
                let transition = execution.advance(
                    ActivityOutcome::Reason(Box::new(reason_result)),
                    pending.count,
                    Utc::now(),
                    HostFacts {
                        act_scheduling,
                        ..HostFacts::default()
                    },
                );
                turn_strategy::perform_effects(adapter, org_id, session_id, transition.effects)
                    .await?;
                plan = transition.plan;
            }
            TurnPlan::ScheduleAct(act_plan) => {
                tool_calls_count += act_plan.input.tool_calls.len();
                let act_result = execute_act_activity(adapter, act_plan.input).await?;
                reply.extend_sent(turn_strategy::sent_texts(&act_result));
                client_tool_calls.clone_from(&act_result.client_tool_calls);
                let outcome = turn_strategy::act_outcome(&act_result);
                let ask_user_calls = turn_strategy::pending_ask_user_calls(&act_result);
                let hints =
                    resolve_pause_hints(adapter, org_id, session_id, input_message_id, outcome)
                        .await?;
                let transition = execution.advance(
                    ActivityOutcome::Act(outcome),
                    0,
                    Utc::now(),
                    HostFacts {
                        setup_connection_hint_enabled: hints.setup_connection,
                        url_elicitation_hint_enabled: hints.url_elicitation,
                        ask_user_hint_enabled: hints.ask_user,
                        ask_user_calls,
                        ..HostFacts::default()
                    },
                );
                turn_strategy::perform_effects(adapter, org_id, session_id, transition.effects)
                    .await?;
                plan = transition.plan;
            }
            TurnPlan::Complete { stop_reason, error } => {
                inbox.close(session_id).await?;
                return Ok(finish_turn(
                    turn_id,
                    stop_reason,
                    error,
                    reply.take(),
                    iterations,
                    tool_calls_count,
                ));
            }
            // A pause returns the turn to the caller, who runs the parked
            // client-side calls and continues it once their results are
            // recorded; the inbox keeps the resume state until then. The
            // session has already been marked `waiting_for_tool_results` by
            // the effect.
            TurnPlan::WaitForToolResults { resume } => {
                inbox
                    .park(
                        session_id,
                        turn_id,
                        std::mem::take(&mut client_tool_calls),
                        resume,
                    )
                    .await?;
                return Ok(finish_turn(
                    turn_id,
                    TurnStopReason::EndTurn,
                    None,
                    reply.take(),
                    iterations,
                    tool_calls_count,
                ));
            }
        }
    }
}

/// Summarize a terminal engine plan into the public [`TurnResult`].
///
/// The engine reports the stop reason and the carried error; the host supplies
/// the running summaries it accumulated while executing the planned steps. A
/// carried error is the failure signal (it is exactly what the pre-engine state
/// machine kept in `pending_error`), and a failed turn reports no response and
/// no tool calls, matching the engine's terminal-failure mapping.
fn finish_turn(
    turn_id: TurnId,
    stop_reason: TurnStopReason,
    error: Option<String>,
    response: String,
    iterations: usize,
    tool_calls_count: usize,
) -> TurnResult {
    if error.is_some() {
        return TurnResult {
            response: String::new(),
            iterations,
            tool_calls_count: 0,
            success: false,
            error,
            stop_reason,
            turn_id,
        };
    }

    TurnResult {
        response,
        iterations,
        tool_calls_count,
        success: true,
        error: None,
        stop_reason,
        turn_id,
    }
}
