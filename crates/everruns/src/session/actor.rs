//! Explicit legacy local/custom-backend facade execution.
use super::*;

pub(super) struct SessionActor {
    execution: Arc<dyn SessionExecution>,
    agent: Agent,
    pub(super) session_id: SessionId,
    event_bus: Arc<FacadeEventBus>,
    pub(super) hook_state: Arc<HookRunState>,
    harness: Option<Harness>,
    environment: Option<everruns_core::host::Environment>,
    pub(super) runtime: Option<InProcessRuntime>,
    /// Runs the turns, built with `runtime`, under the session's lease.
    backend: Option<Arc<dyn TurnBackend>>,
    agent_started: bool,
    deferred: VecDeque<Command>,
    parked_calls: watch::Sender<Option<everruns_core::host::ParkedToolCalls>>,
}

impl SessionActor {
    pub(super) fn new(inner: &SessionInner) -> Self {
        Self {
            execution: inner.execution.clone(),
            agent: inner.execution.agent_snapshot(),
            session_id: inner.session_id,
            event_bus: inner.event_bus.clone(),
            hook_state: inner.hook_state.clone(),
            harness: inner.harness.get().cloned(),
            environment: inner.environment.get().cloned(),
            runtime: None,
            backend: None,
            agent_started: false,
            deferred: VecDeque::new(),
            parked_calls: inner.parked_calls.clone(),
        }
    }

