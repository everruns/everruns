use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::{
    FinalizedToolCallRejection, FinalizedToolCallsContext, FinalizedToolCallsHook,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::capabilities::{Capability, CapabilityLocalization};
use crate::tool_types::{
    ClientSideTool, DeferrablePolicy, HUMAN_INTENT_ARGUMENT, ToolCall, ToolDefinition, ToolHints,
};
use crate::tools::{Tool, ToolExecutionResult};
use crate::typed_id::SessionId;
use everruns_core::tool_context::ToolContext;

pub const ASK_USER_CAPABILITY_ID: &str = "ask_user";
// Defined in `everruns-provider` so the engine can recognise the call without
// depending on this crate; re-exported here so capability authors keep one path.
pub use crate::tool_types::ASK_USER_TOOL_NAME;
pub const DEFAULT_ASK_USER_TIMEOUT_SECONDS: u64 = 300;
pub const MAX_ASK_USER_QUESTIONS: usize = 4;
pub const MAX_ASK_USER_OPTIONS: usize = 6;
pub const MAX_ASK_USER_HEADER_CHARS: usize = 16;
pub const MAX_ASK_USER_SECRET_NAME_CHARS: usize = 255;
/// Scheme of the handle a secret answer returns. The value itself never leaves
/// the encrypted session-secret store, so the model is handed a name to resolve
/// rather than a credential to carry (EVE-1058).
pub const SESSION_SECRET_REF_PREFIX: &str = "session:";

/// The handle a resolved secret question returns in place of the value.
pub fn session_secret_ref(name: &str) -> String {
    format!("{SESSION_SECRET_REF_PREFIX}{name}")
}

/// How long before `expires_at` the card starts counting down, at the default
/// timeout. Shorter windows scale this down rather than nudging before the
/// question was even asked; see `deadlines_for`.
pub const ASK_USER_NUDGE_LEAD_SECONDS: u64 = 60;

fn default_timeout_seconds() -> u64 {
    DEFAULT_ASK_USER_TIMEOUT_SECONDS
}

/// The three deadlines the server stamps on a normalized `ask_user` call.
///
/// Emitted server-side (EVE-1056) rather than derived by each surface: the
/// sweep resolves the call at `expires_at`, so a client that guessed its own
/// deadline would render a countdown the server never agreed to.
///
/// The nudge keeps its shipped shape at the default 300s timeout — 60s before
/// expiry, the intended four-minute mark — but scales with the window below
/// that, because a fixed 60s lead on a 10s timeout would put the nudge before
/// `asked_at`.
pub fn deadlines_for(
    asked_at: chrono::DateTime<chrono::Utc>,
    timeout_seconds: u64,
) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let seconds = timeout_seconds.min(i64::MAX as u64) as i64;
    let expires_at = asked_at + chrono::Duration::seconds(seconds);
    // `div_ceil` keeps a 1s timeout from collapsing the lead to zero.
    let lead = ASK_USER_NUDGE_LEAD_SECONDS.min(timeout_seconds.div_ceil(5)) as i64;
    let nudge_at = expires_at - chrono::Duration::seconds(lead);
    (nudge_at, expires_at)
}

