// MCP 2026-07-28 Tasks extension (SEP-2663, `io.modelcontextprotocol/tasks`).
//
// Spec: https://modelcontextprotocol.github.io/ext-tasks/specification/2026-07-28/tasks.html
// Wire schema of record: `@modelcontextprotocol/ext-tasks` `core/v2` (the
// interop test in `tests/domain/mcp_tasks_interop_test.rs` validates every
// response against it).
//
// Decision: a task is one turn, not a session. `agent_run` and
// `session_send_message` each start a turn by sending a message, and that turn
// is the work the `tools/call` asked for. A session-scoped task would break the
// spec's terminal states: a session goes back to work on the next message, so a
// task reported `completed` could later read `working`, and a client polling an
// old handle would get a newer turn's reply. The handle therefore names the
// input message that started the turn:
//
//   taskId = "{org_public_id}.{session_id}.{message_id}"
//
// The org is in the handle because `tools/call` may target another org via
// `organization_id`, and `tasks/*` carry nothing but the handle. Every request
// re-resolves membership for that org and re-runs the session read policy, so
// the handle grants nothing by itself. The ids are UUIDv7 with 74 random bits
// each, which meets the spec's "cannot enumerate or guess" bar.
//
// Decision: where the turn stands comes from the event log, not session status.
// The first terminal turn event after the input message ends the turn that
// answered it (a message sent mid-turn steers the running turn), which is the
// rule the waited message API already uses. Status mapping:
//
//   turn.completed                    -> completed, result = the turn's reply
//   turn.failed / turn.sealed         -> completed with `isError: true`
//   turn.cancelled                    -> cancelled
//   open, parked on an ask_user form  -> input_required with the form
//   open, anything else               -> working, `statusMessage` says why
//
// A failed turn is a tool error, not a JSON-RPC error, and the spec says
// `failed` MUST NOT carry non-JSON-RPC errors, so nothing here reports `failed`.
//
// Decision: the input key of an ask_user form names the parked tool call
// (`ask_user:{tool_call_id}`), because the spec forbids reusing a key once it
// was answered and one turn can ask more than once.
//
// Gating: the extension exists only for the negotiated 2026-07-28 protocol, and
// only for a client that declared it in the request's `_meta` capabilities
// (servers MUST NOT return a task otherwise). A 2026 client that did not
// declare it gets -32021 on `tasks/*`; 2025-* clients get method not found.

use super::{
    AppState, AuthUser, JsonRpcResponse, ResolvedOrg, dispatch_command, form_elicitation,
    resolve_org_override,
};
use crate::api::question_answers::{QuestionResolver, ResolveError, resolve_question_answers};
use crate::domains::organizations::record::validate_org_public_id;
use chrono::{DateTime, SecondsFormat, Utc};
use everruns_contracts::runtime::events::{
    CONVERSATION_MESSAGE, OUTPUT_MESSAGE_COMPLETED, TURN_CANCELLED, TURN_COMPLETED,
};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::Caller;
use everruns_core::builtins::ask_user::AskUserStatus;
use serde_json::{Value, json};

/// Extension capability key (SEP-2663). Advertised under
/// `capabilities.extensions` and opted into by clients via per-request
/// `_meta["io.modelcontextprotocol/clientCapabilities"].extensions`.
pub(super) const TASKS_EXTENSION_KEY: &str = "io.modelcontextprotocol/tasks";

/// Per-request client-capabilities `_meta` key (SEP-2133 extension framework).
const CLIENT_CAPABILITIES_META_KEY: &str = "io.modelcontextprotocol/clientCapabilities";

/// `MissingRequiredClientCapability`: a 2026 client used `tasks/*` without
/// declaring the extension.
const MISSING_CAPABILITY_ERROR_CODE: i32 = -32021;

/// Suggested client poll interval for `tasks/get`, in milliseconds. Advisory.
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;

/// Task time-to-live, in milliseconds. Turns are durable in Postgres and never
/// expire, so this is a floor clients can rely on, not a deletion deadline.
const DEFAULT_TTL_MS: u64 = 24 * 60 * 60 * 1_000;

/// Prefix of the input key an ask_user form is delivered under.
const ASK_USER_INPUT_KEY_PREFIX: &str = "ask_user:";

