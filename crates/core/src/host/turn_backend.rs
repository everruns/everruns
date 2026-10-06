//! The turn entry point: where a host hands a turn to whatever runs it.
//!
//! Decisions:
//! - One trait covers every way a turn starts, for the framework and the
//!   platform server alike: start a turn, cancel it, and ask what runs. The
//!   sans-IO `Execution` planner stays the inner turn interface; a backend
//!   decides only where and how durably the planned steps run.
//! - Completion is the ticket itself, a future, rather than a `wait` method:
//!   the caller can select on it beside its own mailbox, and each backend
//!   decides how completion arrives without a registry lookup per poll.
//! - Client-side tool results and interrupted turns are inputs of a start,
//!   not separate methods: in process both continue a turn on the caller's
//!   behalf exactly as a new message starts one.
//! - Input comes in two forms. [`TurnInput::Message`] and
//!   [`TurnInput::ToolResults`] hand the backend input to record; the
//!   backend writes it to the session's log and then runs the turn.
//!   [`TurnInput::StoredMessage`] and [`TurnInput::RecordedToolResults`] name
//!   input the caller already wrote, for a host that records input through
//!   its own store (the platform server persists every message and tool
//!   resolution before it starts a turn). Every backend serves the stored
//!   forms; a backend with no session runtime to write through (the
//!   platform's `DurableRunner`) serves only them. There is no server-only
//!   variant: stored input is a framework capability.
//! - A backend that cannot look the session up (one that reaches the
//!   session only through a durable queue) needs the session's organization,
//!   harness and agent on the request: [`TurnRequest::scope`]. A backend that
//!   resolves the session itself ignores it.
//! - Crash recovery (`recover`) is left out: the in-process runtime keeps no
//!   queue to recover from, and an interrupted turn is resumed per session
//!   through [`TurnInput::ResumeInterrupted`], from the session log, on any
//!   backend. The durable memory backend needs nothing more: its queue dies
//!   with the process.
//! - [`InProcessBackend`] drives the turn on the task that polls its ticket,
//!   so moving the facade onto this entry point changed no concurrency: a
//!   turn makes progress only while its owner polls it, and dropping the
//!   ticket stops it, as dropping the runtime's future always did.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::runtime::{AcceptedTurnInput, InProcessRuntime, TurnResult, TurnSteering};
use crate::events::ToolCompletedData;

/// Runs a session's turns: in this process, or on a durable queue.
///
/// The one entry point for turns, used by the framework's sessions and the
/// platform server alike.
///
/// **Experimental.** The trait is public and unsealed so a third-party
/// backend can implement it, but its shape may change without a major
/// version bump; the backend conformance suite in the `everruns` facade
/// (`tests/backend_conformance/`) is the bar a backend meets.
///
/// A backend runs at most one turn per session at a time. Starting a turn
/// returns a [`TurnTicket`] that resolves with the turn's [`TurnResult`];
/// whether the turn advances while nobody polls the ticket is up to the
/// backend (see [`InProcessBackend`]).
///
/// # Example
///
/// ```no_run
/// use everruns_contracts::typed_id::TurnId;
/// use everruns_core::InputMessage;
/// use everruns_core::host::{
///     AcceptedTurnInput, InProcessBackend, InProcessRuntime, TurnBackend, TurnInput,
///     TurnRequest,
/// };
///
/// # async fn run() -> everruns_contracts::error::Result<()> {
/// // A real host registers a provider and a default model on the builder.
/// let runtime = InProcessRuntime::builder()
///     .single_session(|session| session)
///     .build()
///     .await?;
/// let session_id = runtime.default_session_id().expect("single_session seeds one");
/// let backend = InProcessBackend::new(runtime);
///
/// let input = AcceptedTurnInput::new(InputMessage::user("hello"));
/// let ticket = backend
///     .start_turn(TurnRequest::new(
///         session_id,
///         TurnId::new(),
///         TurnInput::Message(Box::new(input)),
///     ))
///     .await?;
/// assert!(backend.is_running(session_id).await);
///
/// let result = ticket.await?;
/// println!("{}", result.response);
/// assert_eq!(backend.active_count().await, 0);
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait TurnBackend: Send + Sync {
    /// Start the turn `request` describes and return its ticket.
    ///
    /// # Errors
    ///
    /// Fails when the session already runs a turn on this backend, or when
    /// the backend cannot accept the turn.
    async fn start_turn(&self, request: TurnRequest) -> Result<TurnTicket>;

    /// Stop the turn `session_id` runs. Returns whether one was running.
    ///
    /// Once this returns, the turn takes no further step; its ticket
    /// resolves with [`AgentLoopError::Cancelled`]. Recording the
    /// cancellation in the session's log stays with the caller.
    async fn cancel(&self, session_id: SessionId) -> Result<bool>;

    /// Whether `session_id` has a turn on this backend that has not ended.
    async fn is_running(&self, session_id: SessionId) -> bool;

    /// How many turns run on this backend, across sessions.
    async fn active_count(&self) -> usize;
}

