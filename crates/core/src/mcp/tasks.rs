//! MCP Tasks extension (`io.modelcontextprotocol/tasks`, `2026-07-28`), client
//! side.
//!
//! A server that supports Tasks may answer a `tools/call` with a
//! `CreateTaskResult` (`resultType: "task"`) instead of the tool result, when
//! the request declared the extension. The client then polls `tasks/get` until
//! the task reaches a terminal state, answers `input_required` tasks with
//! `tasks/update`, and sends `tasks/cancel` when it stops waiting.
//!
//! Spec: https://modelcontextprotocol.github.io/ext-tasks/specification/2026-07-28/tasks.html
//!
//! Decision: the turn waits for the task inline, as it waits for any other tool
//! call. A task only replaces one long HTTP request with short polls, so the
//! per-request timeout stops applying to the work, and a separate limit,
//! [`TASK_WAIT_LIMIT`], bounds how long a turn waits for one task. Parking the
//! turn and resuming it from a timer is left until a server needs longer.
//!
//! Decision: the client cancels a task it stops waiting for. That covers the
//! wait limit, a question that has to go to a person (the call stands down
//! and the turn parks, see [`crate::mcp::form_elicitation`]), a declined URL
//! elicitation, and a turn that is cancelled or crashes mid-wait (the dropped
//! future cancels from its guard). A rerun of the call creates a new task and
//! finds the recorded answer there, so nothing is left running on the server
//! that nobody will collect.
//!
//! Decision: the server's `statusMessage` becomes the tool call's
//! `tool.progress` ([`crate::report_mcp_call_progress`]) each time it changes,
//! so the chat tool card and the session's events show where a long task is.
//!
//! Decision: a task id is sent back exactly as received, as the `taskId` param
//! and the `Mcp-Name` header the HTTP binding requires on `tasks/*`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::mcp::auth::McpCredential;
use crate::mcp::http::{InputHandlers, gather_input_responses, parse_rpc_result, send_op};
use crate::mcp::protocol::{self, ClientCapabilities, InputRequired, Negotiated};
use crate::mcp::result::extract_json_from_response;
use crate::{EgressService, McpToolCallResult};

/// Extension key, in `capabilities.extensions` on both sides.
pub const TASKS_EXTENSION_KEY: &str = "io.modelcontextprotocol/tasks";

/// Longest a turn waits for one task: the same ceiling a server's own
/// `tool_timeout_ms` can reach, since a task is still one tool call.
pub const TASK_WAIT_LIMIT: Duration =
    Duration::from_millis(crate::mcp_server::MCP_TOOL_TIMEOUT_MAX_MS);

/// Poll interval when the server suggests none.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Bounds on a server's `pollIntervalMs`: faster than this is load nobody
/// needs, slower would leave a finished task unnoticed for too long.
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// A task a server created for a `tools/call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedTask {
    pub task_id: String,
    pub poll_interval: Duration,
}

/// Read a `CreateTaskResult` from a `tools/call` response body, if that is
/// what it is. Returns `None` for any other result.
pub fn created_task_from_response(body: &str) -> Option<CreatedTask> {
    let value: Value = serde_json::from_str(extract_json_from_response(body)?).ok()?;
    let result = value.get("result")?;
    if result.get("resultType")?.as_str()? != "task" {
        return None;
    }
    let task_id = result.get("taskId")?.as_str()?.to_string();
    Some(CreatedTask {
        task_id,
        poll_interval: poll_interval(result, DEFAULT_POLL_INTERVAL),
    })
}

/// The server's suggested poll interval, bounded, or `fallback` when absent.
pub fn poll_interval(task: &Value, fallback: Duration) -> Duration {
    task.get("pollIntervalMs")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .unwrap_or(fallback)
        .clamp(MIN_POLL_INTERVAL, MAX_POLL_INTERVAL)
}

/// Where a task stands, from a `tasks/get` result.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    Working {
        status_message: Option<String>,
    },
    InputRequired {
        input: InputRequired,
        status_message: Option<String>,
    },
    /// The tool's result, a `CallToolResult`.
    Completed(Value),
    /// A JSON-RPC error the server ran into: its message.
    Failed(String),
    Cancelled {
        status_message: Option<String>,
    },
}

