// MCP 2026-07-28 Tasks extension (SEP-2663, `io.modelcontextprotocol/tasks`).
//
// This is wire-level interop alignment, NOT new capability. Everruns already
// implements the long-running `tools/call` pattern: `agent_run` returns a
// `session_id` + poll hint, clients poll `session_get_status`, all state is
// Postgres-backed with no server-side session memory. The Tasks extension is
// the standardized vocabulary for exactly that pattern, so we map a task handle
// onto a session:
//
//   task handle / `taskId`          ↔ `session_id`
//   `tools/call` → CreateTaskResult ↔ `agent_run` / `session_send_message`
//   `tasks/get`                     ↔ `session_get_status`
//   `tasks/update` (provide input)  ↔ question resolution / `session_send_message`
//   `tasks/cancel`                  ↔ `cancel_session`
//   lifecycle status                ↔ derived from session status
//
// The whole surface is gated on the negotiated protocol being 2026-07-28 AND
// the client having advertised the extension in its per-request capabilities.
// 2025-* clients (and 2026 clients that did not opt in) see the pre-existing
// shapes unchanged — the task fields are strictly additive.
//
// Schema source: final SEP-2663 and the official MCP Tasks extension overview.
// Both specify `ttlMs` / `pollIntervalMs`, so those field names are canonical.

use super::{
    AppState, AuthUser, JsonRpcResponse, ResolvedOrg, classify_mcp_execute_error, dispatch_command,
    error_result_payload, form_elicitation, resolve_org_override, tool_session_get_status,
    tool_session_send_message,
};
use crate::api::question_answers::{QuestionResolver, ResolveError, resolve_question_answers};
use everruns_builtins::ask_user::AskUserStatus;
use everruns_core::Caller;
use serde_json::{Value, json};

/// Extension capability key (SEP-2663). Advertised under
/// `capabilities.extensions` and opted into by clients via per-request
/// `_meta["io.modelcontextprotocol/clientCapabilities"].extensions`.
pub(super) const TASKS_EXTENSION_KEY: &str = "io.modelcontextprotocol/tasks";

/// Per-request client-capabilities `_meta` key (SEP-2133 extension framework).
const CLIENT_CAPABILITIES_META_KEY: &str = "io.modelcontextprotocol/clientCapabilities";

/// Suggested client poll interval for `tasks/get`, in milliseconds. Mirrors the
/// `agent_run` poll hint cadence; advisory only.
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;

/// Task time-to-live, in milliseconds. Sessions are durable in Postgres and
/// never expire on their own, so we advertise a long TTL; this is advisory for
/// clients deciding how long to keep polling a handle.
const DEFAULT_TTL_MS: u64 = 24 * 60 * 60 * 1_000;

/// Task lifecycle states (SEP-2663). `completed` / `failed` / `cancelled` are
/// terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskStatus {
    Working,
    InputRequired,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Working => "working",
            TaskStatus::InputRequired => "input_required",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }
}

/// Map an Everruns session status string to a Tasks lifecycle state.
///
/// Session statuses (see `everruns_platform::SessionStatus`):
/// - `started` / `active` → `working` (a turn is or will be running)
/// - `waiting_for_tool_results` / `paused` → `input_required` (needs client
///   input: tool results, or a budget/limit action)
/// - `idle` → `completed` (turn finished; from the Tasks view the run's work is
///   done and the result is available to read)
///
/// `failed` and `cancelled` are not persisted as session statuses today
/// (cancellation emits a turn event and the session returns to idle), so they
/// are unreachable from status alone. The variants exist so callers with
/// stronger information (e.g. an explicit cancel just issued) can report them,
/// and so the mapping is total against the SEP-2663 vocabulary.
pub(super) fn task_status_from_session_status(session_status: &str) -> TaskStatus {
    match session_status {
        "started" | "active" | "running" => TaskStatus::Working,
        "waiting_for_tool_results" | "waitingfortoolresults" | "paused" => {
            TaskStatus::InputRequired
        }
        "idle" | "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "cancelled" | "canceled" => TaskStatus::Cancelled,
        // Unknown/legacy values: treat as still working rather than falsely
        // terminal, so a client keeps polling instead of dropping the handle.
        _ => TaskStatus::Working,
    }
}

/// Does the negotiated protocol version plus the client's advertised
/// capabilities enable the Tasks extension for this request?
///
/// Both must hold: the negotiated protocol is 2026-07-28 (the extension does
/// not exist for 2025-* clients) AND the client opted in via per-request
/// `_meta`. Servers MUST NOT return a task to a client that did not declare
/// support (SEP-2663), which is what keeps this fully back-compat.
pub(super) fn tasks_enabled(protocol_version: &str, params: &Value) -> bool {
    protocol_version == super::MCP_PROTOCOL_VERSION_LATEST && client_advertised_tasks(params)
}

