//! Multi-turn sessions (EVE-831).
//!
//! An application-owned [`Engine`](crate::Engine) opens each [`Session`].
//! [`Session::send`] accepts messages and appends canonical events. Two sessions
//! created from the same Agent are independent and never share history. The
//! owning engine reopens dropped sessions while its catalog remains available.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use everruns_contracts::error::AgentLoopError;
use everruns_contracts::turn::TurnStopReason;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::InputMessage;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::host::{
    AcceptedTurnInput, InProcessRuntime, TurnBackend, TurnInput, TurnRequest, TurnResult,
    TurnSteering, TurnSteeringPushError, TurnTicket,
};
use tokio::sync::{OnceCell, mpsc, oneshot, watch};

use crate::engine::SessionExecution;
use crate::events::{EventStream, FacadeEventBus, RunOptions};
use crate::hooks::{
    AgentStartContext, CompletionContext, HookFailure, HookRunState, TurnStartContext,
};
use crate::observers::ObserverDispatcher;
use crate::{Agent, Harness, SessionEnvironmentError};

mod actor;
use actor::SessionActor;
mod attach;
mod controls;
pub use controls::{
    COMPLETION_PURPOSE_KEY, CapabilityChange, CapabilityRegistrationError, CommandOutput,
    CompletionRequest, MAX_COMPLETION_MESSAGES, MAX_COMPLETION_SYSTEM_PROMPTS, SessionControlError,
};
pub(crate) use controls::{HostedQuery, LiveControls, register_engine_capability};
#[cfg(test)]
mod backend_tests;
mod interrupted;

pub use attach::AttachError;
use attach::AttachRequest;
pub use interrupted::InterruptedTurn;
use interrupted::ResumedTurn;

/// A live, multi-turn conversation with an [`Agent`](crate::Agent).
///
/// Open one with [`Engine::create`](crate::Engine::create). The first
/// [`send`](Self::send) or [`inspect`](Self::inspect) materializes an isolated
/// in-process runtime; later operations reuse it, so history accumulates across
/// turns. Keep its typed [`SessionId`](crate::SessionId) to resume it after this
/// handle is dropped.
///
/// # Example
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::prelude::*;
///
/// let agent = Agent::builder()
///     .instructions("You are concise.")
///     .model(Model::simulated("Hello!"))
///     .build()?;
///
/// let session = InMemoryEngine::new().create(agent.clone());
/// let first = session.send_and_wait("hi").await?;
/// let second = session.send_and_wait("continue").await?;
///
/// assert_eq!(first.response, "Hello!");
/// assert!(second.success);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Session {
    pub(crate) inner: Arc<SessionInner>,
}

pub(crate) struct SessionInner {
    pub(crate) execution: Arc<dyn SessionExecution>,
    session_id: SessionId,
    pub(crate) event_bus: Arc<FacadeEventBus>,
    hook_state: Arc<HookRunState>,
    pub(crate) harness: OnceLock<Harness>,
    environment: OnceLock<everruns_core::host::Environment>,
    environment_gate: tokio::sync::Mutex<()>,
    commands: OnceCell<mpsc::Sender<Command>>,
    /// The client-side tool calls the last turn parked on. Updated before
    /// the turn's completion is published, so a waiter that saw the turn
    /// end reads its parked calls.
    parked_calls: watch::Sender<Option<everruns_core::host::ParkedToolCalls>>,
    /// Whether a run gave the session client-side tools that no later run
    /// has cleared.
    #[cfg(feature = "ag-ui")]
    client_tools_declared: std::sync::atomic::AtomicBool,
}

/// Bounds commands accepted while the actor is busy and deferred inspection or
/// next-turn work retained while a turn reaches its terminal boundary.
// THREAT[TM-DOS-036]: never turn slow model execution into an unbounded mailbox.
const SESSION_COMMAND_CAPACITY: usize = 64;

/// The session hint under which the runtime pauses a turn on a client-side
/// tool call (see [`SessionOverrides::client_tools`]).
#[cfg(feature = "ag-ui")]
const CLIENT_ANSWERS_PAUSES_HINT: &str = "setup_connection";

impl Session {
    /// Answer a durable approval request by its synthetic request call id.
    /// Retain `command` when retrying after an uncertain transport response.
    pub async fn approve_tool(
        &self,
        command: uuid::Uuid,
        turn_id: crate::TurnId,
        request_id: &str,
        decision: crate::hosting::ToolApprovalDecision,
    ) -> Result<(), RunError> {
        self.approve_tools(
            command,
            vec![crate::hosting::ApprovalCommit {
                turn_id,
                request_id: request_id.into(),
                decision,
            }],
        )
        .await
    }

