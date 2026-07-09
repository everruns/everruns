// Subagent Capability
//
// Decision: 1 delegation tool — spawn_agent(target.type = "subagent").
// - subagent delegation creates a child session with parent_session_id set
//
// Blueprint support: the subagent target accepts optional `blueprint` and `config`
// params. When blueprint is set, the child session uses the blueprint's
// RuntimeAgent (own prompt, tools, model) instead of inheriting parent's.
//
// Background mode (default): returns immediately with a task_id; a detached
// watcher (same pattern as spawn_background runs) sends the instructions,
// heartbeats the task registry, and settles the task on the child's terminal
// turn status. The task's OnTerminal wake policy notifies the parent session
// through the registry-level waker (specs/session-tasks.md, Wake-ups).
// Foreground mode: blocks until subagent completes (send_message + wait_for_idle).
// When no session task registry is wired (embedders without background
// tracking), an unspecified mode degrades to foreground so results are not lost.
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
        vec![]
    }
}

const SUBAGENT_SYSTEM_PROMPT: &str = "Spawn subagents only for independent workstreams that benefit from parallelism or a separate context window; do not delegate immediate sequential steps. Spawns are background by default: you get a task_id, keep working, and are notified on completion (monitor with get_task/wait_task). Use mode \"foreground\" only when you cannot proceed without the result. No nested subagents. Use blueprints for specialist agents with their own tools and model.";

/// Execution mode for subagent delegation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpawnMode {
    /// Return immediately; a detached watcher settles the task and the
    /// OnTerminal wake policy notifies the parent when the child finishes.
    Background,
    /// Block until the child idles and return its result inline.
    Foreground,
}

impl SpawnMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Foreground => "foreground",
        }
    }
}

/// Per-slice wait used by the background watcher; the watcher loops slices
/// until the child reaches a terminal state or the overall cap is hit.
const BACKGROUND_WAIT_SLICE_SECS: u64 = 300;
/// Overall cap on a background subagent run. The child's own max-iterations
/// guard bounds each turn; this bounds pathological never-terminal children.
const BACKGROUND_MAX_WAIT_SECS: u64 = 6 * 60 * 60;
/// Watcher heartbeat cadence; the session task reaper treats heartbeats
/// stale after ~5 minutes, so this keeps live watchers well inside that.
const BACKGROUND_HEARTBEAT_INTERVAL_SECS: u64 = 15;
/// Backoff between wait slices for statuses that return immediately
/// (paused / waiting_for_tool_results) so the watcher does not spin.
const BACKGROUND_POLL_BACKOFF_SECS: u64 = 5;