    pub(super) async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
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
                Command::Attach { request, response } => {
                    let _ = response.send(self.attach(request).await);
                }
                Command::Cancel { response, .. } => {
                    let _ = response.send(false);
                }
                Command::Interrupted { response } => {
                    let _ = response.send(self.interrupted_turn().await);
                }
                Command::ResumeInterrupted { response } => {
                    if !self.resume_interrupted(response, &mut commands).await {
                        break;
                    }
                }
                #[cfg(feature = "ag-ui")]
                Command::Override {
                    overrides,
                    response,
                } => {
                    let _ = response.send(self.apply_overrides(overrides).await);
                }
                #[cfg(feature = "ag-ui")]
                Command::Resume { results, response } => {
                    if !self.resume_turn(results, response, &mut commands).await {
                        break;
                    }
                }
                #[cfg(feature = "ag-ui")]
                Command::Seed { messages, response } => {
                    let _ = response.send(self.seed_history(messages).await);
                }
            }
        }
    }

    /// Continue the turn parked on client-side tool calls. Lifecycle
    /// turn-start handlers do not run again: it is the same turn.
    #[cfg(feature = "ag-ui")]
    async fn resume_turn(
        &mut self,
        results: Vec<everruns_core::events::ToolCompletedData>,
        response: oneshot::Sender<Result<(), RunError>>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> bool {
        let parked = self
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.parked_tool_calls(self.session_id));
        let Some(parked) = parked else {
            let _ = response.send(Err(AgentLoopError::store(format!(
                "session {} has no turn waiting for tool results",
                self.session_id
            ))
            .into()));
            return true;
        };
        let _ = response.send(Ok(()));
        self.hook_state.begin_turn();
        let (completion_tx, _) = watch::channel(TurnCompletion::Pending);
        self.drive_turn(
            TurnEntry::Resume(results),
            parked.turn_id,
            TurnSteering::new(),
            completion_tx,
            commands,
        )
        .await
    }

    /// Append `messages` as canonical history events, user messages as
    /// `input.message` and assistant ones as `output.message.completed`, the
    /// events history is projected from. Only while the session has no
    /// message yet, so a retried or replayed seed never duplicates history.
    #[cfg(feature = "ag-ui")]
    async fn seed_history(
        &mut self,
        messages: Vec<everruns_core::RuntimeMessage>,
    ) -> Result<bool, RunError> {
        use everruns_core::events::{
            EventContext, EventRequest, InputMessageData, OutputMessageCompletedData,
        };
        use everruns_core::message::RuntimeMessageRole;

        self.ensure_runtime().await?;
        let backends = self
            .execution
            .backends()
            .await
            .map_err(crate::agent::BackendInitError::into_agent_loop)?;
        let page = everruns_core::host::EventHistory::new(backends.host.event_log.clone())
            .read_page(everruns_core::host::EventHistoryReadRequest::new(
                self.session_id,
                everruns_core::host::EventHistoryReadLimit::new(1)
                    .map_err(|error| AgentLoopError::store(error.to_string()))?,
            ))
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        if !page.messages.is_empty() {
            return Ok(false);
        }
        let emitter = self
            .runtime
            .as_ref()
            .expect("runtime built above")
            .host_event_emitter();
        for message in messages {
            let request = match message.role {
                RuntimeMessageRole::Agent => EventRequest::new(
                    self.session_id,
                    EventContext::empty(),
                    OutputMessageCompletedData::new(message),
                ),
                _ => EventRequest::new(
                    self.session_id,
                    EventContext::empty(),
                    InputMessageData::new(message),
                ),
            };
            emitter.emit(request).await?;
        }
        Ok(true)
    }

    #[cfg(feature = "ag-ui")]
    async fn apply_overrides(&mut self, overrides: SessionOverrides) -> Result<(), RunError> {
        self.ensure_runtime().await?;
        let backends = self
            .execution
            .backends()
            .await
            .map_err(crate::agent::BackendInitError::into_agent_loop)?;
        let store = &backends.host.session_store;
        let mut record = store.get_session(self.session_id).await?.ok_or_else(|| {
            AgentLoopError::store(format!("session not found: {}", self.session_id))
        })?;
        if let Some(instructions) = overrides.instructions {
            record.system_prompt = instructions;
        }
        if let Some(tools) = overrides.client_tools {
            // A plain client-side call parks the turn only when the session
            // declares its client can answer a pause, which the runtime
            // reads from the `setup_connection` hint (see
            // `everruns_core::engine::act_pauses_turn`). Declare it while the
            // session has client-side tools; without them nothing would
            // answer the pause.
            let hints = record.hints.get_or_insert_with(Default::default);
            if tools.is_empty() {
                hints.remove(CLIENT_ANSWERS_PAUSES_HINT);
            } else {
                hints.insert(CLIENT_ANSWERS_PAUSES_HINT.to_string(), true.into());
            }
            if hints.is_empty() {
                record.hints = None;
            }
            record.tools = tools;
        }
        // Read-modify-write without a store-level lock: the actor serializes
        // overrides, a running turn is not polled while one applies, and the
        // in-process runtime keeps no status in the record.
        store.add_session(record).await?;
        Ok(())
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

        self.drive_turn(
            TurnEntry::Input(Box::new(input)),
            turn_id,
            steering,
            completion_tx,
            commands,
        )
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

    pub(super) async fn drive_turn(
        &mut self,
        entry: TurnEntry,
        turn_id: TurnId,
        steering: TurnSteering,
        completion: watch::Sender<TurnCompletion>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> bool {
        let runtime = self.runtime.as_ref().expect("runtime built above").clone();
        let backend = self
            .backend
            .clone()
            .expect("backend built with the runtime");
        // The turn this drives parks anew or not at all.
        self.parked_calls.send_replace(None);
        let (outcome, cancelled) = {
            let input = match entry {
                TurnEntry::Input(input) => TurnInput::Message(input),
                TurnEntry::Interrupted => TurnInput::ResumeInterrupted,
                #[cfg(feature = "ag-ui")]
                TurnEntry::Resume(results) => TurnInput::ToolResults(results),
            };
            let request =
                TurnRequest::new(self.session_id, turn_id, input).with_steering(steering.clone());
            // A turn that cannot start fails like one that fails at once.
            let mut run = match backend.start_turn(request).await {
                Ok(ticket) => ticket,
                Err(error) => {
                    TurnTicket::new(self.session_id, turn_id, std::future::ready(Err(error)))
                }
            };
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
                        // The runtime is built while a turn runs.
                        Some(Command::Attach { request, response }) => {
                            let _ = response.send(self.attach(request).await);
                        }
                        // A running turn is not interrupted, whatever the log says.
                        Some(Command::Interrupted { response }) => {
                            let _ = response.send(Ok(None));
                        }
                        Some(Command::ResumeInterrupted { response }) => {
                            let _ = response.send(Ok(None));
                        }
                        #[cfg(feature = "ag-ui")]
                        Some(Command::Override { overrides, response }) => {
                            let _ = response.send(self.apply_overrides(overrides).await);
                        }
                        // Nothing parks while a turn runs; answer once it ends.
                        #[cfg(feature = "ag-ui")]
                        Some(Command::Resume { results, response }) => {
                            self.deferred.push_back(Command::Resume { results, response });
                        }
                        // A running turn means the session has a conversation.
                        #[cfg(feature = "ag-ui")]
                        Some(Command::Seed { response, .. }) => {
                            let _ = response.send(Ok(false));
                        }
                        Some(Command::Cancel { turn_id: requested, response }) => {
                            if requested == turn_id {
                                steering.close();
                                let _ = response.send(true);
                                self.hook_state.take_failures();
                                cancelled = true;
                                // The turn takes no further step; `run` drops
                                // it before finalization records the cancel.
                                if let Err(error) = backend.cancel(self.session_id).await {
                                    tracing::warn!(error = %error, "turn backend failed to cancel a turn");
                                }
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
        // Before the completion: whoever saw the turn end sees what it
        // parked on.
        self.parked_calls
            .send_replace(runtime.parked_tool_calls(self.session_id));
        let _ = completion.send(TurnCompletion::Ready(result));
        true
    }

    async fn inspect(&mut self) -> Result<crate::SessionContext, RunError> {
        self.ensure_runtime().await?;
        let runtime = self.runtime.as_ref().expect("runtime built above");
        let context = runtime.load_context(self.session_id).await?;
        let commands = runtime.list_commands(self.session_id).await?;
        Ok(crate::SessionContext::from_runtime(
            context,
            self.agent.plugin_warnings(),
            commands,
        ))
    }

    pub(super) async fn ensure_runtime(&mut self) -> Result<(), RunError> {
        if self.runtime.is_none() {
            self.execution
                .ensure_harness_requirement()
                .await
                .map_err(|error| {
                    everruns_contracts::error::AgentLoopError::store(error.to_string())
                })?;
            let host = self
                .execution
                .backends()
                .await
                .map_err(crate::agent::BackendInitError::into_agent_loop)?
                .host
                .clone();
            let leases = host.session_leases.clone();
            let runtime = self
                .agent
                .build_runtime_with_event_sink(
                    host,
                    self.session_id,
                    self.environment.clone(),
                    self.harness.as_ref(),
                    self.event_bus.clone(),
                    self.hook_state.clone(),
                )
                .await?;
            // The session runs as an actor: each turn holds the session's
            // lease, so a process sharing these backends cannot run it at
            // the same time.
            self.backend = Some(self.execution.turn_backend(runtime.clone(), leases)?);
            self.runtime = Some(runtime);
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
