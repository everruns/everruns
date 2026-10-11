//! One leased session actor: run its turn (fresh or resumed from the log),
//! or end it (cancel, poison seal), then give the lease back.
//!
//! Decisions:
//! - The log decides. The tail from the latest `turn.started` is folded: an
//!   open turn is resumed (`resume_open_turn`, with unfinished calls settled
//!   or rerun by what their tools declare), an open turn that already has its
//!   answer is completed, and with no open turn the first unconsumed input
//!   starts a fresh turn through the same entry step the in-process runtime
//!   uses (`turn_entry_plan`). One turn per lease: a release that leaves the
//!   actor runnable (input arrived as the turn ended) wakes the next lease.
//! - Lease loss stops the turn where it is: the drive is dropped, events
//!   still queued are dropped, and nothing is released. The new holder folds
//!   the log and goes on.
//! - A retryable failure (a store or transport error) also lets the lease
//!   expire, after storing what the turn emitted: the next lease resumes from
//!   the log, and the TTL is the backoff. A non-retryable one fails the turn
//!   (user-facing message, `turn.failed`, idle) and releases.
//! - Poison guard: a lease that finds more attempts than the threshold (each
//!   lease counts one; any release resets the count) seals the turn instead of
//!   running it, so a turn that kills its worker cannot loop forever.
//! - Cancel: the holder records it (single writer; the server does not when
//!   a live holder runs the actor). The drive is dropped first, and the
//!   cancel is recorded only while the turn is still open in the log, so a
//!   turn ends with exactly one terminal event.
//! - Input that reaches a turn parked on a person resolves the pause, as the
//!   server's message intake does for a session it sees waiting: the pending
//!   calls are settled as cancelled and the turn goes on to the input. Without
//!   it, a message that landed while the turn was still running and then
//!   parked would leave the actor runnable on a turn that can only park again.

use std::sync::Arc;

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, TurnId};
use everruns_contracts::user_facing_error::{
    UserFacingError, UserFacingErrorContext, classify_runtime_error_message, codes,
};

use super::inbox::ActorInbox;
use super::lease::{Lease, renew_loop, sequence, turn_input};
use super::{ActorHosts, ActorRuntime, HostTurn, TurnSignals};
use crate::actor_lease_store::{ActorRelease, LeasedActor};
use crate::core::RuntimeMessage;
use crate::core::events::{
    Event, EventContext, EventData, EventRequest, InputMessageData, OutputMessageCompletedData,
    ToolCompletedData, TurnCancelledData,
};
use crate::host::{
    InProcessExecution, OpenTurn, RuntimeHostAdapter, RuntimeSessionLifecycle, TurnDrive,
    TurnLogFold, drive_turn, resume_entry_plan, resume_open_turn, turn_entry_plan,
};

/// Seal reason recorded on `turn.sealed` by the poison guard.
pub const NO_PROGRESS_SEAL_REASON: &str = "no_progress";

/// How a lease ends.
enum Finish {
    /// Release, consuming input up to the cursor.
    Release,
    /// Keep the lease until it expires: lost, or a retryable failure.
    Expire,
}

