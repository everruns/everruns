//! Turns a process exit cut off while their tool calls ran, and finishing
//! them by running those calls again.
//!
//! Decision: an in-process turn waiting on a person (a tool approval, an
//! `ask_user` question) blocks inside its act; nothing but the process holds
//! the wait. The durable event log does hold everything needed to pick the
//! turn up again: its `turn.started`, the agent message whose tool calls the
//! act runs, and a `tool.completed` for every call that finished. So rather
//! than persisting the wait, a restarted host re-reads the log and runs the
//! unfinished calls again in the same turn, which asks the person again, and
//! then lets the turn carry on as the act would have. Which calls are safe to
//! run twice is the caller's call: a call cut off mid-execution runs again
//! (at-least-once), so a host re-runs only calls it knows were waiting.

use std::collections::HashSet;

use crate::engine::{ActInput, ActPlan};
use crate::events::{EventData, TokenUsage};
use everruns_contracts::tool_types::ToolCall;

use super::*;

/// The unfinished tool calls of a turn a process exit cut off, as
/// [`InProcessRuntime::interrupted_tool_calls`] reports them.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct InterruptedToolCalls {
    /// The cut-off turn; [`InProcessRuntime::resume_interrupted_turn`]
    /// continues it.
    pub turn_id: TurnId,
    /// The calls without a recorded result, in the order the model made them.
    pub tool_calls: Vec<ToolCall>,
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
}

impl OpenTurn {
    fn unfinished(self) -> Option<(Self, Vec<ToolCall>)> {
        let calls: Vec<ToolCall> = self
            .calls
            .iter()
            .filter(|call| !self.settled.contains(&call.id))
            .cloned()
            .collect();
        (!calls.is_empty()).then_some((self, calls))
    }
}

impl InProcessRuntime {
    /// The tool calls `session_id`'s last turn left unfinished, when a
    /// process exit cut that turn off in its act.
    ///
    /// `None` when the last turn ended (completed, failed, cancelled), was
    /// cut off outside an act, or is parked on client-side tool calls in
    /// this process (see [`parked_tool_calls`](Self::parked_tool_calls)).
    /// Only meaningful while no turn of the session runs in this process: a
    /// running turn looks the same in the log.
    ///
    /// # Errors
    ///
    /// A store error when the log cannot be read.
    pub async fn interrupted_tool_calls(
        &self,
        session_id: SessionId,
    ) -> Result<Option<InterruptedToolCalls>> {
        Ok(self
            .open_turn(session_id)
            .await?
            .map(|(turn, tool_calls)| InterruptedToolCalls {
                turn_id: turn.turn_id,
                tool_calls,
            }))
    }

    /// Continue the turn a process exit cut off in its act: run its
    /// unfinished tool calls again, then carry on as the act would have.
    ///
    /// The calls run through the regular act, with the session's current
    /// tools, hooks and approvals, so a call that waited on a person asks
    /// again and one that was cut off mid-execution runs a second time. The
    /// turn keeps its id; its iteration count and usage carry on from the
    /// log. Steering works as in [`run_steerable_turn`](Self::run_steerable_turn).
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
        let (turn, tool_calls) = self.open_turn(session_id).await?.ok_or_else(|| {
            AgentLoopError::store(format!(
                "session {session_id} has no turn interrupted in its tool calls"
            ))
        })?;
        let snapshot = self.resolved_execution_snapshot(session_id).await?;
        let org_id = in_process_internal_org_id(&snapshot.organization_id);
        // The same tool surface a reason step would hand the act.
        let context = self.load_context(session_id).await?;
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
        let plan = TurnPlan::ScheduleAct(ActPlan {
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
        });
        self.drive_turn_plan(
            TurnDrive {
                session_id,
                org_id,
                turn_id: turn.turn_id,
                input_message_id: turn.input_message_id,
                harness_id: snapshot.harness_id,
                agent_id: snapshot.agent_id,
                workspace_id: snapshot.workspace_id,
            },
            InProcessExecution::new(state),
            plan,
            steering,
        )
        .await
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
    fn an_ended_turn_or_a_settled_act_is_not_interrupted() {
        assert_eq!(fold(turn_log(&["a"], &[], true).1), None);
        assert_eq!(fold(turn_log(&["a"], &["a"], false).1), None);
    }

    #[test]
    fn only_the_last_turn_counts() {
        let (_, mut events) = turn_log(&["a"], &[], false);
        events.extend(turn_log(&["b"], &["b"], false).1);
        assert_eq!(fold(events), None);
    }
}