    /// Commit all decisions atomically. Invalid or stale members reject the
    /// entire batch; retries retain the same command identity and decisions.
    pub async fn approve_tools(
        &self,
        command: uuid::Uuid,
        approvals: Vec<crate::hosting::ApprovalCommit>,
    ) -> Result<(), RunError> {
        let profile = self
            .inner
            .execution
            .hosting()
            .ok_or_else(|| AgentLoopError::config("Engine has no session authority"))?;
        profile
            .approve_many(self.session_id(), command, approvals)
            .await?;
        self.inner.execution.activate();
        Ok(())
    }
    /// Atomically answer a question set. Retain the command, turn and call ids
    /// for retries; the entire mixed batch commits or is refused.
    pub async fn resolve_waits(
        &self,
        command: uuid::Uuid,
        turn: crate::TurnId,
        approvals: Vec<crate::hosting::ApprovalCommit>,
        results: Vec<crate::ToolAnswer>,
    ) -> Result<(), RunError> {
        let profile = self
            .inner
            .execution
            .hosting()
            .ok_or_else(|| AgentLoopError::config("Engine has no session authority"))?;
        profile
            .resolve_waits(
                self.session_id(),
                command,
                turn,
                approvals,
                results
                    .into_iter()
                    .map(|answer| answer.into_event_data())
                    .collect(),
            )
            .await?;
        self.inner.execution.activate();
        Ok(())
    }
    /// Atomically answer a question set. Retain the command, turn and call ids
    /// for retries; deduplication precedes validation against current waits.
    pub async fn answer_questions(
        &self,
        request: crate::hosting::QuestionAnswersRequest,
    ) -> Result<(), RunError> {
        use everruns_core::builtins::ask_user::{AskUserAnsweredBy, AskUserResult, AskUserStatus};
        if !matches!(
            request.status,
            AskUserStatus::Answered | AskUserStatus::Declined
        ) {
            return Err(AgentLoopError::config("invalid client question status").into());
        }
        if request.command_id.is_some()
            && (request.turn_id.is_none() || request.tool_call_id.is_none())
        {
            return Err(
                AgentLoopError::config("command_id requires turn_id and tool_call_id").into(),
            );
        }
        let profile = self
            .inner
            .execution
            .hosting()
            .ok_or_else(|| AgentLoopError::config("Engine has no session authority"))?;
        let (turn, call) = match (request.turn_id, request.tool_call_id) {
            (Some(turn), Some(call)) => (turn, call),
            (target, requested) => {
                let status = profile.status(self.session_id()).await?;
                let turn = target
                    .or(status.open_turn)
                    .ok_or_else(|| AgentLoopError::config("no open turn"))?;
                let mut pending = status.waiting.iter().filter(|call| {
                    call.name == "ask_user" && requested.as_ref().is_none_or(|id| id == &call.id)
                });
                let call = pending
                    .next()
                    .ok_or_else(|| AgentLoopError::config("question set is no longer pending"))?;
                if pending.next().is_some() {
                    return Err(AgentLoopError::config(
                        "several question sets are pending; supply tool_call_id",
                    )
                    .into());
                }
                (turn, call.id.clone())
            }
        };
        self.resolve_tools(
            request.command_id.unwrap_or_else(uuid::Uuid::new_v4),
            turn,
            vec![crate::ToolAnswer {
                tool_call_id: call,
                tool_name: "ask_user".into(),
                result: serde_json::to_value(AskUserResult {
                    status: request.status,
                    answered_by: AskUserAnsweredBy::User,
                    answers: request.answers,
                })
                .map_err(|error| AgentLoopError::config(error.to_string()))?,
                error: None,
            }],
        )
        .await
    }
    /// Submit an Execution API batch, atomically rejecting omitted approvals.
    pub async fn submit_approvals(
        &self,
        command: uuid::Uuid,
        turn: crate::TurnId,
        decisions: Vec<everruns_contracts::execution_api::ToolApprovalAnswer>,
    ) -> Result<everruns_contracts::execution_api::SubmitToolApprovalsResponse, RunError> {
        let profile = self
            .inner
            .execution
            .hosting()
            .ok_or_else(|| AgentLoopError::config("Engine has no session authority"))?;
        let response = profile
            .submit_approvals(self.session_id(), command, turn, decisions)
            .await?;
        self.inner.execution.activate();
        Ok(response)
    }
    /// Read the outstanding durable client calls without activating execution.
    pub async fn pending_tools(&self) -> Result<Vec<crate::ToolCall>, RunError> {
        let profile = self
            .inner
            .execution
            .hosting()
            .ok_or_else(|| AgentLoopError::config("Engine has no session authority"))?;
        Ok(profile.status(self.session_id()).await?.waiting)
    }

    /// Atomically resolve matching durable client calls. A retry after a lost
    /// reply must reuse `command`. Partial results leave other calls parked.
    pub async fn resolve_tools(
        &self,
        command: uuid::Uuid,
        turn_id: crate::TurnId,
        results: Vec<crate::ToolAnswer>,
    ) -> Result<(), RunError> {
        if results.iter().any(|answer| {
            answer.tool_name == everruns_contracts::tool_types::APPROVE_TOOL_CALL_TOOL
        }) {
            return Err(AgentLoopError::config("approval requests require approve_tool").into());
        }
        let profile = self
            .inner
            .execution
            .hosting()
            .ok_or_else(|| AgentLoopError::config("Engine has no session authority"))?;
        let results = results
            .into_iter()
            .map(|answer| answer.into_event_data())
            .collect();
        profile
            .resolve(self.session_id(), command, turn_id, results)
            .await?;
        self.inner.execution.activate();
        Ok(())
    }
    /// Resolve an exact canonical event identity to its session sequence using
    /// bounded pages. Event identifiers need not be time ordered.
    pub async fn event_sequence(&self, event_id: &str) -> Result<Option<i32>, crate::HistoryError> {
        if let Some(profile) = self.inner.execution.hosting() {
            return profile
                .event_sequence(self.session_id(), event_id)
                .await
                .map_err(|_| crate::HistoryError::Unavailable);
        }
        let mut cursor = None;
        loop {
            let page = self.event_page(cursor, 256).await?;
            if let Some(event) = page.events.iter().find(|event| event.event_id == event_id) {
                return Ok(event.sequence());
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(None),
            }
        }
    }

    /// Read one bounded, snapshot-consistent canonical event page. Persist the
    /// returned cursor to resume after frontend loss. A cursor is scoped to
    /// this session and is validated by the authority.
    pub async fn event_page(
        &self,
        cursor: Option<crate::SessionEventCursor>,
        limit: usize,
    ) -> Result<crate::SessionEventPage, crate::HistoryError> {
        use everruns_core::host::{EventReadLimit, EventReadRequest};
        self.inner.execution.ensure_cataloged().await?;
        let limit = EventReadLimit::new(limit).map_err(|_| crate::HistoryError::InvalidLimit {
            requested: limit,
            maximum: 1024,
        })?;
        let mut request = EventReadRequest::new(self.session_id(), limit);
        if let Some(cursor) = cursor {
            request = request.with_cursor(cursor);
        }
        let backends = self
            .inner
            .execution
            .backends()
            .await
            .map_err(|error| error.history_error())?;
        let page = backends
            .host
            .event_log
            .read_page(request)
            .await
            .map_err(|_| crate::HistoryError::Unavailable)?;
        Ok(crate::SessionEventPage {
            high_watermark: page.snapshot_high_watermark(),
            events: page
                .events
                .iter()
                .map(crate::SessionEvent::from_core_event)
                .collect(),
            next_cursor: page.next_cursor,
        })
    }
    pub(crate) fn from_inner(inner: Arc<SessionInner>) -> Self {
        Self { inner }
    }