fn default_allow_other() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AskUserQuestionKind {
    #[default]
    Choice,
    /// Collect a free-form answer. Unlike a choice, this has no options or
    /// unattended default.
    Text,
    /// Collect a credential. The answer carries a `secret_ref`, never a value:
    /// an `ask_user` answer is a tool result, so a value here would be
    /// plaintext in the event log *and* permanently in model context
    /// (TM-AGENT-016).
    Secret,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserOption {
    pub label: String,
    pub description: String,
    #[serde(default, rename = "default")]
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserQuestion {
    #[serde(default)]
    pub kind: AskUserQuestionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default = "default_allow_other")]
    pub allow_other: bool,
    /// Offered choices. `text` and `secret` questions carry none.
    #[serde(default)]
    pub options: Vec<AskUserOption>,
    /// Name to store the credential under, on a `secret` question only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_name: Option<String>,
    /// What the credential will be used for, on a `secret` question only.
    /// Required, because nobody should type a key without being told why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserRequest {
    pub questions: Vec<AskUserQuestion>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// When the question was asked. Stamped by normalization.
    ///
    /// These three are server-authoritative: whatever the model sent is
    /// discarded and overwritten, because the deadline the sweep acts on must
    /// not be one the caller chose for itself. They are `Option` only so a
    /// pre-normalization payload deserializes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_at: Option<String>,
    /// When the surface should start warning that time is running out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nudge_at: Option<String>,
    /// When the server resolves the call with declared defaults (EVE-1056).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskUserStatus {
    Answered,
    Declined,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskUserAnsweredBy {
    User,
    Timeout,
    Unattended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserAnswer {
    pub id: String,
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default)]
    pub other_text: Option<String>,
    /// Handle to the stored credential answering a `secret` question.
    ///
    /// There is deliberately no `value` field on this type — not empty,
    /// absent — so no code path can carry a collected secret into a tool
    /// result. Tools resolve the name against the session secret store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserResult {
    pub status: AskUserStatus,
    pub answered_by: AskUserAnsweredBy,
    pub answers: Vec<AskUserAnswer>,
}
/// Where an [`AskUser`] batch comes from: the session and tool call asking.
///
/// A host that serves several sessions uses it to route the questions to the
/// right person or connection. Values are opaque correlation strings; they
/// carry no organization or principal identity.
///
/// Stability: alpha. `#[non_exhaustive]` so it can gain fields without a
/// breaking change; read it through the accessors.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AskContext {
    session_id: SessionId,
    turn_id: Option<String>,
    tool_call_id: String,
}

impl AskContext {
    /// Describe the session and tool call asking.
    ///
    /// Hosts rarely construct this; the capability builds it for each call.
    /// It is public so responders can be driven directly in tests.
    pub fn new(session_id: SessionId, tool_call_id: impl Into<String>) -> Self {
        Self {
            session_id,
            turn_id: None,
            tool_call_id: tool_call_id.into(),
        }
    }

    /// Attach the id of the turn that made the call.
    pub fn with_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    /// The session asking.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// The turn that made the call, when the runtime reported one.
    pub fn turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref()
    }

    /// The `ask_user` tool call id, stable for the lifetime of the call.
    pub fn tool_call_id(&self) -> &str {
        &self.tool_call_id
    }

    fn from_tool_context(context: &ToolContext) -> Self {
        let mut ask = Self::new(
            context.session_id,
            context.tool_call_id.clone().unwrap_or_default(),
        );
        ask.turn_id = context
            .event_context
            .as_ref()
            .and_then(|event| event.turn_id)
            .map(|turn_id| turn_id.to_string());
        ask
    }
}

/// A host that can answer structured questions while a tool call is in flight.
#[async_trait]
pub trait AskUser: Send + Sync {
    /// Ask the host to answer one normalized batch of questions.
    async fn ask(&self, questions: &[AskUserQuestion]) -> AskUserResult;

    /// Ask with the session and tool call that raised the questions.
    ///
    /// The capability always calls this method. The default ignores `context`
    /// and delegates to [`ask`](Self::ask), so existing responders keep
    /// working; a host that serves several sessions overrides it to route the
    /// batch. Stability: alpha.
    async fn ask_in(&self, context: &AskContext, questions: &[AskUserQuestion]) -> AskUserResult {
        let _ = context;
        self.ask(questions).await
    }
}
#[async_trait]
impl<T: AskUser + ?Sized> AskUser for Arc<T> {
    async fn ask(&self, questions: &[AskUserQuestion]) -> AskUserResult {
        self.as_ref().ask(questions).await
    }

    async fn ask_in(&self, context: &AskContext, questions: &[AskUserQuestion]) -> AskUserResult {
        self.as_ref().ask_in(context, questions).await
    }
}

/// An unattended responder that applies declared defaults or the first option.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultsResponder;

#[async_trait]
impl AskUser for DefaultsResponder {
    async fn ask(&self, questions: &[AskUserQuestion]) -> AskUserResult {
        // Free-form text and credentials have no value an unattended responder
        // can supply. Declining leaves proceeding without one as the model's
        // explicit decision.
        if questions_have_no_default_answer(questions) {
            return AskUserResult {
                status: AskUserStatus::Declined,
                answered_by: AskUserAnsweredBy::Unattended,
                answers: Vec::new(),
            };
        }
        AskUserResult {
            status: AskUserStatus::Answered,
            answered_by: AskUserAnsweredBy::Unattended,
            answers: declared_defaults(questions),
        }
    }
}

