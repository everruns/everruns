//! Turns a process exit cut off before they ended: reading one back from the
//! log, and finishing it.

use everruns_contracts::typed_id::TurnId;
use everruns_core::host::TurnSteering;
use tokio::sync::{mpsc, oneshot, watch};

use super::{Command, RunError, Session, SessionActor, TurnCompletion, TurnEntry, TurnHandle};

impl Session {
    /// The turn a process exit cut off, if the session's last turn is one.
    ///
    /// A running turn, or one waiting on a person (a tool approval, an
    /// `ask_user` question), lives inside the process; when the process
    /// exits, the durable log keeps the turn without an end. After a restart,
    /// rebuild the agent, [`Engine::attach`](crate::Engine::attach) and
    /// [`Engine::resume`](crate::Engine::resume) the session, then ask here.
    /// `None` while a turn of this session runs in this process, or when the
    /// last turn ended or already has its answer.
    ///
    /// Stability: alpha.
    pub async fn interrupted_turn(&self) -> Result<Option<InterruptedTurn>, RunError> {
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Interrupted { response })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        result.await.map_err(|_| RunError::SessionClosed)?
    }

    /// Continue the [`interrupted_turn`](Self::interrupted_turn), then let
    /// the turn carry on. `None` when there is none.
    ///
    /// An unfinished call runs again only when that is safe: its tool is
    /// [`idempotent`](crate::FunctionTool::idempotent), or the call waited
    /// on a person (an approval rule, `ask_user`). Those go through the
    /// agent's tools, approver and `ask_user` responder as on the first run,
    /// so a call that waited on a person asks again, under the same tool
    /// call id. Every other call
    /// ([`InterruptedTurn::not_rerun`]) is recorded as interrupted, and the
    /// model learns its outcome is unknown. A turn cut off outside its tool
    /// calls reasons again. The turn keeps its id; turn-start handlers do
    /// not run again.
    ///
    /// Stability: alpha.
    pub async fn resume_interrupted_turn(&self) -> Result<Option<TurnHandle>, RunError> {
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::ResumeInterrupted { response })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        Ok(result
            .await
            .map_err(|_| RunError::SessionClosed)??
            .map(|(turn_id, completion)| TurnHandle {
                session: self.clone(),
                turn_id,
                completion,
            }))
    }
}

/// A resumed interrupted turn: its id and completion feed.
pub(super) type ResumedTurn = (TurnId, watch::Receiver<TurnCompletion>);

/// A turn a process exit cut off. See [`Session::interrupted_turn`].
///
/// Stability: alpha.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct InterruptedTurn {
    /// Opaque id of the cut-off turn, shared with its events.
    pub turn_id: String,
    /// Its tool calls without a recorded result, in the order the model
    /// made them. Empty when the exit cut the turn off outside its tool
    /// calls.
    pub tool_calls: Vec<crate::ToolCall>,
    /// The subset of [`tool_calls`](Self::tool_calls) resume records as
    /// interrupted instead of running again.
    pub not_rerun: Vec<crate::ToolCall>,
}

impl SessionActor {
    /// The facade view of [`interrupted`](Self::interrupted).
    pub(super) async fn interrupted_turn(&mut self) -> Result<Option<InterruptedTurn>, RunError> {
        Ok(self
            .interrupted()
            .await?
            .map(|interrupted| InterruptedTurn {
                turn_id: interrupted.turn_id.to_string(),
                tool_calls: interrupted.tool_calls,
                not_rerun: interrupted.not_rerun,
            }))
    }

    /// The turn a process exit cut off, with its unfinished tool calls.
    async fn interrupted(
        &mut self,
    ) -> Result<Option<everruns_core::host::InterruptedToolCalls>, RunError> {
        self.ensure_runtime().await?;
        Ok(self
            .runtime
            .as_ref()
            .expect("runtime built above")
            .interrupted_tool_calls(self.session_id)
            .await?)
    }

    /// Continue a turn a process exit cut off. Lifecycle
    /// turn-start handlers do not run again: it is the same turn.
    pub(super) async fn resume_interrupted(
        &mut self,
        response: oneshot::Sender<Result<Option<ResumedTurn>, RunError>>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> bool {
        let interrupted = match self.interrupted().await {
            Ok(Some(interrupted)) => interrupted,
            other => {
                let _ = response.send(other.map(|_| None));
                return true;
            }
        };
        self.hook_state.begin_turn();
        let (completion_tx, completion_rx) = watch::channel(TurnCompletion::Pending);
        let _ = response.send(Ok(Some((interrupted.turn_id, completion_rx))));
        self.drive_turn(
            TurnEntry::Interrupted,
            interrupted.turn_id,
            TurnSteering::new(),
            completion_tx,
            commands,
        )
        .await
    }
}
