//! Turns parked on client-side tool calls, and resuming them with results.
//! The `interrupted` child covers turns parked by a process that is gone: a
//! process exit cut them off in their act.

use super::*;

mod interrupted;

pub use interrupted::InterruptedToolCalls;

/// The client-side tool calls a turn parked on, as
/// [`InProcessRuntime::parked_tool_calls`] reports them.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ParkedToolCalls {
    /// The parked turn; [`InProcessRuntime::resume_steerable_turn`] continues it.
    pub turn_id: TurnId,
    /// The calls the turn waits on, in the order the model made them.
    pub tool_calls: Vec<everruns_contracts::tool_types::ToolCall>,
}

/// A turn waiting for client-side tool results, with the engine state its
/// next step resumes from.
pub(super) struct ParkedTurn {
    pub(super) calls: ParkedToolCalls,
    pub(super) resume: TurnState,
}

pub(super) type ParkedTurns = Arc<Mutex<std::collections::HashMap<SessionId, ParkedTurn>>>;

fn no_parked_turn(session_id: SessionId) -> AgentLoopError {
    AgentLoopError::store(format!(
        "session {session_id} has no turn waiting for tool results"
    ))
}

pub(super) fn lock_parked(
    parked: &ParkedTurns,
) -> std::sync::MutexGuard<'_, std::collections::HashMap<SessionId, ParkedTurn>> {
    parked
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl InProcessRuntime {
    /// The client-side tool calls `session_id`'s last turn parked on, if it
    /// parked and has not been resumed or superseded since.
    ///
    /// A turn whose model calls a client-side tool (a
    /// [`ToolDefinition::ClientSide`](everruns_contracts::tool_types::ToolDefinition::ClientSide)
    /// on the session) pauses when the session's `setup_connection` hint
    /// says its client can answer, and [`run_steerable_turn`](Self::run_steerable_turn)
    /// returns. The calls wait here until
    /// [`resume_steerable_turn`](Self::resume_steerable_turn) delivers their
    /// results, or a new turn starts.
    pub fn parked_tool_calls(&self, session_id: SessionId) -> Option<ParkedToolCalls> {
        lock_parked(&self.parked_turns)
            .get(&session_id)
            .map(|parked| parked.calls.clone())
    }

    /// Continue the turn `session_id` parked on client-side tool calls,
    /// recording `results` as their outcomes first.
    ///
    /// The results land under the parked turn, as a hosted runtime records
    /// them for its tool-results endpoint, and the turn's next model call
    /// sees them. Pass one result per parked call: a call left without one
    /// stays unanswered in history. Steering works as in
    /// [`run_steerable_turn`](Self::run_steerable_turn).
    ///
    /// # Errors
    ///
    /// A store error when no turn of `session_id` is parked, or the results
    /// cannot be recorded.
    pub async fn resume_steerable_turn(
        &self,
        session_id: SessionId,
        results: Vec<ToolCompletedData>,
        steering: TurnSteering,
    ) -> Result<TurnResult> {
        let resume = self
            .deliver_parked_tool_results(session_id, results)
            .await?;
        let snapshot = self.resolved_execution_snapshot(session_id).await?;
        let drive = TurnDrive {
            session_id,
            org_id: resume.org_id,
            turn_id: resume.turn_id.unwrap_or_default(),
            input_message_id: resume.input_message_id,
            harness_id: snapshot.harness_id,
            agent_id: snapshot.agent_id,
            workspace_id: snapshot.workspace_id,
        };
        let plan = TurnPlan::ScheduleReason(resume.clone());
        self.drive_turn_plan(drive, InProcessExecution::new(resume), plan, steering)
            .await
    }

    /// Record `results` under the turn `session_id` parked on client-side
    /// tool calls, without continuing it. The turn stays parked until a
    /// [`TurnInput::RecordedToolResults`](crate::host::TurnInput::RecordedToolResults)
    /// start (or [`resume_steerable_turn`](Self::resume_steerable_turn) with
    /// no further results) continues it.
    ///
    /// The results land exactly where `resume_steerable_turn` records them,
    /// so recording first and continuing later runs the turn as one call
    /// would. A host that records results through its own event store does
    /// the same and needs no call here.
    ///
    /// # Errors
    ///
    /// A store error when no turn of `session_id` is parked, or the results
    /// cannot be recorded.
    pub async fn record_parked_tool_results(
        &self,
        session_id: SessionId,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        let (turn_id, input_message_id) = lock_parked(&self.parked_turns)
            .get(&session_id)
            .map(|parked| (parked.calls.turn_id, parked.resume.input_message_id))
            .ok_or_else(|| no_parked_turn(session_id))?;
        self.emit_tool_results(session_id, turn_id, input_message_id, results)
            .await
    }

    async fn emit_tool_results(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        input_message_id: MessageId,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        for result in results {
            self.event_emitter
                .emit(EventRequest::new(
                    session_id,
                    EventContext::turn(turn_id, input_message_id),
                    result,
                ))
                .await?;
        }
        Ok(())
    }

    /// Take the turn `session_id` parked on client-side tool calls and record
    /// `results` under it, as [`resume_steerable_turn`](Self::resume_steerable_turn)
    /// does before it continues the turn. Returns the engine state the turn
    /// continues from: a reason at that state's iteration.
    ///
    /// Not part of the framework surface: it exists so a turn backend that
    /// drives the steps itself (the durable backend) continues a parked turn
    /// exactly as this runtime does.
    ///
    /// # Errors
    ///
    /// A store error when no turn of `session_id` is parked, or the results
    /// cannot be recorded.
    #[doc(hidden)]
    pub async fn deliver_parked_tool_results(
        &self,
        session_id: SessionId,
        results: Vec<ToolCompletedData>,
    ) -> Result<TurnState> {
        let parked = self
            .take_parked_turn(session_id)
            .ok_or_else(|| no_parked_turn(session_id))?;
        let turn_id = parked.calls.turn_id;
        let input_message_id = parked.resume.input_message_id;
        self.emit_tool_results(session_id, turn_id, input_message_id, results)
            .await?;
        let mut resume = parked.resume;
        resume.turn_id = Some(turn_id);
        Ok(resume)
    }

    /// Record that `session_id`'s turn `turn_id` parked on the client-side
    /// `tool_calls`, to continue from `resume`, as a turn this runtime drives
    /// records it when it pauses.
    ///
    /// Not part of the framework surface: it exists so a turn backend that
    /// drives the steps itself (the durable backend) parks a turn where
    /// [`parked_tool_calls`](Self::parked_tool_calls) and
    /// [`resume_steerable_turn`](Self::resume_steerable_turn) find it.
    #[doc(hidden)]
    pub fn park_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        tool_calls: Vec<everruns_contracts::tool_types::ToolCall>,
        resume: TurnState,
    ) {
        lock_parked(&self.parked_turns).insert(
            session_id,
            ParkedTurn {
                calls: ParkedToolCalls {
                    turn_id,
                    tool_calls,
                },
                resume,
            },
        );
    }

    /// Drop the turn `session_id` parked on client-side tool calls, as a new
    /// turn this runtime starts does: its calls stay unanswered in history.
    ///
    /// Not part of the framework surface; see [`park_turn`](Self::park_turn).
    #[doc(hidden)]
    pub fn supersede_parked_turn(&self, session_id: SessionId) {
        self.take_parked_turn(session_id);
    }

    pub(super) fn take_parked_turn(&self, session_id: SessionId) -> Option<ParkedTurn> {
        lock_parked(&self.parked_turns).remove(&session_id)
    }
}