    pub(crate) fn inner(&self) -> Arc<SessionInner> {
        self.inner.clone()
    }

    pub(crate) fn has_started(&self) -> bool {
        self.inner.commands.initialized()
    }

    pub(crate) fn new(
        execution: Arc<dyn SessionExecution>,
        environment: Option<everruns_core::host::Environment>,
        observers: Arc<ObserverDispatcher>,
    ) -> Self {
        let session_id = execution.session_id();
        let hook_state = HookRunState::new(if execution.hosting().is_some() {
            Default::default()
        } else {
            execution.agent_snapshot().lifecycle_hooks()
        });
        let session = Self {
            inner: Arc::new(SessionInner {
                execution,
                session_id,
                event_bus: Arc::new(FacadeEventBus::new(session_id, observers)),
                hook_state,
                harness: OnceLock::new(),
                environment: OnceLock::new(),
                environment_gate: tokio::sync::Mutex::new(()),
                commands: OnceCell::new(),
                parked_calls: watch::Sender::new(None),
                #[cfg(feature = "ag-ui")]
                client_tools_declared: std::sync::atomic::AtomicBool::new(false),
            }),
        };
        if let Some(harness) = session.inner.execution.harness_snapshot() {
            let _ = session.inner.harness.set(harness);
        }
        if let Some(environment) = environment {
            let _ = session.inner.environment.set(environment);
        }
        session
    }

    /// An opaque identifier correlating this session's turns.
    ///
    /// It carries no organization, principal, or platform identity — it is only
    /// useful to line up a session's turns in logs.
    pub fn id(&self) -> String {
        self.inner.session_id.to_string()
    }

    /// The typed Framework identity for this session.
    ///
    /// Use this value with typed session-resumption APIs. [`id`](Self::id)
    /// remains available when a string is needed for display or serialization.
    pub fn session_id(&self) -> SessionId {
        self.inner.session_id
    }

    /// Select and persist this Agent's default head without starting a turn.
    ///
    /// Calling this is optional: [`send`](Self::send) and [`inspect`](Self::inspect)
    /// perform the same one-time selection automatically. After it succeeds,
    /// [`workspace_head`](Self::workspace_head) always returns that exact head.
    pub async fn start(&self) -> Result<(), SessionEnvironmentError> {
        let _environment_guard = self.inner.environment_gate.lock().await;
        if let Some(environment) = self.inner.environment.get() {
            return self.negotiate_environment(environment);
        }
        let environment = self.inner.execution.default_environment().await?;
        self.negotiate_environment(&environment)?;
        self.inner.execution.bind_environment(&environment).await?;
        self.inner
            .environment
            .set(environment)
            .map_err(|_| SessionEnvironmentError::AlreadyBound)
    }

    pub(crate) async fn start_with_harness(
        &self,
        harness: Harness,
    ) -> Result<(), SessionEnvironmentError> {
        if self.inner.execution.hosting().is_some() && !self.inner.execution.provisional() {
            return Err(SessionEnvironmentError::AlreadyBound);
        }
        let _environment_guard = self.inner.environment_gate.lock().await;
        if self.inner.commands.initialized() {
            return Err(SessionEnvironmentError::AlreadyStarted);
        }
        let (environment, needs_binding) = match self.inner.environment.get() {
            Some(environment) => (environment.clone(), false),
            None => (self.inner.execution.default_environment().await?, true),
        };
        harness.negotiate(&environment)?;
        self.bind_harness(harness)?;
        if needs_binding {
            self.inner.execution.bind_environment(&environment).await?;
            self.inner
                .environment
                .set(environment)
                .map_err(|_| SessionEnvironmentError::AlreadyBound)?;
        }
        Ok(())
    }

    pub(crate) fn negotiate_environment(
        &self,
        environment: &everruns_core::host::Environment,
    ) -> Result<(), SessionEnvironmentError> {
        match self.inner.harness.get() {
            Some(harness) => harness.negotiate(environment),
            None => Ok(()),
        }
    }

    /// The permanently selected workspace head after explicit or automatic
    /// Environment start.
    pub fn workspace_head(&self) -> Option<&everruns_core::host::WorkspaceHead> {
        self.inner
            .environment
            .get()
            .map(everruns_core::host::Environment::workspace_head)
    }

