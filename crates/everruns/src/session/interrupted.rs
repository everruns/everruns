//! Turns a process exit cut off while their tool calls ran: reading one back
//! from the log, and finishing it by running its unfinished calls again.

use everruns_host::TurnSteering;
use everruns_provider::typed_id::TurnId;
use tokio::sync::{mpsc, oneshot, watch};

use super::{Command, RunError, Session, SessionActor, TurnCompletion, TurnEntry, TurnHandle};

impl Session {
    /// The turn a process exit cut off while its tool calls ran, if the
    /// session's last turn is one.
    ///
    /// A turn waiting on a person (a tool approval, an `ask_user` question)
    /// waits inside the process; when the process exits, the durable log
    /// keeps the turn without an end. After a restart, rebuild the agent,
    /// [`Engine::attach`](crate::Engine::attach) and
    /// [`Engine::resume`](crate::Engine::resume) the session, then ask here.
    /// `None` while a turn of this session runs in this process, or when the
    /// last turn ended or was cut off outside its tool calls.
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

    /// Continue the [`interrupted_turn`](Self::interrupted_turn): run its
    /// unfinished tool calls again, then let the turn carry on. `None` when
    /// there is none.
    ///
    /// The calls go through the agent's tools, approver and `ask_user`
    /// responder as on the first run, so a call that waited on a person asks
    /// again, under the same tool call id. A call the exit cut off while it
    /// executed runs a second time; check
    /// [`InterruptedTurn::tool_calls`] first when that matters. The turn
    /// keeps its id; turn-start handlers do not run again.
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

/// A turn a process exit cut off while its tool calls ran. See
/// [`Session::interrupted_turn`].
///
/// Stability: alpha.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct InterruptedTurn {
    /// Opaque id of the cut-off turn, shared with its events.
    pub turn_id: String,
    /// Its tool calls without a recorded result, in the order the model
    /// made them.
    pub tool_calls: Vec<crate::ToolCall>,
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
            }))
    }

    /// The unfinished tool calls of a turn a process exit cut off.
    async fn interrupted(
        &mut self,
    ) -> Result<Option<everruns_host::InterruptedToolCalls>, RunError> {
        self.ensure_runtime().await?;
        Ok(self
            .runtime
            .as_ref()
            .expect("runtime built above")
            .interrupted_tool_calls(self.session_id)
            .await?)
    }

    /// Continue a turn a process exit cut off in its tool calls. Lifecycle
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
