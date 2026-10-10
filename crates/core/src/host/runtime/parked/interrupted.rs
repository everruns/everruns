//! Turns a process exit cut off before they ended, and finishing them.
//!
//! Decision: an in-process turn waiting on a person (a tool approval, an
//! `ask_user` question) blocks inside its act; nothing but the process holds
//! the wait. The durable event log does hold everything needed to pick the
//! turn up again: its `turn.started`, the agent message whose tool calls the
//! act runs, and a `tool.completed` for every call that finished. So rather
//! than persisting the wait, a restarted host re-reads the log and continues
//! the turn from it.
//!
//! Decision: what happens to an unfinished call follows what its tool
//! declares, because the log cannot tell a call that waited from one that
//! was running. A call is run again when that is safe: the tool is `Pure` or
//! `Idempotent`, it never ran here (client-side, approval-gated, `ask_user`),
//! or the host says it waits on a person
//! ([`InProcessRuntimeBuilder::waits_on_person`]). Every other call is
//! settled as `interrupted`, so the model learns its outcome is unknown and
//! nothing runs twice that should run at most once. A turn cut off outside
//! its act (in a reason, or between steps) reasons again from the log.

use std::collections::HashSet;

use crate::engine::{ActInput, ActPlan};
use crate::events::{EventData, TokenUsage};
use everruns_contracts::tool_types::{
    ASK_USER_TOOL_NAME, SideEffectClass, ToolCall, ToolDefinition, ToolPolicy,
};

use super::*;

/// A turn a process exit cut off, as
/// [`InProcessRuntime::interrupted_tool_calls`] reports it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct InterruptedToolCalls {
    /// The cut-off turn; [`InProcessRuntime::resume_interrupted_turn`]
    /// continues it.
    pub turn_id: TurnId,
    /// The calls without a recorded result, in the order the model made them.
    /// Empty when the turn was cut off outside its act.
    pub tool_calls: Vec<ToolCall>,
    /// The subset of [`tool_calls`](Self::tool_calls) resume does not run
    /// again, because running them twice may not be safe. Resume settles
    /// them as `interrupted`.
    pub not_rerun: Vec<ToolCall>,
}

/// Whether a tool call waits on a person before it runs; see
/// [`InProcessRuntimeBuilder::waits_on_person`].
pub type WaitsOnPerson = std::sync::Arc<dyn Fn(&ToolCall) -> bool + Send + Sync>;

impl InProcessRuntimeBuilder {
    /// Tell resume which tool calls wait on a person before they run.
    ///
    /// When a process exit cuts a turn off in its act, resume runs a call
    /// again only when that is safe: its tool declares itself
    /// [`Pure`](everruns_contracts::tool_types::SideEffectClass::Pure) or
    /// [`Idempotent`](everruns_contracts::tool_types::SideEffectClass::Idempotent),
    /// it is client-side, approval-gated by policy, or `ask_user`, or this
    /// predicate says it waits on a person (an approval rule the host checks
    /// itself). Every other unfinished call is settled as interrupted, and
    /// the model sees that its outcome is unknown. See
    /// [`InProcessRuntime::resume_interrupted_turn`].
    pub fn waits_on_person(
        mut self,
        predicate: impl Fn(&ToolCall) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.waits_on_person = Some(std::sync::Arc::new(predicate));
        self
    }
}

/// The session's last turn as its log tells it, while it has no terminal event.
struct OpenTurn {
    turn_id: TurnId,
    input_message_id: MessageId,
    started_at: chrono::DateTime<Utc>,
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
    /// The open turn's calls without a result; `None` once it answered,
    /// since reasoning again would answer twice.
    fn unfinished(self) -> Option<(Self, Vec<ToolCall>)> {
        if self.answered {
            return None;
        }
        let calls: Vec<ToolCall> = self
            .calls
            .iter()
            .filter(|call| !self.settled.contains(&call.id))
            .cloned()
            .collect();
        Some((self, calls))
    }
}

/// What resume does with the open turn: the calls it runs again, the calls
/// it settles as interrupted.
struct ResumePlan {
    turn: OpenTurn,
    rerun: Vec<ToolCall>,
    not_rerun: Vec<ToolCall>,
}