    /// Resolve a typed resource attached to this session's Environment.
    pub fn environment_extension<T: std::any::Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.inner
            .environment
            .get()
            .and_then(|environment| environment.extension::<T>())
    }

    /// Build an owned, bounded history query for this session.
    ///
    /// The happy path is `session.history().page().await`. The first page
    /// returns at most 100 messages in canonical event-sequence order and an
    /// opaque continuation cursor when more remain. Use
    /// [`HistoryQuery::limit`](crate::HistoryQuery::limit) to select up to 256
    /// messages, or [`HistoryQuery::pages`](crate::HistoryQuery::pages) for a
    /// lazy bounded walk of the snapshot.
    pub fn history(&self) -> crate::HistoryQuery {
        crate::HistoryQuery::new(self.inner.execution.clone(), self.inner.session_id)
    }

    /// Scope a background-work queue to this session.
    ///
    /// The returned handle fixes this session as the owner of every submitted
    /// task, task read, cancellation request, and direct wake. The queue
    /// determines persistence and restart behavior; the default queue is
    /// process-local and database-free.
    pub fn work(&self, queue: &crate::work::WorkQueue) -> crate::work::SessionWork {
        queue.for_session(self.id())
    }

    /// Subscribe to this session's live [`SessionEvent`](crate::SessionEvent)
    /// feed.
    ///
    /// The returned [`EventStream`] observes every message sent *after* it is
    /// created (subscribe before calling [`send`](Session::send)). Multiple
    /// streams can observe the same session independently, and each session's
    /// events are isolated — one session never sees another's. Dropping a
    /// stream, or letting a consumer fall behind, never affects a running turn.
    /// The stream is
    /// bounded and reports an explicit [`EventStreamError::Lagged`](crate::EventStreamError::Lagged)
    /// gap; it never hides loss or applies observer backpressure to execution.
    /// Each [`SessionEvent`](crate::SessionEvent) also retains the complete
    /// canonical event envelope through
    /// [`SessionEvent::as_json`](crate::SessionEvent::as_json).
    /// Use [`history`](Session::history) to rebuild a bounded persisted
    /// transcript after live lag or a process restart; ephemeral streaming
    /// deltas are intentionally not part of that projection.
    ///
    /// Events are non-blocking observation. For application work that must be
    /// awaited at a lifecycle boundary, register an
    /// [`AgentBuilder::on_turn_start`](crate::AgentBuilder::on_turn_start) or
    /// another typed lifecycle handler instead.
    pub fn events(&self) -> EventStream {
        self.inner.event_bus.subscribe()
    }

    /// Read this session's durable events with a sequence greater than
    /// `after`, oldest first.
    ///
    /// Durable sequences are dense per session and start at 1, so
    /// `events_after(0)` returns the whole persisted log and a client that has
    /// seen sequence `n` resumes with `events_after(n)`. Negative values read
    /// from the start. Ephemeral streaming deltas are never persisted and do
    /// not appear here. Works on a session reopened with
    /// [`Engine::resume`](crate::Engine::resume) as long as its backend keeps
    /// events (for example [`LocalConfig`](crate::LocalConfig)).
    ///
    /// Reads one stable snapshot of at most 100,000 events; a larger backlog
    /// returns [`HistoryError::HistoryTooLarge`](crate::HistoryError::HistoryTooLarge).
    ///
    /// Stability: alpha — may change without a major bump; see
    /// [`stability`](crate::stability).
    pub async fn events_after(
        &self,
        after: i32,
    ) -> Result<Vec<crate::SessionEvent>, crate::HistoryError> {
        crate::history::durable_events_after(
            self.inner.execution.as_ref(),
            self.inner.session_id,
            after,
        )
        .await
    }

    /// Replay durable events after `after`, then continue live.
    ///
    /// The stream subscribes to the live feed first, then reads the backlog
    /// with [`events_after`](Self::events_after), so nothing committed in
    /// between is lost. It yields the backlog in sequence order, then live
    /// events; a live durable event whose sequence was already replayed is
    /// dropped, so no event is delivered twice. Ephemeral deltas carry no
    /// sequence and always pass through. Lag on the live half is reported
    /// exactly as on [`events`](Self::events); recover by calling this method
    /// again with the last sequence seen.
    ///
    /// Stability: alpha — may change without a major bump; see
    /// [`stability`](crate::stability).
    pub async fn events_from(&self, after: i32) -> Result<EventStream, crate::HistoryError> {
        let live = self.inner.event_bus.subscribe();
        let backlog = self.events_after(after).await?;
        if let Some(profile) = self.inner.execution.hosting() {
            let through = backlog
                .last()
                .and_then(crate::SessionEvent::sequence)
                .unwrap_or(after)
                .max(after);
            return Ok(live.with_replay(after.max(0), backlog).with_authority(
                profile,
                self.session_id(),
                through,
            ));
        }
        Ok(live.with_replay(after.max(0), backlog))
    }

    /// Accept a message without waiting for the agent's response.
    ///
    /// When the session is idle, the message starts a new turn. While a turn is
    /// active, the message steers that turn at its next reason boundary. The
    /// returned [`SentMessage`] reports which case occurred and can optionally
    /// be [`wait`](SentMessage::wait)ed.
    pub async fn send(&self, input: impl Into<InputMessage>) -> Result<SentMessage, RunError> {
        self.send_internal(input.into(), None).await
    }

    /// Admit input under a caller-retained message identity. Use the same id
    /// and content after an uncertain reply, including after frontend loss.
    pub async fn send_with_id(
        &self,
        message_id: MessageId,
        input: impl Into<InputMessage>,
    ) -> Result<SentMessage, RunError> {
        self.admit_with_id(message_id, input.into(), false).await
    }

    async fn admit_with_id(
        &self,
        message_id: MessageId,
        input: InputMessage,
        cancelled: bool,
    ) -> Result<SentMessage, RunError> {
        let profile =
            self.inner.execution.hosting().ok_or_else(|| {
                AgentLoopError::config("stable admission requires Engine hosting")
            })?;
        if self.inner.execution.frontend_environment() {
            self.start().await.map_err(RunError::Environment)?;
        }
        self.inner
            .execution
            .ensure_cataloged()
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        let mut message = AcceptedTurnInput::new(input).into_message();
        message.id = message_id;
        let already_accepted = profile
            .receipt(self.session_id(), message_id)
            .await?
            .is_some();
        let receipt = profile.admit(self.session_id(), message, cancelled).await?;
        // Admission commits outside the runtime emitter. Publish its canonical
        // envelope before activating, just as runtime events publish after commit.
        if !already_accepted {
            use everruns_core::host::{EventCursor, EventReadLimit, EventReadRequest, EventSink};
            let page = profile
                .reader(self.session_id())
                .event_log
                .read_page(EventReadRequest::from_cursor(
                    EventCursor::continuation(
                        self.session_id(),
                        receipt.sequence - 1,
                        receipt.sequence,
                    )
                    .map_err(|error| AgentLoopError::store(error.to_string()))?,
                    EventReadLimit::new(1)
                        .map_err(|error| AgentLoopError::store(error.to_string()))?,
                ))
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            for event in page.events {
                let _ = self.inner.event_bus.try_send(event);
            }
        }
        let admission = crate::hosting::Admission {
            hosting: profile,
            session: self.session_id(),
            receipt,
        };
        self.inner.execution.activate();
        Ok(SentMessage::admitted(self.clone(), admission))
    }

    /// Reconstruct an accepted handle without resending its content.
    pub async fn admission(
        &self,
        message: MessageId,
    ) -> Result<Option<crate::hosting::Admission>, RunError> {
        let hosting = self.inner.execution.hosting().ok_or_else(|| {
            AgentLoopError::config("Engine has no authoritative admission profile")
        })?;
        Ok(hosting
            .receipt(self.session_id(), message)
            .await?
            .map(|receipt| crate::hosting::Admission {
                hosting,
                session: self.session_id(),
                receipt,
            }))
    }

    /// Send a message and wait for the turn that accepted it.
    pub async fn send_and_wait(&self, input: impl Into<InputMessage>) -> Result<Turn, RunError> {
        self.send(input).await?.wait().await
    }

    /// Convenience alias for [`send_and_wait`](Self::send_and_wait).
    ///
    /// # Errors
    ///
    /// Returns [`RunError`] if an agent/turn-start handler fails, the runtime
    /// cannot be built, or the turn cannot be executed. A turn that runs but
    /// ends unsuccessfully (e.g. a refusal or a max-iteration stop) is returned
    /// as an `Ok(Turn)` with `success == false` and the
    /// [`stop_reason`](Turn::stop_reason) preserved.
    pub async fn run(&self, input: impl Into<InputMessage>) -> Result<Turn, RunError> {
        self.send_and_wait(input).await
    }

    /// Send, wait, and apply the given [`RunOptions`], enabling cancellation.
    ///
    /// Identical to [`run`](Session::run) when the options carry no cancellation
    /// token. The message follows the same automatic start-or-steer routing as
    /// [`send`](Self::send). When a [`CancellationToken`](crate::CancellationToken)
    /// is attached and cancelled while the accepting turn is in flight, that
    /// turn's future is dropped —
    /// cooperatively tearing down any running tool work — and this returns an
    /// `Ok(Turn)` with [`success == false`](Turn::success) and
    /// [`stop_reason`](Turn::stop_reason) set to
    /// [`TurnStopReason::Cancelled`]. A token already cancelled before the call
    /// stops the turn before it starts. Once the runtime commits an outcome,
    /// completion handlers finish and are no longer interrupted by this token.
    ///
    /// # Errors
    ///
    /// Same as [`run`](Session::run): [`RunError`] if a pre-effect handler fails,
    /// the runtime cannot be built, or the turn cannot be executed.
    pub async fn run_with(
        &self,
        input: impl Into<InputMessage>,
        options: RunOptions,
    ) -> Result<Turn, RunError> {
        if options.cancel.is_none() && options.timeout.is_none() {
            return self.send_and_wait(input).await;
        }
        let token = options.cancel.clone();
        let timeout = options.timeout;
        let sent = self.send_internal(input.into(), token.clone()).await?;
        // Cancellation and timeout share semantics: stop the turn in flight,
        // then wait for its cancelled outcome.
        let cancel_turn = async {
            let _ = sent.turn.cancel().await;
            sent.wait().await
        };
        tokio::select! {
            biased;
            result = sent.wait() => result,
            () = async {
                match token {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            } => cancel_turn.await,
            () = async {
                match timeout {
                    Some(timeout) => everruns_contracts::rt::sleep(timeout).await,
                    None => std::future::pending().await,
                }
            } => cancel_turn.await,
        }
    }

    /// Inspect the exact application-facing context for the next model call.
    ///
    /// This is valid before the first turn and after any later turn. MCP tool
    /// discovery, plugin prompt contributions, message filters, and model
    /// selection use the same runtime assembly path as execution. Inspection
    /// materializes the runtime but does not run any lifecycle handler.
    pub async fn inspect(&self) -> Result<crate::SessionContext, RunError> {
        if self.inner.execution.hosting().is_some() {
            if self.inner.execution.frontend_environment() {
                self.start().await.map_err(RunError::Environment)?;
            }
            self.inner
                .execution
                .ensure_cataloged()
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            return Ok(self.inner.execution.inspect_context().await?);
        }
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Inspect { response })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        result.await.map_err(|_| RunError::SessionClosed)?
    }

    /// Replace this session's run-scoped record fields before a run.
    ///
    /// Ordered with sends: an override issued before a send applies to the
    /// turn that send starts, and mid-turn it applies from the running turn's
    /// next model call. The record is rewritten after the runtime is built,
    /// because building it seeds the record afresh.
    #[cfg(feature = "ag-ui")]
    pub(crate) async fn override_record(
        &self,
        overrides: SessionOverrides,
    ) -> Result<(), RunError> {
        let declared = overrides
            .client_tools
            .as_ref()
            .map(|tools| !tools.is_empty());
        if let Some(profile) = self.inner.execution.hosting() {
            self.inner
                .execution
                .ensure_cataloged()
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            profile
                .configure(
                    self.session_id(),
                    uuid::Uuid::new_v4(),
                    crate::hosting::SessionConfiguration {
                        instructions: overrides.instructions,
                        client_tools: overrides.client_tools,
                        ..Default::default()
                    },
                )
                .await?;
            if let Some(declared) = declared {
                self.inner
                    .client_tools_declared
                    .store(declared, std::sync::atomic::Ordering::SeqCst);
            }
            return Ok(());
        }
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Override {
                overrides,
                response,
            })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        result.await.map_err(|_| RunError::SessionClosed)??;
        if let Some(declared) = declared {
            self.inner
                .client_tools_declared
                .store(declared, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }

    /// Whether the session holds client-side tools a run gave it.
    #[cfg(feature = "ag-ui")]
    pub(crate) fn had_client_tools(&self) -> bool {
        self.inner
            .client_tools_declared
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(feature = "ag-ui")]
    pub(crate) async fn refresh_hosted_calls(&self) -> Result<(), RunError> {
        let Some(profile) = self.inner.execution.hosting() else {
            return Ok(());
        };
        self.inner
            .execution
            .ensure_cataloged()
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        let snapshot = profile.query_snapshot(self.session_id()).await?;
        if let Some(tools) = snapshot.configuration.client_tools {
            self.inner
                .client_tools_declared
                .store(!tools.is_empty(), std::sync::atomic::Ordering::SeqCst);
        }
        let status = profile.status(self.session_id()).await?;
        let parked = status
            .open_turn
            .filter(|_| !status.waiting.is_empty())
            .map(|turn_id| everruns_core::host::ParkedToolCalls::new(turn_id, status.waiting));
        self.inner.parked_calls.send_replace(parked);
        Ok(())
    }

    #[cfg(any(feature = "ag-ui", feature = "channels"))]
    pub(crate) async fn protocol_events(&self) -> Result<EventStream, RunError> {
        if let Some(profile) = self.inner.execution.hosting() {
            let live = self.inner.event_bus.subscribe();
            let snapshot = profile.query_snapshot(self.session_id()).await?;
            return Ok(live.with_authority(profile, self.session_id(), snapshot.high_watermark));
        }
        Ok(self.events())
    }

    #[cfg(any(feature = "ag-ui", feature = "channels"))]
    pub(crate) fn has_hosted_execution(&self) -> bool {
        self.inner.execution.hosting().is_some()
    }

    /// The client-side tool calls the session's last turn parked on, if it
    /// parked and has not been resumed or superseded since.
    #[cfg(feature = "ag-ui")]
    pub(crate) fn parked_tool_calls(&self) -> Option<everruns_core::host::ParkedToolCalls> {
        self.inner.parked_calls.borrow().clone()
    }

    /// A feed of [`parked_tool_calls`](Self::parked_tool_calls), updated
    /// before each turn's completion is published.
    #[cfg(feature = "ag-ui")]
    pub(crate) fn watch_parked_tool_calls(
        &self,
    ) -> watch::Receiver<Option<everruns_core::host::ParkedToolCalls>> {
        self.inner.parked_calls.subscribe()
    }

    /// Continue the turn parked on client-side tool calls with their
    /// results. Returns once the turn is running again; follow it on the
    /// event stream.
    #[cfg(feature = "ag-ui")]
    pub(crate) async fn resume_tool_results(
        &self,
        results: Vec<everruns_core::events::ToolCompletedData>,
    ) -> Result<(), RunError> {
        if let Some(profile) = self.inner.execution.hosting() {
            let target = profile
                .status(self.session_id())
                .await?
                .open_turn
                .ok_or_else(|| AgentLoopError::config("session has no parked turn"))?;
            profile
                .resolve(self.session_id(), uuid::Uuid::new_v4(), target, results)
                .await?;
            self.inner.parked_calls.send_replace(None);
            self.inner.execution.activate();
            return Ok(());
        }
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Resume { results, response })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        result.await.map_err(|_| RunError::SessionClosed)?
    }

    /// Record `messages` as the session's prior conversation when it has
    /// none yet. Returns whether they were recorded: a session that already
    /// holds history, or is running a turn, is left as it is.
    #[cfg(feature = "ag-ui")]
    pub(crate) async fn seed_history(
        &self,
        messages: Vec<everruns_core::RuntimeMessage>,
    ) -> Result<bool, RunError> {
        if messages.is_empty() {
            return Ok(false);
        }
        if let Some(profile) = self.inner.execution.hosting() {
            self.inner
                .execution
                .ensure_cataloged()
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            return Ok(profile
                .seed_history(self.session_id(), uuid::Uuid::new_v4(), messages)
                .await?);
        }
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Seed { messages, response })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        result.await.map_err(|_| RunError::SessionClosed)?
    }

    async fn send_internal(
        &self,
        input: InputMessage,
        cancel: Option<crate::CancellationToken>,
    ) -> Result<SentMessage, RunError> {
        if self.inner.execution.hosting().is_some() {
            return self
                .admit_with_id(
                    MessageId::new(),
                    input,
                    cancel
                        .as_ref()
                        .is_some_and(crate::CancellationToken::is_cancelled),
                )
                .await;
        }
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Send {
                input: Box::new(AcceptedTurnInput::new(input)),
                cancel,
                response,
            })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        let ack = result.await.map_err(|_| RunError::SessionClosed)??;
        Ok(SentMessage::new(self.clone(), ack))
    }

    async fn command_sender(&self) -> Result<mpsc::Sender<Command>, RunError> {
        if self.inner.execution.hosting().is_some() {
            return Err(AgentLoopError::config(
                "this operation requires a hosting query or command port",
            )
            .into());
        }
        self.start().await.map_err(RunError::Environment)?;
        let _environment_guard = self.inner.environment_gate.lock().await;
        Ok(self
            .inner
            .commands
            .get_or_init(|| async {
                let (sender, receiver) = mpsc::channel(SESSION_COMMAND_CAPACITY);
                everruns_contracts::rt::spawn(SessionActor::new(&self.inner).run(receiver));
                sender
            })
            .await
            .clone())
    }

    async fn cancel_turn(&self, turn_id: TurnId) -> Result<(), CancelError> {
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await
            .map_err(|_| CancelError::SessionClosed)?
            .send(Command::Cancel { turn_id, response })
            .await
            .map_err(|_| CancelError::SessionClosed)?;
        match result.await.map_err(|_| CancelError::SessionClosed)? {
            true => Ok(()),
            false => Err(CancelError::TurnFinished),
        }
    }
}

