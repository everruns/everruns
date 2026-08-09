//! Multi-turn sessions (EVE-831).
//!
//! [`Agent::session`](crate::Agent::session) opens a [`Session`]; each
//! [`Session::run`] executes one turn and appends canonical events. Two sessions
//! from the same agent are independent and never share history. Dropped sessions
//! can be reopened through [`Agent::resume`](crate::Agent::resume) while the
//! Agent's configured persistence lifecycle remains available.

use std::future::Future;
use std::sync::Arc;

use everruns_core::traits::EventEmitter;
use everruns_core::turn::TurnStopReason;
use everruns_core::typed_id::TurnId;
use everruns_core::{AgentLoopError, InputMessage, SessionId};
use everruns_host::{InProcessRuntime, TurnResult};

use crate::Agent;
use crate::events::{EventStream, FacadeEventBus, RunOptions};
use crate::hooks::{
    AgentStartContext, CompletionContext, HookFailure, HookRunState, TurnStartContext,
};

/// A live, multi-turn conversation with an [`Agent`](crate::Agent).
///
/// Open one with [`Agent::session`](crate::Agent::session). The first
/// [`run`](Self::run) or [`inspect`](Self::inspect) materializes an isolated
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
/// let mut session = agent.session();
/// let first = session.run("hi").await?;
/// let second = session.run("continue").await?;
///
/// assert_eq!(first.response, "Hello!");
/// assert!(second.success);
/// # Ok(())
/// # }
/// ```
pub struct Session {
    agent: Agent,
    session_id: SessionId,
    runtime: Option<InProcessRuntime>,
    /// The session's event sink, created eagerly so [`events`](Session::events)
    /// can subscribe before the first turn builds the runtime. Handed to the
    /// host as its post-commit event sink on first [`run`](Session::run).
    event_bus: Arc<FacadeEventBus>,
    /// Per-session lifecycle state. Handler definitions come from the agent;
    /// tool-hook failures are accumulated here by the runtime adapters.
    hook_state: Arc<HookRunState>,
    /// Set after the complete agent-start chain succeeds. A failed chain is
    /// retried on the next run so a session never enters a half-started state.
    agent_started: bool,
}

impl Session {
    pub(crate) fn new(agent: Agent, session_id: SessionId) -> Self {
        let hook_state = HookRunState::new(agent.lifecycle_hooks());
        Self {
            agent,
            session_id,
            runtime: None,
            event_bus: Arc::new(FacadeEventBus::new()),
            hook_state,
            agent_started: false,
        }
    }

    /// An opaque identifier correlating this session's turns.
    ///
    /// It carries no organization, principal, or platform identity — it is only
    /// useful to line up a session's turns in logs.
    pub fn id(&self) -> String {
        self.session_id.to_string()
    }

    /// The typed Framework identity for this session.
    ///
    /// Use this value with typed session-resumption APIs. [`id`](Self::id)
    /// remains available when a string is needed for display or serialization.
    pub fn session_id(&self) -> SessionId {
        self.session_id
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
        crate::HistoryQuery::new(self.agent.clone(), self.session_id)
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
    /// The returned [`EventStream`] observes every turn run *after* it is
    /// created (subscribe before calling [`run`](Session::run)). Multiple streams
    /// can observe the same session independently, and each session's events are
    /// isolated — one session never sees another's. Dropping a stream, or letting
    /// a consumer fall behind, never affects a running turn. The stream is
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
        self.event_bus.subscribe()
    }

    /// Run one turn and return its [`Turn`] outcome.
    ///
    /// The first run materializes an isolated in-process runtime unless
    /// [`inspect`](Self::inspect) already did so. Later calls reuse it, so
    /// conversation history from earlier turns is included in the next request.
    ///
    /// # Errors
    ///
    /// Returns [`RunError`] if an agent/turn-start handler fails, the runtime
    /// cannot be built, or the turn cannot be executed. A turn that runs but
    /// ends unsuccessfully (e.g. a refusal or a max-iteration stop) is returned
    /// as an `Ok(Turn)` with `success == false` and the
    /// [`stop_reason`](Turn::stop_reason) preserved.
    pub async fn run(&mut self, input: impl Into<InputMessage>) -> Result<Turn, RunError> {
        self.run_with(input, RunOptions::default()).await
    }

