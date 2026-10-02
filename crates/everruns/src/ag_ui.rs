//! Serve a session over [AG-UI](https://docs.ag-ui.com) 1.0.
//!
//! Stability: alpha — may change without a major bump; see
//! [`stability`](crate::stability). Requires the `ag-ui` feature.
//!
//! AG-UI is the event protocol between an agent and the application that
//! renders it (CopilotKit, `@ag-ui/client`, ...): the application posts a
//! [`RunAgentInput`] and reads back a stream of [`Event`]s, usually as
//! server-sent events. [`Session::ag_ui`] answers one such request from any
//! [`Session`], so a Rust host can mount an AG-UI endpoint on whatever HTTP
//! server it already runs, without the Everruns server.
//!
//! The run is projected by the same state machine the Everruns server and
//! `serve` use, with the trusted [`ProjectionPolicy`]: assistant text,
//! reasoning, and token usage are visible, and failures carry the runtime's
//! message.
//!
//! # Interrupts
//!
//! In-process `ask_user` questions and tool approvals are answered by
//! responders registered on the agent. Register an [`InterruptGate`] as both,
//! and pass it in [`AgUiOptions::gate`]: a question or approval then ends the
//! AG-UI run with the interrupt outcome, the turn stays parked in the
//! process, and the next run's [`RunAgentInput::resume`] entries answer it
//! and stream the rest of the same turn. A host that already parks these
//! requests for an API of its own implements [`InterruptSource`] and passes
//! it in [`AgUiOptions::interrupts`] instead.
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::ag_ui::{AgUiOptions, Event, InterruptGate, Message, RunAgentInput};
//! use everruns::{Agent, Engine, Model};
//! use futures::StreamExt;
//!
//! let gate = InterruptGate::new();
//! let agent = Agent::builder()
//!     .instructions("Be brief.")
//!     .model(Model::simulated("Hello from Everruns."))
//!     .ask_user(gate.clone())
//!     .approver(gate.clone())
//!     .build()?;
//! let session = Engine::new().create(agent);
//!
//! let input = RunAgentInput {
//!     thread_id: "thread-1".into(),
//!     run_id: "run-1".into(),
//!     messages: vec![Message::user("m1", "Hi")],
//!     ..RunAgentInput::default()
//! };
//! let events: Vec<Event> = session
//!     .ag_ui_with(input, AgUiOptions::new().gate(gate))
//!     .await?
//!     .collect()
//!     .await;
//! assert!(matches!(events.first(), Some(Event::RunStarted(_))));
//! assert!(matches!(events.last(), Some(Event::RunFinished(_))));
//! # Ok(())
//! # }
//! ```
//!
//! # What the input carries
//!
//! The [`Session`] owns the conversation, so a run sends only the input's
//! last message, which must be a user message's text, or the trailing
//! `tool` messages that answer its frontend tool calls (below). Earlier
//! messages, `state` and `forwardedProps` are not read, and neither are
//! `system` and `developer` messages or `context` unless the host trusts
//! them (below). The run does not read `threadId`: [`AgUiThreads`] maps
//! each thread to a session (one per thread, kept in a [`ThreadStore`]), or
//! map them yourself. A thread's first run can carry the client's earlier
//! messages into the new session with [`AgUiOptions::seed_history`].
//!
//! # Frontend tools
//!
//! The input's `tools` are tools the client runs. Each run sets them as the
//! session's client-side tools, so the latest set wins. When the model calls
//! one, the turn parks: the run streams `TOOL_CALL_START`/`ARGS`/`END` with
//! the call's name and arguments under the assistant message that made it,
//! and finishes in success with `pendingToolCallIds` (or with the interrupt
//! outcome, the calls beside it, when an interrupt is open too). The next
//! run's trailing `tool` messages are the results: they continue the same
//! turn, and the run streams the rest of it. A result for a call that is not
//! parked is ignored and logged; while a parked call has no result, nothing
//! is recorded and the run reports the calls again, as it does when a new
//! user message arrives instead. Definitions are bounded (64 tools, names
//! `^[A-Za-z0-9_-]{1,64}$`, 4096-character descriptions, 16 KiB schemas,
//! 256 KiB results) and the reserved `mcp_` prefix is refused. Parked calls
//! live in memory: a process exit leaves them unanswered.
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::ag_ui::wire::RunFinishedOutcome;
//! use everruns::ag_ui::{Event, Message, RunAgentInput};
//! use everruns::{Agent, Engine, LlmSimConfig, Model, ToolCall};
//! use futures::StreamExt;
//!
//! let model = Model::simulated_with_config(
//!     LlmSimConfig::fixed("Done.").with_tool_call_sequence(vec![
//!         vec![ToolCall {
//!             id: "call_1".into(),
//!             name: "confirm".into(),
//!             arguments: serde_json::json!({}),
//!         }],
//!         vec![],
//!     ]),
//! );
//! let agent = Agent::builder().instructions("Confirm first.").model(model).build()?;
//! let session = Engine::new().create(agent);
//! let tools = vec![serde_json::from_value(serde_json::json!({
//!     "name": "confirm",
//!     "description": "Ask the person to confirm.",
//! }))?];
//!
//! let first = RunAgentInput {
//!     messages: vec![Message::user("m1", "Deploy.")],
//!     tools: tools.clone(),
//!     ..RunAgentInput::default()
//! };
//! let events: Vec<Event> = session.ag_ui(first).await?.collect().await;
//! let Some(Event::RunFinished(finished)) = events.last() else { panic!() };
//! assert_eq!(
//!     finished.outcome,
//!     Some(RunFinishedOutcome::Success {
//!         pending_tool_call_ids: Some(vec!["call_1".into()]),
//!     })
//! );
//!
//! let second = RunAgentInput {
//!     messages: vec![serde_json::from_value(serde_json::json!({
//!         "id": "t1", "role": "tool", "toolCallId": "call_1", "content": "yes",
//!     }))?],
//!     tools,
//!     ..RunAgentInput::default()
//! };
//! let events: Vec<Event> = session.ag_ui(second).await?.collect().await;
//! assert!(events.iter().any(|event| matches!(event, Event::TextMessageContent(_))));
//! # Ok(())
//! # }
//! ```
//!
//! # Trusted instructions
//!
//! A host that owns both ends, or authenticates whoever posts the input, can
//! let each run carry instructions with [`AgUiOptions::input_instructions`].
//! The run's `system` and `developer` messages, then its `context` entries,
//! become additional system instructions for that run, after the agent's
//! own; the next run replaces them with its own (or none). Such messages may
//! sit anywhere in `messages`, including after the user message, which stays
//! the run's input. Leave it off for a caller you do not trust: these
//! messages speak with the system prompt's authority.
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::ag_ui::{AgUiOptions, Message, RunAgentInput, wire};
//! use everruns::{Agent, Engine, Model};
//! use futures::StreamExt;
//!
//! let agent = Agent::builder()
//!     .instructions("Be brief.")
//!     .model(Model::simulated("ok"))
//!     .build()?;
//! let session = Engine::new().create(agent);
//! let input = RunAgentInput {
//!     messages: vec![
//!         Message::user("m1", "Hi"),
//!         Message::System(wire::TextOnlyMessage {
//!             id: "s1".into(),
//!             content: "You are the release coworker.".into(),
//!             ..Default::default()
//!         }),
//!     ],
//!     ..RunAgentInput::default()
//! };
//! let options = AgUiOptions::new().input_instructions(true);
//! let _events: Vec<_> = session.ag_ui_with(input, options).await?.collect().await;
//! assert!(session.inspect().await?.instructions.contains("release coworker"));
//! # Ok(())
//! # }
//! ```