enum Command {
    Send {
        input: Box<AcceptedTurnInput>,
        cancel: Option<crate::CancellationToken>,
        response: oneshot::Sender<Result<ActorSentMessage, RunError>>,
    },
    Inspect {
        response: oneshot::Sender<Result<crate::SessionContext, RunError>>,
    },
    /// The session's runtime for a control or query (EVE-1297), answered
    /// during an active turn too. Never handed to application code.
    Runtime {
        response: oneshot::Sender<Result<InProcessRuntime, RunError>>,
    },
    Attach {
        request: Box<AttachRequest>,
        response: oneshot::Sender<Result<everruns_core::ContentPart, AttachError>>,
    },
    Cancel {
        turn_id: TurnId,
        response: oneshot::Sender<bool>,
    },
    Interrupted {
        response: oneshot::Sender<Result<Option<InterruptedTurn>, RunError>>,
    },
    ResumeInterrupted {
        response: oneshot::Sender<Result<Option<ResumedTurn>, RunError>>,
    },
    #[cfg(feature = "ag-ui")]
    Override {
        overrides: SessionOverrides,
        response: oneshot::Sender<Result<(), RunError>>,
    },
    #[cfg(feature = "ag-ui")]
    Resume {
        results: Vec<everruns_core::events::ToolCompletedData>,
        response: oneshot::Sender<Result<(), RunError>>,
    },
    #[cfg(feature = "ag-ui")]
    Seed {
        messages: Vec<everruns_core::RuntimeMessage>,
        response: oneshot::Sender<Result<bool, RunError>>,
    },
}