    /// Run one turn under the given [`RunOptions`], enabling cancellation.
    ///
    /// Identical to [`run`](Session::run) when the options carry no cancellation
    /// token. When a [`CancellationToken`](crate::CancellationToken) is attached
    /// and cancelled while the turn is in flight, the turn's future is dropped —
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
        &mut self,
        input: impl Into<InputMessage>,
        options: RunOptions,
    ) -> Result<Turn, RunError> {
        let input = input.into();
        self.hook_state.begin_turn();

        if !self.agent_started {
            let context = AgentStartContext {
                agent_name: self.agent.name().to_string(),
                session_id: self.session_id,
            };
            match cancellable(
                options.cancel.as_ref(),
                self.hook_state.hooks().run_agent_start(context),
            )
            .await
            {
                HookRun::Cancelled => return self.emit_cancelled().await,
                HookRun::Completed(Err(failure)) => return Err(RunError::Hook(failure)),
                HookRun::Completed(Ok(())) => self.agent_started = true,
            }
        }

        let context = TurnStartContext {
            agent_name: self.agent.name().to_string(),
            session_id: self.session_id,
            input: input.clone(),
        };
        match cancellable(
            options.cancel.as_ref(),
            self.hook_state.hooks().run_turn_start(context),
        )
        .await
        {
            HookRun::Cancelled => return self.emit_cancelled().await,
            HookRun::Completed(Err(failure)) => return Err(RunError::Hook(failure)),
            HookRun::Completed(Ok(())) => {}
        }

        self.ensure_runtime().await?;
        let runtime = self.runtime.as_ref().expect("runtime built above");

        let mut turn = match options.cancel {
            None => {
                let result = runtime.run_turn(self.session_id, input).await?;
                Turn::from(result)
            }
            Some(token) => {
                if token.is_cancelled() {
                    return self.emit_cancelled().await;
                }

                // Race the turn against cancellation. A completed result wins
                // when both branches become ready together, preventing a
                // synthetic cancellation after a committed terminal event. On
                // cancellation the turn future is dropped, which is the
                // runtime's own cooperative teardown path.
                tokio::select! {
                    biased;
                    result = runtime.run_turn(self.session_id, input) => Turn::from(result?),
                    () = token.cancelled() => {
                        self.hook_state.take_failures();
                        return self.emit_cancelled().await;
                    },
                }
            }
        };

        turn.hook_failures.extend(self.hook_state.take_failures());
        let completion = CompletionContext {
            agent_name: self.agent.name().to_string(),
            session_id: self.session_id,
            turn: turn.clone(),
        };
        // Once the runtime has settled, cancellation cannot roll the committed
        // turn back. Completion handlers therefore finish in order and report
        // failures on the returned turn instead of racing the run token.
        turn.hook_failures
            .extend(self.hook_state.hooks().run_completion(completion).await);
        Ok(turn)
    }

    /// Inspect the exact application-facing context for the next model call.
    ///
    /// This is valid before the first turn and after any later turn. MCP tool
    /// discovery, plugin prompt contributions, message filters, and model
    /// selection use the same runtime assembly path as execution. Inspection
    /// materializes the runtime but does not run any lifecycle handler.
    pub async fn inspect(&mut self) -> Result<crate::SessionContext, RunError> {
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
                        self.event_bus.clone(),
                        self.hook_state.clone(),
                    )
                    .await?,
            );
        }
        Ok(())
    }

    async fn emit_cancelled(&mut self) -> Result<Turn, RunError> {
        self.ensure_runtime().await?;
        let runtime = self.runtime.as_ref().expect("runtime built above");
        let (turn_id, request) = self.event_bus.cancellation_request(self.session_id);
        runtime.host_event_emitter().emit(request).await?;
        Ok(Turn::cancelled(turn_id))
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
#[derive(Debug)]
#[non_exhaustive]
pub enum RunError {
    /// The in-process runtime failed to build or execute the turn.
    Runtime(AgentLoopError),
    /// A pre-effect lifecycle handler failed before the operation could run.
    Hook(HookFailure),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Runtime(err) => write!(f, "session run failed: {err}"),
            RunError::Hook(err) => write!(f, "session hook failed: {err}"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunError::Runtime(err) => Some(err),
            RunError::Hook(err) => Some(err),
        }
    }
}

impl From<AgentLoopError> for RunError {
    fn from(err: AgentLoopError) -> Self {
        RunError::Runtime(err)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use everruns_core::events::EventData;
    use everruns_core::turn::TurnStopReason;
    use everruns_core::{ContentPart, InputMessage, MessageRole, TurnId};
    use everruns_host::{
        EventHistory, EventHistoryReadLimit, EventHistoryReadRequest, EventReadLimit,
        EventReadRequest, TurnResult,
    };

    use super::Turn;
    use crate::{Agent, Model};

    #[tokio::test]
    async fn history_accumulates_across_turns() {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated_capturing("ok", capture.clone()))
            .build()
            .expect("valid agent");

        let mut session = agent.session();
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
        let mut session = agent.session();
        let session_id = session.session_id();

        session.run("hello").await.expect("turn runs");

        let runtime = session.runtime.as_ref().expect("run built the runtime");
        let event_log = runtime.event_log();
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

        let mut first = agent.session();
        first.run("a1").await.expect("a1");
        first.run("a2").await.expect("a2");

        let mut second = agent.session();
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

        let mut session = agent.session();
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

    use everruns_core::ToolCall;
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

        let mut session = agent.session();
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

        let mut session = agent.session();
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

        let mut session = agent.session();
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

        let turn = agent.session().run("please ping").await.expect("turn runs");

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

        let turn = agent.session().run("ping").await.expect("turn settles");

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

        let turn = agent.session().run("ping").await.expect("turn runs");

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
            agent
                .session()
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
            agent
                .session()
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

        let turn = agent
            .session()
            .run_with("hello", RunOptions::new().cancel_token(token))
            .await
            .expect("committed turn completes its hooks");

        assert!(turn.success);
        assert_eq!(completions.load(Ordering::SeqCst), 2);
    }
}