// Decision: an in-process responder blocks the turn rather than parking it
// durably, so interrupts need a host-side bridge. `InterruptGate` is that
// bridge, built like `serve`'s gate: it parks each request under (session,
// tool call id) until a resume entry answers it. The turn stays alive in the
// process between the interrupted run and the resuming one; a process restart
// cancels it, which the runtime records as a cancelled turn.
//
// Decision: `InterruptSource` is the seam between a run and whatever parks
// requests. `serve` implements it over the pending approvals and questions
// its `/v1` API already answers, so one responder serves both APIs and an
// AG-UI client and a `/question-answers` caller see the same request.
//
// Decision: `ag-ui` carries no HTTP server dependency. The ready-made axum
// route (`AgUiHandler`, `sse_response`) sits behind `ag-ui-axum`, in
// `ag_ui/handler.rs`.
//
// Decision: frontend tools are the session's client-side tools, as on the
// server. The in-process runtime parks a turn on a call to one and keeps its
// resume state (`InProcessRuntime::resume_steerable_turn`), so the next run's
// results continue the same turn instead of starting one with a synthetic
// message. A run that sees the calls requested waits for the turn to record
// the park before it ends, so the next run always finds it.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use async_trait::async_trait;
use everruns_ag_ui::{Tool as WireTool, ToolCall as WireToolCall};
use everruns_core::events::{ToolCallRequestedData, ToolCompletedData};
use everruns_core::message::ContentPart;
use everruns_host::ParkedToolCalls;
use everruns_provider::tool_types::ClientSideTool;
use futures::{Stream, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot, watch};

use crate::approval::{ApprovalDecision, ToolApprover};
use crate::ask_user::{
    Answer, AnsweredBy, AskContext, AskUser, Outcome, Question, QuestionKind, Status,
};
use crate::session::SessionOverrides;
use crate::{
    EventStream, EventStreamError, RunError, SentMessage, Session, SessionId, ToolCall,
    ToolDefinition,
};

mod frontend_tools;
#[cfg(feature = "ag-ui-axum")]
mod handler;
mod seed;
mod shapes;
mod threads;

use frontend_tools::{frontend_definitions, trailing_results};

#[cfg(feature = "ag-ui-axum")]
pub use handler::{
    AgUiAuthorizer, AgUiCaller, AgUiHandler, SSE_KEEPALIVE, StaticToken, Unauthenticated,
    Unauthorized, sse_response,
};
pub use shapes::{approval_decision, approval_interrupt, question_interrupt, question_outcome};
#[cfg(feature = "local")]
pub use threads::SqliteThreadStore;
pub use threads::{
    AgUiThreads, InMemoryThreadStore, ThreadError, ThreadSession, ThreadStore, ThreadStoreError,
};

pub use everruns_ag_ui::projection::{ProjectionPolicy, Projector, TurnFailure};
pub use everruns_ag_ui::{
    Event, Interrupt, Message, PROTOCOL_VERSION, ResumeEntry, ResumeStatus, RunAgentInput,
    RunErrorEvent, RunStartedEvent,
};

/// The complete AG-UI 1.0 wire-type crate, for types not re-exported here.
///
/// ```
/// use everruns::ag_ui::wire::RunFinishedOutcome;
///
/// let outcome = RunFinishedOutcome::Cancelled;
/// assert_eq!(serde_json::to_value(&outcome)?["type"], "cancelled");
/// # Ok::<(), serde_json::Error>(())
/// ```
pub use everruns_ag_ui as wire;

/// `Interrupt.reason` for an `ask_user` question set.
///
/// The interrupt's `responseSchema` describes the answer: the body of the
/// Everruns question-answers request, `{ "status", "answers": [...] }`.
///
/// ```
/// assert_eq!(everruns::ag_ui::ASK_USER_REASON, "everruns.ask_user");
/// ```
pub const ASK_USER_REASON: &str = "everruns.ask_user";

/// `Interrupt.reason` for a question set that asks for a credential.
///
/// A credential must not travel through the transcript, so such an interrupt
/// can only be abandoned (`status: "cancelled"`), which declines it.
///
/// ```
/// assert_eq!(everruns::ag_ui::SECRET_REASON, "everruns.secret_required");
/// ```
pub const SECRET_REASON: &str = "everruns.secret_required";