/// How a driven turn begins.
enum TurnEntry {
    /// A new input starts it.
    Input(Box<AcceptedTurnInput>),
    /// A turn a process exit cut off runs its unfinished tool calls again.
    Interrupted,
    /// Client-side tool results continue a parked one.
    #[cfg(feature = "ag-ui")]
    Resume(Vec<everruns_core::events::ToolCompletedData>),
}

/// Session record fields a run replaces for itself (see
/// [`Session::override_record`]). `None` leaves a field as it is.
// Decision: run-scoped instructions ride the session record's own
// `system_prompt`, the additive session layer of the effective system prompt,
// rather than a message in the transcript: the record is re-read at every turn
// and model call, so the latest run's value wins and nothing accumulates in
// history.
#[cfg(feature = "ag-ui")]
#[derive(Debug, Default)]
pub(crate) struct SessionOverrides {
    /// The session layer of the system prompt; `Some(None)` clears it.
    pub(crate) instructions: Option<Option<String>>,
    /// The session's client-side tools, replacing any set before. A
    /// non-empty set also declares that the client answers a pause, so a
    /// turn parks on a call to one instead of running past it.
    pub(crate) client_tools: Option<Vec<everruns_contracts::tool_types::ToolDefinition>>,
}

struct ActorSentMessage {
    message_id: MessageId,
    turn_id: TurnId,
    disposition: SendDisposition,
    completion: watch::Receiver<TurnCompletion>,
}

