//! Reconstructible handles contain identities, never completion channels.
use super::*;
use crate::{RunError, Turn};
use everruns_contracts::typed_id::TurnId;

/// Fallback receipt poll for [`Admission::wait`] when no local hint fires.
const ADMISSION_POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
/// The logical turn that consumed an admitted input.
pub struct TurnAssignment {
    /// Stable turn identity allocated by the executor.
    pub turn_id: TurnId,
    /// Whether input joined an already started turn.
    pub steered: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
/// Authoritative progress of one accepted input.
pub enum AdmissionState {
    /// Accepted but not consumed.
    Pending,
    /// Consumed by a logical turn.
    Assigned {
        /// Authoritative input-to-turn association.
        assignment: TurnAssignment,
    },
    /// The assigned turn is durably waiting for client tool results.
    Waiting {
        /// The turn that consumed this input.
        assignment: TurnAssignment,
        /// Unresolved calls, in their canonical order.
        calls: Vec<crate::ToolCall>,
    },
    /// The pre-effect gate rejected this exact input. A new command may retry
    /// a known failed preparation; uncertain effects remain suspended.
    Rejected {
        /// Typed application gate error retained by the authority.
        failure: crate::HookFailure,
    },
    /// Runtime termination and awaited lifecycle finalization committed.
    Complete {
        /// Finalized Framework outcome.
        turn: Turn,
    },
    /// Execution requires host intervention.
    Suspended,
}

/// A durable admission identity. Dropping it never cancels accepted work.
#[derive(Clone)]
pub struct Admission {
    pub(crate) hosting: Arc<dyn SessionHosting>,
    pub(crate) session: SessionId,
    pub(crate) receipt: AdmissionReceipt,
}

impl Admission {
    /// Stable command identity.
    pub fn message_id(&self) -> MessageId {
        self.receipt.message_id
    }
    /// Canonical admission sequence.
    pub fn sequence(&self) -> i32 {
        self.receipt.sequence
    }

    /// Read consumption, returning `None` while still in the inbox.
    pub async fn assignment(&self) -> Result<Option<TurnAssignment>> {
        self.hosting
            .record(self.session, &format!("assignment/{}", self.message_id()))
            .await?
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| {
                everruns_contracts::error::AgentLoopError::store("invalid admission assignment")
            })
    }

    /// Read current progress without starting execution.
    pub async fn state(&self) -> Result<AdmissionState> {
        if let Some(rejected) = self
            .hosting
            .record(self.session, &format!("rejected/{}", self.message_id()))
            .await?
        {
            let failure = serde_json::from_value(rejected["failure"].clone()).map_err(|_| {
                everruns_contracts::error::AgentLoopError::store("invalid input rejection receipt")
            })?;
            return Ok(AdmissionState::Rejected { failure });
        }
        if let Some(assignment) = self.assignment().await? {
            if let Some(outcome) = self
                .hosting
                .record(self.session, &format!("outcome/{}", assignment.turn_id))
                .await?
            {
                return Ok(AdmissionState::Complete {
                    turn: serde_json::from_value(outcome).map_err(|_| {
                        everruns_contracts::error::AgentLoopError::store(
                            "invalid Framework outcome",
                        )
                    })?,
                });
            }
            let status = self.hosting.status(self.session).await?;
            if status.suspended {
                return Ok(AdmissionState::Suspended);
            }
            if status.open_turn == Some(assignment.turn_id) && !status.waiting.is_empty() {
                return Ok(AdmissionState::Waiting {
                    assignment,
                    calls: status.waiting,
                });
            }
            return Ok(AdmissionState::Assigned { assignment });
        }
        if self.hosting.status(self.session).await?.suspended {
            return Ok(AdmissionState::Suspended);
        }
        Ok(AdmissionState::Pending)
    }

    /// Poll authoritative receipts. A frontend or executor restart cannot lose
    /// completion. Explicit suspension is returned rather than waiting forever.
    ///
    /// The host's [`SessionHosting::changes`] hint wakes the waiter as soon as
    /// a local commit lands; the poll only covers writes the hint cannot see
    /// (another process, a remote host). Polling alone added up to one
    /// interval to every turn: in-process turns that finish in ~2 ms took
    /// ~51 ms (turn_backends bench, Durable Benchmarks 2026-10-18).
    pub async fn wait(&self) -> std::result::Result<Turn, RunError> {
        let changes = self.hosting.changes(self.session);
        loop {
            // Created before the read: `notify_waiters` reaches a `Notified`
            // from creation on, so a commit between the read and the wait
            // still wakes this loop.
            let changed = changes.as_deref().map(tokio::sync::Notify::notified);
            match self.state().await? {
                AdmissionState::Complete { turn } => return Ok(turn),
                AdmissionState::Suspended => return Err(RunError::Suspended),
                AdmissionState::Rejected { failure } => return Err(RunError::Hook(failure)),
                _ => {
                    let poll = everruns_contracts::rt::sleep(ADMISSION_POLL);
                    match changed {
                        Some(changed) => {
                            tokio::select! {
                                _ = changed => {}
                                _ = poll => {}
                            }
                        }
                        None => poll.await,
                    }
                }
            }
        }
    }

    /// Acceptance of a targeted cancellation is separate from its outcome.
    /// Retain `command` when retrying a lost reply.
    pub async fn cancel(&self, command: uuid::Uuid) -> Result<()> {
        self.hosting
            .cancel(self.session, command, self.message_id())
            .await
    }
}
