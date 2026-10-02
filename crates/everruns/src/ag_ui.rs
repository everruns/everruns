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
//! last message, which must be a user message's text. Earlier messages,
//! `state`, `context`, `forwardedProps` and frontend `tools` are not read.
//! Map `threadId` to a session yourself (one session per thread); the run
//! does not check it.

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
// Decision: no axum handler here. The facade carries no HTTP server
// dependency, and the handler is five lines over `AgUiStream` (see the public
// docs page `framework/ag-ui`).

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};

use crate::approval::{ApprovalDecision, ToolApprover};
use crate::ask_user::{
    Answer, AnsweredBy, AskContext, AskUser, Outcome, Question, QuestionKind, Status,
};
use crate::{
    EventStream, EventStreamError, RunError, SentMessage, Session, SessionId, ToolCall,
    ToolDefinition,
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
}

impl std::fmt::Display for AgUiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(why) => write!(f, "invalid AG-UI input: {why}"),
            Self::Run(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AgUiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidInput(_) => None,
            Self::Run(error) => Some(error),
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
}

impl std::fmt::Debug for AgUiOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgUiOptions")
            .field("policy", &self.policy)
            .field("interrupts", &self.interrupts.is_some())
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

// --- Interrupt and answer shapes ------------------------------------------------

/// The interrupt an `ask_user` question set becomes.
///
/// Its id is the tool call id. A set with a `secret` question gets
/// [`SECRET_REASON`] and no schema; any other gets [`ASK_USER_REASON`], the
/// questions as prose in `message`, the questions themselves under
/// `metadata.everruns.questions`, and a `responseSchema` for the answer.
///
/// ```
/// use everruns::ag_ui::{ASK_USER_REASON, question_interrupt};
/// use everruns::ask_user::Question;
///
/// let questions: Vec<Question> = serde_json::from_value(serde_json::json!([{
///     "id": "target",
///     "header": "Target",
///     "question": "Where should I deploy?",
///     "options": [
///         { "label": "Staging", "description": "Safe" },
///         { "label": "Production", "description": "Live" },
///     ],
/// }]))?;
/// let interrupt = question_interrupt("call_1", &questions);
/// assert_eq!(interrupt.id, "call_1");
/// assert_eq!(interrupt.reason, ASK_USER_REASON);
/// assert!(interrupt.response_schema.is_some());
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn question_interrupt(tool_call_id: &str, questions: &[Question]) -> Interrupt {
    let secret = questions
        .iter()
        .any(|question| question.kind == QuestionKind::Secret);
    let message = questions
        .iter()
        .map(|question| question.question.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut interrupt = Interrupt {
        message: Some(message),
        ..Interrupt::new(
            tool_call_id,
            if secret {
                SECRET_REASON
            } else {
                ASK_USER_REASON
            },
        )
    };
    if !secret {
        interrupt.response_schema = as_object(answer_schema(tool_call_id, questions));
        interrupt.metadata = Some(everruns_metadata(json!({ "questions": questions })));
    }
    interrupt
}

/// The interrupt a tool call waiting on approval becomes.
///
/// Its id and `toolCallId` are the call's id; `metadata.everruns` carries the
/// tool name and arguments so the client can show what it approves.
///
/// ```
/// use everruns::ToolCall;
/// use everruns::ag_ui::{TOOL_APPROVAL_REASON, approval_interrupt};
///
/// let call = ToolCall {
///     id: "call_1".into(),
///     name: "deploy".into(),
///     arguments: serde_json::json!({ "env": "production" }),
/// };
/// let interrupt = approval_interrupt(&call);
/// assert_eq!(interrupt.reason, TOOL_APPROVAL_REASON);
/// assert_eq!(interrupt.tool_call_id.as_deref(), Some("call_1"));
/// ```
pub fn approval_interrupt(call: &ToolCall) -> Interrupt {
    Interrupt {
        message: Some(format!("Allow the agent to run {}?", call.name)),
        tool_call_id: Some(call.id.clone()),
        response_schema: as_object(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["decision"],
            "properties": {
                "decision": {
                    "enum": ["allow", "allow_always", "reject", "reject_always"],
                    "description": "`*_always` applies to every later call of this tool in the session.",
                },
            },
        })),
        metadata: Some(everruns_metadata(json!({
            "tool": call.name,
            "arguments": call.arguments,
        }))),
        ..Interrupt::new(call.id.clone(), TOOL_APPROVAL_REASON)
    }
}

