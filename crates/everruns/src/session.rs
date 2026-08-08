//! Multi-turn sessions (EVE-831).
//!
//! [`Agent::session`](crate::Agent::session) opens a [`Session`]; each
//! [`Session::run`] executes one turn and appends to a conversation history that
//! lives for as long as the `Session`. Two sessions from the same agent are
//! independent and never share history.

use everruns_core::turn::TurnStopReason;
use everruns_core::{AgentLoopError, InputMessage, SessionId};
use everruns_runtime::{InProcessRuntime, TurnResult};

use crate::Agent;

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
}

impl Session {
    pub(crate) fn new(agent: Agent, session_id: SessionId) -> Self {
        Self {
            agent,
            session_id,
            runtime: None,
        }
    }

    /// An opaque identifier correlating this session's turns.
    ///
    /// It carries no organization, principal, or platform identity — it is only
    /// useful to line up a session's turns in logs.
    pub fn id(&self) -> String {
        self.session_id.to_string()
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
        if self.runtime.is_none() {
            self.runtime = Some(self.agent.build_runtime(self.session_id).await?);
        }
        let runtime = self.runtime.as_ref().expect("runtime built above");
        let result = runtime.run_turn(self.session_id, input).await?;
        Ok(Turn::from(result))
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
}