/// What starts or continues a turn.
///
/// **Experimental**, with [`TurnBackend`].
///
/// [`Message`](Self::Message) and [`ToolResults`](Self::ToolResults) carry
/// input for the backend to record before the turn runs.
/// [`StoredMessage`](Self::StoredMessage) and
/// [`RecordedToolResults`](Self::RecordedToolResults) name input the caller
/// already recorded in the session's log; every backend serves them.
#[derive(Debug)]
#[non_exhaustive]
pub enum TurnInput {
    /// A new message starts the turn. The backend records it as the
    /// session's `input.message` event first.
    Message(Box<AcceptedTurnInput>),
    /// A message the caller already recorded as the session's
    /// `input.message` event starts the turn (see
    /// [`InProcessRuntime::run_stored_turn`]). The backend writes nothing
    /// before the turn's input step.
    StoredMessage {
        /// The recorded message's id.
        message_id: MessageId,
    },
    /// The turn a process exit cut off in its tool calls runs them again,
    /// then carries on (see [`InProcessRuntime::resume_interrupted_turn`]).
    ResumeInterrupted,
    /// Client-side tool results continue the turn that parked on them (see
    /// [`InProcessRuntime::resume_steerable_turn`]). The backend records
    /// them under the parked turn first.
    ToolResults(Vec<ToolCompletedData>),
    /// The client-side tool results the caller already recorded under the
    /// parked turn (with [`InProcessRuntime::record_parked_tool_results`],
    /// or through its own event store) continue it, as
    /// [`ToolResults`](Self::ToolResults) does once it has recorded its own.
    RecordedToolResults {
        /// Keys this continuation. A durable backend enqueues the turn's next
        /// step under it, so continuing twice with one id runs the step once.
        resolution_id: Uuid,
    },
}

/// The session a turn runs for, for a backend that cannot look it up.
///
/// **Experimental**, with [`TurnBackend`].
///
/// A backend that holds the session's runtime resolves these itself and
/// ignores the request's scope. One that reaches the session only through a
/// durable queue (the platform's `DurableRunner`) needs it to start a turn
/// from a [`TurnInput::StoredMessage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TurnScope {
    /// The organization the session belongs to, by its internal id.
    pub org_id: i64,
    /// The harness the session runs.
    pub harness_id: HarnessId,
    /// The agent the session runs, when it has one.
    pub agent_id: Option<AgentId>,
}

impl TurnScope {
    /// The scope of a session in `org_id` that runs `harness_id` and,
    /// optionally, `agent_id`.
    pub fn new(org_id: i64, harness_id: HarnessId, agent_id: Option<AgentId>) -> Self {
        Self {
            org_id,
            harness_id,
            agent_id,
        }
    }
}