/// The question-answers body an `ask_user` resume entry carries.
#[derive(Deserialize)]
struct QuestionAnswers {
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    answers: Vec<SubmittedAnswer>,
}

#[derive(Deserialize)]
struct SubmittedAnswer {
    id: String,
    #[serde(default)]
    selected: Vec<String>,
    #[serde(default)]
    other_text: Option<String>,
}

/// The [`Outcome`] a resume entry answers an `ask_user` interrupt with.
///
/// An abandoned entry declines the set. A resolved entry's payload is the
/// question-answers body the interrupt's `responseSchema` describes, checked
/// against the questions asked: every question answered once, only offered
/// options, one selection on a single-select question. A credential is never
/// accepted.
///
/// ```
/// use everruns::ag_ui::{ResumeEntry, ResumeStatus, question_outcome};
/// use everruns::ask_user::{Question, Status};
///
/// let questions: Vec<Question> = serde_json::from_value(serde_json::json!([{
///     "id": "target",
///     "header": "Target",
///     "question": "Where should I deploy?",
///     "options": [
///         { "label": "Staging", "description": "Safe" },
///         { "label": "Production", "description": "Live" },
///     ],
/// }]))?;
/// let entry = ResumeEntry {
///     interrupt_id: "call_1".into(),
///     status: ResumeStatus::Resolved,
///     payload: Some(serde_json::json!({
///         "answers": [{ "id": "target", "selected": ["Staging"] }],
///     })),
///     metadata: None,
/// };
/// let outcome = question_outcome(&entry, "call_1", &questions)?;
/// assert_eq!(outcome.status, Status::Answered);
/// assert_eq!(outcome.answers[0].selected, ["Staging"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// [`AgUiError::InvalidInput`] when the payload is missing, malformed, names
/// another question set, or does not answer the questions asked.
pub fn question_outcome(
    entry: &ResumeEntry,
    tool_call_id: &str,
    questions: &[Question],
) -> Result<Outcome, AgUiError> {
    let declined = Outcome {
        status: Status::Declined,
        answered_by: AnsweredBy::User,
        answers: Vec::new(),
    };
    if entry.status == ResumeStatus::Cancelled {
        return Ok(declined);
    }
    if questions
        .iter()
        .any(|question| question.kind == QuestionKind::Secret)
    {
        return Err(invalid(
            "a credential cannot be sent over AG-UI; abandon the interrupt instead",
        ));
    }
    let payload = entry
        .payload
        .clone()
        .ok_or_else(|| invalid("a resolved entry needs a payload"))?;
    let body: QuestionAnswers = serde_json::from_value(payload)
        .map_err(|error| invalid(format!("invalid ask_user answer: {error}")))?;
    if body
        .tool_call_id
        .as_deref()
        .is_some_and(|id| id != tool_call_id)
    {
        return Err(invalid("the answer names a different question set"));
    }
    match body.status.as_deref() {
        None | Some("answered") => {}
        Some("declined") => return Ok(declined),
        Some(other) => return Err(invalid(format!("unknown answer status {other:?}"))),
    }
    let answers: Vec<Answer> = body
        .answers
        .into_iter()
        .map(|answer| Answer {
            id: answer.id,
            selected: answer.selected,
            other_text: answer.other_text,
            secret_ref: None,
        })
        .collect();
    validate_answers(questions, &answers).map_err(AgUiError::InvalidInput)?;
    Ok(Outcome {
        status: Status::Answered,
        answered_by: AnsweredBy::User,
        answers,
    })
}

