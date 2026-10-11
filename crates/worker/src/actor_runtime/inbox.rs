//! The turn inbox of a session actor: input reaches the running turn from
//! the session's log, at reason boundaries.
//!
//! Decisions:
//! - The server persists every input before it marks the actor, so the inbox
//!   only reads: `take` and a finishing `after_reason` renew the lease (which
//!   reports the inbox's newest sequence and commits the cursor) and, when
//!   input arrived past the cursor, hand its message ids to the next reason.
//! - `close` and `park` keep nothing: input after the turn's last check
//!   stays past the cursor, the release leaves the actor runnable, and the
//!   next lease starts the turn it asks for. A parked turn stays open in the
//!   log, and its resolution marks the actor runnable again.
//! - Phase boundaries are also where the turn's host stores what it queued
//!   (so the next reason reads every event of the act before it) and starts
//!   the next phase's setup reads.

use async_trait::async_trait;
use everruns_contracts::error::Result;
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
}

impl<H: ActorHosts> ActorInbox<'_, H> {
    /// Renew to learn the inbox's newest input; a failed renew is the
    /// renewer's to judge, and the turn goes on with what it knows.
    async fn refresh(&self) {
        if let Err(error) = self.lease.renew().await {
            tracing::warn!(session_id = %self.lease.session_id, error = %error, "Actor inbox renew failed");
        }
    }
}

#[async_trait]
impl<H: ActorHosts> TurnInbox for ActorInbox<'_, H> {
    async fn take(&self, _session_id: SessionId) -> Result<Vec<MessageId>> {
        self.hosts.flush(self.host).await;
        self.refresh().await;
        let inputs = self.lease.take_inputs().await?;
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
        self.hosts.flush(self.host).await;
        self.refresh().await;
        let inputs = self.lease.take_inputs().await?;
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
        Ok(())
    }
}