/// `Interrupt.reason` for a tool call waiting on approval.
///
/// The answer payload is `{ "decision": "allow" | "allow_always" | "reject" |
/// "reject_always" }`; abandoning the interrupt rejects the call.
///
/// ```
/// assert_eq!(everruns::ag_ui::TOOL_APPROVAL_REASON, "tool_approval");
/// ```
pub const TOOL_APPROVAL_REASON: &str = "tool_approval";

/// Why an AG-UI request was refused before its run started.
///
/// ```
/// use everruns::ag_ui::AgUiError;
///
/// let error = AgUiError::InvalidInput("the final message must be a user message".into());
/// assert!(error.to_string().contains("user message"));
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum AgUiError {
    /// The input cannot start or resume a run as sent: answer HTTP 400.
    InvalidInput(String),
    /// The session refused the message.
    Run(RunError),
    /// [`AgUiThreads`] could not resolve the thread to a session.
    Thread(ThreadError),
}

impl std::fmt::Display for AgUiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(why) => write!(f, "invalid AG-UI input: {why}"),
            Self::Run(error) => write!(f, "{error}"),
            Self::Thread(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AgUiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidInput(_) => None,
            Self::Run(error) => Some(error),
            Self::Thread(error) => Some(error),
        }
    }
}

impl From<RunError> for AgUiError {
    fn from(error: RunError) -> Self {
        Self::Run(error)
    }
}

fn invalid(why: impl Into<String>) -> AgUiError {
    AgUiError::InvalidInput(why.into())
}

/// How [`Session::ag_ui_with`] runs.
///
/// ```
/// use everruns::ag_ui::{AgUiOptions, InterruptGate, ProjectionPolicy};
///
/// let options = AgUiOptions::new()
///     .gate(InterruptGate::new())
///     .policy(ProjectionPolicy {
///         usage_visible: false,
///         ..ProjectionPolicy::default()
///     });
/// # let _ = options;
/// ```
#[derive(Clone, Default)]
pub struct AgUiOptions {
    policy: ProjectionPolicy,
    interrupts: Option<Arc<dyn InterruptSource>>,
    input_instructions: bool,
    seed_history: bool,
}

impl std::fmt::Debug for AgUiOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgUiOptions")
            .field("policy", &self.policy)
            .field("interrupts", &self.interrupts.is_some())
            .field("input_instructions", &self.input_instructions)
            .field("seed_history", &self.seed_history)
            .finish()
    }
}