/// True when the request `_meta` advertises the tasks extension in the client's
/// per-request capabilities.
fn client_advertised_tasks(params: &Value) -> bool {
    params
        .get("_meta")
        .and_then(|meta| meta.get(CLIENT_CAPABILITIES_META_KEY))
        .and_then(|caps| caps.get("extensions"))
        .and_then(|exts| exts.get(TASKS_EXTENSION_KEY))
        .is_some()
}

/// The `extensions` map advertised in `initialize` capabilities when the
/// negotiated protocol supports the Tasks extension. `None` for 2025-* so their
/// `initialize` response is byte-for-byte unchanged.
pub(super) fn initialize_extensions(protocol_version: &str) -> Option<Value> {
    (protocol_version == super::MCP_PROTOCOL_VERSION_LATEST)
        .then(|| json!({ TASKS_EXTENSION_KEY: {} }))
}

/// Build a `CreateTaskResult` (`resultType: "task"`) for a freshly created or
/// advanced session, mapping `session_id` → `taskId`. Emitted from `agent_run`
/// / `session_send_message` when the Tasks extension is active. The `status` is
/// derived from the session status the tool already resolved.
pub(super) fn create_task_result(session_id: &str, session_status: &str) -> Value {
    let status = task_status_from_session_status(session_status);
    let mut result = task_handle(session_id, status);
    result["resultType"] = json!("task");
    result
}

/// Base task-handle fields shared by `CreateTaskResult` and the `tasks/get`
/// `Task` object.
pub(super) fn task_handle(session_id: &str, status: TaskStatus) -> Value {
    json!({
        "taskId": session_id,
        "status": status.as_str(),
        "ttlMs": DEFAULT_TTL_MS,
        "pollIntervalMs": DEFAULT_POLL_INTERVAL_MS,
    })
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

    let org = match resolve_org_override(&params, auth_user, org, state).await {
        Ok(org) => org,
        Err(error) => return JsonRpcResponse::invalid_params(id, error),
    };

    match method {
        "tasks/get" => handle_get(id, task_id, &params, &org, state).await,
        "tasks/cancel" => handle_cancel(id, task_id, &org, state).await,
        "tasks/update" => handle_update(id, task_id, &params, &org, state).await,
        _ => JsonRpcResponse::method_not_found(id),
    }
}

async fn handle_get(
    id: Option<Value>,
    task_id: &str,
    params: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let mut args = json!({ "session_id": task_id });
    if let Some(since) = params.get("since_event_id") {
        args["since_event_id"] = since.clone();
    }
    if let Some(types) = params.get("event_types") {
        args["event_types"] = types.clone();
    }

    match tool_session_get_status(&args, org, state).await {
        Ok(status_json) => {
            let mut status_value: Value = serde_json::from_str(&status_json).unwrap_or(Value::Null);
            let session_status = status_value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("");
            let task_status = task_status_from_session_status(session_status);

            if let Ok(session_id) = task_id.parse::<everruns_provider::typed_id::SessionId>()
                && let Ok(Some(structured)) =
                    crate::domains::session_tasks::read_structured_task_result(
                        &state.db, org.org_id, session_id,
                    )
                    .await
                && let Some(obj) = status_value.as_object_mut()
            {
                obj.insert("structured_result".to_string(), structured);
            }

            let mut task = task_handle(task_id, task_status);
            if task_status == TaskStatus::InputRequired
                && let Ok(session_id) = task_id.parse::<everruns_provider::typed_id::SessionId>()
                && let Ok(Some(pending)) = form_elicitation::pending_questions_for_session(
                    &Caller::from(org),
                    session_id,
                    state,
                )
                .await
                && let Some(request) = form_elicitation::form_input_request(&pending.questions)
            {
                task["inputRequests"] = json!({
                    form_elicitation::ASK_USER_REQUEST_KEY: request
                });
            }
            task["result"] = status_value;
            JsonRpcResponse::success(id, task)
        }
        Err(message) => {
            let envelope = classify_mcp_execute_error(&message);
            JsonRpcResponse::success(id, error_result_payload(&message, Some(&envelope)))
        }
    }
}

async fn handle_cancel(
    id: Option<Value>,
    task_id: &str,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    match dispatch_command(
        "cancel_session",
        json!({ "session_id": task_id }),
        org,
        state,
    )
    .await
    {
        Ok(_) => {
            let task_status =
                match dispatch_command("get_session", json!({ "session_id": task_id }), org, state)
                    .await
                {
                    Ok(session) => task_status_from_session_status(
                        session.get("status").and_then(Value::as_str).unwrap_or(""),
                    ),
                    Err(_) => TaskStatus::Cancelled,
                };
            JsonRpcResponse::success(id, task_handle(task_id, task_status))
        }
        Err(message) => {
            let envelope = classify_mcp_execute_error(&message);
            JsonRpcResponse::success(id, error_result_payload(&message, Some(&envelope)))
        }
    }
}