/// Aborts the renewer when the actor's run ends, however it ends.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<H: ActorHosts> ActorRuntime<H> {
    /// Run the leased `actor` to the end of one turn and release it.
    pub async fn run_actor(&self, actor: LeasedActor) {
        let lease = Arc::new(Lease::new(
            self.store.clone(),
            self.config.holder.clone(),
            &actor,
            self.config.lease_ttl,
            self.config.journal_page,
        ));
        let renewer = AbortOnDrop(tokio::spawn(renew_loop(
            lease.clone(),
            self.renew_poke.clone(),
        )));
        let finish = self.run_leased(&lease, &actor).await;
        drop(renewer);
        match finish {
            Ok(Finish::Release) if !lease.is_lost() => match lease.release().await {
                Ok(ActorRelease::Released { still_runnable }) => {
                    tracing::debug!(session_id = %actor.session_id, still_runnable, "Actor released");
                }
                Ok(ActorRelease::LeaseLost) => {
                    tracing::warn!(session_id = %actor.session_id, "Actor lease lost before release");
                }
                Err(error) => {
                    tracing::warn!(session_id = %actor.session_id, error = %error, "Actor release failed; the lease will expire");
                }
            },
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(session_id = %actor.session_id, error = %error, "Actor turn failed; the lease will expire and the turn resumes from the log");
            }
        }
    }

    fn signals(lease: &Lease) -> TurnSignals {
        // Tools stop on either signal; `cancel` alone means the person asked.
        let (stop_tx, stop) = tokio::sync::watch::channel(false);
        let (mut lost, mut cancel) = (lease.lost(), lease.cancel());
        let cancel_view = lease.cancel();
        tokio::spawn(async move {
            tokio::select! {
                _ = lost.wait_for(|lost| *lost) => {}
                _ = cancel.wait_for(|cancel| *cancel) => {}
                () = stop_tx.closed() => return,
            }
            let _ = stop_tx.send(true);
        });
        TurnSignals {
            stop,
            cancel: cancel_view,
        }
    }

    async fn run_leased(&self, lease: &Lease, actor: &LeasedActor) -> Result<Finish> {
        let org_id = actor.org_id;
        let session_id = actor.session_id;
        let signals = Self::signals(lease);
        let setup = self.hosts.host(
            HostTurn {
                org_id,
                session_id,
                agent_id: None,
                input_message_id: None,
            },
            &signals,
        );
        let tail = lease.read_tail().await?;
        let started_at = tail
            .first()
            .filter(|event| event.event_type == crate::core::TURN_STARTED)
            .and_then(|event| event.sequence)
            .map(i64::from);
        let open =
            TurnLogFold::from_events(tail.clone()).filter(|turn| turn.session_id() == session_id);
        if let (Some(_), Some(sequence)) = (&open, started_at) {
            lease.advance(sequence);
        }

        if actor.attempts > self.config.poison_attempts {
            self.seal(lease, &setup, org_id, open, actor.next_input_seq)
                .await?;
            return Ok(Finish::Release);
        }

        let Some(turn) = open else {
            let Some(input) = lease.fresh_input(actor.next_input_seq).await? else {
                // A mark with nothing to run (its input was consumed).
                lease.advance(lease.inbox_seq());
                return Ok(Finish::Release);
            };
            lease.advance(input.sequence);
            let resolved = setup
                .load_resolved_turn_for_execution(org_id, session_id, input.message_id)
                .await?;
            let snapshot = resolved.snapshot;
            let drive = TurnDrive::new(
                session_id,
                org_id,
                TurnId::new(),
                input.message_id,
                snapshot.harness_id,
                snapshot.agent_id,
                snapshot.workspace_id,
            );
            let host = self.turn_host(&drive, &signals);
            let (execution, plan) = turn_entry_plan(&host, &drive).await?;
            return self.drive(lease, &host, drive, execution, plan).await;
        };

        if lease.cancel_requested() {
            let drive = self.open_drive(&setup, org_id, &turn).await?;
            self.record_cancel(lease, &setup, &drive).await?;
            return Ok(Finish::Release);
        }
        if turn.is_answered() {
            let drive = self.open_drive(&setup, org_id, &turn).await?;
            let lifecycle = RuntimeSessionLifecycle::new(setup.clone(), org_id, session_id);
            lifecycle
                .turn_completed(
                    turn.turn_id(),
                    turn.input_message_id(),
                    turn.reasons(),
                    turn.usage().cloned(),
                    None,
                )
                .await?;
            lifecycle
                .fire_turn_end_hooks(drive.harness_id, drive.agent_id, drive.turn_id, true)
                .await;
            lease.skip_to_next_input().await?;
            return Ok(Finish::Release);
        }
        let turn = if settle_pause_for_input(lease, &setup, &turn, &tail).await? {
            match TurnLogFold::from_events(lease.read_tail().await?) {
                Some(turn) => turn,
                None => return Ok(Finish::Release),
            }
        } else {
            turn
        };
        let drive = self.open_drive(&setup, org_id, &turn).await?;
        let host = self.turn_host(&drive, &signals);
        let waits_on_person = self.hosts.waits_on_person();
        let Some((state, act)) =
            resume_open_turn(&host, org_id, turn, waits_on_person.as_deref()).await?
        else {
            return Ok(Finish::Release);
        };
        let plan = resume_entry_plan(&state, act);
        self.drive(lease, &host, drive, InProcessExecution::new(state), plan)
            .await
    }

    /// The identity of the open `turn`, with the session's harness, agent and
    /// workspace.
    async fn open_drive(&self, host: &H::Host, org_id: i64, turn: &OpenTurn) -> Result<TurnDrive> {
        let snapshot = host
            .load_resolved_turn(org_id, turn.session_id())
            .await?
            .snapshot;
        Ok(TurnDrive::new(
            turn.session_id(),
            org_id,
            turn.turn_id(),
            turn.input_message_id(),
            snapshot.harness_id,
            snapshot.agent_id,
            snapshot.workspace_id,
        ))
    }

    fn turn_host(&self, drive: &TurnDrive, signals: &TurnSignals) -> H::Host {
        self.hosts.host(
            HostTurn {
                org_id: drive.org_id,
                session_id: drive.session_id,
                agent_id: drive.agent_id,
                input_message_id: Some(drive.input_message_id),
            },
            signals,
        )
    }

    async fn drive(
        &self,
        lease: &Lease,
        host: &H::Host,
        drive: TurnDrive,
        execution: InProcessExecution,
        plan: crate::engine::TurnPlan,
    ) -> Result<Finish> {
        let inbox = ActorInbox {
            lease,
            hosts: &*self.hosts,
            host,
            drive: &drive,
        };
        let (mut lost, mut cancel) = (lease.lost(), lease.cancel());
        enum Stopped {
            Lost,
            Cancel,
            Done(Result<crate::host::TurnResult>),
        }
        let stopped = tokio::select! {
            biased;
            _ = lost.wait_for(|lost| *lost) => Stopped::Lost,
            _ = cancel.wait_for(|cancel| *cancel) => Stopped::Cancel,
            result = drive_turn(host, &inbox, drive.clone(), execution, plan) => Stopped::Done(result),
        };
        let finish = match stopped {
            Stopped::Lost => {
                self.hosts.abandon(host);
                Finish::Expire
            }
            Stopped::Cancel => {
                self.hosts.flush(host).await;
                self.record_cancel(lease, host, &drive).await?;
                Finish::Release
            }
            Stopped::Done(Ok(_)) => {
                self.hosts.flush(host).await;
                lease.skip_to_next_input().await?;
                Finish::Release
            }
            Stopped::Done(Err(error)) if error.is_non_retryable() => {
                self.hosts.flush(host).await;
                self.record_failure(host, &drive, &error).await?;
                lease.skip_to_next_input().await?;
                Finish::Release
            }
            Stopped::Done(Err(error)) => {
                self.hosts.flush(host).await;
                tracing::warn!(
                    session_id = %drive.session_id,
                    turn_id = %drive.turn_id,
                    error = %error,
                    "Actor turn hit a retryable failure; the lease will expire and the turn resumes from the log"
                );
                Finish::Expire
            }
        };
        self.hosts.turn_ended(&drive).await;
        Ok(finish)
    }

    /// Record the cancel of `drive`'s turn, if the log still has it open:
    /// `turn.cancelled`, the person's request as part of the turn, and the
    /// session idled. Consumes every pending input, as the server's withdraw
    /// does.
    async fn record_cancel(&self, lease: &Lease, host: &H::Host, drive: &TurnDrive) -> Result<()> {
        let still_open = TurnLogFold::from_events(lease.read_tail().await?)
            .is_some_and(|turn| turn.turn_id() == drive.turn_id);
        if still_open {
            let context = EventContext::turn(drive.turn_id, drive.input_message_id);
            let emitter = host.event_emitter();
            emitter
                .emit(EventRequest::new(
                    drive.session_id,
                    context.clone(),
                    TurnCancelledData {
                        turn_id: drive.turn_id,
                        reason: Some("User requested cancellation".to_string()),
                        usage: None,
                    },
                ))
                .await?;
            emitter
                .emit(EventRequest::new(
                    drive.session_id,
                    context,
                    InputMessageData::new(RuntimeMessage::user(
                        "User requested to cancel the work.",
                    )),
                ))
                .await?;
            RuntimeSessionLifecycle::new(host.clone(), drive.org_id, drive.session_id)
                .emit_session_idled(drive.turn_id, drive.input_message_id, None, None)
                .await?;
        }
        lease.advance(lease.inbox_seq());
        Ok(())
    }

    /// Fail `drive`'s turn on a non-retryable error: a user-facing message,
    /// `turn.failed`, the session idled, and the `turn_end` hooks.
    async fn record_failure(
        &self,
        host: &H::Host,
        drive: &TurnDrive,
        error: &AgentLoopError,
    ) -> Result<()> {
        tracing::error!(
            session_id = %drive.session_id,
            turn_id = %drive.turn_id,
            error = %error,
            "Actor turn failed"
        );
        let user_error =
            classify_runtime_error_message(&error.to_string(), &UserFacingErrorContext::default());
        let shown = user_error.fallback_message();
        assistant_error(host, drive, &shown, &user_error).await?;
        let lifecycle = RuntimeSessionLifecycle::new(host.clone(), drive.org_id, drive.session_id);
        lifecycle
            .turn_failed(
                drive.turn_id,
                drive.input_message_id,
                &shown,
                Some(&user_error),
            )
            .await?;
        lifecycle
            .fire_turn_end_hooks(drive.harness_id, drive.agent_id, drive.turn_id, false)
            .await;
        Ok(())
    }

    /// Seal the turn whose holders keep dying: the open turn, or the fresh
    /// input that would start one. Consumes every pending input.
    async fn seal(
        &self,
        lease: &Lease,
        host: &H::Host,
        org_id: i64,
        open: Option<OpenTurn>,
        next_input_seq: i64,
    ) -> Result<()> {
        let session_id = lease.session_id;
        let sealed: Option<(TurnId, MessageId, u32, _)> = match open {
            Some(turn) => Some((
                turn.turn_id(),
                turn.input_message_id(),
                turn.reasons(),
                turn.usage().cloned(),
            )),
            None => lease
                .fresh_input(next_input_seq)
                .await?
                .map(|input| (TurnId::new(), input.message_id, 0, None)),
        };
        if let Some((turn_id, input_message_id, reasons, usage)) = sealed {
            tracing::error!(%session_id, %turn_id, "Sealing a session turn that keeps failing without progress");
            let drive = TurnDrive::new(
                session_id,
                org_id,
                turn_id,
                input_message_id,
                Default::default(),
                None,
                Default::default(),
            );
            assistant_error(
                host,
                &drive,
                "This turn was stopped because it repeatedly failed without making progress.",
                &UserFacingError::new(codes::PROCESSING_ERROR),
            )
            .await?;
            RuntimeSessionLifecycle::new(host.clone(), org_id, session_id)
                .turn_sealed(
                    turn_id,
                    input_message_id,
                    NO_PROGRESS_SEAL_REASON,
                    reasons,
                    usage,
                )
                .await?;
        }
        lease.advance(lease.inbox_seq());
        Ok(())
    }
}