fn terminal_subagent_status(wait_status: &str) -> Option<crate::session::SubagentStatus> {
    match wait_status {
        // Plain `idle` only means the worker is ready for another turn — failed
        // turns also leave the session idle — so only explicit terminal
        // outcomes may settle the spawn handle and persist terminal metadata.
        "completed" => Some(crate::session::SubagentStatus::Completed),
        "error" | "failed" => Some(crate::session::SubagentStatus::Failed),
        "cancelled" => Some(crate::session::SubagentStatus::Cancelled),
        "max_iterations_reached" => Some(crate::session::SubagentStatus::MaxIterationsReached),
        // A sealed turn (no forward progress / budget exhausted) is terminal but
        // distinct from a failure — surface it so the parent can decide next steps.
        "sealed" => Some(crate::session::SubagentStatus::Sealed),
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

use super::util::{get_platform_store, require_str_nonblank as require_str};

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
/// Unified delegation wrapper for the subagent target of `spawn_agent`.
pub struct SpawnSubagentAsAgentTool;

#[async_trait]
impl Tool for SpawnSubagentAsAgentTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_subagent_spawn(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Agent")
    }

    fn description(&self) -> &str {
        "Delegate a task to a subagent in its own context window. Set target.type to \"subagent\". Runs in the background by default and returns a task_id immediately; set mode to \"foreground\" to block until it completes."
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
                "target": {
                    "type": "object",
                    "properties": {
                        "type": {
                            "type": "string",
                            "enum": ["subagent"],
                            "description": "Delegation target type. Use \"subagent\" for a same-agent child session."
                        }
                    },
                    "required": ["type"],
                    "additionalProperties": false
                },
                "mode": {
                    "type": "string",
                    "enum": ["background", "foreground"],
                    "description": "Execution mode. \"background\" (default) returns immediately with a task_id — monitor with get_task/wait_task; the session is notified when the subagent finishes. \"foreground\" blocks until the subagent completes and returns its result inline."
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
            "required": ["name", "instructions", "target"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "spawn_agent requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let target = arguments.get("target").unwrap_or(&Value::Null);
        if target.get("type").and_then(Value::as_str) != Some("subagent") {
            return ToolExecutionResult::tool_error(
                "spawn_agent target.type must be \"subagent\" for the subagents capability",
            );
        }
        spawn_agent_subagent_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Resolve the effective spawn mode from the `mode` argument.
///
/// Background needs a session task registry (it is the only surface through
/// which the parent can observe the result): an explicit `background` without
/// one is an error, while the unspecified default degrades to foreground so
/// embedders without background tracking keep blocking semantics.
fn resolve_spawn_mode(
    arguments: &Value,
    context: &ToolContext,
) -> Result<SpawnMode, ToolExecutionResult> {
    let explicit = match arguments
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some("background") => Some(SpawnMode::Background),
        Some("foreground") => Some(SpawnMode::Foreground),
        Some(other) => {
            return Err(ToolExecutionResult::tool_error(format!(
                "Invalid mode: \"{other}\". Valid modes: background, foreground."
            )));
        }
    };
    let has_registry = context.session_task_registry.is_some();
    match explicit {
        Some(SpawnMode::Background) if !has_registry => Err(ToolExecutionResult::tool_error(
            "Background mode requires a session task registry, which is not available in this environment. Use mode: \"foreground\" instead.",
        )),
        Some(mode) => Ok(mode),
        None if has_registry => Ok(SpawnMode::Background),
        None => Ok(SpawnMode::Foreground),
    }
}

async fn spawn_agent_subagent_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let name = require_str(&arguments, "name")?.trim().to_string();
    let instructions = require_str(&arguments, "instructions")?.to_string();
    let mode = resolve_spawn_mode(&arguments, context)?;

    let store = get_platform_store(context)?;
    let session_store = get_session_store(context)?;

    let blueprint_param = arguments
        .get("blueprint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());
    let config_param = arguments.get("config").filter(|v| !v.is_null()).cloned();

    // Reject config without blueprint
    if config_param.is_some() && blueprint_param.is_none() {
        return Ok(ToolExecutionResult::tool_error(
            "The `config` parameter is only valid when `blueprint` is set.",
        ));
    }

    // Nesting check: reject if current session is already a subagent
    let parent_session = match session_store.get_session(context.session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(ToolExecutionResult::tool_error("Current session not found")),
        Err(e) => return Err(ToolExecutionResult::internal_error(e)),
    };

    if parent_session.parent_session_id.is_some() {
        return Ok(ToolExecutionResult::tool_error(
            "Subagents cannot spawn other subagents (nesting not allowed).",
        ));
    }

    // Validate blueprint exists and is allowed for this parent session.
    if let Some(ref bp_id) = blueprint_param {
        let Some(ref registry) = context.capability_registry else {
            return Ok(ToolExecutionResult::tool_error(
                "Blueprint support requires capability_registry context.",
            ));
        };

        let Some((blueprint_capability_id, blueprint)) = registry.blueprint_with_capability(bp_id)
        else {
            return Ok(ToolExecutionResult::tool_error(format!(
                "Unknown blueprint: \"{bp_id}\". Check available blueprints."
            )));
        };

        // Validate config against schema if blueprint has one.
        if let Some(ref schema) = blueprint.config_schema
            && config_param.is_none()
            && schema
                .get("required")
                .is_some_and(|r| r.as_array().is_some_and(|arr| !arr.is_empty()))
        {
            return Ok(ToolExecutionResult::tool_error(format!(
                "Blueprint \"{bp_id}\" requires config. Schema: {}",
                serde_json::to_string_pretty(schema).unwrap_or_default()
            )));
        }

        let allowed_capability_ids = if let Some(agent_id) = parent_session.agent_id {
            match store.get_agent_by_id(agent_id).await {
                Ok(Some(agent)) => agent
                    .capabilities
                    .iter()
                    .map(|c| c.capability_id().to_string())
                    .collect::<Vec<_>>(),
                Ok(None) => vec![],
                Err(e) => return Err(ToolExecutionResult::internal_error(e)),
            }
        } else {
            match store.get_harness(parent_session.harness_id).await {
                Ok(Some(harness)) => harness
                    .capabilities
                    .iter()
                    .map(|c| c.capability_id().to_string())
                    .collect::<Vec<_>>(),
                Ok(None) => vec![],
                Err(e) => return Err(ToolExecutionResult::internal_error(e)),
            }
        };

        if !allowed_capability_ids
            .iter()
            .any(|capability_id| capability_id == &blueprint_capability_id)
        {
            return Ok(ToolExecutionResult::tool_error(format!(
                "Blueprint \"{bp_id}\" is not enabled for this session."
            )));
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
            .try_claim_spawn(context.session_id, tool_call_id, claim_token)
            .await
        {
            Ok(c) => c,
            Err(e) => return Err(ToolExecutionResult::internal_error(e)),
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
                return Ok(ToolExecutionResult::success(json!({
                    "subagent_id": child_session_id.to_string(),
                    "name": name,
                    "status": terminal_status,
                    "result": terminal_result,
                    "task_id": task_id,
                    "blueprint": blueprint_param,
                })));
            }
            SpawnClaimResult::AlreadyRunning {
                child_session_id,
                claim_token: stored_claim_token,
            } => {
                // Child was spawned before but hasn't settled yet — reattach.
                // Use the stored claim_token so settle succeeds on this replay.
                let task = find_subagent_task(context, child_session_id).await;
                let (task_id, task_attempt) =
                    task.map(|t| (Some(t.id), t.attempt)).unwrap_or((None, 1));
                match mode {
                    SpawnMode::Foreground => {
                        return Ok(run_subagent_wait_and_settle(
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
                        .await);
                    }
                    SpawnMode::Background => {
                        // Re-arm the detached watcher so the task still settles;
                        // the instructions were already sent on the first claim.
                        spawn_background_watcher(
                            context,
                            child_session_id,
                            &name,
                            None,
                            task_id.clone(),
                            task_attempt,
                            Some(stored_claim_token),
                        );
                        return Ok(background_running_result(
                            child_session_id,
                            &name,
                            &task_id,
                            &blueprint_param,
                        ));
                    }
                }
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
                return Ok(spawn_create_and_wait(
                    store,
                    context,
                    &parent_session,
                    &name,
                    &instructions,
                    &blueprint_param,
                    &config_param,
                    mode,
                    Some((
                        spawn_store.as_ref(),
                        tool_call_id.as_str(),
                        spawn_handle_id,
                        actual_claim_token,
                    )),
                )
                .await);
            }
        }
    }

    // --- No-spawn-store path (dev / noop) ---
    Ok(spawn_create_and_wait(
        store,
        context,
        &parent_session,
        &name,
        &instructions,
        &blueprint_param,
        &config_param,
        mode,
        None,
    )
    .await)
}

/// Immediate tool result for a background spawn: the child is running and the
/// task record is the surface for progress and the final result.
fn background_running_result(
    child_id: crate::typed_id::SessionId,
    name: &str,
    task_id: &Option<String>,
    blueprint_param: &Option<String>,
) -> ToolExecutionResult {
    ToolExecutionResult::success(json!({
        "subagent_id": child_id.to_string(),
        "name": name,
        "status": "running",
        "mode": "background",
        "task_id": task_id,
        "blueprint": blueprint_param,
        "message": "Subagent started in the background. Monitor it with get_task or wait_task using task_id; the session is notified when it finishes.",
    }))
}

// =============================================================================
// Helpers for subagent delegation
// =============================================================================

/// Create a new child session, then either wait for completion (foreground)
/// or detach a watcher and return immediately (background). Settles the spawn
/// handle (if a settle context is supplied) when the child reaches a terminal
/// state.
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
    mode: SpawnMode,
    settle_ctx: Option<(
        &dyn crate::traits::SubagentSpawnStore,
        &str,
        uuid::Uuid,
        uuid::Uuid,
    )>,
) -> ToolExecutionResult {
    // Create child session, linking it to the parent (nesting guard).
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
            Some(context.session_id),
        )
        .await
    {
        Ok(s) => s,
        Err(e) => return ToolExecutionResult::internal_error(e),
    };
    // Create the session task tracking this subagent (specs/session-tasks.md).
    // Background tasks wake the parent on terminal transition through the
    // registry-level wake policy; foreground spawns already return the result
    // inline, so a wake would be noise.
    let mut task_id: Option<String> = None;
    let mut task_attempt: i32 = 1;
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
                    "mode": mode.as_str(),
                }),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_session.id),
                    ..Default::default()
                },
                wake_policy: match mode {
                    SpawnMode::Background => TaskWakePolicy::OnTerminal,
                    SpawnMode::Foreground => TaskWakePolicy::Silent,
                },
            })
            .await
    {
        task_id = Some(created.id);
        task_attempt = created.attempt;
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

    if mode == SpawnMode::Background {
        // The first message is sent inside the watcher: local/embedded hosts
        // (everruns-runtime) run the child's turn synchronously inside
        // send_message, so sending here would block the spawn call.
        spawn_background_watcher(
            context,
            child_session.id,
            name,
            Some(instructions.to_string()),
            task_id.clone(),
            task_attempt,
            wait_settle_ctx.map(|(_, _, claim_token)| claim_token),
        );
        return background_running_result(child_session.id, name, &task_id, blueprint_param);
    }

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
    _instructions: &str,
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

    let result_text = match settle_subagent_outcome(
        store,
        context,
        child_id,
        &status,
        task_id.as_deref(),
        settle_ctx,
    )
    .await
    {
        Ok(text) => text,
        Err(error) => return error,
    };

    ToolExecutionResult::success(json!({
        "subagent_id": child_id.to_string(),
        "name": name,
        "status": status,
        "result": result_text,
        "task_id": task_id,
        "blueprint": blueprint_param,
    }))
}