/// A turn to run: which session, under which id, from which input.
///
/// **Experimental**, with [`TurnBackend`].
#[derive(Debug)]
#[non_exhaustive]
pub struct TurnRequest {
    /// The session the turn belongs to.
    pub session_id: SessionId,
    /// The turn's id. A continued turn keeps the id it started with.
    pub turn_id: TurnId,
    /// What starts or continues the turn.
    pub input: TurnInput,
    /// Messages that join the running turn at its next reason boundary.
    ///
    /// The caller keeps a clone to push to. A backend that cannot deliver
    /// input mid-turn closes it, and a rejected push then belongs to the
    /// next turn, as it does once a turn commits to completion.
    pub steering: TurnSteering,
    /// The session's organization, harness and agent, for a backend that
    /// cannot look them up; see [`TurnScope`].
    pub scope: Option<TurnScope>,
    /// The request that caused the turn, for correlation. The durable
    /// backends carry it in the turn's checkpoint; the in-process backend
    /// does not record it.
    pub request_id: Option<String>,
}

impl TurnRequest {
    /// A request with fresh, open steering, no scope and no request id.
    pub fn new(session_id: SessionId, turn_id: TurnId, input: TurnInput) -> Self {
        Self {
            session_id,
            turn_id,
            input,
            steering: TurnSteering::new(),
            scope: None,
            request_id: None,
        }
    }

    /// Name the session's scope, for a backend that cannot look it up.
    pub fn with_scope(mut self, scope: TurnScope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Correlate the turn with the request `request_id` names.
    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    /// Steer the turn through `steering`, a handle the caller keeps a clone of.
    pub fn with_steering(mut self, steering: TurnSteering) -> Self {
        self.steering = steering;
        self
    }
}

type TurnCompletion = Pin<Box<dyn Future<Output = Result<TurnResult>> + Send>>;

/// A started turn. Await it for the turn's [`TurnResult`].
///
/// **Experimental**, with [`TurnBackend`]. Backends build tickets with
/// [`TurnTicket::new`]; what dropping one does is the backend's to define.
pub struct TurnTicket {
    session_id: SessionId,
    turn_id: TurnId,
    completion: TurnCompletion,
}

impl TurnTicket {
    /// A ticket for `turn_id` of `session_id` that resolves with `completion`.
    pub fn new(
        session_id: SessionId,
        turn_id: TurnId,
        completion: impl Future<Output = Result<TurnResult>> + Send + 'static,
    ) -> Self {
        Self {
            session_id,
            turn_id,
            completion: Box::pin(completion),
        }
    }

    /// The session the turn belongs to.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// The turn's id, as the request named it.
    pub fn turn_id(&self) -> TurnId {
        self.turn_id
    }
}

impl Future for TurnTicket {
    type Output = Result<TurnResult>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.completion.as_mut().poll(cx)
    }
}

impl fmt::Debug for TurnTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnTicket")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .finish_non_exhaustive()
    }
}

/// The default [`TurnBackend`]: turns run inside this process, on an
/// [`InProcessRuntime`].
///
/// **Experimental**, with [`TurnBackend`].
///
/// The turn runs on the task that polls its [`TurnTicket`], exactly as
/// awaiting [`InProcessRuntime::run_steerable_turn`] does: nothing advances
/// it while the ticket sits unpolled, and dropping the ticket drops the turn
/// mid-step. Nothing survives the process; a turn a crash cut off in its
/// tool calls continues through [`TurnInput::ResumeInterrupted`].
#[derive(Clone)]
pub struct InProcessBackend {
    runtime: InProcessRuntime,
    turns: Arc<Mutex<Turns>>,
}

#[derive(Default)]
struct Turns {
    next_generation: u64,
    running: HashMap<SessionId, RunningTurn>,
}

struct RunningTurn {
    /// Tells a stale ticket's drop from the current turn's.
    generation: u64,
    cancel: CancellationToken,
}