/// The decision a resume entry answers a tool-approval interrupt with.
///
/// An abandoned entry rejects the call; a resolved one reads
/// `payload.decision`.
///
/// ```
/// use everruns::ag_ui::{ResumeEntry, ResumeStatus, approval_decision};
/// use everruns::approval::ApprovalDecision;
///
/// let entry = ResumeEntry {
///     interrupt_id: "call_1".into(),
///     status: ResumeStatus::Resolved,
///     payload: Some(serde_json::json!({ "decision": "allow" })),
///     metadata: None,
/// };
/// assert_eq!(approval_decision(&entry)?, ApprovalDecision::Allow);
/// # Ok::<(), everruns::ag_ui::AgUiError>(())
/// ```
///
/// # Errors
///
/// [`AgUiError::InvalidInput`] when a resolved entry has no known decision.
pub fn approval_decision(entry: &ResumeEntry) -> Result<ApprovalDecision, AgUiError> {
    if entry.status == ResumeStatus::Cancelled {
        return Ok(ApprovalDecision::Reject);
    }
    let decision = entry
        .payload
        .as_ref()
        .and_then(|payload| payload.get("decision"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("a tool approval needs a decision"))?;
    match decision {
        "allow" => Ok(ApprovalDecision::Allow),
        "allow_always" => Ok(ApprovalDecision::AllowAlways),
        "reject" => Ok(ApprovalDecision::Reject),
        "reject_always" => Ok(ApprovalDecision::RejectAlways),
        other => Err(invalid(format!("unknown decision {other:?}"))),
    }
}

/// The JSON Schema of an `ask_user` answer: the same shape the Everruns
/// server advertises for its question-answers request.
fn answer_schema(tool_call_id: &str, questions: &[Question]) -> Value {
    let answers: Vec<Value> = questions
        .iter()
        .map(|question| {
            let mut properties = serde_json::Map::new();
            properties.insert(
                "id".to_string(),
                json!({ "const": question.id.clone().unwrap_or_default() }),
            );
            if question.kind == QuestionKind::Text {
                properties.insert(
                    "other_text".to_string(),
                    json!({ "type": "string", "description": "The free-form answer." }),
                );
                return json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "other_text"],
                    "properties": properties,
                    "description": question.question,
                });
            }
            let labels: Vec<&str> = question
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect();
            let mut selected = json!({ "type": "array", "items": { "enum": labels } });
            if !question.multi_select
                && let Some(object) = selected.as_object_mut()
            {
                object.insert("maxItems".to_string(), json!(1));
            }
            properties.insert("selected".to_string(), selected);
            if question.allow_other {
                properties.insert(
                    "other_text".to_string(),
                    json!({
                        "type": ["string", "null"],
                        "description": "Free text, when none of the options fit.",
                    }),
                );
            }
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["id"],
                "properties": properties,
                "description": question.question,
            })
        })
        .collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["answers"],
        "properties": {
            "tool_call_id": { "const": tool_call_id },
            "status": {
                "enum": ["answered", "declined"],
                "default": "answered",
                "description": "`declined` is a finished decision the agent must not re-ask.",
            },
            "answers": {
                "type": "array",
                "minItems": answers.len(),
                "maxItems": answers.len(),
                "description": "One answer per asked question, in any order.",
                "items": { "oneOf": answers },
            },
        },
    })
}