#[derive(Clone, Debug)]
enum TurnCompletion {
    Pending,
    Ready(Result<Turn, RunError>),
}

/// A not-yet-running Session with its Environment selected.
pub struct EnvironmentSessionBuilder {
    pub(crate) session: Session,
    pub(crate) environment: everruns_core::host::Environment,
}

impl EnvironmentSessionBuilder {
    /// Persist and freeze the head binding before execution starts.
    pub async fn start(self) -> Result<Session, SessionEnvironmentError> {
        let _guard = self.session.inner.environment_gate.lock().await;
        if self.session.inner.commands.initialized() {
            return Err(SessionEnvironmentError::AlreadyStarted);
        }
        self.session.negotiate_environment(&self.environment)?;
        self.session
            .inner
            .execution
            .bind_environment(&self.environment)
            .await?;
        if let Some(recorded) = self.session.inner.environment.get() {
            if recorded.workspace_head().binding() != self.environment.workspace_head().binding() {
                return Err(SessionEnvironmentError::AlreadyBound);
            }
        } else {
            self.session
                .inner
                .environment
                .set(self.environment)
                .map_err(|_| SessionEnvironmentError::AlreadyBound)?;
        }
        drop(_guard);
        Ok(self.session)
    }
}

enum HookRun<T> {
    Completed(T),
    Cancelled,
}

async fn cancellable<T>(
    token: Option<&crate::CancellationToken>,
    future: impl Future<Output = T>,
) -> HookRun<T> {
    match token {
        None => HookRun::Completed(future.await),
        Some(token) => {
            tokio::select! {
                biased;
                () = token.cancelled() => HookRun::Cancelled,
                output = future => HookRun::Completed(output),
            }
        }
    }
}

/// How [`Session::send`] routed an accepted message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SendDisposition {
    /// Durably admitted; consumption has not yet assigned a turn.
    Pending,
    /// The session was idle and started a new turn.
    Started,
    /// The message joined the currently running turn.
    Steered,
}

/// Receipt returned once [`Session::send`] accepts a message.
#[derive(Clone)]
pub struct SentMessage {
    /// Opaque id of the accepted user message.
    pub message_id: String,
    /// Opaque id of the turn that accepted the message.
    pub turn_id: Option<String>,
    /// Whether acceptance started a turn or steered the active one.
    pub disposition: SendDisposition,
    turn: TurnHandle,
}

impl SentMessage {
    fn new(session: Session, message: ActorSentMessage) -> Self {
        let turn = TurnHandle {
            session,
            source: TurnHandleSource::Local {
                turn_id: message.turn_id,
                completion: message.completion,
            },
        };
        Self {
            message_id: message.message_id.to_string(),
            turn_id: Some(message.turn_id.to_string()),
            disposition: message.disposition,
            turn,
        }
    }

    fn admitted(session: Session, admission: crate::hosting::Admission) -> Self {
        Self {
            message_id: admission.message_id().to_string(),
            turn_id: None,
            disposition: SendDisposition::Pending,
            turn: TurnHandle {
                session,
                source: TurnHandleSource::Admission(admission),
            },
        }
    }

    /// Read the authoritative assignment, which can be pending at admission.
    pub async fn assignment(&self) -> Result<Option<crate::hosting::TurnAssignment>, RunError> {
        match &self.turn.source {
            TurnHandleSource::Admission(admission) => Ok(admission.assignment().await?),
            TurnHandleSource::Local { turn_id, .. } => Ok(Some(crate::hosting::TurnAssignment {
                turn_id: *turn_id,
                steered: self.disposition == SendDisposition::Steered,
            })),
        }
    }

    /// Wait for the turn that accepted this message.
    pub async fn wait(&self) -> Result<Turn, RunError> {
        self.turn.wait().await
    }

    /// Obtain a cloneable handle to the accepting turn.
    pub fn turn(&self) -> TurnHandle {
        self.turn.clone()
    }
}

