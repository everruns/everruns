//! Resuming a turn from the session's event log, generic over the host.
//!
//! Decision: a turn that has not ended is resumed from the log alone. Its
//! `turn.started`, the agent message whose tool calls its act runs, and a
//! `tool.completed` for every call that finished hold everything needed to
//! pick it up again, so no host persists a wait. [`TurnLogFold`] folds those
//! events into the [`OpenTurn`]; [`resume_plan_from_log`] turns it into the
//! engine state and the act that continue it, which
//! [`drive_turn`](crate::host::drive_turn) then runs.
//!
//! Decision: the fold needs only the log's tail from the last `turn.started`
//! on. A host may read just that tail; events before the last `turn.started`
//! never affect the result, and a tail without one has no open turn.
//!
//! Decision: a turn parked on client-side or approval calls is not special.
//! Once their results are recorded as `tool.completed`, it is an open turn
//! whose act has nothing left to run, and resuming it moves it to its next
//! reason. Hosts therefore need no separate parked-turn store to continue it.
//!
//! Decision: what happens to an unfinished call follows what its tool
//! declares, because the log cannot tell a call that waited from one that
//! was running. A call is run again when that is safe: the tool is `Pure` or
//! `Idempotent`, it never ran here (client-side, approval-gated, `ask_user`),
//! or the host says it waits on a person ([`WaitsOnPerson`]). Every other call
//! is settled as `interrupted`, so the model learns its outcome is unknown and
//! nothing runs twice that should run at most once. A turn cut off outside
//! its act (in a reason, or between steps) reasons again from the log.
//!
//! Decision: the tool each call is looked up in is the surface a reason on
//! this host would hand its act now (the reason's MCP and host-augmented
//! tools included), not the plain assembled context, so a rerun decision
//! matches what the act will actually run.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use everruns_contracts::error::Result;
use everruns_contracts::tool_types::{
    ASK_USER_TOOL_NAME, SideEffectClass, ToolCall, ToolDefinition, ToolPolicy,
};
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};

use crate::ExecutionContext;
use crate::engine::{ActInput, ActPlan, TurnPlan, TurnState};
use crate::events::{Event, EventContext, EventData, EventRequest, TokenUsage, ToolCompletedData};
use crate::host::runtime_host::{RuntimeHostAdapter, reason_turn_context};
use crate::runtime_context::AssembledTurnContext;

/// A turn a process exit cut off: its unfinished calls and which of them
/// resume does not run again.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct InterruptedToolCalls {
    /// The cut-off turn; resuming it keeps this id.
    pub turn_id: TurnId,
    /// The calls without a recorded result, in the order the model made them.
    /// Empty when the turn was cut off outside its act.
    pub tool_calls: Vec<ToolCall>,
    /// The subset of [`tool_calls`](Self::tool_calls) resume does not run
    /// again, because running them twice may not be safe. Resume settles
    /// them as `interrupted`.
    pub not_rerun: Vec<ToolCall>,
}

/// Whether a tool call waits on a person before it runs (an approval rule
/// the host checks itself), so resume may safely run it again.
pub type WaitsOnPerson = std::sync::Arc<dyn Fn(&ToolCall) -> bool + Send + Sync>;

/// The borrowed form of [`WaitsOnPerson`] the resume functions take.
pub type WaitsOnPersonFn = dyn Fn(&ToolCall) -> bool + Send + Sync;

/// The session's last turn as its log tells it, while it has no terminal
/// event. Built by [`TurnLogFold`].
#[derive(Debug, Clone)]
pub struct OpenTurn {
    session_id: SessionId,
    turn_id: TurnId,
    input_message_id: MessageId,
    started_at: DateTime<Utc>,
    reasons: u32,
    tool_call_count: u32,
    usage: Option<TokenUsage>,
    final_message_id: Option<MessageId>,
    /// The tool calls of the turn's latest agent message.
    calls: Vec<ToolCall>,
    /// Calls with a `tool.completed`.
    settled: HashSet<String>,
    /// The latest agent message made no tool calls: the turn has its answer
    /// and only its end is missing.
    answered: bool,
}

impl OpenTurn {
    /// The session the turn runs in.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// The open turn's id.
    pub fn turn_id(&self) -> TurnId {
        self.turn_id
    }

    /// The message that started the turn.
    pub fn input_message_id(&self) -> MessageId {
        self.input_message_id
    }

    /// Whether the turn's latest agent message made no tool calls: it has
    /// its answer and only its end is missing, so resuming it would answer
    /// twice.
    pub fn is_answered(&self) -> bool {
        self.answered
    }

    /// The calls of the turn's latest agent message without a recorded
    /// result, in the order the model made them.
    pub fn unfinished_calls(&self) -> Vec<ToolCall> {
        self.calls
            .iter()
            .filter(|call| !self.settled.contains(&call.id))
            .cloned()
            .collect()
    }
}

/// Folds a session's events, read forward, into its last turn while that
/// turn is open.
///
/// Feed it the whole log or only its tail from the last `turn.started`; see
/// the module note.
#[derive(Debug, Clone, Default)]
pub struct TurnLogFold {
    open: Option<OpenTurn>,
}