impl InProcessRuntime {
    /// `session_id`'s last turn, when a process exit cut it off, with the
    /// tool calls it left unfinished and which of them resume runs again.
    ///
    /// `None` when the last turn ended (completed, failed, cancelled), has
    /// its answer and only lacks its end, or is parked on client-side tool
    /// calls in this process (see
    /// [`parked_tool_calls`](Self::parked_tool_calls)). Only meaningful while
    /// no turn of the session runs in this process: a running turn looks the
    /// same in the log.
    ///
    /// # Errors
    ///
    /// A store error when the log or the session cannot be read.
    pub async fn interrupted_tool_calls(
        &self,
        session_id: SessionId,
    ) -> Result<Option<InterruptedToolCalls>> {
        Ok(self
            .resume_plan(session_id)
            .await?
            .map(|(plan, _)| InterruptedToolCalls {
                turn_id: plan.turn.turn_id,
                tool_calls: plan
                    .turn
                    .calls
                    .iter()
                    .filter(|call| !plan.turn.settled.contains(&call.id))
                    .cloned()
                    .collect(),
                not_rerun: plan.not_rerun,
            }))
    }

    /// Continue the turn a process exit cut off, then carry on as it would
    /// have.
    ///
    /// Unfinished calls that are safe to run again (see
    /// [`InProcessRuntimeBuilder::waits_on_person`]) run through the regular
    /// act, with the session's current tools, hooks and approvals, so a call
    /// that waited on a person asks again. The rest are settled as
    /// `interrupted` first. A turn cut off outside its act, or whose calls
    /// were all settled, reasons again. The turn keeps its id; its iteration
    /// count and usage carry on from the log. Steering works as in
    /// [`run_steerable_turn`](Self::run_steerable_turn).
    ///
    /// # Errors
    ///
    /// A store error when the session has no interrupted turn (see
    /// [`interrupted_tool_calls`](Self::interrupted_tool_calls)), or any
    /// error the turn itself fails with.
    pub async fn resume_interrupted_turn(
        &self,
        session_id: SessionId,
        steering: TurnSteering,
    ) -> Result<TurnResult> {
        let (state, plan) = self
            .interrupted_turn_plan(session_id)
            .await?
            .ok_or_else(|| no_interrupted_turn(session_id))?;
        // Cut off before its first reason: there is no act to finish. Any
        // other turn finishes its act, which may have nothing left to run
        // and only moves the turn to its next reason, as the durable backend
        // does.
        let plan = if state.llm_call_count == 0 {
            TurnPlan::ScheduleReason(state.clone())
        } else {
            TurnPlan::ScheduleAct(plan)
        };
        let snapshot = self.resolved_execution_snapshot(session_id).await?;
        let drive = TurnDrive {
            session_id,
            org_id: state.org_id,
            turn_id: state.turn_id.unwrap_or_default(),
            input_message_id: state.input_message_id,
            harness_id: snapshot.harness_id,
            agent_id: snapshot.agent_id,
            workspace_id: snapshot.workspace_id,
        };
        self.drive_turn_plan(drive, InProcessExecution::new(state), plan, steering)
            .await
    }