/// A cloneable handle to one active or completed turn.
#[derive(Clone)]
pub struct TurnHandle {
    session: Session,
    source: TurnHandleSource,
}

#[derive(Clone)]
enum TurnHandleSource {
    Local {
        turn_id: TurnId,
        completion: watch::Receiver<TurnCompletion>,
    },
    Admission(crate::hosting::Admission),
}

impl TurnHandle {
    /// Stable identity of this handle, even before a turn is assigned.
    pub fn identity(&self) -> String {
        match &self.source {
            TurnHandleSource::Local { turn_id, .. } => turn_id.to_string(),
            TurnHandleSource::Admission(admission) => admission.message_id().to_string(),
        }
    }
    /// The opaque turn id shared with events and the final [`Turn`].
    pub fn id(&self) -> Option<String> {
        match &self.source {
            TurnHandleSource::Local { turn_id, .. } => Some(turn_id.to_string()),
            TurnHandleSource::Admission(_) => None,
        }
    }

    /// Wait for this turn's terminal result. Multiple waiters receive the same result.
    pub async fn wait(&self) -> Result<Turn, RunError> {
        let mut completion = match &self.source {
            TurnHandleSource::Admission(admission) => return admission.wait().await,
            TurnHandleSource::Local { completion, .. } => completion.clone(),
        };
        loop {
            let state = completion.borrow().clone();
            match state {
                TurnCompletion::Pending => completion
                    .changed()
                    .await
                    .map_err(|_| RunError::SessionClosed)?,
                TurnCompletion::Ready(result) => return result,
            }
        }
    }

    /// Cooperatively cancel this turn if it is still active.
    pub async fn cancel(&self) -> Result<(), CancelError> {
        match &self.source {
            TurnHandleSource::Local { turn_id, .. } => self.session.cancel_turn(*turn_id).await,
            TurnHandleSource::Admission(admission) => admission
                .cancel(uuid::Uuid::new_v4())
                .await
                .map_err(|_| CancelError::SessionClosed),
        }
    }
}

/// Why a turn handle could not cancel its turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CancelError {
    /// The Session closed before the cancellation request could be delivered.
    SessionClosed,
    /// The turn had already reached a terminal state.
    TurnFinished,
}

impl std::fmt::Display for CancelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::SessionClosed => "session is closed",
            Self::TurnFinished => "turn already finished",
        })
    }
}

impl std::error::Error for CancelError {}

/// The outcome of a single [`Session::run`] turn.
///
/// A small, stable projection of the runtime's turn result — no stores, session
/// records, or platform identity.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Turn {
    /// Final text response produced by the turn.
    pub response: String,
    /// Opaque id correlating this turn with emitted events.
    pub turn_id: String,
    /// Why the turn stopped.
    pub stop_reason: TurnStopReason,
    /// Number of reasoning iterations executed.
    pub iterations: usize,
    /// Number of tool calls executed during the turn.
    pub tool_calls: usize,
    /// Whether the turn completed without an unrecoverable failure.
    pub success: bool,
    /// Failure message when `success` is `false`.
    pub error: Option<String>,
    /// Non-fatal lifecycle handler failures observed during this turn.
    ///
    /// These failures never change `success` or rewrite the committed outcome.
    /// Pre-effect agent/turn failures are returned as [`RunError::Hook`]
    /// instead. Tool-start failures block only their call and appear here;
    /// tool-end and completion failures are isolated and also appear here.
    pub hook_failures: Vec<HookFailure>,
}

impl Turn {
    /// The stable outcome of a cancelled turn.
    ///
    /// Synthesized by [`Session::run_with`] when a turn is cancelled in flight:
    /// its future is dropped before the runtime can report an outcome, so the
    /// facade maps that to a non-success turn carrying
    /// [`TurnStopReason::Cancelled`]. `turn_id` is shared with the durable
    /// cancellation event emitted after the run future is dropped.
    pub(crate) fn cancelled(turn_id: TurnId) -> Self {
        Self {
            response: String::new(),
            turn_id: turn_id.to_string(),
            stop_reason: TurnStopReason::Cancelled,
            iterations: 0,
            tool_calls: 0,
            success: false,
            error: Some("turn cancelled".to_string()),
            hook_failures: Vec::new(),
        }
    }
}

impl From<TurnResult> for Turn {
    fn from(result: TurnResult) -> Self {
        Self {
            response: result.response,
            turn_id: result.turn_id.to_string(),
            stop_reason: result.stop_reason,
            iterations: result.iterations,
            tool_calls: result.tool_calls_count,
            success: result.success,
            error: result.error,
            hook_failures: Vec::new(),
        }
    }
}

/// Why a [`Session::run`] could not complete.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum RunError {
    /// Accepted work is durably suspended and requires host intervention.
    Suspended,
    /// The in-process runtime failed to build or execute the turn.
    Runtime(Arc<AgentLoopError>),
    /// A pre-effect lifecycle handler failed before the operation could run.
    Hook(HookFailure),
    /// The session could not select or persist its permanent Environment.
    Environment(SessionEnvironmentError),
    /// The live session actor is no longer available.
    SessionClosed,
    /// The active turn already has the maximum number of pending steering messages.
    SteeringQueueFull,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Suspended => f.write_str("session execution is suspended"),
            RunError::Runtime(err) => write!(f, "session run failed: {err}"),
            RunError::Hook(err) => write!(f, "session hook failed: {err}"),
            RunError::Environment(err) => write!(f, "session environment failed: {err}"),
            RunError::SessionClosed => f.write_str("session is closed"),
            RunError::SteeringQueueFull => f.write_str("active turn steering queue is full"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunError::Suspended => None,
            RunError::Runtime(err) => Some(err.as_ref()),
            RunError::Hook(err) => Some(err),
            RunError::Environment(err) => Some(err),
            RunError::SessionClosed => None,
            RunError::SteeringQueueFull => None,
        }
    }
}

impl From<AgentLoopError> for RunError {
    fn from(err: AgentLoopError) -> Self {
        RunError::Runtime(Arc::new(err))
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
