// Subagent Capability
//
// Decision: 1 creation tool — spawn_subagent.
// - spawn_subagent creates a child session with parent_session_id set
//
// Blueprint support: spawn_subagent accepts optional `blueprint` and `config`
// params. When blueprint is set, the child session uses the blueprint's
// RuntimeAgent (own prompt, tools, model) instead of inheriting parent's.
//
// Foreground mode: blocks until subagent completes (send_message + wait_for_idle)
// Background mode: returns immediately (deferred to Phase 1b)
//
// Subagent naming: human-readable ("Test Runner"), unique per parent, case-insensitive.
// Nesting prevention: rejects spawn if current session has parent_session_id set.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::platform_store::PlatformStore;
use crate::session_task::{
    CreateSessionTask, SessionTask, SessionTaskFilter, SessionTaskState, SessionTaskUpdate,
    TASK_KIND_SUBAGENT, TaskError, TaskExecutor, TaskExecutorPlugin, TaskLinks, TaskMessage,
    TaskWakePolicy, task_message_text,
};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SpawnClaimResult, ToolContext};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub const SUBAGENTS_CAPABILITY_ID: &str = "subagents";

/// Subagent capability — spawn and manage child agent sessions.
pub struct SubagentCapability;

impl Capability for SubagentCapability {
    fn id(&self) -> &str {
        SUBAGENTS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Subagents"
    }

    fn description(&self) -> &str {
        "Spawn and manage subagents for parallel task execution in isolated context windows."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Субагенти",
            "Запускайте субагентів і керуйте ними для паралельного виконання завдань в ізольованих контекстних вікнах.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("git-branch")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["subagents"]
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SUBAGENT_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(SpawnSubagentTool)]
    }
}

const SUBAGENT_SYSTEM_PROMPT: &str = "Spawn subagents only for independent workstreams that benefit from parallelism or a separate context window. Do not delegate immediate sequential steps you can complete directly. No nested subagents. Use blueprints for specialist agents with their own tools and model.";

fn terminal_subagent_status(wait_status: &str) -> Option<crate::session::SubagentStatus> {
    match wait_status {
        "idle" | "completed" => Some(crate::session::SubagentStatus::Completed),
        "error" | "failed" => Some(crate::session::SubagentStatus::Failed),
        "cancelled" => Some(crate::session::SubagentStatus::Cancelled),
        "max_iterations_reached" => Some(crate::session::SubagentStatus::MaxIterationsReached),
        _ => None,
    }
}

fn terminal_subagent_task_state(
    subagent_status: &crate::session::SubagentStatus,
) -> SessionTaskState {
    match subagent_status {
        crate::session::SubagentStatus::Completed => SessionTaskState::Succeeded,
        crate::session::SubagentStatus::Cancelled => SessionTaskState::Canceled,
        _ => SessionTaskState::Failed,
    }
}

// =============================================================================
// Helper: get platform store from context
// =============================================================================

fn get_platform_store(context: &ToolContext) -> Result<&dyn PlatformStore, ToolExecutionResult> {
    context
        .platform_store
        .as_ref()
        .map(|s| s.as_ref())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "Subagent tools require platform_store context (not available in this environment)",
            )
        })
}

fn get_session_store(
    context: &ToolContext,
) -> Result<&dyn crate::traits::SessionStore, ToolExecutionResult> {
    context
        .session_store
        .as_ref()
        .map(|s| s.as_ref())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error("Subagent tools require session_store context")
        })
}

fn require_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {field}"))
        })
}

/// Extract the last assistant/agent message content from a list of messages.
fn last_agent_message(messages: &[crate::platform_store::PlatformMessage]) -> Option<String> {
    messages
        .iter()
        .rfind(|m| m.role == "agent" || m.role == "assistant")
        .map(|m| m.content.clone())
}

/// Truncated human summary stored on the subagent's task record.
const MAX_TASK_SUMMARY_CHARS: usize = 2_048;

fn truncate_summary(text: &str) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(MAX_TASK_SUMMARY_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}\n[truncated]")
    } else {
        truncated
    }
}