impl AgUiOptions {
    /// The trusted policy and no gate, the same as [`Session::ag_ui`].
    ///
    /// ```
    /// let options = everruns::ag_ui::AgUiOptions::new();
    /// # let _ = options;
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// What the run exposes. The default is trusted: reasoning and usage
    /// visible, runtime errors passed through.
    ///
    /// ```
    /// use everruns::ag_ui::{AgUiOptions, ProjectionPolicy};
    ///
    /// let quiet = AgUiOptions::new().policy(ProjectionPolicy {
    ///     reasoning_visible: false,
    ///     ..ProjectionPolicy::default()
    /// });
    /// # let _ = quiet;
    /// ```
    pub fn policy(mut self, policy: ProjectionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The gate the session's agent answers questions and approvals through.
    /// With it, a parked request ends the run with an interrupt and
    /// [`RunAgentInput::resume`] answers it. Without it, resume entries are
    /// ignored and requests go to whatever responder the agent has.
    ///
    /// ```
    /// use everruns::ag_ui::{AgUiOptions, InterruptGate};
    ///
    /// let gate = InterruptGate::new();
    /// let options = AgUiOptions::new().gate(gate.clone());
    /// # let _ = options;
    /// ```
    pub fn gate(self, gate: InterruptGate) -> Self {
        self.interrupts(gate)
    }

    /// Read and answer interrupts through a host's own
    /// [`InterruptSource`] instead of an [`InterruptGate`]. Replaces any
    /// gate set before.
    ///
    /// ```
    /// use everruns::ag_ui::{AgUiOptions, InterruptGate};
    ///
    /// // An `InterruptGate` is itself a source.
    /// let options = AgUiOptions::new().interrupts(InterruptGate::new());
    /// # let _ = options;
    /// ```
    pub fn interrupts(mut self, source: impl InterruptSource) -> Self {
        self.interrupts = Some(Arc::new(source));
        self
    }

    /// Trust the input's `system` and `developer` messages and `context`
    /// entries as instructions for the run. Off by default.
    ///
    /// When on, every run sets the session's run instructions from its own
    /// input: those messages in order, then the context entries, appended
    /// after the agent's instructions. A run that carries none clears the
    /// previous run's. System and developer messages may appear anywhere in
    /// `messages`, including after the user message the run sends. The
    /// instructions stay on the session until the next run replaces them, so
    /// a turn started with [`Session::send`] in between sees them too.
    ///
    /// Turn it on only when the host authenticates whoever posts the input:
    /// these messages carry the system prompt's authority, so an untrusted
    /// caller could rewrite the agent's rules. When off, they are ignored and
    /// a message after the user message is refused.
    ///
    /// ```
    /// let options = everruns::ag_ui::AgUiOptions::new().input_instructions(true);
    /// # let _ = options;
    /// ```
    pub fn input_instructions(mut self, trusted: bool) -> Self {
        self.input_instructions = trusted;
        self
    }

    /// Record the input's earlier user and assistant messages as the
    /// session's prior history when it has none yet. Off by default;
    /// [`AgUiThreads`] turns it on for each thread it creates. Messages
    /// before the run's user message are recorded in order (assistant tool
    /// calls as text; `tool`, `system`, `developer`, activity and reasoning
    /// messages never), keeping the newest 256 and 512 KiB. They are client
    /// text, assistant turns included: no more trusted than the user message.
    ///
    /// ```
    /// let options = everruns::ag_ui::AgUiOptions::new().seed_history(true);
    /// # let _ = options;
    /// ```
    pub fn seed_history(mut self, seed: bool) -> Self {
        self.seed_history = seed;
        self
    }
}

/// A run's trusted instructions: its `system` and `developer` messages in
/// order, then its `context` entries. `None` when it carries none.
fn input_instructions(input: &RunAgentInput) -> Option<String> {
    let mut parts: Vec<String> = input
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::System(message) | Message::Developer(message) => Some(message.content.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .collect();
    let context: Vec<String> = input
        .context
        .iter()
        .filter(|entry| !entry.value.trim().is_empty())
        .map(|entry| match entry.description.trim() {
            "" => entry.value.trim().to_string(),
            description => format!("{description}:\n{}", entry.value.trim()),
        })
        .collect();
    if !context.is_empty() {
        parts.push(format!(
            "Context from the application:\n\n{}",
            context.join("\n\n")
        ));
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

// --- Interrupt gate -----------------------------------------------------------

/// Parks `ask_user` questions and tool approvals as AG-UI interrupts.
///
/// Register one gate as both the agent's
/// [`ask_user`](crate::AgentBuilder::ask_user) responder and its
/// [`approver`](crate::AgentBuilder::approver), then pass it to
/// [`AgUiOptions::gate`]. Clones share their parked requests, so one gate can
/// serve every session of an engine. A request waits until a resume entry
/// answers it or its turn ends (cancel, process exit, `ask_user` timeout).
///
/// ```
/// use everruns::ag_ui::InterruptGate;
/// use everruns::{Agent, Model};
///
/// let gate = InterruptGate::new();
/// let agent = Agent::builder()
///     .instructions("Ask before deploying.")
///     .model(Model::simulated("ok"))
///     .ask_user(gate.clone())
///     .approver(gate)
///     .build();
/// assert!(agent.is_ok());
/// ```
#[derive(Clone)]
pub struct InterruptGate {
    inner: Arc<GateInner>,
}

struct GateInner {
    parked: Mutex<HashMap<Key, Parked>>,
    next: Mutex<u64>,
    /// The session a request just parked on.
    parked_on: broadcast::Sender<SessionId>,
}

/// (session, tool call id): simulated call ids repeat across sessions.
type Key = (SessionId, String);

struct Parked {
    order: u64,
    request: Request,
}

enum Request {
    Question {
        questions: Vec<Question>,
        tx: oneshot::Sender<Outcome>,
    },
    Approval {
        call: ToolCall,
        tx: oneshot::Sender<ApprovalDecision>,
    },
}

impl Parked {
    fn interrupt(&self, tool_call_id: &str) -> Interrupt {
        match &self.request {
            Request::Question { questions, .. } => question_interrupt(tool_call_id, questions),
            Request::Approval { call, .. } => approval_interrupt(call),
        }
    }
}

impl Default for InterruptGate {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InterruptGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterruptGate")
            .field("parked", &lock(&self.inner.parked).len())
            .finish()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl InterruptGate {
    /// A gate with nothing parked.
    ///
    /// ```
    /// let gate = everruns::ag_ui::InterruptGate::new();
    /// # let _ = gate;
    /// ```
    pub fn new() -> Self {
        let (parked_on, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(GateInner {
                parked: Mutex::new(HashMap::new()),
                next: Mutex::new(0),
                parked_on,
            }),
        }
    }

    /// The interrupts open on `session_id`, oldest first. Empty when the
    /// session waits on nothing.
    ///
    /// ```
    /// use everruns::ag_ui::InterruptGate;
    /// use everruns::{Agent, Engine, Model};
    ///
    /// let gate = InterruptGate::new();
    /// let agent = Agent::builder()
    ///     .instructions("Be brief.")
    ///     .model(Model::simulated("ok"))
    ///     .ask_user(gate.clone())
    ///     .build()
    ///     .expect("valid agent");
    /// let session = Engine::new().create(agent);
    /// assert!(gate.interrupts(session.session_id()).is_empty());
    /// ```
    pub fn interrupts(&self, session_id: SessionId) -> Vec<Interrupt> {
        let parked = lock(&self.inner.parked);
        let mut open: Vec<(&Key, &Parked)> = parked
            .iter()
            .filter(|((session, _), _)| *session == session_id)
            .collect();
        open.sort_by_key(|(_, parked)| parked.order);
        open.into_iter()
            .map(|((_, call_id), parked)| parked.interrupt(call_id))
            .collect()
    }

    fn park(&self, key: Key, request: Request) -> ParkGuard<'_> {
        let order = {
            let mut next = lock(&self.inner.next);
            *next += 1;
            *next
        };
        let session_id = key.0;
        lock(&self.inner.parked).insert(key.clone(), Parked { order, request });
        let _ = self.inner.parked_on.send(session_id);
        ParkGuard { gate: self, key }
    }

    /// Apply a run's resume entries to `session_id`'s parked requests.
    ///
    /// Producer rules from the 1.0 interrupt pattern: an entry naming no open
    /// interrupt is ignored; an open interrupt without an entry is not
    /// abandoned, so nothing is resolved and the run interrupts again; every
    /// entry is validated before any is applied.
    fn resume_entries(
        &self,
        session_id: SessionId,
        entries: &[ResumeEntry],
    ) -> Result<ResumeOutcome, AgUiError> {
        let mut parked = lock(&self.inner.parked);
        let mut open: Vec<(&Key, &Parked)> = parked
            .iter()
            .filter(|((session, _), _)| *session == session_id)
            .collect();
        open.sort_by_key(|(_, parked)| parked.order);
        for entry in entries {
            if !open.iter().any(|((_, id), _)| *id == entry.interrupt_id) {
                tracing::warn!(
                    session_id = %session_id,
                    interrupt_id = %entry.interrupt_id,
                    "AG-UI resume entry names no open interrupt; ignoring it"
                );
            }
        }
        if open.is_empty() {
            return Ok(ResumeOutcome::NothingOpen);
        }
        let entry_for = |id: &str| entries.iter().find(|entry| entry.interrupt_id == id);
        if open.iter().any(|((_, id), _)| entry_for(id).is_none()) {
            return Ok(ResumeOutcome::StillOpen(
                open.iter()
                    .map(|((_, id), parked)| parked.interrupt(id))
                    .collect(),
            ));
        }

        enum Answer {
            Question(Outcome),
            Approval(ApprovalDecision),
        }
        let mut answers = Vec::with_capacity(open.len());
        for ((_, id), parked) in &open {
            let Some(entry) = entry_for(id) else {
                continue;
            };
            let answer = match &parked.request {
                Request::Question { questions, .. } => {
                    Answer::Question(question_outcome(entry, id, questions)?)
                }
                Request::Approval { .. } => Answer::Approval(approval_decision(entry)?),
            };
            answers.push(((*id).clone(), answer));
        }
        for (id, answer) in answers {
            let Some(parked) = parked.remove(&(session_id, id)) else {
                continue;
            };
            // A send fails only if the turn already went away; the guard
            // removing the entry would have beaten us to it.
            match (parked.request, answer) {
                (Request::Question { tx, .. }, Answer::Question(outcome)) => {
                    let _ = tx.send(outcome);
                }
                (Request::Approval { tx, .. }, Answer::Approval(decision)) => {
                    let _ = tx.send(decision);
                }
                _ => {}
            }
        }
        Ok(ResumeOutcome::Resumed)
    }
}

/// What applying a run's resume entries did, as an [`InterruptSource`]
/// reports it.
///
/// ```
/// use everruns::ag_ui::ResumeOutcome;
///
/// let outcome = ResumeOutcome::StillOpen(Vec::new());
/// assert!(matches!(outcome, ResumeOutcome::StillOpen(_)));
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum ResumeOutcome {
    /// The session waits on nothing: the run finishes empty.
    NothingOpen,
    /// Some open interrupt had no entry, so nothing was resolved: the run
    /// ends at once with these interrupts again.
    StillOpen(Vec<Interrupt>),
    /// Every open interrupt was answered: the run streams the rest of the
    /// parked turn.
    Resumed,
}

/// Where an AG-UI run reads its session's open interrupts and applies resume
/// entries.
///
/// [`InterruptGate`] is the built-in source. A host that already parks
/// `ask_user` questions and tool approvals for an API of its own (`serve`
/// does, for its `/question-answers` and `/approvals` routes) implements this
/// trait instead, so the same requests surface as AG-UI interrupts without a
/// second responder. Build interrupts with [`question_interrupt`] and
/// [`approval_interrupt`], and read answers with [`question_outcome`] and
/// [`approval_decision`], so the shapes match the built-in gate.
///
/// Implementations follow the AG-UI 1.0 producer rules: an entry naming no
/// open interrupt is ignored; an open interrupt without an entry is not
/// abandoned, so nothing is resolved and [`ResumeOutcome::StillOpen`] is
/// returned; every entry is validated before any is applied.
///
/// ```
/// use everruns::SessionId;
/// use everruns::ag_ui::{
///     AgUiError, AgUiOptions, Interrupt, InterruptSource, ResumeEntry, ResumeOutcome,
/// };
/// use tokio::sync::broadcast;
///
/// /// A source with nothing ever parked.
/// struct Nothing(broadcast::Sender<SessionId>);
///
/// impl InterruptSource for Nothing {
///     fn interrupts(&self, _session_id: SessionId) -> Vec<Interrupt> {
///         Vec::new()
///     }
///     fn subscribe(&self) -> broadcast::Receiver<SessionId> {
///         self.0.subscribe()
///     }
///     fn resume(
///         &self,
///         _session_id: SessionId,
///         _entries: &[ResumeEntry],
///     ) -> Result<ResumeOutcome, AgUiError> {
///         Ok(ResumeOutcome::NothingOpen)
///     }
/// }
///
/// let options = AgUiOptions::new().interrupts(Nothing(broadcast::channel(8).0));
/// # let _ = options;
/// ```
pub trait InterruptSource: Send + Sync + 'static {
    /// The interrupts open on `session_id`, oldest first. Empty when the
    /// session waits on nothing.
    fn interrupts(&self, session_id: SessionId) -> Vec<Interrupt>;

    /// A feed that names a session each time a request parks on it. A run
    /// re-reads [`interrupts`](Self::interrupts) on each notice for its
    /// session (and when the feed lags), then ends with them.
    fn subscribe(&self) -> broadcast::Receiver<SessionId>;

    /// Apply a run's resume entries to `session_id`'s open interrupts.
    ///
    /// # Errors
    ///
    /// [`AgUiError::InvalidInput`] when an entry cannot be applied; nothing
    /// is resolved then.
    fn resume(
        &self,
        session_id: SessionId,
        entries: &[ResumeEntry],
    ) -> Result<ResumeOutcome, AgUiError>;
}

impl InterruptSource for InterruptGate {
    fn interrupts(&self, session_id: SessionId) -> Vec<Interrupt> {
        InterruptGate::interrupts(self, session_id)
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionId> {
        self.inner.parked_on.subscribe()
    }

    fn resume(
        &self,
        session_id: SessionId,
        entries: &[ResumeEntry],
    ) -> Result<ResumeOutcome, AgUiError> {
        self.resume_entries(session_id, entries)
    }
}

/// Removes a parked request when its waiting turn goes away.
struct ParkGuard<'a> {
    gate: &'a InterruptGate,
    key: Key,
}

impl Drop for ParkGuard<'_> {
    fn drop(&mut self) {
        lock(&self.gate.inner.parked).remove(&self.key);
    }
}

#[async_trait]
impl AskUser for InterruptGate {
    async fn ask(&self, _questions: &[Question]) -> Outcome {
        // The runtime asks through `ask_in`; without a session nobody can
        // resume the question.
        Outcome {
            status: Status::Cancelled,
            answered_by: AnsweredBy::Unattended,
            answers: Vec::new(),
        }
    }

    async fn ask_in(&self, context: &AskContext, questions: &[Question]) -> Outcome {
        let (tx, rx) = oneshot::channel();
        let _guard = self.park(
            (context.session_id(), context.tool_call_id().to_string()),
            Request::Question {
                questions: questions.to_vec(),
                tx,
            },
        );
        rx.await.unwrap_or(Outcome {
            status: Status::Cancelled,
            answered_by: AnsweredBy::User,
            answers: Vec::new(),
        })
    }
}

#[async_trait]
impl ToolApprover for InterruptGate {
    async fn approve(
        &self,
        session_id: SessionId,
        call: &ToolCall,
        _definition: &ToolDefinition,
    ) -> ApprovalDecision {
        let (tx, rx) = oneshot::channel();
        let _guard = self.park(
            (session_id, call.id.clone()),
            Request::Approval {
                call: call.clone(),
                tx,
            },
        );
        rx.await.unwrap_or(ApprovalDecision::Cancelled)
    }
}

// --- The run ------------------------------------------------------------------

/// The events of one AG-UI run: `RUN_STARTED`, the projected run, and
/// exactly one terminal `RUN_FINISHED` or `RUN_ERROR`, after which it ends.
///
/// It implements [`futures::Stream`]; [`recv`](Self::recv) reads it without
/// that trait. Dropping it stops reading, not the turn.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::ag_ui::{Event, Message, RunAgentInput};
/// use everruns::{Agent, Engine, Model};
///
/// let agent = Agent::builder()
///     .instructions("Be brief.")
///     .model(Model::simulated("Hello."))
///     .build()?;
/// let session = Engine::new().create(agent);
/// let mut run = session
///     .ag_ui(RunAgentInput {
///         thread_id: "t".into(),
///         run_id: "r".into(),
///         messages: vec![Message::user("m", "Hi")],
///         ..RunAgentInput::default()
///     })
///     .await?;
/// let mut text = String::new();
/// while let Some(event) = run.recv().await {
///     if let Event::TextMessageContent(content) = event {
///         text.push_str(&content.delta);
///     }
/// }
/// assert_eq!(text, "Hello.");
/// # Ok(())
/// # }
/// ```
pub struct AgUiStream {
    inner: Pin<Box<dyn Stream<Item = Event> + Send>>,
    sent: Option<SentMessage>,
}

impl std::fmt::Debug for AgUiStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgUiStream").finish_non_exhaustive()
    }
}

