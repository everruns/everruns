// Session Tasks Capability
//
// Generic agent-facing tools over the session task registry
// (knowledge/runtime-resources/session-tasks.md): list_tasks / get_task / message_task /
// cancel_task / wait_task. Spawning stays with each creation surface
// (spawn_agent, spawn_background) — every spawn creates a task and returns its
// task_id; these tools provide the uniform query/messaging/cancel/wait plane.
//
// Decision: tools declare `SessionTaskRegistry` as a hard context-service
// requirement, so production runtime assembly rejects the capability before
// model exposure when the host lacks that backend. Direct/test execution still
// returns a tool error instead of panicking.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
use everruns_core::session_task::{
    NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry, SessionTaskState,
    TaskMessage, find_task_executor,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{tool_context::ToolContext, tool_context::ToolContextService};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{Instant, sleep};

pub const SESSION_TASKS_CAPABILITY_ID: &str = "session_tasks";

const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 300;
const WAIT_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Reconcile polled task kinds (e.g. remote A2A tasks) every Nth wait poll.
const WAIT_RECONCILE_EVERY: u64 = 5;
/// Recent-thread size returned by get_task.
const GET_TASK_MESSAGE_LIMIT: u32 = 20;

/// Session tasks capability — uniform tracking of background work.
pub struct SessionTasksCapability;

impl Capability for SessionTasksCapability {
    fn id(&self) -> &str {
        SESSION_TASKS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Session Tasks"
    }

    fn description(&self) -> &str {
        "Track, message, cancel, and wait on the session's background tasks (subagents, external agents, background tools)."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Завдання сесії",
            "Відстежуйте фонові завдання сесії (субагенти, зовнішні агенти, фонові інструменти), надсилайте їм повідомлення, скасовуйте їх та очікуйте на їхнє завершення.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("list-checks")
    }

    fn category(&self) -> Option<&str> {
        Some("Orchestration")
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["session_tasks"]
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SESSION_TASKS_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ListTasksTool),
            Box::new(GetTaskTool),
            Box::new(MessageTaskTool),
            Box::new(CancelTaskTool),
            Box::new(WaitTaskTool),
        ]
    }
}

const SESSION_TASKS_SYSTEM_PROMPT: &str = "Every spawned background work item (subagent, external agent, background tool) is a task with a task_id. Use list_tasks/get_task to check status instead of re-spawning. Answer a task in awaiting_input with message_task (set in_reply_to to the pending input request id). When polling list_tasks, pass its previous snapshot as since with the same filters to avoid repeating unchanged results. Omit since to retrieve the list again. Use wait_task only when you have nothing else to do until the task finishes.";

// =============================================================================
// Helpers
// =============================================================================

fn require_task_registry(
    context: &ToolContext,
) -> Result<&Arc<dyn SessionTaskRegistry>, ToolExecutionResult> {
    context.session_task_registry.as_ref().ok_or_else(|| {
        ToolExecutionResult::tool_error(
            "Session task tools require session_task_registry context (not available in this environment)",
        )
    })
}

use super::util::require_str_trimmed as require_str;