/// Read a task's state from a `tasks/get` result.
pub fn task_state(task: &Value) -> Result<TaskState> {
    let status_message = task
        .get("statusMessage")
        .and_then(Value::as_str)
        .map(str::to_string);
    let status = task
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("MCP task has no status"))?;
    Ok(match status {
        "working" => TaskState::Working { status_message },
        "input_required" => TaskState::InputRequired {
            input: InputRequired {
                requests: protocol::input_requests(task),
                request_state: None,
            },
            status_message,
        },
        "completed" => TaskState::Completed(
            task.get("result")
                .cloned()
                .ok_or_else(|| anyhow!("MCP task completed without a result"))?,
        ),
        "failed" => TaskState::Failed(
            task.get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(status_message)
                .unwrap_or_else(|| "the task failed".to_string()),
        ),
        "cancelled" => TaskState::Cancelled { status_message },
        other => return Err(anyhow!("MCP task has an unknown status '{other}'")),
    })
}

/// Everything a task's requests go out with: the connection the `tools/call`
/// used, owned so the cancel guard can still send after the call is gone.
#[derive(Clone)]
pub(super) struct TaskConnection {
    pub egress: Arc<dyn EgressService>,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub credential: Option<McpCredential>,
    pub negotiated: Negotiated,
    pub capabilities: ClientCapabilities,
    pub request_timeout: Duration,
}

impl TaskConnection {
    async fn request(&self, id: i64, method: &str, task_id: &str, params: Value) -> Result<Value> {
        let mut params = params;
        params["taskId"] = json!(task_id);
        let body = serde_json::to_vec(&protocol::request_body(
            id,
            method,
            &params,
            &self.negotiated.version,
            self.capabilities,
        ))?;
        let response = send_op(
            self.egress.as_ref(),
            &self.url,
            &self.headers,
            self.credential.as_ref(),
            &self.negotiated,
            method,
            Some(task_id),
            body,
            self.request_timeout,
            None,
        )
        .await?;
        if !(200..300).contains(&response.status) {
            return Err(crate::mcp::McpHttpStatusError {
                status: response.status,
                body: response.body,
            }
            .into());
        }
        parse_rpc_result(&response.body)
    }
}

/// Cancels the task when dropped while armed: an early return, an error, or
/// the turn dropping the call's future.
struct CancelGuard {
    connection: Option<TaskConnection>,
    task_id: String,
}

impl CancelGuard {
    async fn cancel(mut self) {
        if let Some(connection) = self.connection.take() {
            send_cancel(connection, self.task_id.clone()).await;
        }
    }

    fn disarm(mut self) {
        self.connection = None;
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        let task_id = std::mem::take(&mut self.task_id);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(send_cancel(connection, task_id));
        }
    }
}

async fn send_cancel(connection: TaskConnection, task_id: String) {
    if let Err(error) = connection
        .request(0, "tasks/cancel", &task_id, json!({}))
        .await
    {
        tracing::debug!(url = %connection.url, error = %error, "MCP tasks/cancel failed");
    }
}