/// The answers an unanswered batch resolves to: each question's declared
/// default, or its first option when the model declared none.
///
/// Shared by the unattended responder and the deadline sweep (EVE-1056) so the
/// two cannot drift. Callers must check
/// [`questions_have_no_default_answer`] first — text and credentials have no
/// default, and this would hand back an empty `selected` that reads as an
/// answer.
pub fn declared_defaults(questions: &[AskUserQuestion]) -> Vec<AskUserAnswer> {
    questions
        .iter()
        .map(|question| {
            let mut selected = question
                .options
                .iter()
                .filter(|option| option.is_default)
                .map(|option| option.label.clone())
                .collect::<Vec<_>>();
            if selected.is_empty() {
                selected.extend(question.options.first().map(|option| option.label.clone()));
            }
            AskUserAnswer {
                id: question.id.clone().unwrap_or_default(),
                selected,
                other_text: None,
                secret_ref: None,
            }
        })
        .collect()
}

/// Whether any question in a batch has no unattended/default answer.
pub fn questions_have_no_default_answer(questions: &[AskUserQuestion]) -> bool {
    questions.iter().any(|question| {
        matches!(
            question.kind,
            AskUserQuestionKind::Text | AskUserQuestionKind::Secret
        )
    })
}

pub fn validate_ask_user_request(request: &AskUserRequest) -> Result<(), String> {
    if !(1..=MAX_ASK_USER_QUESTIONS).contains(&request.questions.len()) {
        return Err(format!(
            "questions must contain between 1 and {MAX_ASK_USER_QUESTIONS} items"
        ));
    }
    if !(1..=DEFAULT_ASK_USER_TIMEOUT_SECONDS).contains(&request.timeout_seconds) {
        return Err(format!(
            "timeout_seconds must be between 1 and {DEFAULT_ASK_USER_TIMEOUT_SECONDS}"
        ));
    }

    let mut ids = HashSet::new();
    for (index, question) in request.questions.iter().enumerate() {
        let position = index + 1;
        if let Some(id) = question.id.as_deref() {
            if id.trim().is_empty() {
                return Err(format!("question {position} id must not be empty"));
            }
            if !ids.insert(id) {
                return Err(format!("question ids must be unique; duplicate {id:?}"));
            }
        }
        if question.header.trim().is_empty() {
            return Err(format!("question {position} header must not be empty"));
        }
        if question.header.chars().count() > MAX_ASK_USER_HEADER_CHARS {
            return Err(format!(
                "question {position} header must not exceed {MAX_ASK_USER_HEADER_CHARS} characters"
            ));
        }
        if question.question.trim().is_empty() {
            return Err(format!("question {position} text must not be empty"));
        }

        if question.kind == AskUserQuestionKind::Secret {
            // One secret per call, alone. A batch mixing a credential with
            // choices has no single honest unattended outcome (declining it
            // would throw away answerable choices, answering it would claim a
            // credential nobody supplied), and the card is a password field
            // rather than a form.
            if request.questions.len() != 1 {
                return Err("a secret question must be the only question in the call".to_string());
            }
            if !question.options.is_empty() {
                return Err(format!(
                    "question {position} is a secret and must not offer options"
                ));
            }
            let secret_name = question
                .secret_name
                .as_deref()
                .ok_or_else(|| format!("question {position} secret_name is required"))?;
            if secret_name.trim().is_empty()
                || secret_name.chars().count() > MAX_ASK_USER_SECRET_NAME_CHARS
            {
                return Err(format!(
                    "question {position} secret_name must be between 1 and {MAX_ASK_USER_SECRET_NAME_CHARS} non-whitespace characters"
                ));
            }
            // Nobody should be asked to type a credential without being told
            // what it will be used for.
            if question
                .purpose
                .as_deref()
                .is_none_or(|purpose| purpose.trim().is_empty())
            {
                return Err(format!("question {position} purpose must not be empty"));
            }
            continue;
        }

        if question.secret_name.is_some() || question.purpose.is_some() {
            return Err(format!(
                "question {position} is not a secret and must not carry secret_name or purpose"
            ));
        }
        if question.kind == AskUserQuestionKind::Text {
            if !question.options.is_empty() {
                return Err(format!(
                    "question {position} is text and must not offer options"
                ));
            }
            continue;
        }
        if !(2..=MAX_ASK_USER_OPTIONS).contains(&question.options.len()) {
            return Err(format!(
                "question {position} options must contain between 2 and {MAX_ASK_USER_OPTIONS} items"
            ));
        }

        let mut labels = HashSet::new();
        let mut default_count = 0;
        for option in &question.options {
            if option.label.trim().is_empty() {
                return Err(format!(
                    "question {position} option labels must not be empty"
                ));
            }
            if option.description.trim().is_empty() {
                return Err(format!(
                    "question {position} option descriptions must not be empty"
                ));
            }
            if !labels.insert(option.label.as_str()) {
                return Err(format!(
                    "question {position} option labels must be unique; duplicate {:?}",
                    option.label
                ));
            }
            default_count += usize::from(option.is_default);
        }
        if !question.multi_select && default_count > 1 {
            return Err(format!(
                "question {position} single-select options may mark at most one default"
            ));
        }
    }
    Ok(())
}