async fn load_task(
    context: &ToolContext,
    task_id: &str,
) -> Result<SessionTask, ToolExecutionResult> {
    let registry = require_task_registry(context)?;
    registry
        .get(context.session_id, task_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
        .ok_or_else(|| ToolExecutionResult::tool_error(format!("No task found with id: {task_id}. Use list_tasks and copy the returned task ID exactly; background run IDs are separate identifiers.")))
}

// Diagnostic bodies can occupy the whole provider output budget. Keep lists
// useful for discovery; get_task and durable artifacts retain the full outcome.
fn compact_summary(summary: &str) -> String {
    const MAX_BYTES: usize = 512;
    if summary.len() <= MAX_BYTES {
        return summary.to_string();
    }
    let end = summary.floor_char_boundary(MAX_BYTES);
    format!("{} ... (full summary: get_task)", &summary[..end])
}

/// Compact list entry: enough to decide whether to drill in with get_task.
fn compact_task_json(task: &SessionTask) -> Value {
    json!({
        "id": task.id,
        "kind": task.kind,
        "display_name": task.display_name,
        "state": task.state,
        "state_detail": task.state_detail,
        "progress": task.progress,
        "input_request": task.input_request,
        "summary": task.summary.as_deref().map(compact_summary),
        "created_at": task.created_at.to_rfc3339(),
        "finished_at": task.finished_at.map(|t| t.to_rfc3339()),
    })
}

fn message_json(message: &TaskMessage) -> Value {
    serde_json::to_value(message).unwrap_or_else(|_| json!({}))
}

fn full_task_json(task: &SessionTask) -> Value {
    serde_json::to_value(task).unwrap_or_else(|_| json!({}))
}

// =============================================================================
// Tool: list_tasks
// =============================================================================

pub struct ListTasksTool;

#[async_trait]
impl Tool for ListTasksTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "list_tasks"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Tasks")
    }

    fn description(&self) -> &str {
        "List this session's background tasks (subagents, external agents, background tools) with state, progress, input request, and summary. For repeated polls, pass the previous snapshot as since with the same filters; unchanged results omit tasks. Omit since to retrieve the list again. get_task always returns full details."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "state": {
                    "type": "string",
                    "enum": ["queued", "running", "awaiting_input", "succeeded", "failed", "canceled"],
                    "description": "Filter by lifecycle state."
                },
                "kind": {
                    "type": "string",
                    "description": "Filter by task kind (e.g. 'subagent', 'external_agent', 'background_tool')."
                },
                "since": {
                    "type": "string",
                    "description": "Snapshot from the previous list_tasks result with the same filters. Returns a short unchanged response if nothing changed. Omit to always return tasks."
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_tasks requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        list_tasks_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn list_tasks_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let registry = require_task_registry(context)?;
    let state = match arguments.get("state").and_then(Value::as_str) {
        Some(raw) => match SessionTaskState::parse(raw) {
            Some(state) => Some(state),
            None => {
                return Ok(ToolExecutionResult::tool_error(format!(
                    "Unknown state filter \"{raw}\". Valid states: queued, running, \
                     awaiting_input, succeeded, failed, canceled."
                )));
            }
        },
        None => None,
    };
    let filter = SessionTaskFilter {
        kind: arguments
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        state,
    };
    let tasks = registry
        .list(context.session_id, Some(&filter))
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    // Caller-held snapshots survive worker/turn changes without hidden cache state.
    // Sort only the fingerprint: registry presentation order remains unchanged.
    // Ignore heartbeat/update timestamps, but include full outcomes so a change
    // beyond the compact summary boundary still invalidates the snapshot.
    let mut snapshots = tasks.iter().collect::<Vec<_>>();
    snapshots.sort_by(|a, b| a.id.cmp(&b.id));
    let snapshots = snapshots
        .into_iter()
        .map(|task| {
            let mut snapshot = full_task_json(task);
            if let Some(fields) = snapshot.as_object_mut() {
                fields.remove("heartbeat_at");
                fields.remove("updated_at");
            }
            snapshot
        })
        .collect::<Vec<_>>();
    let fingerprint = json!({
        "session_id": context.session_id,
        "kind": filter.kind,
        "state": filter.state,
        "tasks": snapshots,
    });
    let digest = Sha256::digest(fingerprint.to_string().as_bytes());
    let snapshot = format!(
        "v1:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    if arguments.get("since").and_then(Value::as_str) == Some(snapshot.as_str()) {
        return Ok(ToolExecutionResult::success(json!({
            "unchanged": true,
            "count": tasks.len(),
            "snapshot": snapshot,
        })));
    }
    let entries = tasks.iter().map(compact_task_json).collect::<Vec<_>>();
    Ok(ToolExecutionResult::success(json!({
        "tasks": entries,
        "count": entries.len(),
        "snapshot": snapshot,
    })))
}

// =============================================================================
// Tool: get_task
// =============================================================================

pub struct GetTaskTool;

#[async_trait]
impl Tool for GetTaskTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "get_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Task")
    }

    fn description(&self) -> &str {
        "Get a task's full snapshot (state, progress, input request, result path, error) plus its recent message thread."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Exact task ID returned by spawn or list_tasks (task_*). Do not derive it from a background run ID."
                }
            },
            "required": ["task_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("get_task requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        get_task_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn get_task_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let task_id = require_str(&arguments, "task_id")?;
    let task = load_task(context, task_id).await?;
    let messages = recent_messages_json(context, task_id).await?;
    Ok(ToolExecutionResult::success(json!({
        "task": full_task_json(&task),
        "messages": messages,
    })))
}

/// Recent message thread (oldest first) shared by get_task and cancel_task.
/// Best-effort: a failed thread read yields an empty tail, not a tool error.
async fn recent_messages_json(
    context: &ToolContext,
    task_id: &str,
) -> Result<Vec<Value>, ToolExecutionResult> {
    let registry = require_task_registry(context)?;
    let messages = registry
        .list_messages(
            context.session_id,
            task_id,
            Some(GET_TASK_MESSAGE_LIMIT),
            None,
        )
        .await
        .unwrap_or_default();
    Ok(messages.iter().map(message_json).collect())
}

// =============================================================================
// Tool: message_task
// =============================================================================

pub struct MessageTaskTool;