impl AgUiStream {
    /// The next event, or `None` after the terminal one.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::ag_ui::{Event, Message, RunAgentInput};
    /// use everruns::{Agent, Engine, Model};
    ///
    /// let agent = Agent::builder().instructions("Hi.").model(Model::simulated("ok")).build()?;
    /// let session = Engine::new().create(agent);
    /// let input = RunAgentInput {
    ///     messages: vec![Message::user("m", "Hi")],
    ///     ..RunAgentInput::default()
    /// };
    /// let mut run = session.ag_ui(input).await?;
    /// assert!(matches!(run.recv().await, Some(Event::RunStarted(_))));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn recv(&mut self) -> Option<Event> {
        self.next().await
    }

    /// The user message this run sent, when it started or steered a turn.
    /// `None` for a run that resumed a parked turn, re-asked open
    /// interrupts, or finished empty.
    ///
    /// A host that tracks its sessions' turns (to cancel one, or to report
    /// a session as active) reads the turn from here, since the run sent
    /// the message itself.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::ag_ui::{Message, RunAgentInput};
    /// use everruns::{Agent, Engine, Model, SendDisposition};
    ///
    /// let agent = Agent::builder().instructions("Hi.").model(Model::simulated("ok")).build()?;
    /// let session = Engine::new().create(agent);
    /// let input = RunAgentInput {
    ///     messages: vec![Message::user("m", "Hi")],
    ///     ..RunAgentInput::default()
    /// };
    /// let run = session.ag_ui(input).await?;
    /// let sent = run.sent().expect("a user message starts a turn");
    /// assert_eq!(sent.disposition, SendDisposition::Started);
    /// assert!(sent.wait().await?.success);
    /// # Ok(())
    /// # }
    /// ```
    pub fn sent(&self) -> Option<&SentMessage> {
        self.sent.as_ref()
    }
}