/// Task lifecycle states (SEP-2663). `completed` / `failed` / `cancelled` are
/// terminal. `failed` is never produced here (see the header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskStatus {
    Working,
    InputRequired,
    Completed,
    Cancelled,
}

impl TaskStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Working => "working",
            TaskStatus::InputRequired => "input_required",
            TaskStatus::Completed => "completed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    fn is_terminal(self) -> bool {
        matches!(self, TaskStatus::Completed | TaskStatus::Cancelled)
    }
}

/// A task handle: the turn started by one input message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TaskRef {
    org_public_id: String,
    session_id: SessionId,
    message_id: MessageId,
}

impl TaskRef {
    pub(super) fn new(org_public_id: &str, session_id: SessionId, message_id: MessageId) -> Self {
        Self {
            org_public_id: org_public_id.to_string(),
            session_id,
            message_id,
        }
    }

    pub(super) fn parse(task_id: &str) -> Option<Self> {
        let mut parts = task_id.split('.');
        let org = parts.next()?;
        let session_id = parts.next()?.parse().ok()?;
        let message_id = parts.next()?.parse().ok()?;
        if parts.next().is_some() || !validate_org_public_id(org) {
            return None;
        }
        Some(Self::new(org, session_id, message_id))
    }

    pub(super) fn id(&self) -> String {
        format!(
            "{}.{}.{}",
            self.org_public_id, self.session_id, self.message_id
        )
    }
}

/// What a task looks like right now.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TaskView {
    task_id: String,
    status: TaskStatus,
    created_at: DateTime<Utc>,
    last_updated_at: DateTime<Utc>,
    status_message: Option<String>,
    /// The outstanding ask_user form: its input key, the request, and the tool
    /// call it answers. Set only while `input_required`.
    input: Option<PendingInput>,
    /// The `CallToolResult` of a completed task. A 2026-07-28 result, so it
    /// carries its own `resultType: "complete"`.
    result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
struct PendingInput {
    key: String,
    request: Value,
    tool_call_id: String,
}

impl TaskView {
    fn base_fields(&self) -> Value {
        let mut task = json!({
            "taskId": self.task_id,
            "status": self.status.as_str(),
            "createdAt": rfc3339(self.created_at),
            "lastUpdatedAt": rfc3339(self.last_updated_at),
            "ttlMs": DEFAULT_TTL_MS,
            "pollIntervalMs": DEFAULT_POLL_INTERVAL_MS,
        });
        if let Some(message) = &self.status_message {
            task["statusMessage"] = json!(message);
        }
        task
    }

    /// `CreateTaskResult`, answered to `tools/call` in place of a result.
    pub(super) fn create_task_result(&self) -> Value {
        let mut task = self.base_fields();
        task["resultType"] = json!("task");
        task
    }

    /// `GetTaskResult`: the detailed task, with the outstanding input or the
    /// final result.
    pub(super) fn get_task_result(&self) -> Value {
        let mut task = self.base_fields();
        task["resultType"] = json!("complete");
        if let Some(input) = &self.input {
            task["inputRequests"] = json!({ input.key.clone(): input.request.clone() });
        }
        if let Some(result) = &self.result {
            task["result"] = result.clone();
        }
        task
    }
}

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The empty acknowledgement `tasks/update` and `tasks/cancel` answer with.
fn complete_ack() -> Value {
    json!({ "resultType": "complete" })
}

/// Did the client declare the Tasks extension in this request's `_meta`?
pub(super) fn client_advertised_tasks(params: &Value) -> bool {
    params
        .get("_meta")
        .and_then(|meta| meta.get(CLIENT_CAPABILITIES_META_KEY))
        .and_then(|caps| caps.get("extensions"))
        .and_then(|exts| exts.get(TASKS_EXTENSION_KEY))
        .is_some()
}

/// Does the negotiated protocol plus the client's declared capabilities let a
/// `tools/call` answer with a task? Servers MUST NOT return a task to a client
/// that did not declare support (SEP-2663).
pub(super) fn tasks_enabled(protocol_version: &str, params: &Value) -> bool {
    protocol_version == super::MCP_PROTOCOL_VERSION_LATEST && client_advertised_tasks(params)
}