#[async_trait]
impl Tool for MessageTaskTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "message_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Message Task")
    }

    fn description(&self) -> &str {
        "Send an inbound message to a task. To answer a pending input request, set in_reply_to to the input request id."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Exact task ID returned by spawn or list_tasks (task_*). Do not derive it from a background run ID."
                },
                "message": {
                    "type": "string",
                    "description": "Message to deliver to the task."
                },
                "in_reply_to": {
                    "type": "string",
                    "description": "ID of the pending input request this message answers."
                }
            },
            "required": ["task_id", "message"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_stays_direct(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("message_task requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        message_task_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn message_task_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let task_id = require_str(&arguments, "task_id")?.to_string();
    let message = require_str(&arguments, "message")?.to_string();
    let in_reply_to = arguments
        .get("in_reply_to")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let task = load_task(context, &task_id).await?;
    let registry = require_task_registry(context)?;

    let mut new_message = NewTaskMessage::inbound_text(message);
    new_message.in_reply_to = in_reply_to;
    let recorded = registry
        .record_message(context.session_id, &task_id, new_message)
        .await
        .map_err(ToolExecutionResult::internal_error)?;

    // Best-effort delivery: the message is durably recorded either way;
    // report delivery outcome instead of failing the whole call.
    let delivery = match find_task_executor(&task.kind) {
        Some(executor) => {
            // Re-read: recording an input-request answer returns the task
            // to running before the executor sees it.
            let current = registry
                .get(context.session_id, &task_id)
                .await
                .ok()
                .flatten()
                .unwrap_or(task);
            match executor.deliver(&current, &recorded, context).await {
                Ok(()) => "delivered".to_string(),
                Err(e) => format!("failed: {e}"),
            }
        }
        None => format!(
            "failed: no executor registered for task kind '{}'",
            task.kind
        ),
    };

    Ok(ToolExecutionResult::success(json!({
        "task_id": task_id,
        "message_id": recorded.id,
        "recorded": true,
        "delivery": delivery,
    })))
}

// =============================================================================
// Tool: cancel_task
// =============================================================================

pub struct CancelTaskTool;

#[async_trait]
impl Tool for CancelTaskTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "cancel_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Cancel Task")
    }

    fn description(&self) -> &str {
        "Request cooperative cancellation of a task. The task winds down and may still end succeeded or failed. For a detached `session` task this also cancels the peer session (not just the tracking chip)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Exact task ID returned by spawn or list_tasks (task_*). Do not derive it from a background run ID."
                }
            },
            "required": ["task_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_stays_direct(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("cancel_task requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        cancel_task_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn cancel_task_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let task_id = require_str(&arguments, "task_id")?.to_string();
    let registry = require_task_registry(context)?;
    let task = match registry.request_cancel(context.session_id, &task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            return Ok(ToolExecutionResult::tool_error(format!(
                "No task found with id: {task_id}"
            )));
        }
        Err(e) => return Err(ToolExecutionResult::internal_error(e)),
    };

    // Best-effort executor wind-down; the intent is recorded regardless.
    let executor_result = if task.state.is_terminal() {
        "task already terminal".to_string()
    } else {
        match find_task_executor(&task.kind) {
            Some(executor) => match executor.cancel(&task, context).await {
                Ok(()) => "cancellation requested".to_string(),
                Err(e) => format!("failed: {e}"),
            },
            None => format!(
                "no executor registered for task kind '{}'; cancel intent recorded",
                task.kind
            ),
        }
    };

    // Decision (EVE-1343): re-read after the wind-down so the result carries
    // the post-cancel state (in-process executors finish synchronously), plus
    // the get_task shape (`task` + `messages` tail) so callers need no
    // follow-up get_task. Additive: `state` keeps its key but now reports the
    // re-read state; a failed re-read falls back to the request_cancel
    // snapshot because the cancel intent is already recorded.
    let current = registry
        .get(context.session_id, &task_id)
        .await
        .ok()
        .flatten()
        .unwrap_or(task);
    let messages = recent_messages_json(context, &task_id).await?;

    Ok(ToolExecutionResult::success(json!({
        "task_id": task_id,
        "state": current.state,
        "cancel_requested": true,
        "executor": executor_result,
        "task": full_task_json(&current),
        "messages": messages,
    })))
}

// =============================================================================
// Tool: wait_task
// =============================================================================

pub struct WaitTaskTool;

#[async_trait]
impl Tool for WaitTaskTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "wait_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Wait Task")
    }

    fn description(&self) -> &str {
        "Wait until a task reaches a terminal state or asks for input. Returns the latest task snapshot."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Exact task ID returned by spawn or list_tasks (task_*). Do not derive it from a background run ID."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 86400,
                    "default": 300,
                    "description": "Maximum seconds to wait before returning the current snapshot."
                }
            },
            "required": ["task_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_stays_direct(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("wait_task requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        wait_task_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn wait_task_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let task_id = require_str(&arguments, "task_id")?.to_string();
    let timeout_secs = arguments
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS);
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut polls: u64 = 0;

    loop {
        let task = load_task(context, &task_id).await?;
        if task.state.is_terminal() || task.state == SessionTaskState::AwaitingInput {
            return Ok(ToolExecutionResult::success(json!({
                "task": full_task_json(&task),
                "timed_out": false,
            })));
        }
        if Instant::now() >= deadline {
            return Ok(ToolExecutionResult::success(json!({
                "task": full_task_json(&task),
                "timed_out": true,
                "message": format!("Task {task_id} still {} after {timeout_secs}s", task.state),
            })));
        }
        polls += 1;
        // Refresh polled kinds (e.g. remote A2A tasks) periodically so the
        // registry snapshot converges even when nothing pushes updates.
        if polls.is_multiple_of(WAIT_RECONCILE_EVERY)
            && let Some(executor) = find_task_executor(&task.kind)
        {
            let _ = executor.reconcile(&task, context).await;
        }
        sleep(WAIT_POLL_INTERVAL).await;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
pub(crate) mod tests;