impl Stream for AgUiStream {
    type Item = Event;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Event>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// How the run begins, decided before the stream opens.
enum Start {
    /// A message started or steered a turn, or results resumed one; stream
    /// it.
    Follow,
    /// The run ends at once with these frontend calls and interrupts.
    Park(Vec<WireToolCall>, Vec<Interrupt>),
    /// Nothing to run: finish empty.
    Empty,
}

struct RunState {
    session: Session,
    events: EventStream,
    gate: Option<Arc<dyn InterruptSource>>,
    parked: Option<broadcast::Receiver<SessionId>>,
    projector: Projector,
    /// Highest durable sequence seen, for recovering from lag.
    last_sequence: Option<i32>,
    /// This run's frontend tool names.
    frontend: HashSet<String>,
    /// The session's parked client-side calls, updated as each turn ends.
    parked_calls: watch::Receiver<Option<ParkedToolCalls>>,
    /// Frontend calls the turn requested, waiting for it to park on them.
    awaiting: Option<Vec<WireToolCall>>,
}

impl Session {
    /// Answer one AG-UI request with the trusted projection and no interrupt
    /// gate. Equivalent to [`ag_ui_with`](Self::ag_ui_with) with
    /// [`AgUiOptions::new`].
    ///
    /// Requires the `ag-ui` feature. Stability: alpha.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::ag_ui::{Event, Message, RunAgentInput};
    /// use everruns::{Agent, Engine, Model};
    /// use futures::StreamExt;
    ///
    /// let agent = Agent::builder()
    ///     .instructions("Be brief.")
    ///     .model(Model::simulated("4"))
    ///     .build()?;
    /// let session = Engine::new().create(agent);
    /// let input = RunAgentInput {
    ///     thread_id: "thread-1".into(),
    ///     run_id: "run-1".into(),
    ///     protocol_version: Some("1.0".into()),
    ///     messages: vec![Message::user("m1", "What is 2 + 2?")],
    ///     ..RunAgentInput::default()
    /// };
    /// let events: Vec<Event> = session.ag_ui(input).await?.collect().await;
    /// let Some(Event::RunStarted(started)) = events.first() else { panic!() };
    /// assert_eq!(started.protocol_version.as_deref(), Some("1.0"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// See [`ag_ui_with`](Self::ag_ui_with).
    pub async fn ag_ui(&self, input: RunAgentInput) -> Result<AgUiStream, AgUiError> {
        self.ag_ui_with(input, AgUiOptions::new()).await
    }

