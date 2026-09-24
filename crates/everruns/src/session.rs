//! Multi-turn sessions (EVE-831).
//!
//! An application-owned [`Engine`](crate::Engine) opens each [`Session`].
//! [`Session::send`] accepts messages and appends canonical events. Two sessions
//! created from the same Agent are independent and never share history. The
//! owning engine reopens dropped sessions while its catalog remains available.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use everruns_core::InputMessage;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::turn::TurnStopReason;
use everruns_host::{
    AcceptedTurnInput, InProcessRuntime, TurnResult, TurnSteering, TurnSteeringPushError,
};
use everruns_provider::error::AgentLoopError;
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use tokio::sync::{OnceCell, mpsc, oneshot, watch};

use crate::engine::SessionExecution;
use crate::events::{EventStream, FacadeEventBus, RunOptions};
use crate::hooks::{
    AgentStartContext, CompletionContext, HookFailure, HookRunState, TurnStartContext,
};
use crate::{Agent, Harness, SessionEnvironmentError};

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
    event_bus: Arc<FacadeEventBus>,
    hook_state: Arc<HookRunState>,
    pub(crate) harness: OnceLock<Harness>,
    environment: OnceLock<everruns_host::Environment>,
    environment_gate: tokio::sync::Mutex<()>,
    commands: OnceCell<mpsc::Sender<Command>>,
}

/// Bounds commands accepted while the actor is busy and deferred inspection or
/// next-turn work retained while a turn reaches its terminal boundary.
// THREAT[TM-DOS-036]: never turn slow model execution into an unbounded mailbox.
const SESSION_COMMAND_CAPACITY: usize = 64;

