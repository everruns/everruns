//! Turns a process exit cut off before they ended, and finishing them.
//!
//! Decision: an in-process turn waiting on a person (a tool approval, an
//! `ask_user` question) blocks inside its act; nothing but the process holds
//! the wait. Rather than persisting it, a restarted host re-reads the log and
//! continues the turn from it. The fold, the rerun rule and the resume step
//! are host-generic ([`crate::host::turn_resume`]); this runtime only reads
//! its own log and keeps a turn parked in this process out of it.

use crate::engine::ActPlan;
use crate::host::turn_resume::{self, InterruptedToolCalls, OpenTurn, TurnLogFold};
use everruns_contracts::tool_types::ToolCall;

use super::*;

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
        let Some((org_id, turn)) = self.open_turn(session_id).await? else {
            return Ok(None);
        };
        turn_resume::interrupted_tool_calls(self, org_id, turn, self.waits_on_person.as_deref())
            .await
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
        let (state, act) = self
            .interrupted_turn_plan(session_id)
            .await?
            .ok_or_else(|| no_interrupted_turn(session_id))?;
        let plan = turn_resume::resume_entry_plan(&state, act);
        let snapshot = self.resolved_execution_snapshot(session_id).await?;
        let drive = TurnDrive::for_state(&state, snapshot.workspace_id);
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
    /// drives the steps itself resumes an interrupted turn as this runtime
    /// does. An act with no calls moves the turn to its next reason. Hosts
    /// that are not this runtime use
    /// [`resume_plan_from_log`](crate::host::resume_plan_from_log).
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
        let Some((org_id, turn)) = self.open_turn(session_id).await? else {
            return Ok(None);
        };
        turn_resume::resume_open_turn(self, org_id, turn, self.waits_on_person.as_deref()).await
    }

    /// Read the log forward, keeping only the last turn while it is open,
    /// with the org id its steps run under. A turn parked in this process is
    /// continued through [`resume_steerable_turn`](Self::resume_steerable_turn)
    /// instead.
    async fn open_turn(&self, session_id: SessionId) -> Result<Option<(i64, OpenTurn)>> {
        if lock_parked(&self.parked_turns).contains_key(&session_id) {
            return Ok(None);
        }
        let limit = EventReadLimit::default();
        let mut request = EventReadRequest::new(session_id, limit);
        let mut fold = TurnLogFold::new();
        loop {
            let page = self
                .event_log
                .read_page(request)
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            for event in page.events {
                fold.observe(event);
            }
            let Some(cursor) = page.next_cursor else {
                break;
            };
            request = EventReadRequest::from_cursor(cursor, limit);
        }
        let Some(turn) = fold.open_turn().filter(|turn| !turn.is_answered()) else {
            return Ok(None);
        };
        let snapshot = self.resolved_execution_snapshot(session_id).await?;
        Ok(Some((
            in_process_internal_org_id(&snapshot.organization_id),
            turn,
        )))
    }
}

fn no_interrupted_turn(session_id: SessionId) -> AgentLoopError {
    AgentLoopError::store(format!("session {session_id} has no interrupted turn"))
}
