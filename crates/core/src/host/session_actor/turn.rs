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
//!   the log and goes on. Every append carries the lease's fence and the
//!   control plane refuses a stale one in the insert's transaction, so a
//!   write already in flight when the lease was lost does not land either;
//!   the refusal itself is a way the holder learns of the loss.
//! - A retryable failure (a store or transport error) runs the turn again from
//!   the log on the same lease, after winding the failed run down (its queued
//!   writes dropped or stored, its turns ended on this worker) and a doubling
//!   backoff, up to `SessionActorConfig::control_plane_retries` times. Past
//!   that the lease is left to expire: the next lease resumes from the log,
//!   and the TTL is the backoff. A non-retryable one fails the turn
//!   (user-facing message, `turn.failed`, idle) and releases.
//! - A deleted session (EVE-1235) stops the turn like a lost lease, but
//!   quietly: its actor row went with it, so there is nothing to fail, record
//!   or release, and a renew or append that finds it gone is not a fault.
//! - Stall watchdog: setup before the turn is driven and the flush and
//!   checkpoint after it are bounded (`SessionActorConfig::stall_timeout`, see
//!   `lease.rs`). Past the bound the lease stops renewing and is lost, the
//!   stuck phase is dropped, and the lease expires: a holder that hangs where
//!   it writes nothing costs the bound plus a TTL, then the next lease (and,
//!   if it keeps hanging, the poison guard) takes over.
//! - Poison guard: a lease that finds more attempts than the threshold (each
//!   lease counts one; any release resets the count) seals the turn instead of
//!   running it, so a turn that kills its worker cannot loop forever.
//! - The poison guard counts holder deaths, not failures to reach the control
//!   plane (EVE-1301). Every lease that ended on a retryable error used to
//!   count like a crash, so a database blip that spanned three leases (chaos
//!   harness: a pool exhaustion, then a server kill) sealed a healthy turn.
//!   Retrying on the lease (above) keeps those failures off `attempts` and
//!   bounds them separately, at `control_plane_retries` per lease: a turn
//!   whose own work keeps failing that way still exhausts the retries, lets
//!   the lease expire and counts an attempt, so it is sealed after
//!   `poison_attempts` such leases instead of looping forever. Rejected:
//!   releasing without committing the cursor instead of expiring. Release
//!   resets `attempts`, so a turn that fails retryably every time would never
//!   be sealed; it clears the runnable marker when no new input is pending,
//!   leaving an open turn with nothing to resume it; and it drops the TTL as
//!   a backoff against a struggling control plane. Also rejected: skipping
//!   the increment in the control plane (a report at the end of the lease,
//!   with its own counter). That report fails in the very outage it reports,
//!   so a blip would still count; the retry waits the blip out instead, and
//!   needs no schema or protocol change.
//! - Hand-off: a draining holder (`SessionActorDriver::with_hand_off`) stops
//!   the turn at its next step boundary through the inbox
//!   (`TurnInbox::should_hand_off`), after at least one step, with that
//!   step's events stored, and gives the lease back with the actor still
//!   runnable. The next holder folds the log like after a crash, but without
//!   waiting out a TTL and without a cut-off call to rerun. A store that
//!   cannot hand off lets the lease expire instead.
//! - Cancel: the holder records it (single writer; the server does not when
//!   a live holder runs the actor). The drive is dropped first, and the
//!   cancel is recorded only while the turn is still open in the log, so a
//!   turn ends with exactly one terminal event. A cancel ends only the turn
//!   it targets (see `lease.rs`): one left pending by a holder that recorded
//!   it and died before releasing does not end the next message's turn.
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
use super::store::{ActorRelease, LeasedActor};
use super::{ActorHosts, HostTurn, SessionActorDriver, TurnSignals};
use crate::RuntimeMessage;
use crate::events::{
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
    /// Release ownership while preserving the retry checkpoint and alarm.
    /// An intentional wait bypasses the control-plane failure retry loop.
    Park,
    /// The holder is draining: the turn stopped between steps with its events
    /// stored; give the lease back with the actor still runnable.
    HandOff,
    /// Keep the lease until it expires: lost, or a stall.
    Expire,
    /// A retryable failure the drive stopped on, its queued events stored or
    /// given up on: run the turn again from the log (on this lease while
    /// retries last, else as `Expire`).
    Retry(AgentLoopError),
}