/// The calls of the tail's latest client pause (`tool.call_requested`) that
/// have no result yet.
fn paused_calls(tail: &[Event]) -> Vec<everruns_contracts::tool_types::ToolCall> {
    let mut paused = Vec::new();
    for event in tail {
        match &event.data {
            EventData::ToolCallRequested(requested) => paused = requested.tool_calls.clone(),
            EventData::ToolCompleted(completed) => {
                paused.retain(|call| call.id != completed.tool_call_id);
            }
            _ => {}
        }
    }
    paused
}

/// When input past the cursor reaches `turn` parked on a person, settle the
/// pending calls as cancelled under the turn (see the module notes). `true`
/// when it settled any.
async fn settle_pause_for_input<A: RuntimeHostAdapter>(
    lease: &Lease,
    host: &A,
    turn: &OpenTurn,
    tail: &[Event],
) -> Result<bool> {
    let input_waiting = tail
        .iter()
        .any(|event| turn_input(event).is_some() && sequence(event) > lease.cursor());
    let paused = paused_calls(tail);
    if !input_waiting || paused.is_empty() {
        return Ok(false);
    }
    let context = EventContext::turn(turn.turn_id(), turn.input_message_id());
    let emitter = host.event_emitter();
    for call in paused {
        // The same results the server's intake records for a waiting session.
        let completed = if call.name == everruns_contracts::ASK_USER_TOOL_NAME {
            let result = crate::core::builtins::ask_user::AskUserResult {
                status: crate::core::builtins::ask_user::AskUserStatus::Cancelled,
                answered_by: crate::core::builtins::ask_user::AskUserAnsweredBy::Unattended,
                answers: Vec::new(),
            };
            ToolCompletedData::success(
                call.id,
                call.name,
                vec![crate::core::ContentPart::tool_result_text(
                    &serde_json::to_value(result).unwrap_or_default(),
                )],
                None,
            )
        } else {
            ToolCompletedData::failure(
                call.id,
                call.name,
                "cancelled".to_string(),
                "Cancelled because the user sent a message instead".to_string(),
                None,
            )
        };
        emitter
            .emit(EventRequest::new(
                turn.session_id(),
                context.clone(),
                completed,
            ))
            .await?;
    }
    Ok(true)
}

/// An assistant message that shows `user_error` in the conversation.
async fn assistant_error<A: RuntimeHostAdapter>(
    host: &A,
    drive: &TurnDrive,
    shown: &str,
    user_error: &UserFacingError,
) -> Result<()> {
    let mut message = RuntimeMessage::assistant(shown);
    let mut metadata = std::collections::HashMap::new();
    user_error.apply_to_message_metadata(&mut metadata);
    message.metadata = Some(metadata);
    host.event_emitter()
        .emit(EventRequest::new(
            drive.session_id,
            EventContext::turn(drive.turn_id, drive.input_message_id),
            OutputMessageCompletedData::new(message).with_user_facing_error(user_error),
        ))
        .await?;
    Ok(())
}