/// The `extensions` map advertised in `initialize` capabilities when the
/// negotiated protocol supports the Tasks extension. `None` for 2025-* so their
/// `initialize` response is byte-for-byte unchanged.
pub(super) fn initialize_extensions(protocol_version: &str) -> Option<Value> {
    (protocol_version == super::MCP_PROTOCOL_VERSION_LATEST)
        .then(|| json!({ TASKS_EXTENSION_KEY: {} }))
}

/// -32021 for a 2026 client calling `tasks/*` without declaring the extension.
pub(super) fn missing_capability(id: Option<Value>) -> JsonRpcResponse {
    let mut response = JsonRpcResponse::error(
        id,
        MISSING_CAPABILITY_ERROR_CODE,
        "tasks/* requires the io.modelcontextprotocol/tasks extension in the request's client capabilities",
    );
    if let Some(error) = response.error.as_mut() {
        error.data = Some(json!({
            "requiredCapabilities": { "extensions": { TASKS_EXTENSION_KEY: {} } }
        }));
    }
    response
}

/// Answer a successful `agent_run` / `session_send_message` with the task for
/// the turn it started. The tool's own JSON names the session and the message;
/// the message is already stored, so the task exists durably (the spec's
/// precondition for returning a handle).
pub(super) async fn created_task(
    id: Option<Value>,
    content: &str,
    arguments: &Value,
    auth_user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let parsed: Value = serde_json::from_str(content).unwrap_or(Value::Null);
    let session_id = parsed
        .get("session_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<SessionId>().ok());
    let message_id = parsed
        .get("message_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<MessageId>().ok());
    let (Some(session_id), Some(message_id)) = (session_id, message_id) else {
        tracing::error!("Conversation tool result names no session and message");
        return JsonRpcResponse::error(id, -32603, "Failed to create task");
    };
    let org = match resolve_org_override(arguments, auth_user, org, state).await {
        Ok(org) => org,
        Err(error) => return JsonRpcResponse::invalid_params(id, error),
    };
    let task_ref = TaskRef::new(&org.public_id, session_id, message_id);
    match load_task(&task_ref, &org, state).await {
        Ok(Some(view)) => JsonRpcResponse::success(id, view.create_task_result()),
        Ok(None) => {
            tracing::error!(task_id = %task_ref.id(), "Task missing right after creation");
            JsonRpcResponse::error(id, -32603, "Failed to create task")
        }
        Err(error) => {
            tracing::error!(error = %error, "Failed to read a created task");
            JsonRpcResponse::error(id, -32603, "Failed to create task")
        }
    }
}

pub(super) async fn handle_method(
    method: &str,
    id: Option<Value>,
    params: Value,
    auth_user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let Some(task_id) = params.get("taskId").and_then(Value::as_str) else {
        return JsonRpcResponse::invalid_params(id, "Missing 'taskId' in params");
    };
    let Some(task_ref) = TaskRef::parse(task_id) else {
        return unknown_task(id);
    };
    // THREAT[TM-MCP-001]: the handle names an org; membership is resolved
    // fresh, exactly as an `organization_id` tool argument would be.
    let org_args = json!({ "organization_id": task_ref.org_public_id });
    let Ok(org) = resolve_org_override(&org_args, auth_user, org, state).await else {
        return unknown_task(id);
    };
    let view = match load_task(&task_ref, &org, state).await {
        Ok(Some(view)) => view,
        Ok(None) => return unknown_task(id),
        Err(error) => {
            tracing::error!(error = %error, "Failed to read a task");
            return JsonRpcResponse::error(id, -32603, "Failed to read task");
        }
    };

    match method {
        "tasks/get" => JsonRpcResponse::success(id, view.get_task_result()),
        "tasks/cancel" => handle_cancel(id, &task_ref, &view, &org, state).await,
        "tasks/update" => handle_update(id, &task_ref, &view, &params, &org, state).await,
        _ => JsonRpcResponse::method_not_found(id),
    }
}

/// -32602 for a handle that does not name a task this caller can see. One
/// answer for malformed, foreign and missing handles, so it reveals nothing.
fn unknown_task(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::invalid_params(id, "Unknown taskId")
}

/// Read a task's current state, or `None` when the caller cannot see it.
pub(super) async fn load_task(
    task_ref: &TaskRef,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Option<TaskView>, String> {
    // The same command `session_get_status` reads through, so the session
    // policy (and Platform Chat ownership) applies unchanged.
    let session = match dispatch_command(
        "get_session",
        json!({ "session_id": task_ref.session_id.to_string() }),
        org,
        state,
    )
    .await
    {
        Ok(session) => session,
        Err(_) => return Ok(None),
    };
    let session_status = session
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let Some(input) = state
        .db
        .find_input_message_event(task_ref.session_id, task_ref.message_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let (terminal, last_updated) = state
        .db
        .input_turn_progress(task_ref.session_id, input.sequence)
        .await
        .map_err(|error| error.to_string())?;

    let mut view = TaskView {
        task_id: task_ref.id(),
        status: TaskStatus::Working,
        created_at: input.ts,
        last_updated_at: last_updated.unwrap_or(input.ts).max(input.ts),
        status_message: None,
        input: None,
        result: None,
    };

    if let Some(terminal) = terminal {
        if terminal.event_type == TURN_CANCELLED {
            view.status = TaskStatus::Cancelled;
            view.status_message = Some("The turn was cancelled".to_string());
            return Ok(Some(view));
        }
        let reply = turn_reply(
            state,
            task_ref.session_id,
            input.sequence,
            terminal.sequence,
        )
        .await
        .map_err(|error| error.to_string())?;
        let structured = crate::domains::session_tasks::read_structured_task_result(
            &state.db,
            org.org_id,
            task_ref.session_id,
        )
        .await
        .ok()
        .flatten();
        view.status = TaskStatus::Completed;
        if terminal.event_type == TURN_COMPLETED {
            view.result = Some(completed_result(task_ref, reply, structured));
        } else {
            let detail = turn_end_detail(&terminal.event_type, &terminal.data);
            view.status_message = Some(detail.clone());
            view.result = Some(failed_result(task_ref, &detail, reply));
        }
        return Ok(Some(view));
    }

    if is_waiting_for_input(&session_status) {
        let caller = Caller::from(org);
        if let Some(pending) =
            form_elicitation::pending_questions_for_session(&caller, task_ref.session_id, state)
                .await?
            && let Some(request) = form_elicitation::form_input_request(&pending.questions)
        {
            view.status = TaskStatus::InputRequired;
            view.status_message = Some("The agent asked a question".to_string());
            view.input = Some(PendingInput {
                key: ask_user_input_key(&pending.tool_call_id),
                request,
                tool_call_id: pending.tool_call_id,
            });
            return Ok(Some(view));
        }
    }
    view.status_message = Some(working_message(&session_status).to_string());
    Ok(Some(view))
}

/// The session is parked on a client-side tool call. The status reads
/// `waitingfortoolresults` on the wire (lowercased enum) and
/// `waiting_for_tool_results` in storage; accept both.
fn is_waiting_for_input(session_status: &str) -> bool {
    matches!(
        session_status,
        "waiting_for_tool_results" | "waitingfortoolresults"
    )
}

fn ask_user_input_key(tool_call_id: &str) -> String {
    format!("{ASK_USER_INPUT_KEY_PREFIX}{tool_call_id}")
}

/// Why an open turn is not done, in words a person reads in their client.
fn working_message(session_status: &str) -> &'static str {
    match session_status {
        // A question this client cannot be handed as a form (a secret, say):
        // the answer has to come from the Everruns app.
        status if is_waiting_for_input(status) => {
            "Waiting for input the client cannot provide here"
        }
        "paused" => "Paused: the session reached a budget or limit",
        _ => "The agent is working",
    }
}

/// The turn's final reply: what the agent said between the input and the turn
/// end, never its commentary, and not a "let me check" before a tool call.
async fn turn_reply(
    state: &AppState,
    session_id: SessionId,
    input_sequence: i32,
    end_sequence: i32,
) -> anyhow::Result<Option<String>> {
    let events = state
        .db
        .list_events(
            session_id,
            Some(input_sequence),
            None,
            &[
                OUTPUT_MESSAGE_COMPLETED.to_string(),
                CONVERSATION_MESSAGE.to_string(),
            ],
            &[],
            None,
            None,
        )
        .await?;
    let said = events
        .iter()
        .filter(|event| event.sequence < end_sequence)
        .filter_map(|event| {
            everruns_core::conversation::said_message_in_event(&event.event_type, &event.data)
        });
    Ok(everruns_core::conversation::final_reply(said))
}

/// What a person reads about a turn that ended without a reply.
fn turn_end_detail(event_type: &str, data: &Value) -> String {
    let detail = data
        .get("error")
        .or_else(|| data.get("reason"))
        .and_then(Value::as_str)
        .filter(|detail| !detail.trim().is_empty());
    let what = if event_type == "turn.sealed" {
        "The turn was stopped"
    } else {
        "The turn failed"
    };
    match detail {
        Some(detail) => format!("{what}: {detail}"),
        None => what.to_string(),
    }
}

fn task_ids_json(task_ref: &TaskRef) -> Value {
    json!({
        "session_id": task_ref.session_id.to_string(),
        "message_id": task_ref.message_id.to_string(),
    })
}

fn completed_result(task_ref: &TaskRef, reply: Option<String>, structured: Option<Value>) -> Value {
    let mut structured_content = task_ids_json(task_ref);
    structured_content["output"] = json!(reply);
    if let Some(structured) = structured {
        structured_content["structured_result"] = structured;
    }
    json!({
        "resultType": "complete",
        "content": [{ "type": "text", "text": reply.unwrap_or_default() }],
        "structuredContent": structured_content,
    })
}

fn failed_result(task_ref: &TaskRef, detail: &str, reply: Option<String>) -> Value {
    let mut structured_content = task_ids_json(task_ref);
    structured_content["error"] = json!(detail);
    structured_content["output"] = json!(reply);
    json!({
        "resultType": "complete",
        "content": [{ "type": "text", "text": detail }],
        "structuredContent": structured_content,
        "isError": true,
    })
}

/// Cancellation is cooperative and acknowledged with an empty result. A turn
/// that already ended has nothing to cancel, so the ack is all it gets.
async fn handle_cancel(
    id: Option<Value>,
    task_ref: &TaskRef,
    view: &TaskView,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    if view.status.is_terminal() {
        return JsonRpcResponse::success(id, complete_ack());
    }
    match dispatch_command(
        "cancel_session",
        json!({ "session_id": task_ref.session_id.to_string() }),
        org,
        state,
    )
    .await
    {
        Ok(_) => JsonRpcResponse::success(id, complete_ack()),
        Err(message) => JsonRpcResponse::error(id, -32603, message),
    }
}

/// Apply the responses to outstanding input. Per the spec, a response under a
/// key that is not outstanding (never issued, already answered) is ignored, and
/// success is an empty acknowledgement.
async fn handle_update(
    id: Option<Value>,
    task_ref: &TaskRef,
    view: &TaskView,
    params: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let Some(responses) = params.get("inputResponses").and_then(Value::as_object) else {
        return JsonRpcResponse::invalid_params(id, "tasks/update requires 'inputResponses'");
    };
    let Some(input) = &view.input else {
        return JsonRpcResponse::success(id, complete_ack());
    };
    let Some(response) = responses.get(&input.key).filter(|value| value.is_object()) else {
        return JsonRpcResponse::success(id, complete_ack());
    };
    answer_question(id, task_ref, input, response, org, state).await
}

/// Resolve the parked question set through the one shared resolution operation
/// (EVE-1054), which validates every selection against what was asked.
async fn answer_question(
    id: Option<Value>,
    task_ref: &TaskRef,
    input: &PendingInput,
    response: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let caller = Caller::from(org);
    // Answering resumes the session, which is a manage action.
    if let Err(error) = crate::domains::sessions::SESSION_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
    {
        return JsonRpcResponse::invalid_params(id, error.to_string());
    }
    let session = match state
        .session_service
        .get(&caller, task_ref.session_id.uuid(), None)
        .await
    {
        Ok(Some(session)) => session,
        Ok(None) => return unknown_task(id),
        Err(error) => {
            tracing::error!(error = %error, "Failed to load task for question authorization");
            return JsonRpcResponse::error(id, -32603, "Failed to authorize task input");
        }
    };
    match crate::domains::sessions::platform_chat_owner_matches_session(
        &state.db, &caller, &session,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            return JsonRpcResponse::invalid_params(
                id,
                "forbidden: Only the Platform Chat session owner can answer questions",
            );
        }
        Err(error) => {
            tracing::error!(error = %error, "Failed to authorize task question input");
            return JsonRpcResponse::error(id, -32603, "Failed to authorize task input");
        }
    }
    let pending =
        match form_elicitation::pending_questions_for_session(&caller, task_ref.session_id, state)
            .await
        {
            Ok(Some(pending)) if pending.tool_call_id == input.tool_call_id => pending,
            // Answered or moved on since the read: the key is no longer
            // outstanding, which the spec says to ignore.
            Ok(_) => return JsonRpcResponse::success(id, complete_ack()),
            Err(error) => {
                tracing::error!(error = %error, "Failed to read a task's pending question set");
                return JsonRpcResponse::error(id, -32603, "Failed to read task input");
            }
        };
    let outcome = match form_elicitation::outcome_from_response(&pending.questions, response) {
        Ok(outcome) => outcome,
        Err(message) => return JsonRpcResponse::invalid_params(id, message),
    };
    let (status, answers) = match outcome {
        form_elicitation::FormOutcome::Answered(answers) => (AskUserStatus::Answered, answers),
        form_elicitation::FormOutcome::Declined => (AskUserStatus::Declined, Vec::new()),
        form_elicitation::FormOutcome::Cancelled => (AskUserStatus::Cancelled, Vec::new()),
    };
    let resolver = QuestionResolver {
        db: &state.db,
        session_service: &state.session_service,
        event_service: &state.event_service,
        runner: state.runner.clone(),
    };
    match resolve_question_answers(
        &resolver,
        &caller,
        task_ref.session_id,
        Some(&pending.tool_call_id),
        status,
        &answers,
    )
    .await
    {
        Ok(_) => JsonRpcResponse::success(id, complete_ack()),
        Err(ResolveError::Invalid(detail)) => JsonRpcResponse::invalid_params(id, detail),
        // A concurrent answer won: this key is no longer outstanding.
        Err(
            ResolveError::AlreadyResolved
            | ResolveError::NotWaiting(_)
            | ResolveError::NoPendingQuestions
            | ResolveError::WrongPendingCall,
        ) => JsonRpcResponse::success(id, complete_ack()),
        Err(ResolveError::Internal(detail)) => {
            tracing::error!(error = %detail, "Failed to resolve task question answers");
            JsonRpcResponse::error(id, -32603, "Failed to record task input")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        MCP_PROTOCOL_VERSION_2025_06, MCP_PROTOCOL_VERSION_FALLBACK, MCP_PROTOCOL_VERSION_LATEST,
    };
    use super::*;
    use serde_json::json;

    const ORG: &str = "org_0123456789abcdef0123456789abcdef";

    fn task_ref() -> TaskRef {
        TaskRef::new(ORG, SessionId::new(), MessageId::new())
    }

    fn view(status: TaskStatus) -> TaskView {
        let at = DateTime::parse_from_rfc3339("2026-10-23T10:00:00.250Z")
            .unwrap()
            .with_timezone(&Utc);
        TaskView {
            task_id: task_ref().id(),
            status,
            created_at: at,
            last_updated_at: at,
            status_message: None,
            input: None,
            result: None,
        }
    }

    #[test]
    fn task_ids_round_trip_and_reject_anything_else() {
        let task = task_ref();
        assert_eq!(TaskRef::parse(&task.id()), Some(task.clone()));
        let session = task.session_id.to_string();
        let message = task.message_id.to_string();
        for bad in [
            session.clone(),
            format!("{session}.{message}"),
            format!("org_nothex.{session}.{message}"),
            format!("{ORG}.{message}.{session}"),
            format!("{}.extra", task.id()),
            String::new(),
        ] {
            assert_eq!(TaskRef::parse(&bad), None, "{bad}");
        }
    }

    #[test]
    fn status_strings_match_sep_2663_vocabulary() {
        assert_eq!(TaskStatus::Working.as_str(), "working");
        assert_eq!(TaskStatus::InputRequired.as_str(), "input_required");
        assert_eq!(TaskStatus::Completed.as_str(), "completed");
        assert_eq!(TaskStatus::Cancelled.as_str(), "cancelled");
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(!TaskStatus::Working.is_terminal());
        assert!(!TaskStatus::InputRequired.is_terminal());
    }

    #[test]
    fn create_task_result_carries_the_required_fields_only() {
        let result = view(TaskStatus::Working).create_task_result();
        assert_eq!(result["resultType"], "task");
        assert_eq!(result["status"], "working");
        assert_eq!(result["createdAt"], "2026-10-23T10:00:00.250Z");
        assert_eq!(result["lastUpdatedAt"], "2026-10-23T10:00:00.250Z");
        assert_eq!(result["ttlMs"], DEFAULT_TTL_MS);
        assert_eq!(result["pollIntervalMs"], DEFAULT_POLL_INTERVAL_MS);
        assert!(result.get("content").is_none());
        assert!(result.get("result").is_none());
    }

    #[test]
    fn get_task_result_shows_input_or_result_by_status() {
        let mut parked = view(TaskStatus::InputRequired);
        parked.input = Some(PendingInput {
            key: ask_user_input_key("call_1"),
            request: json!({ "method": "elicitation/create", "params": {} }),
            tool_call_id: "call_1".to_string(),
        });
        let parked = parked.get_task_result();
        assert_eq!(parked["resultType"], "complete");
        assert!(parked["inputRequests"]["ask_user:call_1"].is_object());
        assert!(parked.get("result").is_none());

        let mut done = view(TaskStatus::Completed);
        done.result = Some(completed_result(
            &task_ref(),
            Some("Reply".to_string()),
            None,
        ));
        let done = done.get_task_result();
        assert_eq!(done["result"]["content"][0]["text"], "Reply");
        assert!(done["result"].get("isError").is_none());
        assert!(done.get("inputRequests").is_none());
    }

    #[test]
    fn a_failed_turn_is_a_tool_error_not_a_failed_task() {
        let detail = turn_end_detail("turn.failed", &json!({ "error": "provider down" }));
        assert_eq!(detail, "The turn failed: provider down");
        let result = failed_result(&task_ref(), &detail, None);
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], detail);
        assert_eq!(
            turn_end_detail("turn.sealed", &json!({ "reason": "no progress" })),
            "The turn was stopped: no progress"
        );
        assert_eq!(
            turn_end_detail("turn.failed", &json!({})),
            "The turn failed"
        );
    }

    #[test]
    fn tasks_enabled_requires_latest_protocol_and_opt_in() {
        let opted_in = json!({
            "_meta": {
                CLIENT_CAPABILITIES_META_KEY: {
                    "extensions": { TASKS_EXTENSION_KEY: {} }
                }
            }
        });
        assert!(tasks_enabled(MCP_PROTOCOL_VERSION_LATEST, &opted_in));
        assert!(!tasks_enabled(MCP_PROTOCOL_VERSION_LATEST, &json!({})));
        assert!(!tasks_enabled(MCP_PROTOCOL_VERSION_2025_06, &opted_in));
        assert!(!tasks_enabled(MCP_PROTOCOL_VERSION_FALLBACK, &opted_in));
    }

    #[test]
    fn initialize_extensions_gated_by_version() {
        assert_eq!(
            initialize_extensions(MCP_PROTOCOL_VERSION_LATEST),
            Some(json!({ TASKS_EXTENSION_KEY: {} }))
        );
        assert_eq!(initialize_extensions(MCP_PROTOCOL_VERSION_2025_06), None);
        assert_eq!(initialize_extensions(MCP_PROTOCOL_VERSION_FALLBACK), None);
    }

    #[test]
    fn missing_capability_is_32021_naming_the_extension() {
        let response = missing_capability(Some(json!(1)));
        let error = response.error.unwrap();
        assert_eq!(error.code, -32021);
        assert!(
            error.data.unwrap()["requiredCapabilities"]["extensions"][TASKS_EXTENSION_KEY]
                .is_object()
        );
    }
}