pub fn normalize_ask_user_arguments(arguments: &Value) -> Result<Value, String> {
    let human_intent = arguments.get(HUMAN_INTENT_ARGUMENT).cloned();
    let mut contract_arguments = arguments.clone();
    if let Value::Object(object) = &mut contract_arguments {
        object.remove(HUMAN_INTENT_ARGUMENT);
    }
    let mut request: AskUserRequest = serde_json::from_value(contract_arguments)
        .map_err(|error| format!("invalid ask_user arguments: {error}"))?;
    validate_ask_user_request(&request)
        .map_err(|error| format!("invalid ask_user arguments: {error}"))?;

    let mut used_ids: HashSet<String> = request
        .questions
        .iter()
        .filter_map(|question| question.id.clone())
        .collect();
    for (index, question) in request.questions.iter_mut().enumerate() {
        match question.kind {
            AskUserQuestionKind::Secret => {
                // `allow_other` defaults to true, and free text is exactly the
                // trap this kind exists to close: it would carry the typed
                // credential into the tool result.
                question.allow_other = false;
                question.multi_select = false;
            }
            AskUserQuestionKind::Text => {
                question.allow_other = true;
                question.multi_select = false;
            }
            AskUserQuestionKind::Choice => {}
        }
        if question.id.is_some() {
            continue;
        }
        let base = format!("question_{}", index + 1);
        let mut generated = base.clone();
        let mut suffix = 2;
        while used_ids.contains(&generated) {
            generated = format!("{base}_{suffix}");
            suffix += 1;
        }
        used_ids.insert(generated.clone());
        question.id = Some(generated);
    }

    // Stamped last and unconditionally, so a model that supplied its own
    // deadlines does not get to keep them.
    let asked_at = chrono::Utc::now();
    let (nudge_at, expires_at) = deadlines_for(asked_at, request.timeout_seconds);
    request.asked_at = Some(asked_at.to_rfc3339());
    request.nudge_at = Some(nudge_at.to_rfc3339());
    request.expires_at = Some(expires_at.to_rfc3339());

    let mut normalized = serde_json::to_value(request)
        .map_err(|error| format!("failed to normalize ask_user arguments: {error}"))?;
    if let (Some(human_intent), Value::Object(object)) = (human_intent, &mut normalized) {
        object.insert(HUMAN_INTENT_ARGUMENT.to_string(), human_intent);
    }
    Ok(normalized)
}

#[derive(Clone)]
enum AskUserStrategy {
    ClientSide,
    InProcess(Arc<dyn AskUser>),
}

/// Structured questions executed by either a client or an in-process host.
#[derive(Clone)]
pub struct AskUserCapability {
    strategy: AskUserStrategy,
}

impl AskUserCapability {
    /// Execute questions inside the current process with `responder`.
    pub fn new(responder: impl AskUser + 'static) -> Self {
        Self {
            strategy: AskUserStrategy::InProcess(Arc::new(responder)),
        }
    }

