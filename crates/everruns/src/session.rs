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

use crate::events::{EventStream, FacadeEventBus, RunOptions};
use crate::hooks::{
    AgentStartContext, CompletionContext, HookFailure, HookRunState, TurnStartContext,
};
use crate::{Agent, SessionExecution};

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
    inner: Arc<SessionInner>,
}

pub(crate) struct SessionInner {
    execution: Arc<dyn SessionExecution>,
    session_id: SessionId,
    event_bus: Arc<FacadeEventBus>,
    hook_state: Arc<HookRunState>,
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
                environment: OnceLock::new(),
                environment_gate: tokio::sync::Mutex::new(()),
                commands: OnceCell::new(),
            }),
        };
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

    /// The object-safe engine binding that owns this session identity.
    ///
    /// Advanced applications can retain this binding without depending on a
    /// concrete engine, store, platform, or host runtime type.
    pub fn execution(&self) -> Arc<dyn SessionExecution> {
        self.inner.execution.clone()
    }

    /// Permanently bind this new session to an explicit Environment.
    ///
    /// [`EnvironmentSessionBuilder::start`] persists the opaque head binding
    /// before any runtime can execute. The same head is observable through
    /// [`workspace_head`](Self::workspace_head) for the session lifetime.
    pub fn environment(self, environment: everruns_host::Environment) -> EnvironmentSessionBuilder {
        EnvironmentSessionBuilder {
            session: self,
            environment,
        }
    }

    /// Select and persist this Agent's default head without starting a turn.
    ///
    /// Calling this is optional: [`send`](Self::send) and [`inspect`](Self::inspect)
    /// perform the same one-time selection automatically. After it succeeds,
    /// [`workspace_head`](Self::workspace_head) always returns that exact head.
    pub async fn start(&self) -> Result<(), SessionEnvironmentError> {
        let _environment_guard = self.inner.environment_gate.lock().await;
        if self.inner.environment.get().is_some() {
            return Ok(());
        }
        let environment = self
            .inner
            .execution
            .agent_snapshot()
            .bind_default_session_environment(self.inner.session_id)
            .await?;
        self.inner
            .environment
            .set(environment)
            .map_err(|_| SessionEnvironmentError::AlreadyBound)
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
        let Some(token) = options.cancel else {
            return self.send_and_wait(input).await;
        };
        let sent = self
            .send_internal(input.into(), Some(token.clone()))
            .await?;
        tokio::select! {
            biased;
            result = sent.wait() => result,
            () = token.cancelled() => {
                let _ = sent.turn.cancel().await;
                sent.wait().await
            },
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
    agent: Agent,
    session_id: SessionId,
    event_bus: Arc<FacadeEventBus>,
    hook_state: Arc<HookRunState>,
    environment: Option<everruns_host::Environment>,
    runtime: Option<InProcessRuntime>,
    agent_started: bool,
    deferred: VecDeque<Command>,
}

impl SessionActor {
    fn new(inner: &SessionInner) -> Self {
        Self {
            agent: inner.execution.agent_snapshot(),
            session_id: inner.session_id,
            event_bus: inner.event_bus.clone(),
            hook_state: inner.hook_state.clone(),
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
                .append_accepted_inputs(self.session_id, remaining)
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
            self.runtime = Some(
                self.agent
                    .build_runtime_with_event_sink(
                        self.session_id,
                        self.environment.clone(),
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
    session: Session,
    environment: everruns_host::Environment,
}

impl EnvironmentSessionBuilder {
    /// Persist and freeze the head binding before execution starts.
    pub async fn start(self) -> Result<Session, SessionEnvironmentError> {
        let _guard = self.session.inner.environment_gate.lock().await;
        if self.session.inner.commands.initialized() {
            return Err(SessionEnvironmentError::AlreadyStarted);
        }
        self.session
            .inner
            .execution
            .agent_snapshot()
            .bind_session_environment(self.session.inner.session_id, &self.environment)
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

/// Why an Environment could not be fixed to a Session.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionEnvironmentError {
    AlreadyStarted,
    AlreadyBound,
    ProviderConflict,
    Unavailable,
    Workspace(everruns_host::WorkspaceError),
}

impl From<everruns_host::EnvironmentBindingError> for SessionEnvironmentError {
    fn from(error: everruns_host::EnvironmentBindingError) -> Self {
        match error {
            everruns_host::EnvironmentBindingError::Conflict => Self::AlreadyBound,
            everruns_host::EnvironmentBindingError::Unavailable
            | everruns_host::EnvironmentBindingError::Corrupt => Self::Unavailable,
            _ => Self::Unavailable,
        }
    }
}

impl std::fmt::Display for SessionEnvironmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyStarted => formatter.write_str("session execution already started"),
            Self::AlreadyBound => formatter.write_str("session is already bound to another head"),
            Self::ProviderConflict => {
                formatter.write_str("another workspace provider uses the same provider id")
            }
            Self::Unavailable => formatter.write_str("environment binding store is unavailable"),
            Self::Workspace(error) => write!(formatter, "workspace selection failed: {error}"),
        }
    }
}

impl std::error::Error for SessionEnvironmentError {}

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
    SessionClosed,
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
mod tests {
    use std::sync::{Arc, Mutex};

    use everruns_core::events::EventData;
    use everruns_core::turn::TurnStopReason;
    use everruns_core::{ContentPart, InputMessage, MessageRole};
    use everruns_host::{
        EventHistory, EventHistoryReadLimit, EventHistoryReadRequest, EventReadLimit,
        EventReadRequest, TurnResult,
    };
    use everruns_provider::typed_id::TurnId;

    use super::Turn;
    use crate::{Agent, InMemoryEngine, Model};

    #[tokio::test]
    async fn history_accumulates_across_turns() {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated_capturing("ok", capture.clone()))
            .build()
            .expect("valid agent");

        let session = InMemoryEngine::new().create(agent.clone());
        session.run("hello").await.expect("first turn");
        session.run("continue").await.expect("second turn");

        let calls = capture.lock().unwrap();
        assert_eq!(calls.len(), 2, "two turns => two LLM calls");
        assert!(
            calls[1].len() > calls[0].len(),
            "the second turn's request must include the first turn's messages"
        );
    }

    #[tokio::test]
    async fn normal_session_history_is_rebuilt_from_canonical_events() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .build()
            .expect("valid agent");
        let session = InMemoryEngine::new().create(agent.clone());
        let session_id = session.session_id();

        session.run("hello").await.expect("turn runs");

        let event_log = agent
            .shared_backends()
            .await
            .unwrap_or_else(|_| panic!("run built the shared backends"))
            .event_log
            .clone();
        let events = event_log
            .read_page(EventReadRequest::new(session_id, EventReadLimit::default()))
            .await
            .expect("canonical events replay");
        assert!(
            events.events.iter().all(|event| event.sequence.is_some()),
            "durable replay excludes sequence-less live deltas"
        );

        let canonical_messages: Vec<_> = events
            .events
            .iter()
            .filter_map(|event| match &event.data {
                EventData::InputMessage(data) => Some(data.message.clone()),
                EventData::OutputMessageCompleted(data) => Some(data.message.clone()),
                _ => None,
            })
            .collect();
        let history = EventHistory::new(event_log);
        let page = history
            .read_page(EventHistoryReadRequest::new(
                session_id,
                EventHistoryReadLimit::new(8).expect("valid message limit"),
            ))
            .await
            .expect("event-derived history page");

        assert_eq!(
            serde_json::to_value(&page.messages).expect("history serializes"),
            serde_json::to_value(&canonical_messages).expect("events serialize")
        );
        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[0].text(), Some("hello"));
        assert_eq!(page.messages[1].text(), Some("ok"));
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn two_sessions_do_not_share_history() {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated_capturing("ok", capture.clone()))
            .build()
            .expect("valid agent");

        let first = InMemoryEngine::new().create(agent.clone());
        first.run("a1").await.expect("a1");
        first.run("a2").await.expect("a2");

        let second = InMemoryEngine::new().create(agent.clone());
        second.run("b1").await.expect("b1");

        assert_ne!(first.id(), second.id(), "sessions have distinct ids");

        let calls = capture.lock().unwrap();
        assert_eq!(calls.len(), 3);
        // The second session's first call starts fresh: same size as the first
        // session's first call, and smaller than its accumulated second call.
        assert_eq!(
            calls[2].len(),
            calls[0].len(),
            "a second session must not inherit the first session's history"
        );
        assert!(calls[1].len() > calls[2].len());
    }

    #[tokio::test]
    async fn accepts_multimodal_input() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .build()
            .expect("valid agent");

        let session = InMemoryEngine::new().create(agent.clone());
        // A rich, multi-part InputMessage goes through unchanged.
        let message = InputMessage {
            role: MessageRole::User,
            content: vec![
                ContentPart::text("describe"),
                ContentPart::text("this attachment"),
            ],
            controls: None,
            metadata: None,
            tags: vec![],
        };
        let turn = session.run(message).await.expect("turn runs");
        assert!(turn.success);
    }

    #[test]
    fn turn_preserves_failure_and_stop_reason() {
        let result = TurnResult {
            response: String::new(),
            iterations: 3,
            tool_calls_count: 0,
            success: false,
            error: Some("hit the ceiling".to_string()),
            stop_reason: TurnStopReason::MaxTurnRequests,
            turn_id: TurnId::new(),
        };
        let turn = Turn::from(result);
        assert!(!turn.success);
        assert_eq!(turn.stop_reason, TurnStopReason::MaxTurnRequests);
        assert_eq!(turn.error.as_deref(), Some("hit the ceiling"));
    }

    // --- Events and cancellation (EVE-833) -------------------------------
    //
    // These reach the crate-internal simulator helpers (`simulated_scripted`,
    // `simulated_delayed`) that an external integration test cannot see. The
    // public-surface event/cancellation behaviors live in
    // `tests/session_events.rs`.

    use std::time::Duration;

    use everruns_provider::tool_types::ToolCall;
    use serde_json::json;

    use crate::{CancellationToken, RunOptions, SessionEvent, SessionEventKind};

    async fn drain(mut stream: crate::EventStream) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        while let Some(event) = stream.recv().await.expect("event stream stays lossless") {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn tool_events_correlate_with_parent_turn() {
        let tool = crate::FunctionTool::new(
            "ping",
            "Respond to a ping.",
            json!({ "type": "object", "properties": {} }),
            |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "ok": true })) },
        );
        let agent = Agent::builder()
            .instructions("Call ping when asked.")
            .model(Model::simulated_scripted(
                "done",
                vec![
                    vec![ToolCall {
                        id: "call_ping_1".into(),
                        name: "ping".into(),
                        arguments: json!({}),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent");

        let session = InMemoryEngine::new().create(agent.clone());
        let stream = session.events();
        let turn = session.run("please ping").await.expect("turn runs");
        assert!(turn.success, "turn should succeed: {:?}", turn.error);
        assert_eq!(turn.tool_calls, 1);

        drop(session);
        let events = drain(stream).await;

        let tool_started = events
            .iter()
            .find(|e| matches!(e.kind, SessionEventKind::ToolStarted { .. }))
            .expect("a tool.started event");
        let tool_completed = events
            .iter()
            .find(|e| matches!(e.kind, SessionEventKind::ToolCompleted { .. }))
            .expect("a tool.completed event");

        // Both tool events carry the parent turn's id.
        assert_eq!(tool_started.turn_id.as_deref(), Some(turn.turn_id.as_str()));
        assert_eq!(
            tool_completed.turn_id.as_deref(),
            Some(turn.turn_id.as_str())
        );

        let SessionEventKind::ToolStarted {
            tool_call_id: started_id,
            tool_name,
        } = &tool_started.kind
        else {
            unreachable!("matched ToolStarted above")
        };
        assert_eq!(tool_name, "ping");
        let SessionEventKind::ToolCompleted {
            tool_call_id: completed_id,
            success,
            ..
        } = &tool_completed.kind
        else {
            unreachable!("matched ToolCompleted above")
        };
        assert_eq!(started_id, completed_id, "same tool call across the pair");
        assert!(success, "the ping tool succeeded");
    }

    #[tokio::test]
    async fn cancellation_stops_a_running_turn_with_cancelled_stop_reason() {
        // A long TTFT delay parks the turn so we can cancel it mid-flight.
        let agent = Agent::builder()
            .instructions("You are slow.")
            .model(Model::simulated_delayed(
                "eventually",
                Duration::from_secs(30),
            ))
            .build()
            .expect("valid agent");

        let session = InMemoryEngine::new().create(agent.clone());
        let token = CancellationToken::new();

        let canceller = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            canceller.cancel();
        });

        let turn = session
            .run_with("hi", RunOptions::new().cancel_token(token))
            .await
            .expect("run_with resolves");

        assert!(!turn.success, "a cancelled turn is not a success");
        assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
    }

    #[tokio::test]
    async fn an_uncancelled_run_with_matches_run() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .build()
            .expect("valid agent");

        let session = InMemoryEngine::new().create(agent.clone());
        let turn = session
            .run_with("hi", RunOptions::new())
            .await
            .expect("turn runs");
        assert!(turn.success);
        assert_eq!(turn.response, "ok");
        assert!(turn.hook_failures.is_empty());
    }

    #[tokio::test]
    async fn lifecycle_hooks_wrap_a_tool_call_in_registration_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let tool_order = order.clone();
        let tool = crate::FunctionTool::new(
            "ping",
            "Respond to a ping.",
            json!({ "type": "object", "properties": {} }),
            move |_args: serde_json::Value| {
                let tool_order = tool_order.clone();
                async move {
                    tool_order.lock().unwrap().push("tool");
                    Ok::<_, String>(json!({ "ok": true }))
                }
            },
        );
        let start_one = order.clone();
        let start_two = order.clone();
        let end_one = order.clone();
        let end_two = order.clone();
        let completion = order.clone();
        let agent = Agent::builder()
            .instructions("Call ping when asked.")
            .model(Model::simulated_scripted(
                "done",
                vec![
                    vec![ToolCall {
                        id: "call_ping_hooks".into(),
                        name: "ping".into(),
                        arguments: json!({}),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .on_tool_start(move |context| {
                let start_one = start_one.clone();
                async move {
                    assert_eq!(context.tool_name, "ping");
                    assert!(context.turn_id.is_some());
                    start_one.lock().unwrap().push("start-1");
                }
            })
            .on_tool_start(move |_context| {
                let start_two = start_two.clone();
                async move { start_two.lock().unwrap().push("start-2") }
            })
            .on_tool_end(move |context| {
                let end_one = end_one.clone();
                async move {
                    assert!(context.success());
                    end_one.lock().unwrap().push("end-1");
                }
            })
            .on_tool_end(move |_context| {
                let end_two = end_two.clone();
                async move { end_two.lock().unwrap().push("end-2") }
            })
            .on_completion(move |context| {
                let completion = completion.clone();
                async move {
                    assert!(context.turn.success);
                    completion.lock().unwrap().push("completion");
                }
            })
            .build()
            .expect("valid agent");

        let turn = InMemoryEngine::new()
            .create(agent.clone())
            .run("please ping")
            .await
            .expect("turn runs");

        assert!(turn.hook_failures.is_empty());
        assert_eq!(
            *order.lock().unwrap(),
            ["start-1", "start-2", "tool", "end-1", "end-2", "completion"]
        );
    }

    #[tokio::test]
    async fn tool_start_error_blocks_call_and_skips_later_start_hooks() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let tool_ran = Arc::new(AtomicBool::new(false));
        let tool_ran_in_handler = tool_ran.clone();
        let later_ran = Arc::new(AtomicBool::new(false));
        let later = later_ran.clone();
        let end_context = Arc::new(Mutex::new(None));
        let end_context_in_hook = end_context.clone();
        let tool = crate::FunctionTool::new(
            "ping",
            "Respond to a ping.",
            json!({ "type": "object", "properties": {} }),
            move |_args: serde_json::Value| {
                let tool_ran_in_handler = tool_ran_in_handler.clone();
                async move {
                    tool_ran_in_handler.store(true, Ordering::SeqCst);
                    Ok::<_, String>(json!({ "ok": true }))
                }
            },
        );
        let agent = Agent::builder()
            .instructions("Call ping when asked.")
            .model(Model::simulated_scripted(
                "recovered",
                vec![
                    vec![ToolCall {
                        id: "call_blocked_by_framework_hook".into(),
                        name: "ping".into(),
                        arguments: json!({}),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .on_tool_start(
                |_context| async move { Err::<(), _>("policy backend diagnostic: secret") },
            )
            .on_tool_start(move |_context| {
                let later = later.clone();
                async move { later.store(true, Ordering::SeqCst) }
            })
            .on_tool_end(move |context| {
                let end_context_in_hook = end_context_in_hook.clone();
                async move {
                    *end_context_in_hook.lock().unwrap() = Some(context);
                }
            })
            .build()
            .expect("valid agent");

        let turn = InMemoryEngine::new()
            .create(agent.clone())
            .run("ping")
            .await
            .expect("turn settles");

        assert!(turn.success, "model can recover from a blocked tool call");
        assert!(!tool_ran.load(Ordering::SeqCst));
        assert!(!later_ran.load(Ordering::SeqCst));
        let end_context = end_context.lock().unwrap();
        let end_context = end_context.as_ref().expect("blocked call still ends");
        assert!(!end_context.success());
        let model_visible_error = end_context.error.as_deref().expect("blocked call error");
        assert!(model_visible_error.contains("tool call blocked by tool_start hook #0"));
        assert!(!model_visible_error.contains("secret"));
        assert_eq!(turn.hook_failures.len(), 1);
        assert_eq!(turn.hook_failures[0].point, crate::HookPoint::ToolStart);
        assert_eq!(
            turn.hook_failures[0].message,
            "policy backend diagnostic: secret"
        );
        assert_eq!(
            turn.hook_failures[0].tool_call_id.as_deref(),
            Some("call_blocked_by_framework_hook")
        );
    }

    #[tokio::test]
    async fn tool_end_error_is_isolated_and_later_handlers_run() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let later_ran = Arc::new(AtomicBool::new(false));
        let later = later_ran.clone();
        let tool = crate::FunctionTool::new(
            "ping",
            "Respond to a ping.",
            json!({ "type": "object", "properties": {} }),
            |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "ok": true })) },
        );
        let agent = Agent::builder()
            .instructions("Call ping when asked.")
            .model(Model::simulated_scripted(
                "done",
                vec![
                    vec![ToolCall {
                        id: "call_post_hook_error".into(),
                        name: "ping".into(),
                        arguments: json!({}),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .on_tool_end(|_context| async move { Err::<(), _>("audit sink offline") })
            .on_tool_end(move |_context| {
                let later = later.clone();
                async move { later.store(true, Ordering::SeqCst) }
            })
            .build()
            .expect("valid agent");

        let turn = InMemoryEngine::new()
            .create(agent.clone())
            .run("ping")
            .await
            .expect("turn runs");

        assert!(turn.success);
        assert!(later_ran.load(Ordering::SeqCst));
        assert_eq!(turn.hook_failures.len(), 1);
        assert_eq!(turn.hook_failures[0].point, crate::HookPoint::ToolEnd);
    }

    #[tokio::test]
    async fn cancellation_drops_an_in_flight_hook_and_skips_remaining_hooks() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let started = Arc::new(tokio::sync::Notify::new());
        let started_in_hook = started.clone();
        let later_ran = Arc::new(AtomicBool::new(false));
        let later = later_ran.clone();
        let completion_ran = Arc::new(AtomicBool::new(false));
        let completion = completion_ran.clone();
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("unreachable"))
            .on_turn_start(move |_context| {
                let started_in_hook = started_in_hook.clone();
                async move {
                    started_in_hook.notify_one();
                    std::future::pending::<()>().await;
                }
            })
            .on_turn_start(move |_context| {
                let later = later.clone();
                async move { later.store(true, Ordering::SeqCst) }
            })
            .on_completion(move |_context| {
                let completion = completion.clone();
                async move { completion.store(true, Ordering::SeqCst) }
            })
            .build()
            .expect("valid agent");

        let token = CancellationToken::new();
        let canceller = token.clone();
        tokio::spawn(async move {
            started.notified().await;
            canceller.cancel();
        });
        let turn = tokio::time::timeout(
            Duration::from_secs(2),
            InMemoryEngine::new()
                .create(agent)
                .run_with("hello", RunOptions::new().cancel_token(token)),
        )
        .await
        .expect("cancellation is prompt")
        .expect("run resolves");

        assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
        assert!(!later_ran.load(Ordering::SeqCst));
        assert!(!completion_ran.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancellation_drops_an_in_flight_tool_hook() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let hook_started = Arc::new(tokio::sync::Notify::new());
        let hook_started_inside = hook_started.clone();
        let tool_ran = Arc::new(AtomicBool::new(false));
        let tool_ran_inside = tool_ran.clone();
        let completion_ran = Arc::new(AtomicBool::new(false));
        let completion = completion_ran.clone();
        let tool = crate::FunctionTool::new(
            "ping",
            "Respond to a ping.",
            json!({ "type": "object", "properties": {} }),
            move |_args: serde_json::Value| {
                let tool_ran_inside = tool_ran_inside.clone();
                async move {
                    tool_ran_inside.store(true, Ordering::SeqCst);
                    Ok::<_, String>(json!({ "ok": true }))
                }
            },
        );
        let agent = Agent::builder()
            .instructions("Call ping when asked.")
            .model(Model::simulated_scripted(
                "unreachable",
                vec![vec![ToolCall {
                    id: "call_cancelled_hook".into(),
                    name: "ping".into(),
                    arguments: json!({}),
                }]],
            ))
            .tool(tool)
            .on_tool_start(move |_context| {
                let hook_started_inside = hook_started_inside.clone();
                async move {
                    hook_started_inside.notify_one();
                    std::future::pending::<()>().await;
                }
            })
            .on_completion(move |_context| {
                let completion = completion.clone();
                async move { completion.store(true, Ordering::SeqCst) }
            })
            .build()
            .expect("valid agent");

        let token = CancellationToken::new();
        let canceller = token.clone();
        tokio::spawn(async move {
            hook_started.notified().await;
            canceller.cancel();
        });
        let turn = tokio::time::timeout(
            Duration::from_secs(2),
            InMemoryEngine::new()
                .create(agent)
                .run_with("ping", RunOptions::new().cancel_token(token)),
        )
        .await
        .expect("cancellation is prompt")
        .expect("run resolves");

        assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
        assert!(!tool_ran.load(Ordering::SeqCst));
        assert!(!completion_ran.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn completion_finishes_after_the_runtime_commits_even_if_token_is_cancelled() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let token = CancellationToken::new();
        let cancel_inside = token.clone();
        let completions = Arc::new(AtomicUsize::new(0));
        let first = completions.clone();
        let second = completions.clone();
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .on_completion(move |_context| {
                let cancel_inside = cancel_inside.clone();
                let first = first.clone();
                async move {
                    cancel_inside.cancel();
                    tokio::task::yield_now().await;
                    first.fetch_add(1, Ordering::SeqCst);
                }
            })
            .on_completion(move |_context| {
                let second = second.clone();
                async move {
                    second.fetch_add(1, Ordering::SeqCst);
                }
            })
            .build()
            .expect("valid agent");

        let turn = InMemoryEngine::new()
            .create(agent)
            .run_with("hello", RunOptions::new().cancel_token(token))
            .await
            .expect("committed turn completes its hooks");

        assert!(turn.success);
        assert_eq!(completions.load(Ordering::SeqCst), 2);
    }
}
