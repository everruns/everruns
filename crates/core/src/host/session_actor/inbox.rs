//! The turn inbox of a session actor: input reaches the running turn from
//! the session's log, at reason boundaries.
//!
//! Decisions:
//! - The server persists every input before it marks the actor, so the inbox
//!   only reads: `take` and a finishing `after_reason` renew the lease (which
//!   reports the inbox's newest sequence and commits the cursor) and, when
//!   input arrived past the cursor, hand its message ids to the next reason.
//! - A parked drive must keep its last consumed cursor: a resolution can
//!   arrive during its final flush, before it releases the lease. Scanning
//!   past that resolution would acknowledge work the parked drive never ran.
//! - Phase boundaries are also where the turn's host stores what it queued
//!   (so the next reason reads every event of the act before it) and starts
//!   the next phase's setup reads.

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};

use super::lease::Lease;
use super::{ActorHosts, Phase};
use crate::engine::TurnState;
use crate::host::{PendingInput, TurnDrive, TurnInbox};

pub(crate) struct ActorInbox<'a, H: ActorHosts> {
    pub(crate) lease: &'a Lease,
    pub(crate) hosts: &'a H,
    pub(crate) host: &'a H::Host,
    pub(crate) drive: &'a TurnDrive,
    pub(crate) parked: AtomicBool,
    /// The driver's drain flag (`SessionActorDriver::hand_off_running_turns`).
    pub(crate) hand_off: &'a AtomicBool,
}

impl<H: ActorHosts> ActorInbox<'_, H> {
    pub(crate) fn parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    /// Renew to learn the inbox's newest input; a failed renew is the
    /// renewer's to judge, and the turn goes on with what it knows.
    async fn refresh(&self) {
        match self.lease.renew().await {
            // The session was deleted (EVE-1235): the lease is lost and the
            // turn stops on it.
            Ok(()) | Err(AgentLoopError::SessionNotFound(_)) => {}
            Err(error) => {
                tracing::warn!(session_id = %self.lease.session_id, error = %error, "Actor inbox renew failed");
            }
        }
    }
}

#[async_trait]
impl<H: ActorHosts> TurnInbox for ActorInbox<'_, H> {
    async fn take(&self, _session_id: SessionId) -> Result<Vec<MessageId>> {
        // A gap in the log stops the turn before its input is committed.
        self.hosts.flush(self.host).await?;
        self.lease.turn_stored();
        self.refresh().await;
        let inputs = self.lease.take_inputs().await?;
        self.hosts.inputs_consumed(self.drive, &inputs).await?;
        if !inputs.is_empty() {
            self.refresh().await;
        }
        if self.lease.cancel_requested() {
            return Err(AgentLoopError::store("turn cancellation accepted"));
        }
        self.hosts.begin_phase(self.host, self.drive, Phase::Reason);
        Ok(inputs)
    }

    async fn after_reason(
        &self,
        _session_id: SessionId,
        would_finish: bool,
    ) -> Result<PendingInput> {
        if !would_finish {
            // The turn goes on regardless; the input is delivered at the next
            // `take`. The count only decides the stop reason of a turn that
            // hit its iteration cap.
            let pending = usize::from(self.lease.has_pending());
            self.hosts.begin_phase(self.host, self.drive, Phase::Act);
            return Ok(PendingInput::new(pending, Vec::new()));
        }
        // A gap in the log stops the turn before its input is committed.
        self.hosts.flush(self.host).await?;
        self.lease.turn_stored();
        self.refresh().await;
        let inputs = self.lease.take_inputs().await?;
        self.hosts.inputs_consumed(self.drive, &inputs).await?;
        if !inputs.is_empty() {
            self.refresh().await;
        }
        if self.lease.cancel_requested() {
            return Err(AgentLoopError::store("turn cancellation accepted"));
        }
        Ok(PendingInput::new(inputs.len(), inputs))
    }

    async fn close(&self, _session_id: SessionId) -> Result<()> {
        Ok(())
    }

    async fn park(
        &self,
        _session_id: SessionId,
        _turn_id: TurnId,
        _client_tool_calls: Vec<ToolCall>,
        _resume: TurnState,
    ) -> Result<()> {
        self.parked.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn should_hand_off(&self) -> bool {
        self.hand_off.load(Ordering::SeqCst)
    }
}