impl InProcessBackend {
    /// Run turns on `runtime`.
    pub fn new(runtime: InProcessRuntime) -> Self {
        Self {
            runtime,
            turns: Arc::default(),
        }
    }

    fn turns(&self) -> MutexGuard<'_, Turns> {
        lock_turns(&self.turns)
    }
}

fn lock_turns(turns: &Mutex<Turns>) -> MutexGuard<'_, Turns> {
    // Every critical section is a map insert or remove that cannot leave the
    // map half-written, so a poisoned lock still guards a consistent value.
    turns.lock().unwrap_or_else(PoisonError::into_inner)
}

impl fmt::Debug for InProcessBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InProcessBackend")
            .field("active", &self.turns().running.len())
            .finish_non_exhaustive()
    }
}

/// Clears a turn's registration when its ticket resolves or is dropped.
struct Registration {
    turns: Arc<Mutex<Turns>>,
    session_id: SessionId,
    generation: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut turns = lock_turns(&self.turns);
        if turns
            .running
            .get(&self.session_id)
            .is_some_and(|turn| turn.generation == self.generation)
        {
            turns.running.remove(&self.session_id);
        }
    }
}

#[async_trait]
impl TurnBackend for InProcessBackend {
    async fn start_turn(&self, request: TurnRequest) -> Result<TurnTicket> {
        let TurnRequest {
            session_id,
            turn_id,
            input,
            steering,
            ..
        } = request;
        let cancel = CancellationToken::new();
        let registration = {
            let mut turns = self.turns();
            if turns.running.contains_key(&session_id) {
                return Err(AgentLoopError::store(format!(
                    "session {session_id} already runs a turn"
                )));
            }
            let generation = turns.next_generation;
            turns.next_generation += 1;
            turns.running.insert(
                session_id,
                RunningTurn {
                    generation,
                    cancel: cancel.clone(),
                },
            );
            Registration {
                turns: self.turns.clone(),
                session_id,
                generation,
            }
        };

        let runtime = self.runtime.clone();
        let turn = async move {
            match input {
                TurnInput::Message(input) => {
                    runtime
                        .run_steerable_turn(session_id, *input, turn_id, steering)
                        .await
                }
                TurnInput::ResumeInterrupted => {
                    runtime.resume_interrupted_turn(session_id, steering).await
                }
                TurnInput::ToolResults(results) => {
                    runtime
                        .resume_steerable_turn(session_id, results, steering)
                        .await
                }
                TurnInput::StoredMessage { message_id } => {
                    runtime
                        .run_stored_turn(session_id, message_id, turn_id, steering)
                        .await
                }
                // One turn parks per session and resuming takes it, so the
                // runtime already continues it at most once; the id keys
                // nothing here.
                TurnInput::RecordedToolResults { resolution_id: _ } => {
                    runtime
                        .resume_steerable_turn(session_id, Vec::new(), steering)
                        .await
                }
            }
        };
        let completion = async move {
            let _registration = registration;
            tokio::select! {
                biased;
                () = cancel.cancelled() => Err(AgentLoopError::Cancelled),
                result = turn => result,
            }
        };
        Ok(TurnTicket::new(session_id, turn_id, completion))
    }

    async fn cancel(&self, session_id: SessionId) -> Result<bool> {
        // The ticket checks the token before it polls the turn again, so the
        // turn takes no further step; its registration clears when the
        // ticket resolves or drops.
        let turns = self.turns();
        Ok(match turns.running.get(&session_id) {
            Some(turn) => {
                turn.cancel.cancel();
                true
            }
            None => false,
        })
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        self.turns()
            .running
            .get(&session_id)
            .is_some_and(|turn| !turn.cancel.is_cancelled())
    }

    async fn active_count(&self) -> usize {
        self.turns()
            .running
            .values()
            .filter(|turn| !turn.cancel.is_cancelled())
            .count()
    }
}