/// Wait up to `wait_limit` for a task to finish and return the tool's result.
pub(super) async fn await_task(
    connection: TaskConnection,
    task: CreatedTask,
    server_name: &str,
    tool_name: &str,
    handlers: InputHandlers<'_>,
    wait_limit: Duration,
) -> Result<McpToolCallResult> {
    let guard = CancelGuard {
        connection: Some(connection.clone()),
        task_id: task.task_id.clone(),
    };
    let deadline = Instant::now() + wait_limit;
    let mut interval = task.poll_interval;
    // A key is answered once: the server may still show it on the next poll
    // before it has taken the answer in.
    let mut answered: HashSet<String> = HashSet::new();
    // Progress is reported when the server's status message changes, not on
    // every poll, so a task that says "Building" for ten minutes is one event.
    let mut last_status_message: Option<String> = None;
    let mut id: i64 = 2;

    loop {
        let now = Instant::now();
        if now >= deadline {
            guard.cancel().await;
            return Err(anyhow!(
                "MCP tool '{tool_name}' on server '{server_name}' was still running after {} \
                 minutes; Everruns cancelled the task",
                wait_limit.as_secs() / 60
            ));
        }
        tokio::time::sleep(interval.min(deadline - now)).await;

        let result = connection
            .request(id, "tasks/get", &task.task_id, json!({}))
            .await?;
        id += 1;
        interval = poll_interval(&result, interval);
        match task_state(&result)? {
            TaskState::Working { status_message } => {
                report_change(&mut last_status_message, status_message).await;
            }
            TaskState::InputRequired {
                input,
                status_message,
            } => {
                report_change(&mut last_status_message, status_message).await;
                let pending = InputRequired {
                    requests: input
                        .requests
                        .into_iter()
                        .filter(|request| !answered.contains(&request.key))
                        .collect(),
                    request_state: None,
                };
                if pending.requests.is_empty() {
                    continue;
                }
                let responses: BTreeMap<String, Value> = match gather_input_responses(
                    &pending,
                    server_name,
                    tool_name,
                    handlers,
                    &connection.url,
                )
                .await
                {
                    Ok(responses) => responses,
                    Err(error) => {
                        guard.cancel().await;
                        return Err(error);
                    }
                };
                connection
                    .request(
                        id,
                        "tasks/update",
                        &task.task_id,
                        json!({ "inputResponses": responses }),
                    )
                    .await?;
                id += 1;
                answered.extend(responses.into_keys());
            }
            TaskState::Completed(result) => {
                guard.disarm();
                return serde_json::from_value(result).map_err(|error| {
                    anyhow!("MCP tool '{tool_name}' returned a task result that is not a tool result: {error}")
                });
            }
            TaskState::Failed(message) => {
                guard.disarm();
                return Err(anyhow!("MCP tool '{tool_name}' failed: {message}"));
            }
            TaskState::Cancelled { status_message } => {
                guard.disarm();
                return Err(anyhow!(
                    "MCP tool '{tool_name}' was cancelled by server '{server_name}'{}",
                    status_message
                        .map(|message| format!(": {message}"))
                        .unwrap_or_default()
                ));
            }
        }
    }
}

/// Forward a task's status message as the tool call's progress when it
/// differs from the last one forwarded.
async fn report_change(last: &mut Option<String>, status_message: Option<String>) {
    let Some(message) = status_message.filter(|message| !message.trim().is_empty()) else {
        return;
    };
    if last.as_deref() == Some(message.as_str()) {
        return;
    }
    crate::report_mcp_call_progress(&message).await;
    *last = Some(message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_create_task_result() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"task","taskId":"t-1",
            "status":"working","createdAt":"2026-10-23T00:00:00Z",
            "lastUpdatedAt":"2026-10-23T00:00:00Z","ttlMs":60000,"pollIntervalMs":2000}}"#;
        assert_eq!(
            created_task_from_response(body),
            Some(CreatedTask {
                task_id: "t-1".to_string(),
                poll_interval: Duration::from_secs(2),
            })
        );
        let complete =
            r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","content":[]}}"#;
        assert_eq!(created_task_from_response(complete), None);
    }

    #[test]
    fn poll_interval_is_bounded() {
        let fallback = Duration::from_secs(1);
        assert_eq!(poll_interval(&json!({}), fallback), fallback);
        assert_eq!(
            poll_interval(&json!({ "pollIntervalMs": 1 }), fallback),
            MIN_POLL_INTERVAL
        );
        assert_eq!(
            poll_interval(&json!({ "pollIntervalMs": 86_400_000u64 }), fallback),
            MAX_POLL_INTERVAL
        );
    }

    #[test]
    fn reads_each_task_status() {
        assert_eq!(
            task_state(&json!({ "status": "working", "statusMessage": "Building" })).unwrap(),
            TaskState::Working {
                status_message: Some("Building".to_string())
            }
        );
        let TaskState::InputRequired { input, .. } = task_state(&json!({
            "status": "input_required",
            "inputRequests": { "k": { "method": "elicitation/create", "params": {} } }
        }))
        .unwrap() else {
            panic!("expected input_required");
        };
        assert_eq!(input.keys(), vec!["k".to_string()]);
        assert_eq!(
            task_state(&json!({ "status": "completed", "result": { "content": [] } })).unwrap(),
            TaskState::Completed(json!({ "content": [] }))
        );
        assert_eq!(
            task_state(
                &json!({ "status": "failed", "error": { "code": -32603, "message": "boom" } })
            )
            .unwrap(),
            TaskState::Failed("boom".to_string())
        );
        assert!(task_state(&json!({ "status": "completed" })).is_err());
        assert!(task_state(&json!({ "status": "paused" })).is_err());
    }
}