/// The hosts and turns one run of a lease made, so a retry on the same lease
/// can wind them down first: a write the failed run still had queued must not
/// land under the same fence after the retry folded the log.
struct Made<H: ActorHosts> {
    hosts: std::sync::Mutex<Vec<H::Host>>,
    drives: std::sync::Mutex<Vec<TurnDrive>>,
}

impl<H: ActorHosts> Made<H> {
    fn new() -> Self {
        Self {
            hosts: std::sync::Mutex::new(Vec::new()),
            drives: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn host(&self, host: H::Host) -> H::Host {
        self.hosts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(host.clone());
        host
    }

    fn turn(&self, host: H::Host, drive: &TurnDrive) -> H::Host {
        self.drives
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(drive.clone());
        self.host(host)
    }

    fn take(&self) -> (Vec<H::Host>, Vec<TurnDrive>) {
        let hosts = std::mem::take(
            &mut *self
                .hosts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        let drives = std::mem::take(
            &mut *self
                .drives
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        (hosts, drives)
    }
}

/// Aborts the renewer when the actor's run ends, however it ends.
struct AbortOnDrop(everruns_contracts::rt::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<H: ActorHosts> SessionActorDriver<H> {
    /// Run the leased `actor` to the end of one turn and release it.
    pub async fn run_actor(&self, actor: LeasedActor) {
        let lease = Arc::new(
            Lease::new(
                self.store.clone(),
                self.config.holder.clone(),
                &actor,
                self.config.lease_ttl,
                self.config.journal_page,
            )
            .with_stall_timeout(self.config.stall_timeout),
        );
        let renewer = AbortOnDrop(everruns_contracts::rt::spawn(renew_loop(
            lease.clone(),
            self.renew_poke.clone(),
        )));
        // A retryable failure runs the turn again on this lease, off the
        // poison guard's count (module notes).
        let mut retries = 0;
        let finish = loop {
            let made = Made::new();
            // Boxed: the run's future (setup, the whole driven turn) is large,
            // and inline in this loop it overflowed a test thread's stack.
            let finish = Box::pin(self.run_once(&lease, &actor, &made)).await;
            let error = match finish {
                Ok(Finish::Retry(error)) | Err(error)
                    if retries < self.config.control_plane_retries
                        && !lease.is_lost()
                        && !error.is_non_retryable()
                        && !session_deleted(&lease, &error).await =>
                {
                    error
                }
                finish => break finish,
            };
            retries += 1;
            tracing::warn!(
                session_id = %actor.session_id,
                retry = retries,
                error = %error,
                "Actor turn hit a retryable failure; running it again from the log on this lease"
            );
            if !self.wind_down(&lease, &made, retries).await {
                break Err(error);
            }
            lease.rewind();
        };
        // Lease-release replies may arrive after another holder takes over;
        // record the execution boundary before that administrative RPC.
        tracing::debug!(session_id = %actor.session_id, "Actor turn execution finished");
        drop(renewer);
        match finish {
            Ok(Finish::Release | Finish::Park) if !lease.is_lost() => match lease.release().await {
                Ok(ActorRelease::Released { still_runnable }) => {
                    tracing::debug!(session_id = %actor.session_id, still_runnable, "Actor released");
                }
                Ok(ActorRelease::LeaseLost) => {
                    if matches!(lease.renew().await, Err(AgentLoopError::SessionNotFound(_))) {
                        // Deleted as the turn ended (EVE-1235).
                        tracing::info!(session_id = %actor.session_id, "Session deleted, stopping turn");
                    } else {
                        tracing::warn!(session_id = %actor.session_id, "Actor lease lost before release");
                    }
                }
                Err(error) => {
                    tracing::warn!(session_id = %actor.session_id, error = %error, "Actor release failed; the lease will expire");
                }
            },
            Ok(Finish::HandOff) if !lease.is_lost() => match lease.hand_off().await {
                Ok(Some(ActorRelease::Released { .. })) => {
                    tracing::info!(session_id = %actor.session_id, "Actor handed off between steps");
                }
                Ok(Some(ActorRelease::LeaseLost)) => {
                    tracing::warn!(session_id = %actor.session_id, "Actor lease lost before hand-off");
                }
                Ok(None) => {
                    tracing::info!(session_id = %actor.session_id, "Store cannot hand off; the lease will expire and the turn resumes from the log");
                }
                Err(error) => {
                    tracing::warn!(session_id = %actor.session_id, error = %error, "Actor hand-off failed; the lease will expire");
                }
            },
            Ok(Finish::Retry(error)) => {
                tracing::warn!(session_id = %actor.session_id, error = %error, "Actor turn kept failing; the lease will expire and the turn resumes from the log");
            }
            Ok(_) => {}
            Err(error) if session_deleted(&lease, &error).await => {
                tracing::info!(session_id = %actor.session_id, "Session deleted, stopping turn");
            }
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
        everruns_contracts::rt::spawn(async move {
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
            fence: Some(lease.fence()),
        }
    }

    /// One run of the lease's turn, and the flush and checkpoint after it
    /// when it ends in a release.
    async fn run_once(&self, lease: &Lease, actor: &LeasedActor, made: &Made<H>) -> Result<Finish> {
        // A fired stall watchdog abandons a setup that never returns: the
        // lease is already lost and stops renewing (see `lease.rs`). Biased to
        // the turn, and the watchdog is disarmed while a turn is driven, so a
        // driven turn is never dropped here.
        let mut stalled = lease.stalled();
        let mut finish = tokio::select! {
            biased;
            finish = self.run_leased(lease, actor, made) => finish,
            _ = stalled.wait_for(|stalled| *stalled) => Ok(Finish::Expire),
        };
        if matches!(finish, Ok(Finish::Release)) && !lease.is_lost() {
            lease.arm_watchdog();
            let signals = Self::signals(lease);
            let host = made.host(self.hosts.host(
                HostTurn {
                    org_id: actor.org_id,
                    session_id: actor.session_id,
                    agent_id: None,
                    input_message_id: None,
                },
                &signals,
            ));
            let mut stalled = lease.stalled();
            let settled = tokio::select! {
                biased;
                settled = async {
                    self.hosts.flush(&host).await?;
                    let tail = match lease.read_tail().await {
                        Ok(tail) => tail,
                        // A tail too large to read, sealed from its first
                        // event: failing here would only lease it again.
                        Err(error) if error.is_non_retryable() => return Ok(()),
                        Err(error) => return Err(error),
                    };
                    self.hosts.checkpoint(&host, &tail).await
                } => settled,
                // The lease is lost: nothing is released below.
                _ = stalled.wait_for(|stalled| *stalled) => Ok(()),
            };
            if let Err(error) = settled {
                finish = Err(error);
            }
        }
        finish
    }

    /// Wind a failed run down before the lease runs the turn again: drop the
    /// events its hosts still queue and wait out any store in flight, end its
    /// turns on this worker, then back off (doubling from
    /// `control_plane_backoff`). Armed under the stall watchdog. `false` when
    /// the lease was lost meanwhile (renews failed for a whole TTL, or the
    /// watchdog fired): the lease expires instead.
    async fn wind_down(&self, lease: &Lease, made: &Made<H>, retry: u32) -> bool {
        lease.arm_watchdog();
        let (hosts, drives) = made.take();
        for host in &hosts {
            self.hosts.abandon(host);
        }
        for drive in &drives {
            self.hosts.turn_ended(drive).await;
        }
        let backoff = self
            .config
            .control_plane_backoff
            .saturating_mul(1 << retry.saturating_sub(1).min(6));
        let mut lost = lease.lost();
        tokio::select! {
            biased;
            _ = lost.wait_for(|lost| *lost) => {}
            () = async {
                for host in &hosts {
                    // A queue that gave up already dropped what it held.
                    let _ = self.hosts.flush(host).await;
                }
                everruns_contracts::rt::sleep(backoff).await;
            } => {}
        }
        !lease.is_lost()
    }

    async fn run_leased(
        &self,
        lease: &Lease,
        actor: &LeasedActor,
        made: &Made<H>,
    ) -> Result<Finish> {
        let org_id = actor.org_id;
        let session_id = actor.session_id;
        let signals = Self::signals(lease);
        let setup = made.host(self.hosts.host(
            HostTurn {
                org_id,
                session_id,
                agent_id: None,
                input_message_id: None,
            },
            &signals,
        ));
        let poisoned = actor.attempts > self.config.poison_attempts;
        let tail = match lease.read_tail().await {
            Ok(tail) => {
                self.hosts.checkpoint(&setup, &tail).await?;
                tail
            }
            // A tail too large to transfer failed every lease here, ahead of
            // the guard, so one turn re-read ~200 MB every few seconds for
            // hours. The guard seals it from its first event instead.
            Err(error) if poisoned && error.is_non_retryable() => {
                lease.read_tail_head().await?.events
            }
            Err(error) => return Err(error),
        };
        let started_at = tail
            .first()
            .filter(|event| event.event_type == crate::TURN_STARTED)
            .and_then(|event| event.sequence)
            .map(i64::from);
        let open =
            TurnLogFold::from_events(tail.clone()).filter(|turn| turn.session_id() == session_id);
        if let (Some(_), Some(sequence)) = (&open, started_at) {
            lease.advance(sequence);
        }

        if poisoned {
            self.seal(lease, &setup, org_id, open, actor.next_input_seq)
                .await?;
            return Ok(Finish::Release);
        }

        let Some(turn) = open else {
            // The latest turn is closed. Its holder may have died between the
            // turn's terminal event and `session.idled` (chaos harness: the
            // session stayed `active` with nothing running), and before a
            // renew committed the cursor over its input (the input would run
            // a second turn). Finish both here, idempotently: the log says
            // whether the idle and the input are still owed.
            let closed = unidled_turn(&tail);
            if let Some((turn_id, input_message_id)) = closed {
                RuntimeSessionLifecycle::new(setup.clone(), org_id, session_id)
                    .emit_session_idled(turn_id, input_message_id, None, None)
                    .await?;
            }
            let mut fresh = lease.fresh_input(actor.next_input_seq).await?;
            if let Some(found) = fresh
                && closed_input(&tail) == Some(found.message_id)
            {
                lease.advance(found.sequence);
                fresh = lease.fresh_input(actor.next_input_seq).await?;
            }
            let Some(input) = fresh else {
                // A mark with nothing to run (its input was consumed).
                lease.advance(lease.inbox_seq());
                return Ok(Finish::Release);
            };
            lease.start_fresh(input.sequence);
            lease.begin_turn(input.sequence);
            let resolved = setup
                .load_resolved_turn_for_execution(org_id, session_id, input.message_id)
                .await?;
            let input_message = resolved
                .messages
                .iter()
                .find(|message| message.id == input.message_id)
                .cloned();
            let snapshot = resolved.snapshot;
            let drive = TurnDrive::new(
                session_id,
                org_id,
                self.hosts
                    .fresh_turn_id(input.message_id, TurnId::new())
                    .await?,
                input.message_id,
                snapshot.harness_id,
                snapshot.agent_id,
                snapshot.workspace_id,
            );
            let host = made.turn(self.turn_host(&drive, &signals), &drive);
            self.hosts
                .inputs_consumed(&drive, &[input.message_id])
                .await?;
            lease.renew().await?;
            if !lease.cancel_requested() {
                let mut stop = signals.stop.clone();
                tokio::select! {
                    biased;
                    _=stop.wait_for(|stop|*stop)=>{},
                    result=self.hosts.before_turn(&host,&drive,input_message.as_ref())=> {
                        if result?==super::InputPreparation::Rejected {
                            // The host durably settled the input without starting a
                            // turn. It is safe to commit past the fresh-input guard.
                            lease.turn_stored();
                            lease.advance(input.sequence);
                            return Ok(Finish::Release);
                        }
                    },
                }
            }
            if lease.is_lost() {
                return Ok(Finish::Expire);
            }
            let (execution, plan) = turn_entry_plan(&host, &drive).await?;
            lease.renew().await?;
            return self.drive(lease, &host, drive, execution, plan).await;
        };

        turn.restore_causal_context(&setup);
        lease.begin_turn(
            lease
                .open_turn_input(turn.input_message_id(), started_at)
                .await?,
        );
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
        let host = made.turn(self.turn_host(&drive, &signals), &drive);
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
        if !lease.disarm_watchdog() {
            // The watchdog fired as setup ended: the lease is lost.
            self.hosts.abandon(host);
            self.hosts.turn_ended(&drive).await;
            return Ok(Finish::Expire);
        }
        let inbox = ActorInbox {
            lease,
            hosts: &*self.hosts,
            host,
            drive: &drive,
            parked: Default::default(),
            hand_off: &self.hand_off,
        };
        let (mut lost, mut cancel) = (lease.lost(), lease.cancel());
        enum Stopped {
            Lost,
            Cancel,
            Deleted,
            Done(Result<crate::host::TurnResult>),
        }
        let mut stopped = tokio::select! {
            biased;
            _ = lost.wait_for(|lost| *lost) => Stopped::Lost,
            _ = cancel.wait_for(|cancel| *cancel) => Stopped::Cancel,
            result = drive_turn(host, &inbox, drive.clone(), execution, plan) => Stopped::Done(result),
        };
        // A refused append (the control plane found another holder's fence)
        // flips the lease lost from inside the drive, which then returns its
        // error in the same poll: the lease is gone either way.
        if lease.is_lost() {
            stopped = Stopped::Lost;
        } else if lease.cancel_requested() {
            stopped = Stopped::Cancel;
        } else if let Stopped::Done(Err(error)) = &stopped
            && session_deleted(lease, error).await
        {
            stopped = Stopped::Deleted;
        }
        // Every way the drive ended but a lost lease or a deleted session
        // waits for the turn's queued events first. A queued store that gave
        // up left a gap in the host's write-behind log: the turn ends
        // as on a retryable failure, without committing its input or writing
        // anything more, and the turn runs again from the log.
        let flushed = match &stopped {
            Stopped::Cancel | Stopped::Done(_) => self.hosts.flush(host).await,
            Stopped::Lost | Stopped::Deleted => Ok(()),
        };
        let finish = match (stopped, flushed) {
            (Stopped::Lost, _) => {
                self.hosts.abandon(host);
                Finish::Expire
            }
            (Stopped::Deleted, _) => {
                tracing::info!(
                    session_id = %drive.session_id,
                    turn_id = %drive.turn_id,
                    "Session deleted, stopping turn"
                );
                self.hosts.abandon(host);
                Finish::Expire
            }
            (_, Err(error)) => {
                tracing::warn!(
                    session_id = %drive.session_id,
                    turn_id = %drive.turn_id,
                    error = %error,
                    "Actor turn's queued events were not stored; the turn resumes from the log"
                );
                Finish::Retry(error)
            }
            (Stopped::Cancel, Ok(())) => {
                lease.turn_stored();
                self.record_cancel(lease, host, &drive).await?;
                Finish::Release
            }
            (Stopped::Done(Ok(_)), Ok(())) => {
                lease.turn_stored();
                // Only a closed turn consumed its resolution. One that parked
                // must leave a concurrent resolution for the next lease.
                if !inbox.parked() {
                    lease.skip_to_next_input().await?;
                }
                Finish::Release
            }
            (Stopped::Done(Err(error)), Ok(()))
                if everruns_contracts::durable_retry::is_parked(&error) =>
            {
                lease.turn_stored();
                Finish::Park
            }
            (Stopped::Done(Err(error)), Ok(())) if crate::host::is_handed_off(&error) => {
                lease.turn_stored();
                Finish::HandOff
            }
            (Stopped::Done(Err(error)), Ok(())) if error.is_non_retryable() => {
                lease.turn_stored();
                self.record_failure(host, &drive, &error).await?;
                lease.skip_to_next_input().await?;
                Finish::Release
            }
            (Stopped::Done(Err(error)), Ok(())) => {
                tracing::warn!(
                    session_id = %drive.session_id,
                    turn_id = %drive.turn_id,
                    error = %error,
                    "Actor turn hit a retryable failure; the turn resumes from the log"
                );
                Finish::Retry(error)
            }
        };
        // Calls the act detached outlive the turn only when it parked with its
        // own events stored; see `tool_detach`.
        crate::engine::tool_detach::turn_ended(matches!(finish, Finish::Park));
        self.hosts.turn_ended(&drive).await;
        Ok(finish)
    }

    /// Record the cancel of `drive`'s turn, if the log still has it open:
    /// `turn.cancelled`, the person's request as part of the turn, and the
    /// session idled. Consumes the input that was pending when the cancel was
    /// requested, as the server's withdraw does; input sent after it starts
    /// the next turn.
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
        lease.advance(lease.cancelled_inbox());
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
            let result = crate::builtins::ask_user::AskUserResult {
                status: crate::builtins::ask_user::AskUserStatus::Cancelled,
                answered_by: crate::builtins::ask_user::AskUserAnsweredBy::Unattended,
                answers: Vec::new(),
            };
            ToolCompletedData::success(
                call.id,
                call.name,
                vec![crate::ContentPart::tool_result_text(
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

/// Whether `error` ended the turn because its session was deleted (EVE-1235):
/// the error says so, or the session is gone when asked. A turn whose session
/// was deleted under it fails on whatever it reads next (its input message,
/// the conversation), and those errors do not name the session; the renew
/// answers for the actor row, which goes only with the session. It runs only
/// on a failure path, where one more renew is harmless.
pub(super) async fn session_deleted(lease: &Lease, error: &AgentLoopError) -> bool {
    matches!(error, AgentLoopError::SessionNotFound(_))
        || matches!(lease.renew().await, Err(AgentLoopError::SessionNotFound(_)))
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

/// Terminal events: a turn that logged one is closed.
fn is_turn_terminal(event_type: &str) -> bool {
    matches!(
        event_type,
        crate::events::TURN_COMPLETED
            | crate::events::TURN_FAILED
            | crate::events::TURN_CANCELLED
            | crate::events::TURN_SEALED
    )
}

/// The input of the closed turn the tail starts with: its `turn.started`, when
/// a terminal event follows it.
fn closed_input(tail: &[Event]) -> Option<MessageId> {
    let started = tail
        .first()
        .filter(|event| event.event_type == crate::TURN_STARTED)?;
    tail.iter()
        .any(|event| is_turn_terminal(&event.event_type))
        .then_some(started.context.input_message_id)
        .flatten()
}

/// The turn (id, input) whose terminal event is the tail's last and has no
/// `session.idled` after it: its holder died before idling the session.
fn unidled_turn(tail: &[Event]) -> Option<(TurnId, MessageId)> {
    for event in tail.iter().rev() {
        if event.event_type == crate::events::SESSION_IDLED {
            return None;
        }
        if is_turn_terminal(&event.event_type) {
            return event.context.turn_id.zip(event.context.input_message_id);
        }
    }
    None
}