/// Collect the child's final message and, when `status` is terminal, settle
/// the spawn handle and mirror the outcome onto the session task. Non-terminal
/// statuses (paused, waiting_for_tool_results, timeout) only produce the
/// result text — the child stays active and the spawn stays reattachable.
async fn settle_subagent_outcome(
    store: &dyn PlatformStore,
    context: &ToolContext,
    child_id: crate::typed_id::SessionId,
    status: &str,
    task_id: Option<&str>,
    settle_ctx: Option<(&dyn crate::traits::SubagentSpawnStore, &str, uuid::Uuid)>,
) -> Result<String, ToolExecutionResult> {
    // Get the subagent's response messages
    let messages = match store.get_messages(child_id, Some(5)).await {
        Ok(m) => m,
        Err(e) => return Err(ToolExecutionResult::internal_error(e)),
    };

    let result_text = last_agent_message(&messages)
        .unwrap_or_else(|| format!("Subagent completed with status: {status}"));

    let terminal_status = terminal_subagent_status(status);

    // Settle the spawn handle only when the child reached a terminal state.
    // Non-terminal waits must stay reattachable on replay.
    if let Some((spawn_store, tool_call_id, claim_token)) = settle_ctx
        && terminal_status.is_some()
        && let Err(e) = spawn_store
            .settle_spawn(
                context.session_id,
                tool_call_id,
                claim_token,
                status,
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

    // Update the session task only when the child reached a terminal state.
    if let Some(subagent_status) = terminal_status {
        let task_state = terminal_subagent_task_state(&subagent_status);
        let task_error = if task_state == SessionTaskState::Failed {
            Some(TaskError {
                kind: status.to_string(),
                message: format!("Subagent session ended with status: {status}"),
            })
        } else {
            None
        };
        finish_subagent_task(
            context,
            task_id,
            task_state,
            Some(truncate_summary(&result_text)),
            task_error,
        )
        .await;
    }

    Ok(result_text)
}

/// Detach a watcher that drives a background subagent to completion: send the
/// first message (fresh spawns only), heartbeat the task registry so the
/// reaper can detect worker loss, wait for the child's terminal turn status,
/// then settle the task and spawn handle. The task's OnTerminal wake policy
/// notifies the parent session at the registry level.
fn spawn_background_watcher(
    context: &ToolContext,
    child_id: crate::typed_id::SessionId,
    name: &str,
    first_message: Option<String>,
    task_id: Option<String>,
    task_attempt: i32,
    claim_token: Option<uuid::Uuid>,
) {
    let context = context.clone();
    let name = name.to_string();
    tokio::spawn(async move {
        let Some(store) = context.platform_store.clone() else {
            // Callers only enter background mode with a platform store wired.
            return;
        };

        if let Some(instructions) = first_message
            && let Err(e) = store.send_message(child_id, &instructions).await
        {
            finish_subagent_task(
                &context,
                task_id.as_deref(),
                SessionTaskState::Failed,
                None,
                Some(TaskError {
                    kind: "error".to_string(),
                    message: e.to_string(),
                }),
            )
            .await;
            return;
        }

        // Heartbeat so the session task reaper can fail an orphaned watcher
        // (worker loss) instead of leaving the task running forever. Fenced on
        // the attempt captured at spawn so a superseded watcher's writes are
        // rejected once the reaper bumps the attempt counter.
        let heartbeat = async {
            let (Some(registry), Some(task_id)) =
                (context.session_task_registry.clone(), task_id.clone())
            else {
                return std::future::pending::<()>().await;
            };
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(
                    BACKGROUND_HEARTBEAT_INTERVAL_SECS,
                ))
                .await;
                let _ = registry
                    .update(
                        context.session_id,
                        &task_id,
                        SessionTaskUpdate {
                            heartbeat_at: Some(chrono::Utc::now()),
                            expected_attempt: Some(task_attempt),
                            ..Default::default()
                        },
                    )
                    .await;
            }
        };

        let wait_and_settle = async {
            let started = tokio::time::Instant::now();
            loop {
                let status = match store
                    .wait_for_idle(child_id, Some(BACKGROUND_WAIT_SLICE_SECS))
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        finish_subagent_task(
                            &context,
                            task_id.as_deref(),
                            SessionTaskState::Failed,
                            None,
                            Some(TaskError {
                                kind: "error".to_string(),
                                message: e.to_string(),
                            }),
                        )
                        .await;
                        return;
                    }
                };

                // Local/embedded hosts (everruns-runtime) run the child's turn
                // synchronously inside send_message and report a bare `idle`;
                // hosted adapters never return it (they poll until a terminal
                // turn event lands). Map it to completion so embedder tasks
                // settle instead of looping until the cap.
                let effective = if status == "idle" {
                    "completed".to_string()
                } else {
                    status
                };

                if terminal_subagent_status(&effective).is_some() {
                    let settle_ctx = match (
                        context.subagent_spawn_store.as_ref(),
                        context.tool_call_id.as_ref(),
                        claim_token,
                    ) {
                        (Some(spawn_store), Some(tool_call_id), Some(token)) => Some((
                            spawn_store.as_ref() as &dyn crate::traits::SubagentSpawnStore,
                            tool_call_id.as_str(),
                            token,
                        )),
                        _ => None,
                    };
                    if let Err(error) = settle_subagent_outcome(
                        store.as_ref(),
                        &context,
                        child_id,
                        &effective,
                        task_id.as_deref(),
                        settle_ctx,
                    )
                    .await
                    {
                        tracing::warn!(
                            subagent_name = name,
                            child_session_id = %child_id,
                            ?error,
                            "Background subagent settle failed; marking task failed"
                        );
                        finish_subagent_task(
                            &context,
                            task_id.as_deref(),
                            SessionTaskState::Failed,
                            None,
                            Some(TaskError {
                                kind: "error".to_string(),
                                message: "Failed to read subagent result".to_string(),
                            }),
                        )
                        .await;
                    }
                    return;
                }

                if started.elapsed().as_secs() >= BACKGROUND_MAX_WAIT_SECS {
                    finish_subagent_task(
                        &context,
                        task_id.as_deref(),
                        SessionTaskState::Failed,
                        None,
                        Some(TaskError {
                            kind: "timeout".to_string(),
                            message: format!(
                                "Background subagent did not finish within {BACKGROUND_MAX_WAIT_SECS}s (last status: {effective})"
                            ),
                        }),
                    )
                    .await;
                    return;
                }

                // Non-terminal: record progress and keep waiting. Statuses
                // other than the wait-slice timeout return immediately, so
                // back off before re-waiting to avoid spinning.
                if let (Some(registry), Some(task_id)) =
                    (context.session_task_registry.as_ref(), task_id.as_deref())
                {
                    let _ = registry
                        .update(
                            context.session_id,
                            task_id,
                            SessionTaskUpdate {
                                state_detail: Some(format!(
                                    "waiting for subagent ({}s elapsed, last status: {effective})",
                                    started.elapsed().as_secs()
                                )),
                                expected_attempt: Some(task_attempt),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                if !effective.starts_with("timeout") {
                    tokio::time::sleep(std::time::Duration::from_secs(
                        BACKGROUND_POLL_BACKOFF_SECS,
                    ))
                    .await;
                }
            }
        };

        tokio::select! {
            () = wait_and_settle => {}
            () = heartbeat => {}
        }
    });
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

    /// Converge a subagent task whose background watcher is gone (worker
    /// loss): probe the child's terminal turn status and mirror it onto the
    /// task. Called from wait_task's poll loop; no-op while the child is
    /// still working.
    async fn reconcile(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> crate::error::Result<()> {
        if task.state.is_terminal() {
            return Ok(());
        }
        let (Some(store), Some(child_id)) =
            (context.platform_store.as_ref(), task.links.child_session_id)
        else {
            return Ok(());
        };
        // Zero-timeout probe: returns the terminal turn status when the child
        // already finished, or a timeout marker while it is still running.
        let status = store.wait_for_idle(child_id, Some(0)).await?;
        if terminal_subagent_status(&status).is_none() {
            return Ok(());
        }
        settle_subagent_outcome(
            store.as_ref(),
            context,
            child_id,
            &status,
            Some(&task.id),
            None,
        )
        .await
        .map(|_| ())
        .map_err(|_| {
            crate::error::AgentLoopError::tool("Failed to read subagent result during reconcile")
        })
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

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn capability_features() {
        let cap = SubagentCapability;
        assert_eq!(cap.features(), vec!["subagents"]);
    }

    #[test]
    fn terminal_subagent_status_maps_only_terminal_wait_states() {
        // `idle` is not terminal: a failed turn also idles the worker, so it
        // must not settle the subagent as completed.
        assert_eq!(terminal_subagent_status("idle"), None);
        assert_eq!(
            terminal_subagent_status("completed"),
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
            terminal_subagent_status("sealed"),
            Some(crate::session::SubagentStatus::Sealed)
        );
        assert_eq!(
            terminal_subagent_task_state(&crate::session::SubagentStatus::Completed),
            SessionTaskState::Succeeded
        );
        // A sealed subagent settles as a terminal, non-retryable failed task.
        assert_eq!(
            terminal_subagent_task_state(&crate::session::SubagentStatus::Sealed),
            SessionTaskState::Failed
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
    fn spawn_agent_subagent_schema_advertises_only_subagent_target() {
        let tool = SpawnSubagentAsAgentTool;
        let schema = tool.parameters_schema();
        assert_eq!(
            schema["properties"]["target"]["properties"]["type"]["enum"],
            json!(["subagent"])
        );
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("target")));
        assert!(required.contains(&json!("name")));
        assert!(required.contains(&json!("instructions")));
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("blueprint"));
        assert!(props.contains_key("config"));
        assert!(!required.contains(&json!("blueprint")));
        assert!(!required.contains(&json!("config")));
        assert_eq!(
            schema["properties"]["mode"]["enum"],
            json!(["background", "foreground"])
        );
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
            .try_claim_spawn(parent, "call-1", token)
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
            .try_claim_spawn(parent, "call-arc", token)
            .await
            .expect("arc delegation should not error");

        assert!(matches!(result, SpawnClaimResult::Claimed { .. }));
    }

    // =========================================================================
    // Background mode
    // =========================================================================

    use crate::capabilities::session_tasks::tests::InMemorySessionTaskRegistry;
    use crate::platform_store::tests::MockPlatformStore;
    use crate::session_task::SessionTaskRegistry;

    /// SessionStore view over the mock platform store (nesting guard lookup).
    struct MockSessionStore(Arc<MockPlatformStore>);

    #[async_trait]
    impl crate::traits::SessionStore for MockSessionStore {
        async fn get_session(
            &self,
            session_id: crate::typed_id::SessionId,
        ) -> crate::error::Result<Option<crate::session::Session>> {
            self.0.get_session_by_id(session_id).await
        }
    }

    fn spawn_context(
        store: &Arc<MockPlatformStore>,
        registry: Option<Arc<InMemorySessionTaskRegistry>>,
    ) -> ToolContext {
        let mut context = ToolContext::new(store.session.id);
        context.platform_store = Some(store.clone());
        context.session_store = Some(Arc::new(MockSessionStore(store.clone())));
        if let Some(registry) = registry {
            context.session_task_registry = Some(registry);
        }
        context
    }

    async fn spawn(context: &ToolContext, args: Value) -> ToolExecutionResult {
        let mut args = args;
        if let Some(object) = args.as_object_mut() {
            object
                .entry("target")
                .or_insert_with(|| json!({"type": "subagent"}));
        }
        SpawnSubagentAsAgentTool
            .execute_with_context(args, context)
            .await
    }

    /// Poll the registry until the subagent task reaches `state` (the
    /// background watcher settles it from a detached tokio task).
    async fn wait_for_task_state(
        registry: &InMemorySessionTaskRegistry,
        session_id: crate::typed_id::SessionId,
        task_id: &str,
        state: crate::session_task::SessionTaskState,
    ) -> crate::session_task::SessionTask {
        for _ in 0..200 {
            let task = registry
                .get(session_id, task_id)
                .await
                .expect("registry get")
                .expect("task exists");
            if task.state == state {
                return task;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("task {task_id} did not reach {state:?}");
    }

    #[tokio::test]
    async fn spawn_agent_subagent_rejects_invalid_mode() {
        let context = ToolContext::new(crate::typed_id::SessionId::new());
        let result = spawn(
            &context,
            json!({"name": "Runner", "instructions": "go", "mode": "asap"}),
        )
        .await;
        let ToolExecutionResult::ToolError(msg) = result else {
            panic!("expected ToolError, got {result:?}");
        };
        assert!(msg.contains("Invalid mode"), "got: {msg}");
    }

    #[tokio::test]
    async fn spawn_agent_subagent_rejects_other_target_types() {
        let context = ToolContext::new(crate::typed_id::SessionId::new());
        let result = SpawnSubagentAsAgentTool
            .execute_with_context(
                json!({
                    "name": "Runner",
                    "instructions": "go",
                    "target": {"type": "external_a2a"}
                }),
                &context,
            )
            .await;
        let ToolExecutionResult::ToolError(msg) = result else {
            panic!("expected ToolError, got {result:?}");
        };
        assert!(msg.contains("subagent"), "got: {msg}");
    }

    #[tokio::test]
    async fn spawn_agent_subagent_creates_subagent_task() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));

        let result = SpawnSubagentAsAgentTool
            .execute_with_context(
                json!({
                    "name": "Runner",
                    "instructions": "go",
                    "target": {"type": "subagent"},
                    "mode": "foreground"
                }),
                &context,
            )
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        let task_id = value["task_id"].as_str().expect("task_id");
        let task = registry
            .get(context.session_id, task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.kind, TASK_KIND_SUBAGENT);
        assert_eq!(task.spec["mode"], "foreground");
        assert!(task.links.child_session_id.is_some());
    }

    #[tokio::test]
    async fn explicit_background_without_registry_errors() {
        let context = ToolContext::new(crate::typed_id::SessionId::new());
        let result = spawn(
            &context,
            json!({"name": "Runner", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::ToolError(msg) = result else {
            panic!("expected ToolError, got {result:?}");
        };
        assert!(
            msg.contains("task registry") && msg.contains("foreground"),
            "got: {msg}"
        );
    }

    #[tokio::test]
    async fn default_mode_without_registry_degrades_to_foreground() {
        let store = Arc::new(MockPlatformStore::new());
        let context = spawn_context(&store, None);
        let result = spawn(&context, json!({"name": "Runner", "instructions": "go"})).await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        // Foreground semantics: waited inline and returned the child's reply.
        assert_eq!(value["status"], "idle");
        assert_eq!(value["result"], "Hi!");
        assert!(value.get("mode").is_none());
    }

    #[tokio::test]
    async fn background_spawn_returns_immediately_and_settles_task() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));

        let result = spawn(&context, json!({"name": "Runner", "instructions": "go"})).await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        assert_eq!(value["status"], "running");
        assert_eq!(value["mode"], "background");
        let task_id = value["task_id"].as_str().expect("task_id").to_string();

        let task = wait_for_task_state(
            &registry,
            context.session_id,
            &task_id,
            SessionTaskState::Succeeded,
        )
        .await;
        // Background tasks wake the parent on terminal transition.
        assert_eq!(task.wake_policy, TaskWakePolicy::OnTerminal);
        assert_eq!(task.spec["mode"], "background");
        // Summary carries the child's last agent message.
        assert_eq!(task.summary.as_deref(), Some("Hi!"));
    }

    #[tokio::test]
    async fn background_settles_bare_idle_as_completed() {
        // Local/embedded hosts run the child's turn synchronously inside
        // send_message and report a bare `idle`; the watcher must settle it.
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));

        let result = spawn(&context, json!({"name": "Runner", "instructions": "go"})).await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        let task_id = value["task_id"].as_str().expect("task_id").to_string();
        wait_for_task_state(
            &registry,
            context.session_id,
            &task_id,
            SessionTaskState::Succeeded,
        )
        .await;
    }

    #[tokio::test]
    async fn background_failed_child_settles_task_failed() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "failed".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));

        let result = spawn(&context, json!({"name": "Runner", "instructions": "go"})).await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        let task_id = value["task_id"].as_str().expect("task_id").to_string();
        let task = wait_for_task_state(
            &registry,
            context.session_id,
            &task_id,
            SessionTaskState::Failed,
        )
        .await;
        assert_eq!(task.error.as_ref().map(|e| e.kind.as_str()), Some("failed"));
    }

    #[tokio::test]
    async fn explicit_foreground_blocks_and_returns_result() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));

        let result = spawn(
            &context,
            json!({"name": "Runner", "instructions": "go", "mode": "foreground"}),
        )
        .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        assert_eq!(value["status"], "completed");
        assert_eq!(value["result"], "Hi!");
        // Foreground spawn settles the task before returning.
        let task_id = value["task_id"].as_str().expect("task_id");
        let task = registry
            .get(context.session_id, task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::Succeeded);
        assert_eq!(task.wake_policy, TaskWakePolicy::Silent);
    }

    #[tokio::test]
    async fn reconcile_settles_finished_child() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));

        let child_id = crate::typed_id::SessionId::new();
        let task = registry
            .create(CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Runner".to_string(),
                spec: json!({"mode": "background"}),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::OnTerminal,
            })
            .await
            .unwrap();

        SubagentTaskExecutor
            .reconcile(&task, &context)
            .await
            .expect("reconcile succeeds");

        let task = registry
            .get(context.session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::Succeeded);
        assert_eq!(task.summary.as_deref(), Some("Hi!"));
    }

    #[tokio::test]
    async fn reconcile_is_noop_while_child_still_working() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "timeout (last status: Active)".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));

        let task = registry
            .create(CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Runner".to_string(),
                spec: json!({"mode": "background"}),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(crate::typed_id::SessionId::new()),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::OnTerminal,
            })
            .await
            .unwrap();

        SubagentTaskExecutor
            .reconcile(&task, &context)
            .await
            .expect("reconcile succeeds");

        let task = registry
            .get(context.session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::Running);
    }
}