impl TurnLogFold {
    /// An empty fold.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold every event of `events`, read forward, and return the open turn.
    pub fn from_events(events: impl IntoIterator<Item = Event>) -> Option<OpenTurn> {
        let mut fold = Self::new();
        for event in events {
            fold.observe(event);
        }
        fold.open_turn()
    }

    /// Fold the next event of the log.
    pub fn observe(&mut self, event: Event) {
        let open = &mut self.open;
        let ends = |turn_id: &TurnId| open.as_ref().is_some_and(|turn| turn.turn_id == *turn_id);
        match &event.data {
            EventData::TurnStarted(data) => {
                *open = Some(OpenTurn {
                    session_id: event.session_id,
                    turn_id: data.turn_id,
                    input_message_id: data.input_message_id,
                    started_at: event.ts,
                    reasons: 0,
                    tool_call_count: 0,
                    usage: None,
                    final_message_id: None,
                    calls: Vec::new(),
                    settled: HashSet::new(),
                    answered: false,
                });
                return;
            }
            EventData::TurnCompleted(data) if ends(&data.turn_id) => *open = None,
            EventData::TurnFailed(data) if ends(&data.turn_id) => *open = None,
            EventData::TurnCancelled(data) if ends(&data.turn_id) => *open = None,
            EventData::TurnSealed(data) if ends(&data.turn_id) => *open = None,
            _ => {}
        }
        let Some(turn) = open
            .as_mut()
            .filter(|turn| event.context.turn_id == Some(turn.turn_id))
        else {
            return;
        };
        match event.data {
            EventData::ReasonCompleted(data) => {
                turn.reasons = turn.reasons.saturating_add(1);
                turn.tool_call_count = turn.tool_call_count.saturating_add(data.tool_call_count);
                if let Some(usage) = &data.usage {
                    match &mut turn.usage {
                        Some(total) => total.add(usage),
                        None => turn.usage = Some(usage.clone()),
                    }
                }
            }
            EventData::OutputMessageCompleted(data) => {
                turn.final_message_id = Some(data.message.id);
                turn.calls = data
                    .message
                    .tool_calls()
                    .into_iter()
                    .map(|call| ToolCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .collect();
                turn.answered = turn.calls.is_empty();
                turn.settled.clear();
            }
            EventData::ToolCompleted(data) => {
                turn.settled.insert(data.tool_call_id);
            }
            _ => {}
        }
    }

    /// The last turn, when it has no terminal event.
    pub fn open_turn(self) -> Option<OpenTurn> {
        self.open
    }
}

/// What resume does with an open turn: the calls it runs again, the calls
/// it settles as interrupted, and the context that decision used.
struct ResumeDecision {
    turn: OpenTurn,
    rerun: Vec<ToolCall>,
    not_rerun: Vec<ToolCall>,
    context: AssembledTurnContext,
}

/// Decide what resume does with `turn`. `None` once the turn answered.
async fn decide<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    turn: OpenTurn,
    waits_on_person: Option<&WaitsOnPersonFn>,
) -> Result<Option<ResumeDecision>> {
    if turn.answered {
        return Ok(None);
    }
    let unfinished = turn.unfinished_calls();
    let context =
        reason_turn_context(adapter, org_id, turn.session_id, turn.input_message_id).await?;
    let (rerun, not_rerun) = unfinished.into_iter().partition(|call| {
        reruns(
            call,
            context
                .runtime_agent
                .tools
                .iter()
                .find(|tool| tool.name() == call.name),
            waits_on_person,
        )
    });
    Ok(Some(ResumeDecision {
        turn,
        rerun,
        not_rerun,
        context,
    }))
}

/// `turn`'s unfinished calls and which of them resume does not run again,
/// without writing anything. `None` when the turn already has its answer
/// and only lacks its end.
///
/// # Errors
///
/// A store error when the session's turn context cannot be loaded.
pub async fn interrupted_tool_calls<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    turn: OpenTurn,
    waits_on_person: Option<&WaitsOnPersonFn>,
) -> Result<Option<InterruptedToolCalls>> {
    Ok(decide(adapter, org_id, turn, waits_on_person)
        .await?
        .map(|decision| InterruptedToolCalls {
            turn_id: decision.turn.turn_id,
            tool_calls: decision.turn.unfinished_calls(),
            not_rerun: decision.not_rerun,
        }))
}