/// Mirror a terminal outcome onto the subagent's session task (best-effort;
/// tolerates a missing registry or task).
async fn finish_subagent_task(
    context: &ToolContext,
    task_id: Option<&str>,
    state: SessionTaskState,
    summary: Option<String>,
    error: Option<TaskError>,
) {
    let (Some(registry), Some(task_id)) = (context.session_task_registry.as_ref(), task_id) else {
        return;
    };
    let _ = registry
        .update(
            context.session_id,
            task_id,
            SessionTaskUpdate {
                state: Some(state),
                summary,
                error,
                ..Default::default()
            },
        )
        .await;
}

/// Find the session task tracking a subagent by its child session id.
async fn find_subagent_task(
    context: &ToolContext,
    child_id: crate::typed_id::SessionId,
) -> Option<SessionTask> {
    let registry = context.session_task_registry.as_ref()?;
    let tasks = registry
        .list(
            context.session_id,
            Some(&SessionTaskFilter {
                kind: Some(TASK_KIND_SUBAGENT.to_string()),
                state: None,
            }),
        )
        .await
        .ok()?;
    tasks
        .into_iter()
        .find(|task| task.links.child_session_id == Some(child_id))
}

// =============================================================================
// Tool: spawn_subagent
// =============================================================================

pub struct SpawnSubagentTool;

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Subagent")
    }

    fn description(&self) -> &str {
        "Spawn a named subagent to handle a specific task in its own context window. Use `blueprint` to spawn a specialist agent with its own tools and model."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable name for the subagent (e.g. 'Test Runner', 'Auth Explorer'). Must be unique within this session."
                },
                "instructions": {
                    "type": "string",
                    "description": "Instructions for the subagent — what it should do."
                },
                "blueprint": {
                    "type": "string",
                    "description": "Blueprint ID to spawn a specialist agent with its own tools and model. Omit to inherit parent's configuration."
                },
                "config": {
                    "type": "object",
                    "description": "Blueprint-specific configuration. Only valid when `blueprint` is set. Validated against the blueprint's config schema."
                }
            },
            "required": ["name", "instructions"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "spawn_subagent requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = match get_platform_store(context) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let session_store = match get_session_store(context) {
            Ok(s) => s,
            Err(e) => return e,
        };

        let name = match require_str(&arguments, "name") {
            Ok(s) => s.trim().to_string(),
            Err(e) => return e,
        };
        let instructions = match require_str(&arguments, "instructions") {
            Ok(s) => s.to_string(),
            Err(e) => return e,
        };

        let blueprint_param = arguments
            .get("blueprint")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());
        let config_param = arguments.get("config").filter(|v| !v.is_null()).cloned();

        // Reject config without blueprint
        if config_param.is_some() && blueprint_param.is_none() {
            return ToolExecutionResult::tool_error(
                "The `config` parameter is only valid when `blueprint` is set.",
            );
        }

        // Nesting check: reject if current session is already a subagent
        let parent_session = match session_store.get_session(context.session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => return ToolExecutionResult::tool_error("Current session not found"),
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        if parent_session.parent_session_id.is_some() {
            return ToolExecutionResult::tool_error(
                "Subagents cannot spawn other subagents (nesting not allowed).",
            );
        }

        // Validate blueprint exists and is allowed for this parent session.
        if let Some(ref bp_id) = blueprint_param {
            let Some(ref registry) = context.capability_registry else {
                return ToolExecutionResult::tool_error(
                    "Blueprint support requires capability_registry context.",
                );
            };

            let Some((blueprint_capability_id, blueprint)) =
                registry.blueprint_with_capability(bp_id)
            else {
                return ToolExecutionResult::tool_error(format!(
                    "Unknown blueprint: \"{bp_id}\". Check available blueprints."
                ));
            };

            // Validate config against schema if blueprint has one.
            if let Some(ref schema) = blueprint.config_schema
                && config_param.is_none()
                && schema
                    .get("required")
                    .is_some_and(|r| r.as_array().is_some_and(|arr| !arr.is_empty()))
            {
                return ToolExecutionResult::tool_error(format!(
                    "Blueprint \"{bp_id}\" requires config. Schema: {}",
                    serde_json::to_string_pretty(schema).unwrap_or_default()
                ));
            }

            let allowed_capability_ids = if let Some(agent_id) = parent_session.agent_id {
                match store.get_agent_by_id(agent_id).await {
                    Ok(Some(agent)) => agent
                        .capabilities
                        .iter()
                        .map(|c| c.capability_id().to_string())
                        .collect::<Vec<_>>(),
                    Ok(None) => vec![],
                    Err(e) => return ToolExecutionResult::internal_error(e),
                }
            } else {
                match store.get_harness(parent_session.harness_id).await {
                    Ok(Some(harness)) => harness
                        .capabilities
                        .iter()
                        .map(|c| c.capability_id().to_string())
                        .collect::<Vec<_>>(),
                    Ok(None) => vec![],
                    Err(e) => return ToolExecutionResult::internal_error(e),
                }
            };

            if !allowed_capability_ids
                .iter()
                .any(|capability_id| capability_id == &blueprint_capability_id)
            {
                return ToolExecutionResult::tool_error(format!(
                    "Blueprint \"{bp_id}\" is not enabled for this session."
                ));
            }
        }

        // --- Durable spawn handle claim (EVE-535) ---
        //
        // When a spawn store and tool_call_id are available, attempt to claim a
        // spawn slot before creating the child session.  On reclaim, this lets us
        // reattach to the existing child instead of spawning a duplicate.
        if let (Some(spawn_store), Some(tool_call_id)) =
            (&context.subagent_spawn_store, &context.tool_call_id)
        {
            let claim_token = uuid::Uuid::new_v4();

            let claim = match spawn_store
                .try_claim_spawn(
                    context.session_id,
                    tool_call_id,
                    &name,
                    &instructions,
                    claim_token,
                )
                .await
            {
                Ok(c) => c,
                Err(e) => return ToolExecutionResult::internal_error(e),
            };

            match claim {
                SpawnClaimResult::AlreadySettled {
                    child_session_id,
                    terminal_status,
                    terminal_result,
                } => {
                    // Already settled on a previous execution: return stored result.
                    let task_id = find_subagent_task(context, child_session_id)
                        .await
                        .map(|t| t.id);
                    return ToolExecutionResult::success(json!({
                        "subagent_id": child_session_id.to_string(),
                        "name": name,
                        "status": terminal_status,
                        "result": terminal_result,
                        "task_id": task_id,
                        "blueprint": blueprint_param,
                    }));
                }
                SpawnClaimResult::AlreadyRunning {
                    child_session_id,
                    claim_token: stored_claim_token,
                } => {
                    // Child was spawned before but hasn't settled yet — reattach.
                    // Use the stored claim_token so settle succeeds on this replay.
                    let task_id = find_subagent_task(context, child_session_id)
                        .await
                        .map(|t| t.id);
                    return run_subagent_wait_and_settle(
                        store,
                        context,
                        child_session_id,
                        &name,
                        &instructions,
                        &blueprint_param,
                        task_id,
                        Some((
                            spawn_store.as_ref(),
                            tool_call_id.as_str(),
                            stored_claim_token,
                        )),
                    )
                    .await;
                }
                SpawnClaimResult::Claimed {
                    spawn_handle_id,
                    claim_token: actual_claim_token,
                }
                | SpawnClaimResult::ClaimedPendingChild {
                    spawn_handle_id,
                    claim_token: actual_claim_token,
                } => {
                    // First claim (or re-claim after crash before register):
                    // create child and register it durably before waiting.
                    return spawn_create_and_wait(
                        store,
                        context,
                        &parent_session,
                        &name,
                        &instructions,
                        &blueprint_param,
                        &config_param,
                        Some((
                            spawn_store.as_ref(),
                            tool_call_id.as_str(),
                            spawn_handle_id,
                            actual_claim_token,
                        )),
                    )
                    .await;
                }
            }
        }

        // --- No-spawn-store path (dev / noop) ---
        spawn_create_and_wait(
            store,
            context,
            &parent_session,
            &name,
            &instructions,
            &blueprint_param,
            &config_param,
            None,
        )
        .await
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Helpers for SpawnSubagentTool
// =============================================================================

/// Create a new child session, send the instructions, wait for completion, and
/// settle the spawn handle (if a settle context is supplied).
///
/// `settle_ctx` = (spawn_store, tool_call_id, spawn_handle_id, claim_token).
/// `spawn_handle_id` is used to call `register_child_session` after child creation.
#[allow(clippy::too_many_arguments)]
async fn spawn_create_and_wait(
    store: &dyn PlatformStore,
    context: &ToolContext,
    parent_session: &crate::session::Session,
    name: &str,
    instructions: &str,
    blueprint_param: &Option<String>,
    config_param: &Option<Value>,
    settle_ctx: Option<(
        &dyn crate::traits::SubagentSpawnStore,
        &str,
        uuid::Uuid,
        uuid::Uuid,
    )>,
) -> ToolExecutionResult {
    // Create child session
    let child_session = match store
        .create_session(
            parent_session.harness_id,
            if blueprint_param.is_some() {
                None // Blueprint sessions don't inherit agent
            } else {
                parent_session.agent_id
            },
            Some(name),
            parent_session.locale.as_deref(),
            blueprint_param.as_deref(),
            config_param.as_ref(),
        )
        .await
    {
        Ok(s) => s,
        Err(e) => return ToolExecutionResult::internal_error(e),
    };
    let child_session = match store
        .set_subagent_metadata(
            child_session.id,
            context.session_id,
            name,
            instructions,
            crate::session::SubagentStatus::Running,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => return ToolExecutionResult::internal_error(e),
    };

    // Create the session task tracking this subagent (specs/session-tasks.md).
    let mut task_id: Option<String> = None;
    if let Some(ref task_registry) = context.session_task_registry
        && let Ok(created) = task_registry
            .create(CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: name.to_string(),
                spec: json!({
                    "instructions": instructions,
                    "blueprint_id": blueprint_param,
                }),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_session.id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
    {
        task_id = Some(created.id);
    }

    // Register child session ID durably BEFORE waiting.
    // This is the durability boundary: once registered, a reclaim/replay can
    // reattach to this child instead of spawning another.
    let wait_settle_ctx = if let Some((spawn_store, tool_call_id, spawn_handle_id, claim_token)) =
        settle_ctx
    {
        if let Err(e) = spawn_store
            .register_child_session(spawn_handle_id, claim_token, child_session.id)
            .await
        {
            tracing::warn!(
                tool_call_id,
                error = %e,
                "Failed to register child session in spawn handle; proceeding without durable reattach"
            );
        }
        Some((spawn_store, tool_call_id, claim_token))
    } else {
        None
    };

    // Send the instructions as the first message
    if let Err(e) = store.send_message(child_session.id, instructions).await {
        finish_subagent_task(
            context,
            task_id.as_deref(),
            SessionTaskState::Failed,
            None,
            Some(TaskError {
                kind: "error".to_string(),
                message: e.to_string(),
            }),
        )
        .await;
        return ToolExecutionResult::internal_error(e);
    }

    run_subagent_wait_and_settle(
        store,
        context,
        child_session.id,
        name,
        instructions,
        blueprint_param,
        task_id,
        wait_settle_ctx,
    )
    .await
}

/// Wait for a child session to reach idle, collect its result, update the
/// registry, and settle the spawn handle (if a settle context is supplied).
#[allow(clippy::too_many_arguments)]
async fn run_subagent_wait_and_settle(
    store: &dyn PlatformStore,
    context: &ToolContext,
    child_id: crate::typed_id::SessionId,
    name: &str,
    instructions: &str,
    blueprint_param: &Option<String>,
    task_id: Option<String>,
    settle_ctx: Option<(&dyn crate::traits::SubagentSpawnStore, &str, uuid::Uuid)>,
) -> ToolExecutionResult {
    // Foreground mode: wait for completion
    let status = match store.wait_for_idle(child_id, Some(300)).await {
        Ok(s) => s,
        Err(e) => {
            finish_subagent_task(
                context,
                task_id.as_deref(),
                SessionTaskState::Failed,
                None,
                Some(TaskError {
                    kind: "error".to_string(),
                    message: e.to_string(),
                }),
            )
            .await;
            return ToolExecutionResult::success(json!({
                "subagent_id": child_id.to_string(),
                "name": name,
                "status": "failed",
                "error": e.to_string(),
                "task_id": task_id,
                "blueprint": blueprint_param,
            }));
        }
    };

    // Get the subagent's response messages
    let messages = match store.get_messages(child_id, Some(5)).await {
        Ok(m) => m,
        Err(e) => return ToolExecutionResult::internal_error(e),
    };

    let result_text = last_agent_message(&messages)
        .unwrap_or_else(|| format!("Subagent completed with status: {status}"));

    let terminal_status = terminal_subagent_status(&status);

    // Settle the spawn handle only when the child reached a terminal state.
    // Non-terminal waits must stay reattachable on replay.
    if let Some((spawn_store, tool_call_id, claim_token)) = settle_ctx
        && terminal_status.is_some()
        && let Err(e) = spawn_store
            .settle_spawn(
                context.session_id,
                tool_call_id,
                claim_token,
                &status,
                &result_text,
            )
            .await
    {
        // Best-effort: log but don't fail the tool execution.
        tracing::warn!(
            tool_call_id,
            error = %e,
            "Failed to settle subagent spawn handle"
        );
    }

    // Persist metadata and update the session task only when the child reached a
    // terminal state. Non-terminal results from wait_for_idle (paused,
    // waiting_for_tool_results, timeout) leave the child Active.
    if let Some(subagent_status) = terminal_status {
        let task_state = terminal_subagent_task_state(&subagent_status);
        if let Err(e) = store
            .set_subagent_metadata(
                child_id,
                context.session_id,
                name,
                instructions,
                subagent_status,
            )
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                child_session_id = %child_id,
                error = %e,
                "failed to persist terminal subagent metadata"
            );
        }
        let task_error = if task_state == SessionTaskState::Failed {
            Some(TaskError {
                kind: status.clone(),
                message: format!("Subagent session ended with status: {status}"),
            })
        } else {
            None
        };
        finish_subagent_task(
            context,
            task_id.as_deref(),
            task_state,
            Some(truncate_summary(&result_text)),
            task_error,
        )
        .await;
    }

    ToolExecutionResult::success(json!({
        "subagent_id": child_id.to_string(),
        "name": name,
        "status": status,
        "result": result_text,
        "task_id": task_id,
        "blueprint": blueprint_param,
    }))
}