    /// Answer one AG-UI request: start or resume a turn and stream it as an
    /// AG-UI run.
    ///
    /// Without resume entries, the input's last message must be a user
    /// message (system and developer messages after it are skipped when
    /// [`AgUiOptions::input_instructions`] is on); its text starts a turn,
    /// or steers the running one. With
    /// resume entries and a [gate](AgUiOptions::gate), the entries answer the
    /// session's open interrupts and the run streams the rest of the parked
    /// turn. A run that would start while interrupts are open, or a resume
    /// that leaves one unanswered, ends at once with the interrupts again.
    /// The input's `tools` become the session's frontend tools, and trailing
    /// `tool` messages answer the calls to them the turn parked on (see
    /// [Frontend tools](crate::ag_ui#frontend-tools)).
    ///
    /// `RUN_STARTED` carries `protocolVersion: "1.0"` only when the input
    /// declared a version, so a pre-1.0 client sees the stream it expects.
    ///
    /// Requires the `ag-ui` feature. Stability: alpha.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::ag_ui::{AgUiOptions, Event, InterruptGate, Message, RunAgentInput};
    /// use everruns::{Agent, Engine, Model};
    /// use futures::StreamExt;
    ///
    /// let gate = InterruptGate::new();
    /// let agent = Agent::builder()
    ///     .instructions("Be brief.")
    ///     .model(Model::simulated("ok"))
    ///     .ask_user(gate.clone())
    ///     .build()?;
    /// let session = Engine::new().create(agent);
    /// let input = RunAgentInput {
    ///     messages: vec![Message::user("m1", "Hi")],
    ///     ..RunAgentInput::default()
    /// };
    /// let events: Vec<Event> = session
    ///     .ag_ui_with(input, AgUiOptions::new().gate(gate))
    ///     .await?
    ///     .collect()
    ///     .await;
    /// assert!(matches!(events.last(), Some(Event::RunFinished(_))));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`AgUiError::InvalidInput`] when the input has neither a trailing user
    /// message nor resume entries nor trailing tool results, a frontend tool
    /// definition or result is out of bounds, or an entry cannot be applied
    /// (nothing is resolved then); [`AgUiError::Run`] when the session
    /// refuses the message.
    pub async fn ag_ui_with(
        &self,
        input: RunAgentInput,
        options: AgUiOptions,
    ) -> Result<AgUiStream, AgUiError> {
        let AgUiOptions {
            policy,
            interrupts: gate,
            input_instructions: trusted,
            seed_history,
        } = options;
        let session_id = self.session_id();
        let frontend_tools = frontend_definitions(&input.tools)?;
        let frontend: HashSet<String> = input.tools.iter().map(|tool| tool.name.clone()).collect();
        let results = trailing_results(&input.messages);
        // A resuming run (AG-UI 1.0) answers the interrupts or frontend tool
        // calls that ended the last one and starts no new turn, so it needs
        // no trailing user message.
        let resuming = !input.resume.is_empty() || !results.is_empty();
        // A trusted run's system and developer messages are instructions,
        // not the conversation: the input is the last message of any other
        // role.
        let trigger = if !resuming {
            let last = input.messages.iter().rposition(|message| {
                !(trusted && matches!(message, Message::System(_) | Message::Developer(_)))
            });
            Some(match last.map(|index| (index, &input.messages[index])) {
                Some((index, Message::User(message))) => (index, message.content.to_text()),
                Some(_) => return Err(invalid("the final AG-UI message must have role=user")),
                None => {
                    return Err(invalid(
                        "messages must contain at least one user message, or resume entries",
                    ));
                }
            })
        } else {
            None
        };
        // The consumer sends its frontend tools on every run, so the session
        // takes the latest set; a session that never had any is left alone.
        let client_tools =
            (!frontend_tools.is_empty() || self.had_client_tools()).then_some(frontend_tools);
        if trusted || client_tools.is_some() {
            self.override_record(SessionOverrides {
                instructions: trusted.then(|| input_instructions(&input)),
                client_tools,
            })
            .await?;
        }
        // Seed before subscribing, so the seeded history is not streamed as
        // this run's output, and before the user message is sent.
        if seed_history && let Some((index, _)) = &trigger {
            let earlier = seed::seed_messages(&input.messages[..*index]);
            if !earlier.is_empty() && !self.seed_history(earlier).await? {
                tracing::debug!(
                    session_id = %session_id,
                    "AG-UI session already has history; not seeding"
                );
            }
        }
        // Subscribe before anything can happen, so no event of this run and
        // no park is missed.
        let events = self.events();
        let parked = gate.as_ref().map(|gate| gate.subscribe());
        let parked_calls = self.watch_parked_tool_calls();
        let open_interrupts = || {
            gate.as_ref()
                .map(|gate| gate.interrupts(session_id))
                .unwrap_or_default()
        };

        let mut sent = None;
        let start = if let Some((_, text)) = trigger {
            let open = open_interrupts();
            let pending = self.pending_frontend_calls(&frontend);
            if open.is_empty() && pending.is_empty() {
                if text.trim().is_empty() {
                    return Err(invalid("the user message has no text"));
                }
                sent = Some(self.send(text.as_str()).await?);
                Start::Follow
            } else {
                // AG-UI 1.0: an interrupt without an answer is not abandoned;
                // ask again instead of running past it. A parked frontend
                // call without a result is reported again the same way.
                Start::Park(pending, open)
            }
        } else if input.resume.is_empty() {
            self.submit_frontend_results(&frontend, results, open_interrupts)
                .await?
        } else {
            if !results.is_empty() {
                // A run carrying resume entries resolves only those.
                tracing::warn!(
                    session_id = %session_id,
                    "AG-UI tool messages beside resume entries; ignoring them"
                );
            }
            match &gate {
                Some(gate) => match gate.resume(session_id, &input.resume)? {
                    ResumeOutcome::NothingOpen => Start::Empty,
                    ResumeOutcome::StillOpen(interrupts) => Start::Park(Vec::new(), interrupts),
                    ResumeOutcome::Resumed => Start::Follow,
                },
                None => {
                    tracing::warn!(
                        session_id = %session_id,
                        "AG-UI resume entries without an interrupt gate; ignoring them"
                    );
                    Start::Empty
                }
            }
        };

        let mut started = RunStartedEvent::new(input.thread_id.clone(), input.run_id.clone());
        if input.protocol_version.is_some() {
            started = started.with_protocol_version();
        }
        let mut projector = Projector::new(input.thread_id, input.run_id, policy);
        match start {
            Start::Follow => {}
            Start::Park(calls, interrupts) => projector.park(calls, interrupts),
            Start::Empty => projector.project("turn.completed", &Value::Null),
        }
        let state = RunState {
            session: self.clone(),
            events,
            gate,
            parked,
            projector,
            last_sequence: None,
            frontend,
            parked_calls,
            awaiting: None,
        };
        let run = futures::stream::unfold(state, next_event);
        let stream = futures::stream::once(async move { Event::RunStarted(started) }).chain(run);
        Ok(AgUiStream {
            inner: Box::pin(stream),
            sent,
        })
    }
}

async fn next_event(mut state: RunState) -> Option<(Event, RunState)> {
    loop {
        if let Some(event) = state.projector.pop() {
            return Some((event, state));
        }
        if state.projector.is_finished() {
            return None;
        }
        let session_id = state.session.session_id();
        tokio::select! {
            next = state.events.recv() => match next {
                Ok(Some(event)) => state.project(&event),
                Ok(None) => state.projector.fail(RunErrorEvent::new(
                    "the session closed before the run finished",
                )),
                Err(EventStreamError::Lagged { .. }) => state.recover().await,
            },
            () = parked_on(&mut state.parked, session_id) => state.interrupt_if_parked(),
            () = parked_with(&mut state.parked_calls, state.awaiting.as_deref()) => {
                state.park_frontend_calls();
            }
        }
    }
}

/// Resolves once the session has parked on every call in `awaiting`, which
/// it records as the turn ends. Pending while nothing is awaited.
async fn parked_with(
    parked: &mut watch::Receiver<Option<ParkedToolCalls>>,
    awaiting: Option<&[WireToolCall]>,
) {
    let Some(calls) = awaiting else {
        return std::future::pending().await;
    };
    let covered = parked.wait_for(|parked| {
        parked.as_ref().is_some_and(|parked| {
            calls
                .iter()
                .all(|call| parked.tool_calls.iter().any(|made| made.id == call.id))
        })
    });
    if covered.await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Resolves when a request parks on `session_id`, or the gate's notices
/// lagged (the open interrupts are re-read either way).
async fn parked_on(parked: &mut Option<broadcast::Receiver<SessionId>>, session_id: SessionId) {
    let Some(rx) = parked else {
        return std::future::pending().await;
    };
    loop {
        match rx.recv().await {
            Ok(id) if id == session_id => return,
            Ok(_) => continue,
            Err(broadcast::error::RecvError::Lagged(_)) => return,
            Err(broadcast::error::RecvError::Closed) => return std::future::pending().await,
        }
    }
}

impl RunState {
    fn project(&mut self, event: &crate::SessionEvent) {
        if let Some(sequence) = event.sequence() {
            self.last_sequence = Some(self.last_sequence.map_or(sequence, |s| s.max(sequence)));
        }
        let data = event
            .canonical_json()
            .get("data")
            .cloned()
            .unwrap_or(Value::Null);
        // Calls to this run's frontend tools are the consumer's to run: the
        // turn parks on them and the run ends naming them, once it has.
        if event.event_type() == "tool.call_requested" {
            let calls: Vec<WireToolCall> = serde_json::from_value::<ToolCallRequestedData>(data)
                .map(|requested| {
                    requested
                        .tool_calls
                        .into_iter()
                        .filter(|call| self.frontend.contains(&call.name))
                        .map(|call| {
                            WireToolCall::function(call.id, call.name, call.arguments.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !calls.is_empty() {
                self.awaiting = Some(calls);
            }
            return;
        }
        self.projector.project(event.event_type(), &data);
    }

    /// The turn parked on the frontend calls it requested: flush what it
    /// emitted, then end the run with them, beside any open interrupt.
    fn park_frontend_calls(&mut self) {
        let Some(calls) = self.awaiting.take() else {
            return;
        };
        while let Ok(Some(event)) = self.events.try_recv() {
            self.project(&event);
            if self.projector.is_finished() {
                return;
            }
        }
        let interrupts = self
            .gate
            .as_ref()
            .map(|gate| gate.interrupts(self.session.session_id()))
            .unwrap_or_default();
        self.projector.park(calls, interrupts);
    }

    /// The run fell behind the live feed: replay the durable log after the
    /// last durable event seen. Ephemeral deltas in the gap are lost, which
    /// only shortens streamed text; completions are durable.
    async fn recover(&mut self) {
        let Some(after) = self.last_sequence else {
            self.projector.fail(RunErrorEvent::new(
                "the run fell behind the session's event stream",
            ));
            return;
        };
        match self.session.events_from(after).await {
            Ok(events) => self.events = events,
            Err(error) => self.projector.fail(RunErrorEvent::new(format!(
                "the run fell behind the session's event stream: {error}"
            ))),
        }
    }

    /// A request parked: flush what the turn already emitted, then end the
    /// run with every open interrupt.
    fn interrupt_if_parked(&mut self) {
        let Some(gate) = &self.gate else { return };
        let interrupts = gate.interrupts(self.session.session_id());
        if interrupts.is_empty() {
            return;
        }
        while let Ok(Some(event)) = self.events.try_recv() {
            self.project(&event);
            if self.projector.is_finished() {
                return;
            }
        }
        self.projector.interrupt(interrupts);
    }
}