/// The step that continues `turn`: the engine state it resumes from and the
/// act that runs its rerunnable calls again (no calls when the turn should
/// reason again). Settles the calls it does not run again as `interrupted`
/// through `adapter.event_emitter()` before it returns. `None` when the turn
/// already has its answer and only lacks its end.
///
/// Pass the result to [`resume_entry_plan`] for the plan
/// [`drive_turn`](crate::host::drive_turn) starts from.
///
/// # Errors
///
/// A store error when the session's turn context cannot be loaded, or the
/// settled results cannot be recorded.
pub async fn resume_open_turn<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    turn: OpenTurn,
    waits_on_person: Option<&WaitsOnPersonFn>,
) -> Result<Option<(TurnState, ActPlan)>> {
    let Some(ResumeDecision {
        turn,
        rerun: tool_calls,
        not_rerun,
        context,
    }) = decide(adapter, org_id, turn, waits_on_person).await?
    else {
        return Ok(None);
    };
    for call in &not_rerun {
        settle_interrupted(adapter, &turn, call).await?;
    }
    let snapshot = &context.snapshot;
    let state = TurnState {
        org_id,
        session_id: turn.session_id,
        harness_id: snapshot.harness_id,
        agent_id: snapshot.agent_id,
        input_message_id: turn.input_message_id,
        turn_id: Some(turn.turn_id),
        // A resumed turn holds no provider-side conversation.
        previous_response_id: None,
        iteration: turn.reasons.max(1),
        request_id: None,
        started_at: Some(turn.started_at),
        cumulative_usage: turn.usage.clone(),
        tool_call_count: turn.tool_call_count,
        llm_call_count: turn.reasons,
        time_to_first_token_ms: None,
        final_message_id: turn.final_message_id,
        final_answer_preview: None,
    };
    let plan = ActPlan {
        input: ActInput {
            org_id: Some(org_id),
            context: ExecutionContext::new(turn.session_id, turn.turn_id, turn.input_message_id)
                .with_workspace_id(snapshot.workspace_id)
                .next_exec(),
            harness_id: snapshot.harness_id,
            agent_id: snapshot.agent_id,
            tool_calls,
            tool_definitions: context.runtime_agent.tools.clone(),
            locale: context.resolved_locale.clone(),
            blueprint_id: snapshot.blueprint_id.clone(),
            network_access: context.runtime_agent.network_access.clone(),
            parallel_tool_calls: context.runtime_agent.parallel_tool_calls,
        },
        previous_response_id: None,
        iteration: state.iteration,
        request_id: None,
        resume_state: Box::new(state.clone()),
    };
    Ok(Some((state, plan)))
}

/// Fold `events` (the session's log, or its tail from the last
/// `turn.started`) and build the step that continues its open turn, as
/// [`resume_open_turn`] does. `None` when the session has no open turn, or
/// its open turn already has its answer.
///
/// `session_id` guards against a log of another session: an open turn of a
/// different session is ignored.
///
/// # Errors
///
/// As [`resume_open_turn`].
pub async fn resume_plan_from_log<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    events: Vec<Event>,
    waits_on_person: Option<&WaitsOnPersonFn>,
) -> Result<Option<(TurnState, ActPlan)>> {
    let Some(turn) = TurnLogFold::from_events(events).filter(|turn| turn.session_id == session_id)
    else {
        return Ok(None);
    };
    resume_open_turn(adapter, org_id, turn, waits_on_person).await
}

/// The plan [`drive_turn`](crate::host::drive_turn) resumes a turn with,
/// from [`resume_open_turn`]'s result.
///
/// A turn cut off before its first reason has no act to finish and reasons.
/// Any other turn finishes its act, which may have nothing left to run and
/// only moves the turn to its next reason.
pub fn resume_entry_plan(state: &TurnState, act: ActPlan) -> TurnPlan {
    if state.llm_call_count == 0 {
        TurnPlan::ScheduleReason(state.clone())
    } else {
        TurnPlan::ScheduleAct(act)
    }
}

/// Record `call` as cut off with an unknown outcome, so the next reason
/// sees it answered and nothing runs it twice.
async fn settle_interrupted<A: RuntimeHostAdapter>(
    adapter: &A,
    turn: &OpenTurn,
    call: &ToolCall,
) -> Result<()> {
    let message = format!(
        "tool '{}' was cut off by a restart while it ran; its outcome is unknown \
         and it was not run again, because running it twice may not be safe",
        call.name
    );
    adapter
        .event_emitter()
        .emit(EventRequest::new(
            turn.session_id,
            EventContext::turn(turn.turn_id, turn.input_message_id),
            ToolCompletedData::failure(
                call.id.clone(),
                call.name.clone(),
                "interrupted".to_string(),
                message,
                None,
            ),
        ))
        .await?;
    Ok(())
}

/// Whether resume may run `call` again. `definition` is its tool in the
/// session's current surface; a call whose tool is gone is not run.
fn reruns(
    call: &ToolCall,
    definition: Option<&ToolDefinition>,
    waits_on_person: Option<&WaitsOnPersonFn>,
) -> bool {
    let Some(definition) = definition else {
        return false;
    };
    // Never ran here: the client runs it, or it waits for a decision first.
    if matches!(
        definition.policy(),
        ToolPolicy::ClientSide | ToolPolicy::RequiresApproval
    ) || call.name == ASK_USER_TOOL_NAME
    {
        return true;
    }
    matches!(
        definition.side_effect_class(),
        SideEffectClass::Pure | SideEffectClass::Idempotent
    ) || waits_on_person.is_some_and(|predicate| predicate(call))
}

#[cfg(test)]
#[path = "turn_resume_tests.rs"]
mod tests;
