//! Multi-turn sessions (EVE-831).
//!
//! [`Agent::session`](crate::Agent::session) opens a [`Session`]; each
//! [`Session::run`] executes one turn and appends to a conversation history that
//! lives for as long as the `Session`. Two sessions from the same agent are
//! independent and never share history.

use std::sync::Arc;

use everruns_core::turn::TurnStopReason;
use everruns_core::typed_id::TurnId;
use everruns_core::{AgentLoopError, InputMessage, SessionId};
use everruns_runtime::{InProcessRuntime, RuntimeMessageStore, TurnResult};

use crate::Agent;
use crate::events::{EventStream, FacadeEventBus, RunOptions};

/// A live, multi-turn conversation with an [`Agent`](crate::Agent).
///
/// Open one with [`Agent::session`](crate::Agent::session). The first
/// [`run`](Self::run) materializes an isolated in-process runtime; later runs
/// reuse it, so history accumulates across turns. Move the `Session` to keep its
/// history; drop it to discard the conversation.
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
    /// runtime as its raw event bus on first [`run`](Session::run).
    event_bus: Arc<FacadeEventBus>,
    /// Optional persisting message store (EVE-836). When set, it replaces the
    /// default in-memory message backend so the session's history is written to
    /// disk and can be reloaded to resume the conversation in a fresh process.
    message_store: Option<Arc<dyn RuntimeMessageStore>>,
}

impl Session {
    pub(crate) fn new(agent: Agent, session_id: SessionId) -> Self {
        Self {
            agent,
            session_id,
            runtime: None,
            event_bus: Arc::new(FacadeEventBus::new()),
            message_store: None,
        }
    }

    /// Open a session whose messages are persisted through `store` (EVE-836).
    #[cfg(feature = "jsonl")]
    pub(crate) fn with_message_store(
        agent: Agent,
        session_id: SessionId,
        store: Arc<dyn RuntimeMessageStore>,
    ) -> Self {
        Self {
            agent,
            session_id,
            runtime: None,
            event_bus: Arc::new(FacadeEventBus::new()),
            message_store: Some(store),
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

    /// Subscribe to this session's live [`SessionEvent`](crate::SessionEvent)
    /// feed.
    ///
    /// The returned [`EventStream`] observes every turn run *after* it is
    /// created (subscribe before calling [`run`](Session::run)). Multiple streams
    /// can observe the same session independently, and each session's events are
    /// isolated — one session never sees another's. Dropping a stream, or letting
    /// a consumer fall behind, never affects a running turn.
    pub fn events(&self) -> EventStream {
        self.event_bus.subscribe()
    }

    /// Run one turn and return its [`Turn`] outcome.
    ///
    /// The first call materializes an isolated in-process runtime for this
    /// session; later calls reuse it, so conversation history from earlier turns
    /// is included in the next request.
    ///
    /// # Errors
    ///
    /// Returns [`RunError`] if the runtime cannot be built or the turn cannot be
    /// executed. A turn that runs but ends unsuccessfully (e.g. a refusal or a
    /// max-iteration stop) is returned as an `Ok(Turn)` with `success == false`
    /// and the [`stop_reason`](Turn::stop_reason) preserved.
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
    /// stops the turn before it starts.
    ///
    /// # Errors
    ///
    /// Same as [`run`](Session::run): [`RunError`] if the runtime cannot be built
    /// or the turn cannot be executed.
    pub async fn run_with(
        &mut self,
        input: impl Into<InputMessage>,
        options: RunOptions,
    ) -> Result<Turn, RunError> {
        self.ensure_runtime().await?;
        let runtime = self.runtime.as_ref().expect("runtime built above");

        match options.cancel {
            None => {
                let result = runtime.run_turn(self.session_id, input).await?;
                Ok(Turn::from(result))
            }
            Some(token) => {
                // Race the turn against cancellation. `biased` checks the token
                // first, so a pre-cancelled token stops the turn before it runs.
                // On cancellation the turn future is dropped, which is the
                // runtime's own cooperative teardown path — no second stop
                // mechanism is introduced.
                tokio::select! {
                    biased;
                    () = token.cancelled() => Ok(Turn::cancelled()),
                    result = runtime.run_turn(self.session_id, input) => Ok(Turn::from(result?)),
                }
            }
        }
    }

    /// Inspect the exact application-facing context for the next model call.
    ///
    /// This is valid before the first turn and after any later turn. MCP tool
    /// discovery, plugin prompt contributions, message filters, and model
    /// selection use the same runtime assembly path as execution.
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
                    .build_runtime_with_event_bus(
                        self.session_id,
                        self.event_bus.clone(),
                        self.message_store.clone(),
                    )
                    .await?,
            );
        }
        Ok(())
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
}

impl Turn {
    /// The stable outcome of a cancelled turn.
    ///
    /// Synthesized by [`Session::run_with`] when a turn is cancelled in flight:
    /// its future is dropped before the runtime can report an outcome, so the
    /// facade maps that to a non-success turn carrying
    /// [`TurnStopReason::Cancelled`]. The `turn_id` is a fresh correlation id —
    /// the dropped turn's own id is not recoverable.
    fn cancelled() -> Self {
        Self {
            response: String::new(),
            turn_id: TurnId::new().to_string(),
            stop_reason: TurnStopReason::Cancelled,
            iterations: 0,
            tool_calls: 0,
            success: false,
            error: Some("turn cancelled".to_string()),
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
        }
    }
}

/// Why a [`Session::run`] could not complete.
#[derive(Debug)]
#[non_exhaustive]
pub enum RunError {
    /// The in-process runtime failed to build or execute the turn.
    Runtime(AgentLoopError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Runtime(err) => write!(f, "session run failed: {err}"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunError::Runtime(err) => Some(err),
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

    use everruns_core::turn::TurnStopReason;
    use everruns_core::{ContentPart, InputMessage, MessageRole, TurnId};
    use everruns_runtime::TurnResult;

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
        while let Some(event) = stream.recv().await {
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
    }
}