impl Session {
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
        environment: Option<everruns_host::Environment>,
    ) -> Self {
        let session_id = execution.session_id();
        let agent = execution.agent_snapshot();
        let hook_state = HookRunState::new(agent.lifecycle_hooks());
        let session = Self {
            inner: Arc::new(SessionInner {
                execution,
                session_id,
                event_bus: Arc::new(FacadeEventBus::new()),
                hook_state,
                harness: OnceLock::new(),
                environment: OnceLock::new(),
                environment_gate: tokio::sync::Mutex::new(()),
                commands: OnceCell::new(),
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
        environment: &everruns_host::Environment,
    ) -> Result<(), SessionEnvironmentError> {
        match self.inner.harness.get() {
            Some(harness) => harness.negotiate(environment),
            None => Ok(()),
        }
    }

    /// The permanently selected workspace head after explicit or automatic
    /// Environment start.
    pub fn workspace_head(&self) -> Option<&everruns_host::WorkspaceHead> {
        self.inner
            .environment
            .get()
            .map(everruns_host::Environment::workspace_head)
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
                    Some(timeout) => tokio::time::sleep(timeout).await,
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
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Inspect { response })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        result.await.map_err(|_| RunError::SessionClosed)?
    }

    async fn send_internal(
        &self,
        input: InputMessage,
        cancel: Option<crate::CancellationToken>,
    ) -> Result<SentMessage, RunError> {
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
        self.start().await.map_err(RunError::Environment)?;
        let _environment_guard = self.inner.environment_gate.lock().await;
        Ok(self
            .inner
            .commands
            .get_or_init(|| async {
                let (sender, receiver) = mpsc::channel(SESSION_COMMAND_CAPACITY);
                tokio::spawn(SessionActor::new(&self.inner).run(receiver));
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
    Cancel {
        turn_id: TurnId,
        response: oneshot::Sender<bool>,
    },
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

struct SessionActor {
    execution: Arc<dyn SessionExecution>,
    agent: Agent,
    session_id: SessionId,
    event_bus: Arc<FacadeEventBus>,
    hook_state: Arc<HookRunState>,
    harness: Option<Harness>,
    environment: Option<everruns_host::Environment>,
    runtime: Option<InProcessRuntime>,
    agent_started: bool,
    deferred: VecDeque<Command>,
}

impl SessionActor {
    fn new(inner: &SessionInner) -> Self {
        Self {
            execution: inner.execution.clone(),
            agent: inner.execution.agent_snapshot(),
            session_id: inner.session_id,
            event_bus: inner.event_bus.clone(),
            hook_state: inner.hook_state.clone(),
            harness: inner.harness.get().cloned(),
            environment: inner.environment.get().cloned(),
            runtime: None,
            agent_started: false,
            deferred: VecDeque::new(),
        }
    }

    async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        loop {
            let command = match self.deferred.pop_front() {
                Some(command) => command,
                None => match commands.recv().await {
                    Some(command) => command,
                    None => break,
                },
            };
            match command {
                Command::Send {
                    input,
                    cancel,
                    response,
                } => {
                    if !self
                        .start_turn(input, cancel, response, &mut commands)
                        .await
                    {
                        break;
                    }
                }
                Command::Inspect { response } => {
                    let result = self.inspect().await;
                    let _ = response.send(result);
                }
                Command::Cancel { response, .. } => {
                    let _ = response.send(false);
                }
            }
        }
    }

    async fn start_turn(
        &mut self,
        input: Box<AcceptedTurnInput>,
        cancel: Option<crate::CancellationToken>,
        response: oneshot::Sender<Result<ActorSentMessage, RunError>>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> bool {
        let input = *input;
        let turn_id = TurnId::new();
        let message_id = input.message_id();
        let steering = TurnSteering::new();
        let (completion_tx, completion_rx) = watch::channel(TurnCompletion::Pending);
        let _ = response.send(Ok(ActorSentMessage {
            message_id,
            turn_id,
            disposition: SendDisposition::Started,
            completion: completion_rx,
        }));

        match self
            .prepare_turn(input.input().clone(), cancel.as_ref())
            .await
        {
            HookRun::Cancelled => {
                steering.close();
                self.hook_state.take_failures();
                let result = self.emit_cancelled(turn_id).await;
                let _ = completion_tx.send(TurnCompletion::Ready(result));
                return true;
            }
            HookRun::Completed(Err(error)) => {
                steering.close();
                let _ = completion_tx.send(TurnCompletion::Ready(Err(error)));
                return true;
            }
            HookRun::Completed(Ok(())) => {}
        }

        self.drive_turn(input, turn_id, steering, completion_tx, commands)
            .await
    }

    async fn prepare_turn(
        &mut self,
        input: InputMessage,
        cancel: Option<&crate::CancellationToken>,
    ) -> HookRun<Result<(), RunError>> {
        self.hook_state.begin_turn();
        if !self.agent_started {
            let context = AgentStartContext {
                agent_name: self.agent.name().to_string(),
                session_id: self.session_id,
            };
            match cancellable(cancel, self.hook_state.hooks().run_agent_start(context)).await {
                HookRun::Cancelled => return HookRun::Cancelled,
                HookRun::Completed(Err(failure)) => {
                    return HookRun::Completed(Err(RunError::Hook(failure)));
                }
                HookRun::Completed(Ok(())) => self.agent_started = true,
            }
        }

        let context = TurnStartContext {
            agent_name: self.agent.name().to_string(),
            session_id: self.session_id,
            input,
        };
        match cancellable(cancel, self.hook_state.hooks().run_turn_start(context)).await {
            HookRun::Cancelled => return HookRun::Cancelled,
            HookRun::Completed(Err(failure)) => {
                return HookRun::Completed(Err(RunError::Hook(failure)));
            }
            HookRun::Completed(Ok(())) => {}
        }
        HookRun::Completed(self.ensure_runtime().await)
    }

    async fn drive_turn(
        &mut self,
        input: AcceptedTurnInput,
        turn_id: TurnId,
        steering: TurnSteering,
        completion: watch::Sender<TurnCompletion>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> bool {
        let runtime = self.runtime.as_ref().expect("runtime built above").clone();
        let (outcome, cancelled) = {
            let run = runtime.run_steerable_turn(self.session_id, input, turn_id, steering.clone());
            tokio::pin!(run);
            let mut cancelled = false;
            let outcome = loop {
                tokio::select! {
                    biased;
                    result = &mut run => break Some(result.map(Turn::from).map_err(RunError::from)),
                    command = commands.recv(), if self.deferred.len() < SESSION_COMMAND_CAPACITY => match command {
                        None => {
                            steering.close();
                            return false;
                        }
                        Some(Command::Send { input, cancel, response }) => {
                            let message_id = input.message_id();
                            match steering.try_push(*input) {
                                Ok(()) => {
                                    let _ = response.send(Ok(ActorSentMessage {
                                        message_id,
                                        turn_id,
                                        disposition: SendDisposition::Steered,
                                        completion: completion.subscribe(),
                                    }));
                                }
                                Err(TurnSteeringPushError::Closed(input)) => self.deferred.push_back(Command::Send {
                                    input,
                                    cancel,
                                    response,
                                }),
                                Err(TurnSteeringPushError::Full(_)) => {
                                    let _ = response.send(Err(RunError::SteeringQueueFull));
                                }
                            }
                        }
                        Some(Command::Inspect { response }) => {
                            self.deferred.push_back(Command::Inspect { response });
                        }
                        Some(Command::Cancel { turn_id: requested, response }) => {
                            if requested == turn_id {
                                steering.close();
                                let _ = response.send(true);
                                self.hook_state.take_failures();
                                cancelled = true;
                                break Some(Ok(Turn::cancelled(turn_id)));
                            }
                            let _ = response.send(false);
                        }
                    },
                }
            };
            (outcome, cancelled)
        };

        let finalization = async {
            let remaining = steering.close_and_drain();
            runtime
                .append_accepted_inputs(self.session_id, turn_id, remaining)
                .await?;
            if cancelled {
                let (_, request) = self
                    .event_bus
                    .cancellation_request_for_turn(self.session_id, turn_id);
                runtime.host_event_emitter().emit(request).await?;
            }
            Ok::<_, RunError>(())
        }
        .await;
        let result = match (finalization, outcome.expect("turn outcome set")) {
            (Err(error), _) => Err(error),
            (Ok(()), Ok(turn)) if turn.stop_reason == TurnStopReason::Cancelled => Ok(turn),
            (Ok(()), Ok(mut turn)) => {
                turn.hook_failures.extend(self.hook_state.take_failures());
                let context = CompletionContext {
                    agent_name: self.agent.name().to_string(),
                    session_id: self.session_id,
                    turn: turn.clone(),
                };
                turn.hook_failures
                    .extend(self.hook_state.hooks().run_completion(context).await);
                Ok(turn)
            }
            (Ok(()), Err(error)) => Err(error),
        };
        let _ = completion.send(TurnCompletion::Ready(result));
        true
    }

    async fn inspect(&mut self) -> Result<crate::SessionContext, RunError> {
        self.ensure_runtime().await?;
        let context = self
            .runtime
            .as_ref()
            .expect("runtime built above")
            .load_context(self.session_id)
            .await?;
        Ok(crate::SessionContext::from_runtime(
            context,
            self.agent.plugin_warnings(),
        ))
    }

    async fn ensure_runtime(&mut self) -> Result<(), RunError> {
        if self.runtime.is_none() {
            self.execution
                .ensure_harness_requirement()
                .await
                .map_err(|error| {
                    everruns_provider::error::AgentLoopError::store(error.to_string())
                })?;
            self.runtime = Some(
                self.agent
                    .build_runtime_with_event_sink(
                        self.execution
                            .backends()
                            .await
                            .map_err(crate::agent::BackendInitError::into_agent_loop)?
                            .host
                            .clone(),
                        self.session_id,
                        self.environment.clone(),
                        self.harness.as_ref(),
                        self.event_bus.clone(),
                        self.hook_state.clone(),
                    )
                    .await?,
            );
        }
        Ok(())
    }

    async fn emit_cancelled(&mut self, turn_id: TurnId) -> Result<Turn, RunError> {
        self.ensure_runtime().await?;
        let runtime = self.runtime.as_ref().expect("runtime built above");
        let (_, request) = self
            .event_bus
            .cancellation_request_for_turn(self.session_id, turn_id);
        runtime.host_event_emitter().emit(request).await?;
        Ok(Turn::cancelled(turn_id))
    }
}

/// A not-yet-running Session with its Environment selected.
pub struct EnvironmentSessionBuilder {
    pub(crate) session: Session,
    pub(crate) environment: everruns_host::Environment,
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
    pub turn_id: String,
    /// Whether acceptance started a turn or steered the active one.
    pub disposition: SendDisposition,
    turn: TurnHandle,
}

impl SentMessage {
    fn new(session: Session, message: ActorSentMessage) -> Self {
        let turn = TurnHandle {
            session,
            turn_id: message.turn_id,
            completion: message.completion,
        };
        Self {
            message_id: message.message_id.to_string(),
            turn_id: message.turn_id.to_string(),
            disposition: message.disposition,
            turn,
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
    turn_id: TurnId,
    completion: watch::Receiver<TurnCompletion>,
}

impl TurnHandle {
    /// The opaque turn id shared with events and the final [`Turn`].
    pub fn id(&self) -> String {
        self.turn_id.to_string()
    }

    /// Wait for this turn's terminal result. Multiple waiters receive the same result.
    pub async fn wait(&self) -> Result<Turn, RunError> {
        let mut completion = self.completion.clone();
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
        self.session.cancel_turn(self.turn_id).await
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
#[derive(Clone, Debug)]
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