async fn handle_update(
    id: Option<Value>,
    task_id: &str,
    params: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let input_responses = params.get("inputResponses").and_then(Value::as_object);
    if params.get("message").and_then(Value::as_str).is_none()
        && let Some(response) = input_responses
            .and_then(|responses| responses.get(form_elicitation::ASK_USER_REQUEST_KEY))
            .filter(|response| response.is_object())
    {
        return handle_question_update(id, task_id, response, org, state).await;
    }

    let message = params.get("message").and_then(Value::as_str).or_else(|| {
        input_responses.and_then(|responses| responses.values().find_map(Value::as_str))
    });
    let Some(message) = message else {
        return JsonRpcResponse::invalid_params(
            id,
            "tasks/update requires a 'message' string or an 'inputResponses' entry containing an ask_user response or string value",
        );
    };

    let args = json!({ "session_id": task_id, "message": message });
    match tool_session_send_message(&args, org, state).await {
        Ok(send_json) => {
            let parsed: Value = serde_json::from_str(&send_json).unwrap_or(Value::Null);
            let session_status = parsed
                .get("session_status")
                .and_then(Value::as_str)
                .unwrap_or("");
            let task_status = task_status_from_session_status(session_status);
            let mut task = task_handle(task_id, task_status);
            task["result"] = parsed;
            JsonRpcResponse::success(id, task)
        }
        Err(message) => {
            let envelope = classify_mcp_execute_error(&message);
            JsonRpcResponse::success(id, error_result_payload(&message, Some(&envelope)))
        }
    }
}

async fn handle_question_update(
    id: Option<Value>,
    task_id: &str,
    response: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let session_id = match task_id.parse::<everruns_provider::typed_id::SessionId>() {
        Ok(session_id) => session_id,
        Err(error) => {
            return JsonRpcResponse::invalid_params(id, format!("Invalid taskId: {error}"));
        }
    };
    let caller = Caller::from(org);
    let pending =
        match form_elicitation::pending_questions_for_session(&caller, session_id, state).await {
            Ok(Some(pending)) => pending,
            Ok(None) => {
                return JsonRpcResponse::invalid_params(
                    id,
                    "Task has no pending ask_user input request",
                );
            }
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
        session_id,
        Some(&pending.tool_call_id),
        status,
        &answers,
    )
    .await
    {
        Ok(_) => handle_get(id, task_id, &json!({}), org, state).await,
        Err(ResolveError::Invalid(detail)) => JsonRpcResponse::invalid_params(id, detail),
        Err(ResolveError::AlreadyResolved) => {
            JsonRpcResponse::invalid_params(id, "This question set has already been answered")
        }
        Err(ResolveError::NotWaiting(detail)) => JsonRpcResponse::invalid_params(
            id,
            format!("Task is not waiting for input (current status: {detail})"),
        ),
        Err(ResolveError::NoPendingQuestions | ResolveError::WrongPendingCall) => {
            JsonRpcResponse::invalid_params(id, "Task has no matching pending question set")
        }
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

    #[test]
    fn status_mapping_covers_session_states() {
        assert_eq!(
            task_status_from_session_status("started"),
            TaskStatus::Working
        );
        assert_eq!(
            task_status_from_session_status("active"),
            TaskStatus::Working
        );
        assert_eq!(
            task_status_from_session_status("waiting_for_tool_results"),
            TaskStatus::InputRequired
        );
        assert_eq!(
            task_status_from_session_status("waitingfortoolresults"),
            TaskStatus::InputRequired
        );
        assert_eq!(
            task_status_from_session_status("paused"),
            TaskStatus::InputRequired
        );
        assert_eq!(
            task_status_from_session_status("idle"),
            TaskStatus::Completed
        );
        assert_eq!(
            task_status_from_session_status("failed"),
            TaskStatus::Failed
        );
        assert_eq!(
            task_status_from_session_status("cancelled"),
            TaskStatus::Cancelled
        );
    }

    #[test]
    fn unknown_status_defaults_to_working() {
        assert_eq!(
            task_status_from_session_status("something_new"),
            TaskStatus::Working
        );
    }

    #[test]
    fn status_strings_match_sep_2663_vocabulary() {
        assert_eq!(TaskStatus::Working.as_str(), "working");
        assert_eq!(TaskStatus::InputRequired.as_str(), "input_required");
        assert_eq!(TaskStatus::Completed.as_str(), "completed");
        assert_eq!(TaskStatus::Failed.as_str(), "failed");
        assert_eq!(TaskStatus::Cancelled.as_str(), "cancelled");
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
        // Right protocol, no opt-in.
        assert!(!tasks_enabled(MCP_PROTOCOL_VERSION_LATEST, &json!({})));
        // Opted in, but 2025-* protocol never exposes the extension.
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
    fn create_task_result_uses_session_id_as_task_id() {
        let result = create_task_result("session_abc", "started");
        assert_eq!(result["resultType"], "task");
        assert_eq!(result["taskId"], "session_abc");
        assert_eq!(result["status"], "working");
        assert!(result.get("ttlMs").is_some());
        assert!(result.get("pollIntervalMs").is_some());
        assert!(result.get("ttlSeconds").is_none());
        assert!(result.get("pollIntervalMilliseconds").is_none());
    }
}