    /// Park the turn until a client submits a correlated tool result.
    pub fn client_side() -> Self {
        Self {
            strategy: AskUserStrategy::ClientSide,
        }
    }
}

impl Default for AskUserCapability {
    fn default() -> Self {
        Self::new(DefaultsResponder)
    }
}

impl Capability for AskUserCapability {
    fn id(&self) -> &str {
        ASK_USER_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Ask User"
    }

    fn description(&self) -> &str {
        "Lets an agent ask choice or free-form questions, or collect a credential, through its host."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Запитати користувача",
            "Дає агенту змогу поставити структуровані запитання з варіантами відповіді через хост.",
        )]
    }

    fn icon(&self) -> Option<&str> {
        Some("message-circle-question")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "`ask_user` handles decisions/preferences. Ask only when blocked; batch questions, and never ask what code or context answers. Use `kind: \"text\"` for an open question with no options; it never auto-resolves. Put likely choice options first; timeout uses default/first, and `answered_by` names its source. Do not re-ask a declined question. Never use `ask_user` as a consent gate: destructive, irreversible, or outward-facing actions require `request_approval`, which does not auto-resolve. For A2A `input_required` unanswered, ask the user and relay with `message_task`; never answer for them. For a credential use `kind: \"secret\"` with `secret_name`/`purpose`, alone in the call — never ask for one in prose or an option. It returns a `secret_ref`, never the value, and never auto-resolves.",
        )
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        match &self.strategy {
            AskUserStrategy::ClientSide => vec![ToolDefinition::ClientSide(
                ClientSideTool::new(
                    ASK_USER_TOOL_NAME,
                    "Ask the user 1–4 choice or free-form questions, or collect one credential, then wait for the answer. Use for decisions and preferences, never for consent to destructive, irreversible, or outward-facing actions.",
                    ask_user_parameters_schema(),
                )
                .with_display_name("Ask User")
                .with_category("Core")
                .with_deferrable(DeferrablePolicy::Never)
                .with_hints(ask_user_tool_hints()),
            )],
            AskUserStrategy::InProcess(_) => self
                .tools()
                .iter()
                .map(|tool| {
                    let mut definition = tool.to_definition();
                    if let ToolDefinition::Builtin(tool) = &mut definition {
                        tool.category = Some("Core".to_string());
                    }
                    definition
                })
                .collect(),
        }
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        match &self.strategy {
            AskUserStrategy::ClientSide => vec![],
            AskUserStrategy::InProcess(responder) => vec![Box::new(AskUserTool {
                responder: responder.clone(),
            })],
        }
    }

    fn finalized_tool_calls_hook(
        &self,
        _config: &Value,
    ) -> Option<Arc<dyn FinalizedToolCallsHook>> {
        Some(Arc::new(AskUserFinalizedToolCallsHook))
    }
}

fn ask_user_tool_hints() -> ToolHints {
    ToolHints::default()
        .with_readonly(true)
        .with_destructive(false)
        .with_open_world(false)
}

struct AskUserTool {
    responder: Arc<dyn AskUser>,
}

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        ASK_USER_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("Ask User")
    }

    fn description(&self) -> &str {
        "Ask the user 1–4 choice or free-form questions, or collect one credential. Use for decisions and preferences, never for consent to destructive, irreversible, or outward-facing actions."
    }

    fn parameters_schema(&self) -> Value {
        ask_user_parameters_schema()
    }

    fn hints(&self) -> ToolHints {
        ask_user_tool_hints()
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        self.run(arguments, None).await
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        self.run(arguments, Some(AskContext::from_tool_context(context)))
            .await
    }
}

