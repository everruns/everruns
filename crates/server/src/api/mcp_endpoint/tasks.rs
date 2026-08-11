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
//   `tasks/update` (provide input)  ↔ `session_send_message`
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
        "waiting_for_tool_results" | "paused" => TaskStatus::InputRequired,
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