/// Every question answered once, with offered options only, as the Everruns
/// server checks a question-answers request.
fn validate_answers(questions: &[Question], answers: &[Answer]) -> Result<(), String> {
    for answer in answers {
        if !questions
            .iter()
            .any(|question| question.id.as_deref() == Some(answer.id.as_str()))
        {
            return Err(format!("no question with id {:?} was asked", answer.id));
        }
    }
    for question in questions {
        let id = question.id.as_deref().unwrap_or_default();
        let mut matching = answers.iter().filter(|answer| answer.id == id);
        let answer = matching
            .next()
            .ok_or_else(|| format!("question {id:?} was not answered"))?;
        if matching.next().is_some() {
            return Err(format!("question {id:?} was answered more than once"));
        }
        for label in &answer.selected {
            if !question.options.iter().any(|option| &option.label == label) {
                return Err(format!(
                    "question {id:?} was not asked with option {label:?}"
                ));
            }
        }
        let other = answer
            .other_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if other.is_some() && question.kind != QuestionKind::Text && !question.allow_other {
            return Err(format!("question {id:?} does not allow free text"));
        }
        if answer.selected.is_empty() && other.is_none() {
            return Err(format!("question {id:?} has no selection and no free text"));
        }
        if !question.multi_select && answer.selected.len() > 1 {
            return Err(format!("question {id:?} is single-select"));
        }
    }
    Ok(())
}

fn as_object(value: Value) -> Option<serde_json::Map<String, Value>> {
    match value {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Our keys go under `everruns`; `ag-ui` is reserved for the protocol.
fn everruns_metadata(value: Value) -> everruns_ag_ui::Metadata {
    let mut metadata = everruns_ag_ui::Metadata::new();
    metadata.insert("everruns".to_string(), value);
    metadata
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
    /// A message started or steered a turn; stream it.
    Follow,
    /// The run ends at once with these interrupts.
    Interrupt(Vec<Interrupt>),
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
    /// message; its text starts a turn, or steers the running one. With
    /// resume entries and a [gate](AgUiOptions::gate), the entries answer the
    /// session's open interrupts and the run streams the rest of the parked
    /// turn. A run that would start while interrupts are open, or a resume
    /// that leaves one unanswered, ends at once with the interrupts again.
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
    /// message nor resume entries, or an entry cannot be applied (nothing is
    /// resolved then); [`AgUiError::Run`] when the session refuses the
    /// message.
    pub async fn ag_ui_with(
        &self,
        input: RunAgentInput,
        options: AgUiOptions,
    ) -> Result<AgUiStream, AgUiError> {
        let AgUiOptions {
            policy,
            interrupts: gate,
        } = options;
        let session_id = self.session_id();
        // Subscribe before anything can happen, so no event of this run and
        // no park is missed.
        let events = self.events();
        let parked = gate.as_ref().map(|gate| gate.subscribe());

        let mut sent = None;
        let start = if !input.resume.is_empty() {
            match &gate {
                Some(gate) => match gate.resume(session_id, &input.resume)? {
                    ResumeOutcome::NothingOpen => Start::Empty,
                    ResumeOutcome::StillOpen(interrupts) => Start::Interrupt(interrupts),
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
        } else {
            let text = match input.messages.last() {
                Some(Message::User(message)) => message.content.to_text(),
                Some(_) => return Err(invalid("the final AG-UI message must have role=user")),
                None => {
                    return Err(invalid(
                        "messages must contain at least one user message, or resume entries",
                    ));
                }
            };
            let open = gate
                .as_ref()
                .map(|gate| gate.interrupts(session_id))
                .unwrap_or_default();
            if open.is_empty() {
                if text.trim().is_empty() {
                    return Err(invalid("the user message has no text"));
                }
                sent = Some(self.send(text.as_str()).await?);
                Start::Follow
            } else {
                // AG-UI 1.0: an interrupt without an answer is not abandoned;
                // ask again instead of running past it.
                Start::Interrupt(open)
            }
        };

        let mut started = RunStartedEvent::new(input.thread_id.clone(), input.run_id.clone());
        if input.protocol_version.is_some() {
            started = started.with_protocol_version();
        }
        let mut projector = Projector::new(input.thread_id, input.run_id, policy);
        match start {
            Start::Follow => {}
            Start::Interrupt(interrupts) => projector.interrupt(interrupts),
            Start::Empty => projector.project("turn.completed", &Value::Null),
        }
        let state = RunState {
            session: self.clone(),
            events,
            gate,
            parked,
            projector,
            last_sequence: None,
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
        }
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
        self.projector.project(event.event_type(), &data);
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