impl AskUserTool {
    async fn run(&self, arguments: Value, context: Option<AskContext>) -> ToolExecutionResult {
        let normalized = match normalize_ask_user_arguments(&arguments) {
            Ok(arguments) => arguments,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let mut contract_arguments = normalized;
        if let Value::Object(object) = &mut contract_arguments {
            object.remove(HUMAN_INTENT_ARGUMENT);
        }
        let request = match serde_json::from_value::<AskUserRequest>(contract_arguments) {
            Ok(request) => request,
            Err(error) => {
                return ToolExecutionResult::internal_error_msg(format!(
                    "normalized ask_user arguments were invalid: {error}"
                ));
            }
        };
        let outcome = match &context {
            Some(context) => self.responder.ask_in(context, &request.questions).await,
            None => self.responder.ask(&request.questions).await,
        };
        match serde_json::to_value(outcome) {
            Ok(outcome) => ToolExecutionResult::success(outcome),
            Err(error) => ToolExecutionResult::internal_error_msg(format!(
                "ask_user responder returned an invalid outcome: {error}"
            )),
        }
    }
}
struct AskUserFinalizedToolCallsHook;

#[async_trait]
impl FinalizedToolCallsHook for AskUserFinalizedToolCallsHook {
    async fn apply(&self, _context: &FinalizedToolCallsContext<'_>, calls: &mut [ToolCall]) {
        let _ = normalize_ask_user_calls(calls);
    }

    async fn apply_with_rejections(
        &self,
        _context: &FinalizedToolCallsContext<'_>,
        calls: &mut [ToolCall],
    ) -> Vec<FinalizedToolCallRejection> {
        normalize_ask_user_calls(calls)
    }
}

fn normalize_ask_user_calls(calls: &mut [ToolCall]) -> Vec<FinalizedToolCallRejection> {
    let mut rejections = Vec::new();
    for call in calls {
        if call.name != ASK_USER_TOOL_NAME {
            continue;
        }
        match normalize_ask_user_arguments(&call.arguments) {
            Ok(arguments) => call.arguments = arguments,
            Err(error) => rejections.push(FinalizedToolCallRejection {
                tool_call_id: call.id.clone(),
                error,
            }),
        }
    }
    rejections
}
fn ask_user_parameters_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_ASK_USER_QUESTIONS,
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["choice", "text", "secret"],
                            "default": "choice",
                            "description": "`choice` offers options. `text` collects free-form text and never auto-resolves. `secret` collects one credential and must be the only question in the call; its answer returns a reference, never the value."
                        },
                        "id": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Stable answer correlation key. Generated when omitted."
                        },
                        "header": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_ASK_USER_HEADER_CHARS,
                            "description": "Short chip label."
                        },
                        "question": {"type": "string", "minLength": 1},
                        "multi_select": {"type": "boolean", "default": false},
                        "allow_other": {
                            "type": "boolean",
                            "default": true,
                            "description": "Allow a free-text answer."
                        },
                        "secret_name": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_ASK_USER_SECRET_NAME_CHARS,
                            "description": "Required on a `secret` question: the session-secret name to store the credential under. Tools resolve it by this name."
                        },
                        "purpose": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Required on a `secret` question: what the credential will be used for."
                        },
                        "options": {
                            "type": "array",
                            "minItems": 0,
                            "maxItems": MAX_ASK_USER_OPTIONS,
                            "description": "Required on a `choice` question, which offers 2 to 6. `text` and `secret` questions offer none.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "label": {"type": "string", "minLength": 1},
                                    "description": {"type": "string", "minLength": 1},
                                    "default": {"type": "boolean", "default": false}
                                },
                                "required": ["label", "description"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["header", "question"],
                    "additionalProperties": false
                }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 1,
                "maximum": DEFAULT_ASK_USER_TIMEOUT_SECONDS,
                "default": DEFAULT_ASK_USER_TIMEOUT_SECONDS,
                "description": "How long the client waits before applying a default. May shorten but not exceed the platform ceiling."
            },
            // Declared because the schema validates the *normalized* call, which
            // carries the deadlines normalization stamped on it (EVE-1056), and
            // `additionalProperties: false` would otherwise reject it. They are
            // read-only: anything supplied here is discarded and replaced.
            "asked_at": {
                "type": "string",
                "format": "date-time",
                "readOnly": true,
                "description": "Set by the server. When the question was asked; ignored if supplied."
            },
            "nudge_at": {
                "type": "string",
                "format": "date-time",
                "readOnly": true,
                "description": "Set by the server. When the surface starts warning time is short; ignored if supplied."
            },
            "expires_at": {
                "type": "string",
                "format": "date-time",
                "readOnly": true,
                "description": "Set by the server. When the declared defaults are applied; ignored if supplied."
            }
        },
        "required": ["questions"],
        "additionalProperties": false
    })
}

#[cfg(test)]
#[path = "ask_user_tests.rs"]
mod tests;