    /// The step [`resume_interrupted_turn`](Self::resume_interrupted_turn)
    /// continues `session_id`'s interrupted turn with: the act that runs its
    /// rerunnable calls again (none when the turn should reason again), and
    /// the engine state that act starts from. Settles the calls it does not
    /// run again as `interrupted` before it returns. `None` when the session
    /// has no interrupted turn (see
    /// [`interrupted_tool_calls`](Self::interrupted_tool_calls)).
    ///
    /// Not part of the framework surface: it exists so a turn backend that
    /// drives the steps itself (the durable backend) resumes an interrupted
    /// turn as this runtime does. An act with no calls moves the turn to its
    /// next reason.
    ///
    /// # Errors
    ///
    /// A store error when the log or the session cannot be read, or the
    /// settled results cannot be recorded.
    #[doc(hidden)]
    pub async fn interrupted_turn_plan(
        &self,
        session_id: SessionId,
    ) -> Result<Option<(TurnState, ActPlan)>> {
        let Some((
            ResumePlan {
                turn,
                rerun: tool_calls,
                not_rerun,
            },
            context,
        )) = self.resume_plan(session_id).await?
        else {
            return Ok(None);
        };
        for call in &not_rerun {
            self.settle_interrupted(session_id, &turn, call).await?;
        }
        let snapshot = self.resolved_execution_snapshot(session_id).await?;
        let org_id = in_process_internal_org_id(&snapshot.organization_id);
        let state = TurnState {
            org_id,
            session_id,
            harness_id: snapshot.harness_id,
            agent_id: snapshot.agent_id,
            input_message_id: turn.input_message_id,
            turn_id: Some(turn.turn_id),
            // A restarted process holds no provider-side conversation.
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
                context: ExecutionContext::new(session_id, turn.turn_id, turn.input_message_id)
                    .with_workspace_id(snapshot.workspace_id)
                    .next_exec(),
                harness_id: snapshot.harness_id,
                agent_id: snapshot.agent_id,
                tool_calls,
                tool_definitions: context.runtime_agent.tools.clone(),
                locale: context.resolved_locale.clone(),
                blueprint_id: context.snapshot.blueprint_id.clone(),
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

    /// The open turn and what resume does with each unfinished call, plus
    /// the context (tool surface) that decision used.
    async fn resume_plan(
        &self,
        session_id: SessionId,
    ) -> Result<Option<(ResumePlan, AssembledTurnContext)>> {
        let Some((turn, unfinished)) = self.open_turn(session_id).await? else {
            return Ok(None);
        };
        // The same tool surface a reason step would hand the act.
        let context = self.load_context(session_id).await?;
        let (rerun, not_rerun) = unfinished.into_iter().partition(|call| {
            reruns(
                call,
                context
                    .runtime_agent
                    .tools
                    .iter()
                    .find(|tool| tool.name() == call.name),
                self.waits_on_person.as_deref(),
            )
        });
        Ok(Some((
            ResumePlan {
                turn,
                rerun,
                not_rerun,
            },
            context,
        )))
    }

    /// Record `call` as cut off with an unknown outcome, so the next reason
    /// sees it answered and nothing runs it twice.
    async fn settle_interrupted(
        &self,
        session_id: SessionId,
        turn: &OpenTurn,
        call: &ToolCall,
    ) -> Result<()> {
        let message = format!(
            "tool '{}' was cut off by a restart while it ran; its outcome is unknown \
             and it was not run again, because running it twice may not be safe",
            call.name
        );
        self.event_emitter
            .emit(EventRequest::new(
                session_id,
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

    /// Read the log forward, keeping only the last turn while it is open.
    async fn open_turn(&self, session_id: SessionId) -> Result<Option<(OpenTurn, Vec<ToolCall>)>> {
        if lock_parked(&self.parked_turns).contains_key(&session_id) {
            return Ok(None);
        }
        let limit = EventReadLimit::default();
        let mut request = EventReadRequest::new(session_id, limit);
        let mut open: Option<OpenTurn> = None;
        loop {
            let page = self
                .event_log
                .read_page(request)
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            for event in page.events {
                observe(&mut open, event);
            }
            let Some(cursor) = page.next_cursor else {
                break;
            };
            request = EventReadRequest::from_cursor(cursor, limit);
        }
        Ok(open.and_then(OpenTurn::unfinished))
    }
}

fn no_interrupted_turn(session_id: SessionId) -> AgentLoopError {
    AgentLoopError::store(format!("session {session_id} has no interrupted turn"))
}

/// Whether resume may run `call` again. `definition` is its tool in the
/// session's current surface; a call whose tool is gone is not run.
fn reruns(
    call: &ToolCall,
    definition: Option<&ToolDefinition>,
    waits_on_person: Option<&(dyn Fn(&ToolCall) -> bool + Send + Sync)>,
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

/// Fold one event into the open turn.
fn observe(open: &mut Option<OpenTurn>, event: Event) {
    let ends = |turn_id: &TurnId| open.as_ref().is_some_and(|turn| turn.turn_id == *turn_id);
    match &event.data {
        EventData::TurnStarted(data) => {
            *open = Some(OpenTurn {
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

#[cfg(test)]
mod tests {
    use crate::events::{OutputMessageCompletedData, TurnCancelledData, TurnStartedData};

    use super::*;

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "share".into(),
            arguments: serde_json::json!({ "to": id }),
        }
    }

    /// A turn that asked for `calls`, of which `settled` finished, and that
    /// was cancelled when `cancelled`.
    fn turn_log(calls: &[&str], settled: &[&str], cancelled: bool) -> (TurnId, Vec<Event>) {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let context = EventContext::turn(turn_id, input_message_id);
        let mut events = vec![
            Event::new(
                session_id,
                context.clone(),
                TurnStartedData {
                    turn_id,
                    input_message_id,
                    input_content: None,
                    agent_id: None,
                    agent_name: None,
                    agent_description: None,
                },
            ),
            Event::new(
                session_id,
                context.clone(),
                OutputMessageCompletedData::new(RuntimeMessage::assistant_with_tools(
                    "",
                    calls.iter().map(|id| call(id)).collect(),
                )),
            ),
        ];
        for id in settled {
            events.push(Event::new(
                session_id,
                context.clone(),
                ToolCompletedData::success(id.to_string(), "share".into(), Vec::new(), None),
            ));
        }
        if cancelled {
            events.push(Event::new(
                session_id,
                context,
                TurnCancelledData {
                    turn_id,
                    reason: None,
                    usage: None,
                },
            ));
        }
        (turn_id, events)
    }

    fn fold(events: Vec<Event>) -> Option<(TurnId, Vec<ToolCall>)> {
        let mut open = None;
        for event in events {
            observe(&mut open, event);
        }
        open.and_then(OpenTurn::unfinished)
            .map(|(turn, calls)| (turn.turn_id, calls))
    }

    #[test]
    fn a_turn_without_an_end_reports_its_unfinished_calls() {
        let (turn_id, events) = turn_log(&["a", "b"], &["a"], false);
        assert_eq!(fold(events), Some((turn_id, vec![call("b")])));
    }

    #[test]
    fn an_ended_turn_is_not_interrupted() {
        assert_eq!(fold(turn_log(&["a"], &[], true).1), None);
    }

    #[test]
    fn a_settled_act_without_an_end_reasons_again() {
        let (turn_id, events) = turn_log(&["a"], &["a"], false);
        assert_eq!(fold(events), Some((turn_id, vec![])));
    }

    #[test]
    fn an_answered_turn_without_an_end_is_not_resumed() {
        assert_eq!(fold(turn_log(&[], &[], false).1), None);
    }

    #[test]
    fn only_the_last_turn_counts() {
        let (_, mut events) = turn_log(&["a"], &[], false);
        let (last, later) = turn_log(&["b"], &["b"], false);
        events.extend(later);
        assert_eq!(fold(events), Some((last, vec![])));
    }

    fn tool(policy: ToolPolicy, class: Option<SideEffectClass>) -> ToolDefinition {
        let mut hints = everruns_contracts::tool_types::ToolHints::default();
        if let Some(class) = class {
            hints = hints.with_side_effect_class(class);
        }
        ToolDefinition::Builtin(everruns_contracts::tool_types::BuiltinTool {
            name: "share".into(),
            display_name: None,
            description: String::new(),
            parameters: serde_json::json!({}),
            policy,
            category: None,
            deferrable: Default::default(),
            hints,
            full_parameters: None,
        })
    }

    #[test]
    fn only_calls_safe_to_run_twice_rerun() {
        let share = call("a");
        let auto = tool(ToolPolicy::Auto, None);
        assert!(
            !reruns(&share, Some(&auto), None),
            "at most once by default"
        );
        assert!(!reruns(&share, None, None), "a tool that is gone");
        for class in [SideEffectClass::Pure, SideEffectClass::Idempotent] {
            assert!(reruns(
                &share,
                Some(&tool(ToolPolicy::Auto, Some(class))),
                None
            ));
        }
        for policy in [ToolPolicy::ClientSide, ToolPolicy::RequiresApproval] {
            assert!(reruns(&share, Some(&tool(policy, None)), None));
        }
        let ask = ToolCall {
            name: ASK_USER_TOOL_NAME.into(),
            ..share.clone()
        };
        assert!(reruns(&ask, Some(&auto), None));
        let gated = |call: &ToolCall| call.arguments["to"] == "a";
        assert!(reruns(&share, Some(&auto), Some(&gated)));
        assert!(!reruns(&call("b"), Some(&auto), Some(&gated)));
    }
}