// =============================================================================
// Task executor: subagent
// =============================================================================

/// Control plane for `subagent` tasks. Inbound messages and cooperative
/// cancellation route through the child session's message channel — there is
/// no hard kill, so cancel delivers a graceful stop request via `cancel_task`.
pub struct SubagentTaskExecutor;

#[async_trait]
impl TaskExecutor for SubagentTaskExecutor {
    fn kind(&self) -> &str {
        TASK_KIND_SUBAGENT
    }

    async fn deliver(
        &self,
        task: &SessionTask,
        message: &TaskMessage,
        context: &ToolContext,
    ) -> crate::error::Result<()> {
        let Some(store) = context.platform_store.as_ref() else {
            return Err(crate::error::AgentLoopError::tool(
                "subagent task delivery requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(crate::error::AgentLoopError::tool(format!(
                "subagent task {} has no child session link",
                task.id
            )));
        };
        let text = task_message_text(&message.content);
        store.send_message(child_id, &text).await
    }

    async fn cancel(&self, task: &SessionTask, context: &ToolContext) -> crate::error::Result<()> {
        let Some(store) = context.platform_store.as_ref() else {
            return Err(crate::error::AgentLoopError::tool(
                "subagent task cancellation requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(crate::error::AgentLoopError::tool(format!(
                "subagent task {} has no child session link",
                task.id
            )));
        };
        // Graceful stop request; takes effect after the current turn.
        store
            .send_message(
                child_id,
                "Cancellation requested by the parent session. Stop work, wind down, and reply with a brief summary of progress so far.",
            )
            .await
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(SubagentTaskExecutor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tool;

    #[test]
    fn capability_basics() {
        let cap = SubagentCapability;
        assert_eq!(cap.id(), "subagents");
        assert_eq!(cap.tools().len(), 1);
        assert!(cap.system_prompt_addition().is_some());
        assert_eq!(cap.features(), vec!["subagents"]);
    }

    #[test]
    fn tool_names() {
        let cap = SubagentCapability;
        let tools = cap.tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, vec!["spawn_subagent"]);
    }

    #[test]
    fn terminal_subagent_status_maps_only_terminal_wait_states() {
        assert_eq!(
            terminal_subagent_status("idle"),
            Some(crate::session::SubagentStatus::Completed)
        );
        assert_eq!(
            terminal_subagent_status("failed"),
            Some(crate::session::SubagentStatus::Failed)
        );
        assert_eq!(
            terminal_subagent_status("cancelled"),
            Some(crate::session::SubagentStatus::Cancelled)
        );
        assert_eq!(
            terminal_subagent_task_state(&crate::session::SubagentStatus::Completed),
            SessionTaskState::Succeeded
        );
        assert_eq!(
            terminal_subagent_task_state(&crate::session::SubagentStatus::Cancelled),
            SessionTaskState::Canceled
        );
        assert_eq!(
            terminal_subagent_task_state(&crate::session::SubagentStatus::MaxIterationsReached),
            SessionTaskState::Failed
        );
        assert_eq!(terminal_subagent_status("waiting_for_tool_results"), None);
        assert_eq!(terminal_subagent_status("paused"), None);
    }

    #[test]
    fn spawn_subagent_schema_has_required_fields() {
        let tool = SpawnSubagentTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("name")));
        assert!(required.contains(&json!("instructions")));
    }

    #[test]
    fn spawn_subagent_schema_has_blueprint_fields() {
        let tool = SpawnSubagentTool;
        let schema = tool.parameters_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("blueprint"));
        assert!(props.contains_key("config"));
        // blueprint and config should NOT be required
        let required = schema["required"].as_array().unwrap();
        assert!(!required.contains(&json!("blueprint")));
        assert!(!required.contains(&json!("config")));
    }

    #[tokio::test]
    async fn spawn_subagent_without_context_returns_error() {
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({"name": "Test", "instructions": "test"}))
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    // =========================================================================
    // Spawn handle tests (EVE-535)
    // =========================================================================

    use crate::traits::{NoopSubagentSpawnStore, SpawnClaimResult, SubagentSpawnStore};
    use std::sync::Arc;

    /// NoopSubagentSpawnStore always returns Claimed with a fresh token.
    #[tokio::test]
    async fn noop_spawn_store_always_claims() {
        let store = NoopSubagentSpawnStore;
        let parent = crate::typed_id::SessionId::new();
        let token = uuid::Uuid::new_v4();

        let result = store
            .try_claim_spawn(parent, "call-1", "Worker", "do work", token)
            .await
            .expect("noop should not error");

        assert!(
            matches!(result, SpawnClaimResult::Claimed { claim_token, .. } if claim_token == token),
            "noop store should return Claimed with the supplied token"
        );
    }

    /// NoopSubagentSpawnStore register and settle are always successful.
    #[tokio::test]
    async fn noop_spawn_store_register_and_settle_are_noops() {
        let store = NoopSubagentSpawnStore;
        let parent = crate::typed_id::SessionId::new();
        let child = crate::typed_id::SessionId::new();
        let handle_id = uuid::Uuid::new_v4();
        let token = uuid::Uuid::new_v4();

        store
            .register_child_session(handle_id, token, child)
            .await
            .expect("noop register should not error");

        store
            .settle_spawn(parent, "call-1", token, "idle", "result text")
            .await
            .expect("noop settle should not error");
    }

    /// Arc<dyn SubagentSpawnStore> blanket impl delegates correctly.
    #[tokio::test]
    async fn arc_spawn_store_delegates() {
        let store: Arc<dyn SubagentSpawnStore> = Arc::new(NoopSubagentSpawnStore);
        let parent = crate::typed_id::SessionId::new();
        let token = uuid::Uuid::new_v4();

        let result = store
            .try_claim_spawn(parent, "call-arc", "ArcWorker", "arc task", token)
            .await
            .expect("arc delegation should not error");

        assert!(matches!(result, SpawnClaimResult::Claimed { .. }));
    }
}
