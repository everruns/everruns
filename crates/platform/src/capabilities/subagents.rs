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
// through the registry-level waker (knowledge/runtime-resources/session-tasks.md, Wake-ups).
// Foreground mode: blocks until subagent completes (send_message + wait_for_idle).
// When no session task registry is wired (embedders without background
// tracking), an unspecified mode degrades to foreground so results are not lost.
//
// Subagent naming: human-readable ("Test Runner"), unique per parent, case-insensitive.
// Spawn governance: child depth and root-tree task fan-out are bounded.

use super::delegation_result::{
    MESSAGE_SCHEMA_SPEC_KEY, RESULT_SCHEMA_SPEC_KEY, normalize_message_schema,
    normalize_result_schema, required_result_is_missing, result_value_for_task, truncate_summary,
};
#[cfg(test)]
use super::delegation_result::{ReportResultTool, ReportTaskProgressTool};
use super::{
    Capability, CapabilityLocalization, CapabilityStatus, DelegationTargetProvider, RiskLevel,
    SPAWN_AGENT_CONCURRENCY_CLASS, SpawnMode,
};
use crate::background_run::{BackgroundRunPermit, try_acquire_background_run_permit};
use async_trait::async_trait;
use everruns_core::session::SessionSeedMode;
use everruns_core::session_task::{
    CreateSessionTask, SessionTask, SessionTaskFilter, SessionTaskState, SessionTaskUpdate,
    TASK_KIND_SESSION, TASK_KIND_SUBAGENT, TaskError, TaskExecutor, TaskExecutorPlugin, TaskLinks,
    TaskMessage, TaskWakePolicy, task_message_text,
};
use everruns_core::subagent_delegation::{PlatformCreateSessionRequest, SubagentSessionDelegate};
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::typed_id::SessionId;
use everruns_core::{
    delegation_services::SpawnClaimResult, execution_loading::SessionStore,
    tool_context::ToolContext,
};

use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
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

    fn risk_level(&self) -> RiskLevel {
        // Subagent recursion controls bound org cost/DoS exposure; keep
        // caller-supplied session capability overrides behind the admin gate.
        RiskLevel::High
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["subagents"]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "max_subagent_depth": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 16,
                    "default": everruns_core::delegation_services::DEFAULT_MAX_SUBAGENT_DEPTH,
                    "description": "Maximum child depth allowed from a top-level session. Top-level sessions are depth 0; setting 0 blocks all subagent spawning."
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 16,
                    "description": "Alias for max_subagent_depth."
                },
                "max_active_descendant_tasks": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 1024,
                    "default": everruns_core::delegation_services::DEFAULT_MAX_ACTIVE_DESCENDANT_SUBAGENT_TASKS,
                    "description": "Maximum non-terminal descendant subagent tasks allowed under one root session. Counts queued, running, and awaiting_input tasks."
                },
                "max_concurrent_descendant_tasks": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 1024,
                    "description": "Alias for max_active_descendant_tasks."
                },
                "max_total_descendant_tasks": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 10000,
                    "default": everruns_core::delegation_services::DEFAULT_MAX_TOTAL_DESCENDANT_SUBAGENT_TASKS,
                    "description": "Maximum descendant subagent task records allowed under one root session before rejecting new spawns."
                },
                "max_active_detached_tasks": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 1024,
                    "default": everruns_core::delegation_services::DEFAULT_MAX_ACTIVE_DETACHED_TASKS,
                    "description": "Maximum non-terminal detached peer sessions allowed under one origin root session. Detached spawns reset depth but are still capped here so a loop cannot run unbounded (EVE-767)."
                },
                "max_total_detached_tasks": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 10000,
                    "default": everruns_core::delegation_services::DEFAULT_MAX_TOTAL_DETACHED_TASKS,
                    "description": "Maximum detached peer session task records allowed under one origin root session before rejecting new detached spawns."
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        for key in ["max_subagent_depth", "max_depth"] {
            let Some(value) = config.get(key) else {
                continue;
            };
            let Some(depth) = value.as_u64() else {
                return Err(format!("{key} must be a non-negative integer"));
            };
            if depth > 16 {
                return Err(format!("{key} must be <= 16"));
            }
        }
        for key in [
            "max_active_descendant_tasks",
            "max_concurrent_descendant_tasks",
        ] {
            let Some(value) = config.get(key) else {
                continue;
            };
            let Some(max_active) = value.as_u64() else {
                return Err(format!("{key} must be a non-negative integer"));
            };
            if max_active > 1024 {
                return Err(format!("{key} must be <= 1024"));
            }
        }
        for key in ["max_total_descendant_tasks", "max_total_detached_tasks"] {
            let Some(value) = config.get(key) else {
                continue;
            };
            let Some(max_total) = value.as_u64() else {
                return Err(format!("{key} must be a non-negative integer"));
            };
            if max_total > 10_000 {
                return Err(format!("{key} must be <= 10000"));
            }
        }
        if let Some(value) = config.get("max_active_detached_tasks") {
            let Some(max_active) = value.as_u64() else {
                return Err("max_active_detached_tasks must be a non-negative integer".to_string());
            };
            if max_active > 1024 {
                return Err("max_active_detached_tasks must be <= 1024".to_string());
            }
        }
        Ok(())
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SUBAGENT_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![]
    }

    fn delegation_target_with_config(&self, _config: &Value) -> Option<DelegationTargetProvider> {
        Some(DelegationTargetProvider {
            target_type: "subagent",
            tool: Box::new(SpawnSubagentAsAgentTool),
        })
    }
}

const SUBAGENT_SYSTEM_PROMPT: &str = "Spawn subagents for independent parallel work or separate context; avoid immediate sequential steps. Spawns are background by default: you get a task_id, keep working, and are notified on completion (monitor with get_task/wait_task). Use mode \"foreground\" only when blocked on the result. Nested subagents are allowed up to max_subagent_depth and root-tree task caps. Use blueprints for specialist tools/model.";
/// Task spec key holding spawn-time per-task push configs (EVE-682). The
/// webhook notifier reads this in addition to the DB-backed configs so
/// spawn-time and endpoint-created configs share one delivery path.
const PUSH_CONFIGS_SPEC_KEY: &str = "push_configs";
/// Valid `event_filter` members for a per-task push config.
const VALID_PUSH_EVENT_FILTERS: [&str; 3] = ["terminal", "awaiting_input", "message"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnLifetime {
    Linked,
    Detached,
}

impl SpawnLifetime {
    pub fn parse(arguments: &Value) -> Result<Self, ToolExecutionResult> {
        match arguments.get("lifetime").and_then(Value::as_str) {
            None | Some("linked") => Ok(Self::Linked),
            Some("detached") => Ok(Self::Detached),
            Some(other) => Err(ToolExecutionResult::tool_error(format!(
                "Invalid lifetime: {other}. Expected 'linked' or 'detached'."
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Detached => "detached",
        }
    }
}

fn parse_seed(arguments: &Value) -> Result<SessionSeedMode, ToolExecutionResult> {
    match arguments.get("seed").and_then(Value::as_str) {
        None | Some("fresh") => Ok(SessionSeedMode::Fresh),
        Some("fork") => Ok(SessionSeedMode::Fork),
        Some("workspace") => Ok(SessionSeedMode::Workspace),
        Some(other) => Err(ToolExecutionResult::tool_error(format!(
            "Invalid seed: {other}. Expected 'fresh', 'fork', or 'workspace'."
        ))),
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

fn terminal_subagent_status(wait_status: &str) -> Option<everruns_core::session::SubagentStatus> {
    match wait_status {
        // Plain `idle` only means the worker is ready for another turn — failed
        // turns also leave the session idle — so only explicit terminal
        // outcomes may settle the spawn handle and persist terminal metadata.
        "completed" => Some(everruns_core::session::SubagentStatus::Completed),
        "error" | "failed" => Some(everruns_core::session::SubagentStatus::Failed),
        "cancelled" => Some(everruns_core::session::SubagentStatus::Cancelled),
        "max_iterations_reached" => {
            Some(everruns_core::session::SubagentStatus::MaxIterationsReached)
        }
        // A sealed turn (no forward progress / budget exhausted) is terminal but
        // distinct from a failure — surface it so the parent can decide next steps.
        "sealed" => Some(everruns_core::session::SubagentStatus::Sealed),
        _ => None,
    }
}

fn terminal_subagent_task_state(
    subagent_status: &everruns_core::session::SubagentStatus,
) -> SessionTaskState {
    match subagent_status {
        everruns_core::session::SubagentStatus::Completed => SessionTaskState::Succeeded,
        everruns_core::session::SubagentStatus::Cancelled => SessionTaskState::Canceled,
        _ => SessionTaskState::Failed,
    }
}

/// Parse + validate the optional `push_configs` spawn arg (EVE-682).
///
/// # Security
///
/// Each config URL is SSRF-validated here at create time via
/// `validate_safe_url`, before it is embedded in the task spec. Delivery
/// (the webhook notifier) additionally pins DNS, closing the create→deliver
/// rebinding window. Returns the normalized array to embed under
/// `PUSH_CONFIGS_SPEC_KEY`, or `None` when absent/empty.
fn normalize_push_configs(arguments: &Value) -> Result<Option<Value>, ToolExecutionResult> {
    let Some(raw) = arguments
        .get(PUSH_CONFIGS_SPEC_KEY)
        .filter(|v| !v.is_null())
    else {
        return Ok(None);
    };
    let Some(entries) = raw.as_array() else {
        return Err(ToolExecutionResult::tool_error(
            "push_configs must be an array of { url, secret?, event_filter? } objects.",
        ));
    };
    if entries.is_empty() {
        return Ok(None);
    }
    let mut normalized = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(url) = entry.get("url").and_then(Value::as_str) else {
            return Err(ToolExecutionResult::tool_error(
                "Each push_configs entry requires a string `url`.",
            ));
        };
        if let Err(e) = everruns_core::url_validation::validate_safe_url(url) {
            return Err(ToolExecutionResult::tool_error(format!(
                "Invalid push_configs url \"{url}\": {e}"
            )));
        }
        let mut obj = serde_json::Map::new();
        obj.insert("url".to_string(), Value::String(url.to_string()));
        if let Some(secret) = entry
            .get("secret")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            obj.insert("secret".to_string(), Value::String(secret.to_string()));
        }
        if let Some(filters) = entry.get("event_filter").filter(|v| !v.is_null()) {
            let Some(arr) = filters.as_array() else {
                return Err(ToolExecutionResult::tool_error(
                    "push_configs event_filter must be an array of strings.",
                ));
            };
            let mut out: Vec<Value> = Vec::new();
            for f in arr {
                let Some(f) = f.as_str() else {
                    return Err(ToolExecutionResult::tool_error(
                        "push_configs event_filter members must be strings.",
                    ));
                };
                if !VALID_PUSH_EVENT_FILTERS.contains(&f) {
                    return Err(ToolExecutionResult::tool_error(format!(
                        "Unknown push_configs event_filter \"{f}\". Valid: {}.",
                        VALID_PUSH_EVENT_FILTERS.join(", ")
                    )));
                }
                if !out.iter().any(|x| x.as_str() == Some(f)) {
                    out.push(Value::String(f.to_string()));
                }
            }
            if !out.is_empty() {
                obj.insert("event_filter".to_string(), Value::Array(out));
            }
        }
        normalized.push(Value::Object(obj));
    }
    Ok(Some(Value::Array(normalized)))
}

// =============================================================================
// Helper: get platform store from context
// =============================================================================

use super::util::{get_subagent_delegate, require_str_nonblank as require_str};

fn get_session_store(
    context: &ToolContext,
) -> Result<&dyn everruns_core::execution_loading::SessionStore, ToolExecutionResult> {
    context
        .session_store
        .as_ref()
        .map(|s| s.as_ref())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error("Subagent tools require session_store context")
        })
}

async fn current_subagent_depth(
    session_store: &dyn SessionStore,
    session: &everruns_core::session::ExecutionSession,
    max_subagent_depth: u32,
) -> Result<u32, ToolExecutionResult> {
    let mut depth = 0_u32;
    let mut cursor = session.parent_session_id;

    while let Some(parent_id) = cursor {
        depth = depth.saturating_add(1);
        if depth > max_subagent_depth {
            return Ok(depth);
        }

        let parent = match session_store.get_session(parent_id).await {
            Ok(Some(parent)) => parent,
            Ok(None) => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Cannot enforce max_subagent_depth: parent session {parent_id} was not found."
                )));
            }
            Err(error) => return Err(ToolExecutionResult::internal_error(error)),
        };
        cursor = parent.parent_session_id;
    }

    Ok(depth)
}

async fn root_session_for_subagent_tree(
    session_store: &dyn SessionStore,
    session: &everruns_core::session::ExecutionSession,
) -> Result<SessionId, ToolExecutionResult> {
    let mut root_id = session.id;
    let mut cursor = session.parent_session_id;
    let mut seen = HashSet::new();
    seen.insert(session.id);

    while let Some(parent_id) = cursor {
        if !seen.insert(parent_id) {
            return Err(ToolExecutionResult::tool_error(format!(
                "Cannot enforce subagent descendant task caps: session parent cycle detected at {parent_id}."
            )));
        }

        let parent = match session_store.get_session(parent_id).await {
            Ok(Some(parent)) => parent,
            Ok(None) => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Cannot enforce subagent descendant task caps: parent session {parent_id} was not found."
                )));
            }
            Err(error) => return Err(ToolExecutionResult::internal_error(error)),
        };
        root_id = parent.id;
        cursor = parent.parent_session_id;
    }

    Ok(root_id)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DescendantTaskCounts {
    active: u32,
    total: u32,
}

async fn descendant_subagent_task_counts(
    registry: &dyn everruns_core::session_task::SessionTaskRegistry,
    root_session_id: SessionId,
    max_active: u32,
    max_total: u32,
) -> Result<DescendantTaskCounts, ToolExecutionResult> {
    let mut counts = DescendantTaskCounts::default();
    let mut queue = VecDeque::from([root_session_id]);
    let mut visited_sessions = HashSet::from([root_session_id]);

    while let Some(session_id) = queue.pop_front() {
        let tasks = registry
            .list(
                session_id,
                Some(&SessionTaskFilter {
                    kind: Some(TASK_KIND_SUBAGENT.to_string()),
                    state: None,
                }),
            )
            .await
            .map_err(ToolExecutionResult::internal_error)?;

        for task in tasks {
            counts.total = counts.total.saturating_add(1);
            if !task.state.is_terminal() {
                counts.active = counts.active.saturating_add(1);
            }

            if let Some(child_session_id) = task.links.child_session_id
                && visited_sessions.insert(child_session_id)
            {
                queue.push_back(child_session_id);
            }

            if counts.active >= max_active || counts.total >= max_total {
                return Ok(counts);
            }
        }
    }

    Ok(counts)
}

async fn enforce_subagent_task_caps(
    session_store: &dyn SessionStore,
    session: &everruns_core::session::ExecutionSession,
    context: &ToolContext,
) -> Result<(), ToolExecutionResult> {
    let Some(registry) = context.session_task_registry.as_ref() else {
        return Ok(());
    };
    let policy = context.subagent_nesting_policy;
    let max_active = policy.max_active_descendant_tasks();
    let max_total = policy.max_total_descendant_tasks();
    let root_session_id = root_session_for_subagent_tree(session_store, session).await?;
    let counts =
        descendant_subagent_task_counts(registry.as_ref(), root_session_id, max_active, max_total)
            .await?;

    if counts.active >= max_active {
        let attempted = counts.active.saturating_add(1);
        return Err(ToolExecutionResult::tool_error(format!(
            "Subagent active descendant task cap exceeded: spawning this subagent would create {attempted} non-terminal descendant tasks under root session {root_session_id}, but max_active_descendant_tasks is {max_active}."
        )));
    }

    if counts.total >= max_total {
        let attempted = counts.total.saturating_add(1);
        return Err(ToolExecutionResult::tool_error(format!(
            "Subagent total descendant task cap exceeded: spawning this subagent would create {attempted} descendant task records under root session {root_session_id}, but max_total_descendant_tasks is {max_total}."
        )));
    }

    Ok(())
}

/// Count detached peer tasks (`TASK_KIND_SESSION`) anywhere under the origin
/// subagent tree root (EVE-767). Unlike `descendant_subagent_task_counts`, the
/// BFS follows *every* task's `child_session_id` (subagent and detached alike)
/// so detached spawns made deep in the tree — by subagents or by other detached
/// peers — are all attributed to the origin root. Only `session`-kind tasks are
/// counted; subagent accounting is untouched.
async fn descendant_detached_task_counts(
    registry: &dyn everruns_core::session_task::SessionTaskRegistry,
    root_session_id: SessionId,
    max_active: u32,
    max_total: u32,
) -> Result<DescendantTaskCounts, ToolExecutionResult> {
    let mut counts = DescendantTaskCounts::default();
    let mut queue = VecDeque::from([root_session_id]);
    let mut visited_sessions = HashSet::from([root_session_id]);

    while let Some(session_id) = queue.pop_front() {
        // No kind filter: traversal must cross both subagent and detached
        // subtrees to find every detached spawn under the root.
        let tasks = registry
            .list(session_id, None)
            .await
            .map_err(ToolExecutionResult::internal_error)?;

        for task in tasks {
            if task.kind == TASK_KIND_SESSION {
                counts.total = counts.total.saturating_add(1);
                if !task.state.is_terminal() {
                    counts.active = counts.active.saturating_add(1);
                }
            }

            if let Some(child_session_id) = task.links.child_session_id
                && visited_sessions.insert(child_session_id)
            {
                queue.push_back(child_session_id);
            }

            if counts.active >= max_active || counts.total >= max_total {
                return Ok(counts);
            }
        }
    }

    Ok(counts)
}

/// Governance gate for a detached spawn (EVE-767). A detached peer resets depth
/// but is priced against the origin tree root: a loop of detached spawns is
/// bounded by the active/total detached caps, closing the uncapped-runaway side
/// door (TM-DOS). Non-detached subagent caps are enforced separately and are
/// unchanged.
async fn enforce_detached_spawn_caps(
    context: &ToolContext,
    root_session_id: SessionId,
) -> Result<(), ToolExecutionResult> {
    let Some(registry) = context.session_task_registry.as_ref() else {
        return Ok(());
    };
    let policy = context.subagent_nesting_policy;
    let max_active = policy.max_active_detached_tasks();
    let max_total = policy.max_total_detached_tasks();
    let counts =
        descendant_detached_task_counts(registry.as_ref(), root_session_id, max_active, max_total)
            .await?;

    if counts.active >= max_active {
        let attempted = counts.active.saturating_add(1);
        return Err(ToolExecutionResult::tool_error(format!(
            "Detached spawn active cap exceeded: spawning this detached session would create {attempted} non-terminal detached peer tasks under origin root session {root_session_id}, but max_active_detached_tasks is {max_active}."
        )));
    }

    if counts.total >= max_total {
        let attempted = counts.total.saturating_add(1);
        return Err(ToolExecutionResult::tool_error(format!(
            "Detached spawn total cap exceeded: spawning this detached session would create {attempted} detached peer task records under origin root session {root_session_id}, but max_total_detached_tasks is {max_total}."
        )));
    }

    Ok(())
}

async fn enforce_subagent_depth_cap(
    session_store: &dyn SessionStore,
    session: &everruns_core::session::ExecutionSession,
    context: &ToolContext,
) -> Result<(), ToolExecutionResult> {
    let max_subagent_depth = context.subagent_nesting_policy.max_subagent_depth();
    let current_depth = current_subagent_depth(session_store, session, max_subagent_depth).await?;
    let child_depth = current_depth.saturating_add(1);

    if child_depth > max_subagent_depth {
        return Err(ToolExecutionResult::tool_error(format!(
            "Subagent nesting depth cap exceeded: spawning this subagent would create depth {child_depth}, but max_subagent_depth is {max_subagent_depth}."
        )));
    }

    Ok(())
}

/// Extract the last assistant/agent message content from a list of messages.
fn last_agent_message(
    messages: &[everruns_core::subagent_delegation::PlatformMessage],
) -> Option<String> {
    messages
        .iter()
        .rfind(|m| m.role == "agent" || m.role == "assistant")
        .map(|m| m.content.clone())
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
async fn find_subagent_task(context: &ToolContext, child_id: SessionId) -> Option<SessionTask> {
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
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_subagent_spawn(
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
                },
                "result_schema": {
                    "type": "object",
                    "description": "Optional JSON Schema for the subagent's final structured result. When set, the child receives report_result and must call it before the task can succeed."
                },
                "message_schema": {
                    "type": "object",
                    "description": "Optional JSON Schema for structured progress messages. When set, the child receives report_task_progress and valid calls post data messages to the task thread."
                },
                "push_configs": {
                    "type": "array",
                    "description": "Optional per-task webhook targets notified on task events. Each entry: { url, secret? (HMAC-SHA256 signing key), event_filter? (subset of [\"terminal\", \"awaiting_input\", \"message\"]; defaults to [\"terminal\"]) }. URLs are SSRF-validated.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "url": { "type": "string" },
                            "secret": { "type": "string" },
                            "event_filter": {
                                "type": "array",
                                "items": {
                                    "type": "string",
                                    "enum": ["terminal", "awaiting_input", "message"]
                                }
                            }
                        },
                        "required": ["url"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["name", "instructions", "target"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_concurrency_class(SPAWN_AGENT_CONCURRENCY_CLASS)
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
        Some(value) => match SpawnMode::parse(value) {
            Some(mode) => Some(mode),
            None => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Invalid mode: \"{value}\". Valid modes: background, foreground."
                )));
            }
        },
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
    let goal = arguments
        .get("goal")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mode = resolve_spawn_mode(&arguments, context)?;
    let lifetime = SpawnLifetime::parse(&arguments)?;
    let seed = parse_seed(&arguments)?;

    let store = get_subagent_delegate(context)?;
    let session_store = get_session_store(context)?;

    let blueprint_param = arguments
        .get("blueprint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());
    let config_param = arguments.get("config").filter(|v| !v.is_null()).cloned();
    let result_schema = normalize_result_schema(&arguments)?;
    let message_schema = normalize_message_schema(&arguments)?;
    // SSRF-validate spawn-time push config URLs before they enter the task spec.
    let push_configs = normalize_push_configs(&arguments)?;

    // Reject config without blueprint
    if config_param.is_some() && blueprint_param.is_none() {
        return Ok(ToolExecutionResult::tool_error(
            "The `config` parameter is only valid when `blueprint` is set.",
        ));
    }

    // Nesting check: allow governed nesting up to the resolved depth cap.
    let parent_session = match session_store.get_session(context.session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(ToolExecutionResult::tool_error("Current session not found")),
        Err(e) => return Err(ToolExecutionResult::internal_error(e)),
    };

    if lifetime == SpawnLifetime::Linked
        && let Err(error) =
            enforce_subagent_depth_cap(session_store, &parent_session, context).await
    {
        return Ok(error);
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
    if lifetime == SpawnLifetime::Linked
        && let (Some(spawn_store), Some(tool_call_id)) =
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
                        let background_run_permit =
                            match try_acquire_background_run_permit(context.session_id) {
                                Ok(permit) => permit,
                                Err(message) => {
                                    return Ok(ToolExecutionResult::tool_error(message));
                                }
                            };
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
                            background_run_permit,
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
                    goal.as_deref(),
                    &instructions,
                    &blueprint_param,
                    &config_param,
                    &result_schema,
                    &message_schema,
                    &push_configs,
                    mode,
                    lifetime,
                    seed,
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
        goal.as_deref(),
        &instructions,
        &blueprint_param,
        &config_param,
        &result_schema,
        &message_schema,
        &push_configs,
        mode,
        lifetime,
        seed,
        None,
    )
    .await)
}

/// Immediate tool result for a background spawn: the child is running and the
/// task record is the surface for progress and the final result.
fn background_running_result(
    child_id: everruns_core::typed_id::SessionId,
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
    store: &dyn SubagentSessionDelegate,
    context: &ToolContext,
    parent_session: &everruns_core::session::ExecutionSession,
    name: &str,
    goal: Option<&str>,
    instructions: &str,
    blueprint_param: &Option<String>,
    config_param: &Option<Value>,
    result_schema: &Option<Value>,
    message_schema: &Option<Value>,
    push_configs: &Option<Value>,
    mode: SpawnMode,
    lifetime: SpawnLifetime,
    seed: SessionSeedMode,
    settle_ctx: Option<(
        &dyn everruns_core::delegation_services::SubagentSpawnStore,
        &str,
        uuid::Uuid,
        uuid::Uuid,
    )>,
) -> ToolExecutionResult {
    let background_run_permit = if mode == SpawnMode::Background {
        match try_acquire_background_run_permit(context.session_id) {
            Ok(permit) => Some(permit),
            Err(message) => return ToolExecutionResult::tool_error(message),
        }
    } else {
        None
    };

    let Some(session_store) = context.session_store.as_ref() else {
        return ToolExecutionResult::tool_error("Subagent spawn requires session_store context");
    };
    // THREAT[TM-AUTHZ-014][TM-AGENT-028][TM-DOS-030]: Detached peers require
    // explicit session-creation authority. The host
    // returns the org-validated origin root so detached chains cannot reset
    // either spend attribution or their count-cap scope.
    let budget_root_session_id = if lifetime == SpawnLifetime::Detached {
        let Some(authority) = context.session_creation_authority.as_ref() else {
            return ToolExecutionResult::tool_error(
                "Detached spawn requires session-creation authority.",
            );
        };
        match authority
            .authorize_session_creation(context.session_id)
            .await
        {
            Ok(root_session_id) => Some(root_session_id),
            Err(error) => {
                return ToolExecutionResult::tool_error(format!(
                    "Detached spawn is not authorized to create a session: {error}"
                ));
            }
        }
    } else {
        None
    };

    // Governance gate before creating the child session. Linked subagents are
    // bounded by the descendant task caps; detached peers reset depth but are
    // bounded by the detached caps against the same origin root (EVE-767) so a
    // loop of detached spawns cannot escape governance.
    let caps_result = match lifetime {
        SpawnLifetime::Linked => {
            enforce_subagent_task_caps(session_store.as_ref(), parent_session, context).await
        }
        SpawnLifetime::Detached => {
            enforce_detached_spawn_caps(
                context,
                budget_root_session_id.expect("detached authority returned a root"),
            )
            .await
        }
    };
    if let Err(error) = caps_result {
        return error;
    }

    // Linked sessions are lifecycle children. Detached sessions are peers: no
    // parent_session_id, but fork lineage records who spawned them.
    let child_session = match store
        .create_session_with_options(PlatformCreateSessionRequest {
            harness_id: parent_session.harness_id,
            agent_id: if blueprint_param.is_some() {
                None // Blueprint sessions don't inherit agent
            } else {
                parent_session.agent_id
            },
            title: Some(name.to_string()),
            goal: goal.map(str::to_string),
            locale: parent_session.locale.clone(),
            blueprint_id: blueprint_param.clone(),
            blueprint_config: config_param.clone(),
            parent_session_id: (lifetime == SpawnLifetime::Linked).then_some(context.session_id),
            forked_from_session_id: (lifetime == SpawnLifetime::Detached)
                .then_some(context.session_id),
            budget_root_session_id,
            seed,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => return ToolExecutionResult::internal_error(e),
    };
    // Create the session task tracking this subagent (knowledge/runtime-resources/session-tasks.md).
    // Background tasks wake the parent on terminal transition through the
    // registry-level wake policy; foreground spawns already return the result
    // inline, so a wake would be noise.
    let mut task_id: Option<String> = None;
    let mut task_attempt: i32 = 1;
    let mut task_spec = json!({
        "instructions": instructions,
        "blueprint_id": blueprint_param,
        "mode": mode.as_str(),
        "lifetime": lifetime.as_str(),
        "seed": seed.as_str(),
    });
    if let Some(schema) = result_schema
        && let Some(spec) = task_spec.as_object_mut()
    {
        spec.insert(RESULT_SCHEMA_SPEC_KEY.to_string(), schema.clone());
    }
    if let Some(schema) = message_schema
        && let Some(spec) = task_spec.as_object_mut()
    {
        spec.insert(MESSAGE_SCHEMA_SPEC_KEY.to_string(), schema.clone());
    }
    // Spawn-time push configs (EVE-682): embed in the task spec so the webhook
    // notifier delivers alongside endpoint-created (DB-backed) configs. URLs
    // were SSRF-validated in normalize_push_configs before reaching here.
    if let Some(configs) = push_configs
        && let Some(spec) = task_spec.as_object_mut()
    {
        spec.insert(PUSH_CONFIGS_SPEC_KEY.to_string(), configs.clone());
    }

    if let Some(ref task_registry) = context.session_task_registry
        && let Ok(created) = task_registry
            .create(CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: match lifetime {
                    SpawnLifetime::Linked => TASK_KIND_SUBAGENT,
                    SpawnLifetime::Detached => TASK_KIND_SESSION,
                }
                .to_string(),
                display_name: name.to_string(),
                spec: task_spec,
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_session.id),
                    ..Default::default()
                },
                wake_policy: match (lifetime, mode, message_schema.is_some()) {
                    (SpawnLifetime::Detached, _, _) => TaskWakePolicy::Silent,
                    (SpawnLifetime::Linked, SpawnMode::Background, true) => {
                        TaskWakePolicy::OnActivity
                    }
                    (SpawnLifetime::Linked, SpawnMode::Background, false) => {
                        TaskWakePolicy::OnTerminal
                    }
                    (SpawnLifetime::Linked, SpawnMode::Foreground, _) => TaskWakePolicy::Silent,
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
        // (everruns-host) run the child's turn synchronously inside
        // send_message, so sending here would block the spawn call.
        spawn_background_watcher(
            context,
            child_session.id,
            name,
            Some(instructions.to_string()),
            task_id.clone(),
            task_attempt,
            wait_settle_ctx.map(|(_, _, claim_token)| claim_token),
            background_run_permit.expect("background permit acquired for background mode"),
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
    store: &dyn SubagentSessionDelegate,
    context: &ToolContext,
    child_id: everruns_core::typed_id::SessionId,
    name: &str,
    _instructions: &str,
    blueprint_param: &Option<String>,
    task_id: Option<String>,
    settle_ctx: Option<(
        &dyn everruns_core::delegation_services::SubagentSpawnStore,
        &str,
        uuid::Uuid,
    )>,
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
    let result = result_value_for_task(context, task_id.as_deref())
        .await
        .unwrap_or_else(|| json!(result_text));

    ToolExecutionResult::success(json!({
        "subagent_id": child_id.to_string(),
        "name": name,
        "status": status,
        "result": result,
        "task_id": task_id,
        "blueprint": blueprint_param,
    }))
}

/// Collect the child's final message and, when `status` is terminal, settle
/// the spawn handle and mirror the outcome onto the session task. Non-terminal
/// statuses (paused, waiting_for_tool_results, timeout) only produce the
/// result text — the child stays active and the spawn stays reattachable.
async fn settle_subagent_outcome(
    store: &dyn SubagentSessionDelegate,
    context: &ToolContext,
    child_id: everruns_core::typed_id::SessionId,
    status: &str,
    task_id: Option<&str>,
    settle_ctx: Option<(
        &dyn everruns_core::delegation_services::SubagentSpawnStore,
        &str,
        uuid::Uuid,
    )>,
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
        let mut task_state = terminal_subagent_task_state(&subagent_status);
        let mut task_error = if task_state == SessionTaskState::Failed {
            Some(TaskError {
                kind: status.to_string(),
                message: format!("Subagent session ended with status: {status}"),
            })
        } else {
            None
        };
        let mut summary = Some(truncate_summary(&result_text));
        if task_state == SessionTaskState::Succeeded
            && required_result_is_missing(context, task_id).await
        {
            task_state = SessionTaskState::Failed;
            task_error = Some(TaskError {
                kind: "no_result".to_string(),
                message:
                    "Subagent completed without calling report_result for its result_schema task."
                        .to_string(),
            });
            summary = Some("Subagent completed without reporting a structured result.".to_string());
        }
        finish_subagent_task(context, task_id, task_state, summary, task_error).await;
    }

    Ok(result_text)
}

/// Detach a watcher that drives a background subagent to completion: send the
/// first message (fresh spawns only), heartbeat the task registry so the
/// reaper can detect worker loss, wait for the child's terminal turn status,
/// then settle the task and spawn handle. The task's OnTerminal wake policy
/// notifies the parent session at the registry level.
#[allow(clippy::too_many_arguments)]
fn spawn_background_watcher(
    context: &ToolContext,
    child_id: everruns_core::typed_id::SessionId,
    name: &str,
    first_message: Option<String>,
    task_id: Option<String>,
    task_attempt: i32,
    claim_token: Option<uuid::Uuid>,
    background_run_permit: BackgroundRunPermit,
) {
    let context = context.clone();
    let name = name.to_string();
    tokio::spawn(async move {
        let _background_run_permit = background_run_permit;
        let Some(store) = context.subagent_delegate.clone() else {
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

                // Local/embedded hosts (everruns-host) run the child's turn
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
                            spawn_store.as_ref()
                                as &dyn everruns_core::delegation_services::SubagentSpawnStore,
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
    ) -> everruns_core::error::Result<()> {
        let Some(store) = context.subagent_delegate.as_ref() else {
            return Err(everruns_core::error::AgentLoopError::tool(
                "subagent task delivery requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(everruns_core::error::AgentLoopError::tool(format!(
                "subagent task {} has no child session link",
                task.id
            )));
        };
        let text = task_message_text(&message.content);
        store.send_message(child_id, &text).await
    }

    async fn cancel(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_core::error::Result<()> {
        let Some(store) = context.subagent_delegate.as_ref() else {
            return Err(everruns_core::error::AgentLoopError::tool(
                "subagent task cancellation requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(everruns_core::error::AgentLoopError::tool(format!(
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
    ) -> everruns_core::error::Result<()> {
        if task.state.is_terminal() {
            return Ok(());
        }
        let (Some(store), Some(child_id)) = (
            context.subagent_delegate.as_ref(),
            task.links.child_session_id,
        ) else {
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
            everruns_core::error::AgentLoopError::tool(
                "Failed to read subagent result during reconcile",
            )
        })
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(SubagentTaskExecutor),
    }
}

/// Control plane for detached peer-session tracking tasks. `cancel` means
/// cancel everywhere (EVE-766): it cooperatively requests the peer session to
/// stop via the standard session cancel path, then settles the tracking task
/// `canceled`. The peer is a same-org session (inherited via fork lineage), so
/// the cooperative-cancel message routes through the ordinary send path — the
/// same mechanism linked subagents use.
pub struct DetachedSessionTaskExecutor;

#[async_trait]
impl TaskExecutor for DetachedSessionTaskExecutor {
    fn kind(&self) -> &str {
        TASK_KIND_SESSION
    }

    async fn cancel(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_core::error::Result<()> {
        let Some(registry) = context.session_task_registry.as_ref() else {
            return Ok(());
        };
        // Option A (EVE-766): cancel actually cancels. Deliver a cooperative
        // stop to the peer session first; only settle the tracking task once
        // the request is in flight, so a delivery failure surfaces to the
        // caller instead of silently claiming the peer was canceled.
        let summary = match (
            context.subagent_delegate.as_ref(),
            task.links.child_session_id,
        ) {
            (Some(store), Some(peer_id)) => {
                store
                    .send_message(
                        peer_id,
                        "Cancellation requested by the session that spawned you. Stop work, wind down, and end your run.",
                    )
                    .await?;
                "Peer session cancellation requested; tracking settled canceled.".to_string()
            }
            (None, Some(_)) => {
                return Err(everruns_core::error::AgentLoopError::tool(
                    "detached session task cancellation requires platform_store context",
                ));
            }
            // No peer link to signal (e.g. never wired): nothing to stop, just
            // settle the tracking task so the intent is honored.
            (_, None) => {
                "Detached session tracking canceled; no peer session link to signal.".to_string()
            }
        };
        registry
            .update(
                task.session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Canceled),
                    summary: Some(summary),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(DetachedSessionTaskExecutor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::Tool;
    use everruns_core::session_task::{TaskMessageDirection, TaskMessagePart, task_result_path};

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
            Some(everruns_core::session::SubagentStatus::Completed)
        );
        assert_eq!(
            terminal_subagent_status("failed"),
            Some(everruns_core::session::SubagentStatus::Failed)
        );
        assert_eq!(
            terminal_subagent_status("cancelled"),
            Some(everruns_core::session::SubagentStatus::Cancelled)
        );
        assert_eq!(
            terminal_subagent_status("sealed"),
            Some(everruns_core::session::SubagentStatus::Sealed)
        );
        assert_eq!(
            terminal_subagent_task_state(&everruns_core::session::SubagentStatus::Completed),
            SessionTaskState::Succeeded
        );
        // A sealed subagent settles as a terminal, non-retryable failed task.
        assert_eq!(
            terminal_subagent_task_state(&everruns_core::session::SubagentStatus::Sealed),
            SessionTaskState::Failed
        );
        assert_eq!(
            terminal_subagent_task_state(&everruns_core::session::SubagentStatus::Cancelled),
            SessionTaskState::Canceled
        );
        assert_eq!(
            terminal_subagent_task_state(
                &everruns_core::session::SubagentStatus::MaxIterationsReached
            ),
            SessionTaskState::Failed
        );
        assert_eq!(terminal_subagent_status("waiting_for_tool_results"), None);
        assert_eq!(terminal_subagent_status("paused"), None);
    }

    #[test]
    fn subagent_nesting_policy_resolves_platform_org_agent_precedence() {
        let platform = everruns_core::delegation_services::SubagentNestingPolicy::default()
            .with_platform_default(4);
        assert_eq!(platform.max_subagent_depth(), 4);

        let org = platform.with_org_override(Some(3));
        assert_eq!(org.max_subagent_depth(), 3);

        let agent = org.with_agent_override(Some(1));
        assert_eq!(agent.max_subagent_depth(), 1);
    }

    #[test]
    fn subagent_capability_is_high_risk() {
        assert_eq!(SubagentCapability.risk_level(), RiskLevel::High);
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
        assert!(props.contains_key("result_schema"));
        assert!(props.contains_key("message_schema"));
        assert!(!required.contains(&json!("blueprint")));
        assert!(!required.contains(&json!("config")));
        assert_eq!(
            schema["properties"]["mode"]["enum"],
            json!(["background", "foreground"])
        );
        assert_eq!(
            tool.hints().concurrency_class.as_deref(),
            Some(SPAWN_AGENT_CONCURRENCY_CLASS),
            "spawn_agent calls share one scheduler class so cap admission is serialized"
        );
    }

    // =========================================================================
    // Spawn handle tests (EVE-535)
    // =========================================================================

    use everruns_core::{
        delegation_services::SpawnClaimResult, delegation_services::SubagentSpawnStore,
    };
    use std::sync::Arc;

    struct TestSubagentSpawnStore;

    #[async_trait]
    impl SubagentSpawnStore for TestSubagentSpawnStore {
        async fn try_claim_spawn(
            &self,
            _parent_session_id: SessionId,
            _tool_call_id: &str,
            claim_token: uuid::Uuid,
        ) -> everruns_core::Result<SpawnClaimResult> {
            Ok(SpawnClaimResult::Claimed {
                spawn_handle_id: uuid::Uuid::new_v4(),
                claim_token,
            })
        }

        async fn register_child_session(
            &self,
            _spawn_handle_id: uuid::Uuid,
            _claim_token: uuid::Uuid,
            _child_session_id: SessionId,
        ) -> everruns_core::Result<()> {
            Ok(())
        }

        async fn settle_spawn(
            &self,
            _parent_session_id: SessionId,
            _tool_call_id: &str,
            _claim_token: uuid::Uuid,
            _terminal_status: &str,
            _terminal_result: &str,
        ) -> everruns_core::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn spawn_store_contract_can_claim() {
        let store = TestSubagentSpawnStore;
        let parent = everruns_core::typed_id::SessionId::new();
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

    #[tokio::test]
    async fn spawn_store_contract_registers_and_settles() {
        let store = TestSubagentSpawnStore;
        let parent = everruns_core::typed_id::SessionId::new();
        let child = everruns_core::typed_id::SessionId::new();
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
        let store: Arc<dyn SubagentSpawnStore> = Arc::new(TestSubagentSpawnStore);
        let parent = everruns_core::typed_id::SessionId::new();
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
    use crate::{PlatformStore, PlatformStoreSubagentDelegate};
    use chrono::Utc;
    use everruns_core::session_file::SessionFile;
    use everruns_core::session_files::SessionFileSystem;
    use everruns_core::session_task::SessionTaskRegistry;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn delegate(
        store: Arc<MockPlatformStore>,
    ) -> Arc<dyn everruns_core::subagent_delegation::SubagentSessionDelegate> {
        Arc::new(PlatformStoreSubagentDelegate(store))
    }

    /// SessionStore view over the mock platform store (depth-policy lookup).
    struct MockSessionStore(Arc<MockPlatformStore>);

    struct MockSessionCreationAuthority {
        root: everruns_core::typed_id::SessionId,
        allowed: bool,
    }

    #[async_trait]
    impl everruns_core::delegation_services::SessionCreationAuthority for MockSessionCreationAuthority {
        async fn authorize_session_creation(
            &self,
            _session_id: everruns_core::typed_id::SessionId,
        ) -> everruns_core::error::Result<everruns_core::typed_id::SessionId> {
            if self.allowed {
                Ok(self.root)
            } else {
                Err(everruns_core::error::AgentLoopError::tool(
                    "org:sessions:manage is required",
                ))
            }
        }
    }

    #[async_trait]
    impl everruns_core::execution_loading::SessionStore for MockSessionStore {
        async fn get_session(
            &self,
            session_id: everruns_core::typed_id::SessionId,
        ) -> everruns_core::error::Result<Option<everruns_core::session::ExecutionSession>>
        {
            // EVE-882: the store holds the platform record; execution sees the
            // projected view.
            Ok(self
                .0
                .get_session_by_id(session_id)
                .await?
                .map(|session| session.execution_session()))
        }
    }

    fn spawn_context(
        store: &Arc<MockPlatformStore>,
        registry: Option<Arc<InMemorySessionTaskRegistry>>,
    ) -> ToolContext {
        spawn_context_for_session(store, registry, store.session.id)
    }

    fn spawn_context_for_session(
        store: &Arc<MockPlatformStore>,
        registry: Option<Arc<InMemorySessionTaskRegistry>>,
        session_id: everruns_core::typed_id::SessionId,
    ) -> ToolContext {
        let mut context = ToolContext::new(session_id);
        context.subagent_delegate = Some(delegate(store.clone()));
        context.session_store = Some(Arc::new(MockSessionStore(store.clone())));
        context.session_creation_authority = Some(Arc::new(MockSessionCreationAuthority {
            root: store.session.id,
            allowed: true,
        }));
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
        session_id: everruns_core::typed_id::SessionId,
        task_id: &str,
        state: everruns_core::session_task::SessionTaskState,
    ) -> everruns_core::session_task::SessionTask {
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

    #[derive(Default)]
    struct MemoryFileStore {
        files: Mutex<HashMap<(uuid::Uuid, String), String>>,
    }

    #[async_trait]
    impl SessionFileSystem for MemoryFileStore {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            session_id: everruns_core::typed_id::SessionId,
            path: &str,
        ) -> everruns_core::error::Result<Option<SessionFile>> {
            let content = self
                .files
                .lock()
                .unwrap()
                .get(&(session_id.uuid(), path.to_string()))
                .cloned();
            Ok(content.map(|content| SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                content: Some(content.clone()),
                encoding: "utf-8".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }))
        }

        async fn write_file(
            &self,
            session_id: everruns_core::typed_id::SessionId,
            path: &str,
            content: &str,
            _encoding: &str,
        ) -> everruns_core::error::Result<SessionFile> {
            self.files
                .lock()
                .unwrap()
                .insert((session_id.uuid(), path.to_string()), content.to_string());
            Ok(SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                content: Some(content.to_string()),
                encoding: "utf-8".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            session_id: everruns_core::typed_id::SessionId,
            path: &str,
            _recursive: bool,
        ) -> everruns_core::error::Result<bool> {
            Ok(self
                .files
                .lock()
                .unwrap()
                .remove(&(session_id.uuid(), path.to_string()))
                .is_some())
        }

        async fn list_directory(
            &self,
            _session_id: everruns_core::typed_id::SessionId,
            _path: &str,
        ) -> everruns_core::error::Result<Vec<everruns_core::session_file::FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(
            &self,
            session_id: everruns_core::typed_id::SessionId,
            path: &str,
        ) -> everruns_core::error::Result<Option<everruns_core::session_file::FileStat>> {
            let content = self
                .files
                .lock()
                .unwrap()
                .get(&(session_id.uuid(), path.to_string()))
                .cloned();
            Ok(
                content.map(|content| everruns_core::session_file::FileStat {
                    path: path.to_string(),
                    name: path.rsplit('/').next().unwrap_or(path).to_string(),
                    is_directory: false,
                    is_readonly: false,
                    size_bytes: content.len() as i64,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }),
            )
        }

        async fn grep_files(
            &self,
            _session_id: everruns_core::typed_id::SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> everruns_core::error::Result<Vec<everruns_core::session_file::GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(
            &self,
            session_id: everruns_core::typed_id::SessionId,
            path: &str,
        ) -> everruns_core::error::Result<everruns_core::session_file::FileInfo> {
            Ok(everruns_core::session_file::FileInfo {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }
    }

    #[tokio::test]
    async fn spawn_agent_subagent_rejects_invalid_mode() {
        let context = ToolContext::new(everruns_core::typed_id::SessionId::new());
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
        let context = ToolContext::new(everruns_core::typed_id::SessionId::new());
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
    async fn detached_spawn_creates_peer_session_task_with_goal_and_lineage() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));

        let result = spawn(
            &context,
            json!({
                "name": "Research Peer",
                "goal": "Investigate latency",
                "instructions": "go",
                "lifetime": "detached",
                "seed": "workspace",
                "mode": "foreground"
            }),
        )
        .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        let child_id: everruns_core::typed_id::SessionId = value["subagent_id"]
            .as_str()
            .expect("subagent_id")
            .parse()
            .expect("valid session id");
        let child = store
            .get_session_by_id(child_id)
            .await
            .unwrap()
            .expect("child session");
        assert_eq!(child.parent_session_id, None);
        assert_eq!(child.forked_from_session_id, Some(context.session_id));
        assert_eq!(child.title.as_deref(), Some("Research Peer"));
        assert_eq!(child.goal.as_deref(), Some("Investigate latency"));
        assert_eq!(
            store
                .created_session_budget_roots
                .lock()
                .unwrap()
                .as_slice(),
            &[Some(store.session.id)]
        );

        let task_id = value["task_id"].as_str().expect("task_id");
        let task = registry
            .get(context.session_id, task_id)
            .await
            .unwrap()
            .expect("task");
        assert_eq!(task.kind, TASK_KIND_SESSION);
        assert_eq!(task.wake_policy, TaskWakePolicy::Silent);
        assert_eq!(task.links.child_session_id, Some(child_id));
        assert_eq!(task.spec["lifetime"], "detached");
        assert_eq!(task.spec["seed"], "workspace");
    }

    #[tokio::test]
    async fn detached_spawn_requires_session_creation_authority_before_creation() {
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let mut context = spawn_context(&store, Some(registry));
        context.session_creation_authority = None;

        let result = spawn(
            &context,
            json!({"name": "Denied", "instructions": "go", "lifetime": "detached"}),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected authority ToolError, got {result:?}");
        };
        assert!(message.contains("session-creation authority"));
        assert!(
            store
                .created_session_budget_roots
                .lock()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn detached_spawn_reports_permission_denial_before_creation() {
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let mut context = spawn_context(&store, Some(registry));
        context.session_creation_authority = Some(Arc::new(MockSessionCreationAuthority {
            root: store.session.id,
            allowed: false,
        }));

        let result = spawn(
            &context,
            json!({"name": "Denied", "instructions": "go", "lifetime": "detached"}),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected permission ToolError, got {result:?}");
        };
        assert!(message.contains("not authorized"));
        assert!(message.contains("org:sessions:manage"));
        assert!(
            store
                .created_session_budget_roots
                .lock()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn detached_spawn_bypasses_subagent_depth_guard() {
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry)).with_subagent_nesting_policy(
            everruns_core::delegation_services::SubagentNestingPolicy::default()
                .with_agent_override(Some(0)),
        );

        let linked = spawn(
            &context,
            json!({"name": "Linked", "instructions": "go", "mode": "foreground"}),
        )
        .await;
        assert!(matches!(linked, ToolExecutionResult::ToolError(_)));

        let detached = spawn(
            &context,
            json!({
                "name": "Detached",
                "instructions": "go",
                "mode": "foreground",
                "lifetime": "detached"
            }),
        )
        .await;
        assert!(
            matches!(detached, ToolExecutionResult::Success(_)),
            "detached spawn should bypass linked depth guard, got {detached:?}"
        );
    }

    // EVE-767: detached spawns reset depth but are still capped against the
    // origin root so a loop of detached spawns cannot run unbounded (TM-DOS).

    fn session_task_under(
        root: everruns_core::typed_id::SessionId,
        kind: &str,
        state: SessionTaskState,
    ) -> CreateSessionTask {
        CreateSessionTask {
            session_id: root,
            id: None,
            kind: kind.to_string(),
            display_name: "t".to_string(),
            spec: json!({}),
            state,
            links: TaskLinks {
                child_session_id: Some(everruns_core::typed_id::SessionId::new()),
                ..Default::default()
            },
            wake_policy: TaskWakePolicy::Silent,
        }
    }

    #[tokio::test]
    async fn detached_task_counts_ignore_subagent_and_terminal_active() {
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let root = store.session.id;

        registry
            .create(session_task_under(
                root,
                TASK_KIND_SESSION,
                SessionTaskState::Running,
            ))
            .await
            .unwrap();
        registry
            .create(session_task_under(
                root,
                TASK_KIND_SESSION,
                SessionTaskState::Running,
            ))
            .await
            .unwrap();
        // Terminal detached task: counts toward total, not active.
        registry
            .create(session_task_under(
                root,
                TASK_KIND_SESSION,
                SessionTaskState::Canceled,
            ))
            .await
            .unwrap();
        // Subagent task: must not count toward the detached budget at all.
        registry
            .create(session_task_under(
                root,
                TASK_KIND_SUBAGENT,
                SessionTaskState::Running,
            ))
            .await
            .unwrap();

        let counts = descendant_detached_task_counts(registry.as_ref(), root, 100, 100)
            .await
            .unwrap();
        assert_eq!(
            counts.active, 2,
            "only non-terminal session tasks are active"
        );
        assert_eq!(
            counts.total, 3,
            "terminal session task counts toward total; subagent task excluded"
        );
    }

    #[tokio::test]
    async fn detached_spawn_rejected_at_cap_and_allowed_under_cap() {
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone())).with_subagent_nesting_policy(
            everruns_core::delegation_services::SubagentNestingPolicy::default()
                .with_agent_detached_task_caps_override(Some(1), Some(4)),
        );

        // Under the ceiling (0 existing): one authorized detached spawn succeeds.
        let ok = spawn(
            &context,
            json!({"name": "D0", "instructions": "go", "mode": "background", "lifetime": "detached"}),
        )
        .await;
        assert!(
            matches!(ok, ToolExecutionResult::Success(_)),
            "detached spawn under cap should succeed, got {ok:?}"
        );

        // The spawn created one active detached peer task under the root → at
        // the active cap. The next detached spawn is refused with a clear error.
        let refused = spawn(
            &context,
            json!({"name": "D1", "instructions": "go", "mode": "background", "lifetime": "detached"}),
        )
        .await;
        let ToolExecutionResult::ToolError(msg) = refused else {
            panic!("expected detached active cap ToolError, got {refused:?}");
        };
        assert!(
            msg.contains("max_active_detached_tasks is 1"),
            "cap error should name the limit, got: {msg}"
        );
    }

    #[tokio::test]
    async fn detached_cap_does_not_affect_linked_subagent_spawn() {
        // A root already at the detached ceiling must still allow linked
        // subagent spawns — the two budgets are independent (regression guard).
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone())).with_subagent_nesting_policy(
            everruns_core::delegation_services::SubagentNestingPolicy::default()
                .with_agent_detached_task_caps_override(Some(1), Some(4)),
        );
        let root = store.session.id;

        // Saturate the detached active cap.
        registry
            .create(session_task_under(
                root,
                TASK_KIND_SESSION,
                SessionTaskState::Running,
            ))
            .await
            .unwrap();

        // A detached spawn is refused…
        let refused = spawn(
            &context,
            json!({"name": "D", "instructions": "go", "mode": "background", "lifetime": "detached"}),
        )
        .await;
        assert!(matches!(refused, ToolExecutionResult::ToolError(_)));

        // …but a linked subagent spawn is unaffected by the detached cap.
        let linked = spawn(
            &context,
            json!({"name": "L", "instructions": "go", "mode": "background"}),
        )
        .await;
        assert!(
            matches!(linked, ToolExecutionResult::Success(_)),
            "linked subagent spawn must not be blocked by the detached cap, got {linked:?}"
        );
    }

    #[tokio::test]
    async fn detached_session_task_cancel_requests_peer_cancellation() {
        // EVE-766: cancel_task on a detached-session task must cooperatively
        // cancel the peer session, not just detach the tracking chip.
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));
        let child_id = everruns_core::typed_id::SessionId::new();
        let task = registry
            .create(CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: TASK_KIND_SESSION.to_string(),
                display_name: "Peer".to_string(),
                spec: json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        DetachedSessionTaskExecutor
            .cancel(&task, &context)
            .await
            .unwrap();

        // The peer session was signaled to stop via the standard send path.
        let sent = store.sent_messages.lock().unwrap().clone();
        assert_eq!(
            sent.len(),
            1,
            "exactly one cooperative-cancel message expected, got {sent:?}"
        );
        assert_eq!(sent[0].0, child_id, "cancel must target the peer session");
        assert!(
            sent[0].1.contains("Cancellation requested"),
            "cancel message should ask the peer to stop, got {:?}",
            sent[0].1
        );

        // The tracking task settles canceled and keeps its peer link.
        let updated = registry
            .get(context.session_id, &task.id)
            .await
            .unwrap()
            .expect("task should remain present");
        assert_eq!(updated.state, SessionTaskState::Canceled);
        assert_eq!(updated.links.child_session_id, Some(child_id));
        assert_eq!(
            updated.summary.as_deref(),
            Some("Peer session cancellation requested; tracking settled canceled.")
        );
    }

    #[tokio::test]
    async fn detached_session_task_cancel_without_peer_link_still_settles() {
        // Defensive: a session-kind task with no peer link has nothing to
        // signal, but the cancel intent must still be honored.
        let store = Arc::new(MockPlatformStore::new());
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry.clone()));
        let task = registry
            .create(CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: TASK_KIND_SESSION.to_string(),
                display_name: "Peer".to_string(),
                spec: json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        DetachedSessionTaskExecutor
            .cancel(&task, &context)
            .await
            .unwrap();

        assert!(store.sent_messages.lock().unwrap().is_empty());
        let updated = registry
            .get(context.session_id, &task.id)
            .await
            .unwrap()
            .expect("task should remain present");
        assert_eq!(updated.state, SessionTaskState::Canceled);
        assert_eq!(
            updated.summary.as_deref(),
            Some("Detached session tracking canceled; no peer session link to signal.")
        );
    }

    #[tokio::test]
    async fn detached_session_task_cancel_without_platform_store_fails_closed() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let mut context = ToolContext::new(everruns_core::typed_id::SessionId::new());
        context.session_task_registry = Some(registry.clone());
        let task = registry
            .create(CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: TASK_KIND_SESSION.to_string(),
                display_name: "Peer".to_string(),
                spec: json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(everruns_core::typed_id::SessionId::new()),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let error = DetachedSessionTaskExecutor
            .cancel(&task, &context)
            .await
            .expect_err("missing platform store must prevent false cancellation");

        assert!(
            error
                .to_string()
                .contains("requires platform_store context")
        );
        let unchanged = registry
            .get(context.session_id, &task.id)
            .await
            .unwrap()
            .expect("task should remain present");
        assert_eq!(unchanged.state, SessionTaskState::Running);
        assert!(unchanged.summary.is_none());
    }

    #[tokio::test]
    async fn spawn_agent_subagent_allows_depth_two_and_rejects_depth_three_by_default() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let root_context = spawn_context(&store, Some(registry.clone()));

        let first = spawn(
            &root_context,
            json!({"name": "B", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::Success(first_value) = first else {
            panic!("expected first spawn success, got {first:?}");
        };
        let b_id: everruns_core::typed_id::SessionId = first_value["subagent_id"]
            .as_str()
            .expect("subagent_id")
            .parse()
            .expect("valid session id");

        let b_context = spawn_context_for_session(&store, Some(registry.clone()), b_id);
        let second = spawn(
            &b_context,
            json!({"name": "C", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::Success(second_value) = second else {
            panic!("expected second spawn success, got {second:?}");
        };
        let c_id: everruns_core::typed_id::SessionId = second_value["subagent_id"]
            .as_str()
            .expect("subagent_id")
            .parse()
            .expect("valid session id");

        let c_context = spawn_context_for_session(&store, Some(registry), c_id);
        let third = spawn(
            &c_context,
            json!({"name": "D", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = third else {
            panic!("expected depth cap ToolError, got {third:?}");
        };
        assert!(
            message.contains("max_subagent_depth is 2"),
            "got: {message}"
        );
        assert!(message.contains("depth 3"), "got: {message}");
    }

    #[tokio::test]
    async fn spawn_agent_subagent_depth_zero_restores_hard_block() {
        let store = Arc::new(MockPlatformStore::new());
        let mut context = spawn_context(&store, None).with_subagent_nesting_policy(
            everruns_core::delegation_services::SubagentNestingPolicy::default()
                .with_agent_override(Some(0)),
        );
        context.session_task_registry = Some(Arc::new(InMemorySessionTaskRegistry::default()));

        let result = spawn(
            &context,
            json!({"name": "Blocked", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected depth cap ToolError, got {result:?}");
        };
        assert!(
            message.contains("max_subagent_depth is 0"),
            "got: {message}"
        );
        assert!(message.contains("depth 1"), "got: {message}");
    }

    #[tokio::test]
    async fn spawn_agent_subagent_rejects_when_active_descendant_cap_is_full() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "waiting_for_tool_results".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry)).with_subagent_nesting_policy(
            everruns_core::delegation_services::SubagentNestingPolicy::default()
                .with_agent_task_caps_override(Some(1), Some(200)),
        );

        let first = spawn(
            &context,
            json!({"name": "First", "instructions": "go", "mode": "background"}),
        )
        .await;
        assert!(
            matches!(first, ToolExecutionResult::Success(_)),
            "expected first spawn success, got {first:?}"
        );

        let second = spawn(
            &context,
            json!({"name": "Second", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = second else {
            panic!("expected active cap ToolError, got {second:?}");
        };
        assert!(
            message.contains("max_active_descendant_tasks is 1"),
            "got: {message}"
        );
        assert!(
            message.contains("2 non-terminal descendant tasks"),
            "got: {message}"
        );
    }

    #[tokio::test]
    async fn spawn_agent_subagent_counts_grandchildren_for_active_descendant_cap() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "waiting_for_tool_results".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let policy = everruns_core::delegation_services::SubagentNestingPolicy::default()
            .with_agent_override(Some(4))
            .with_agent_task_caps_override(Some(2), Some(200));
        let root_context =
            spawn_context(&store, Some(registry.clone())).with_subagent_nesting_policy(policy);

        let first = spawn(
            &root_context,
            json!({"name": "B", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::Success(first_value) = first else {
            panic!("expected first spawn success, got {first:?}");
        };
        let b_id: everruns_core::typed_id::SessionId = first_value["subagent_id"]
            .as_str()
            .expect("subagent_id")
            .parse()
            .expect("valid session id");

        let b_context = spawn_context_for_session(&store, Some(registry.clone()), b_id)
            .with_subagent_nesting_policy(policy);
        let second = spawn(
            &b_context,
            json!({"name": "C", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::Success(second_value) = second else {
            panic!("expected second spawn success, got {second:?}");
        };
        let c_id: everruns_core::typed_id::SessionId = second_value["subagent_id"]
            .as_str()
            .expect("subagent_id")
            .parse()
            .expect("valid session id");

        let c_context = spawn_context_for_session(&store, Some(registry), c_id)
            .with_subagent_nesting_policy(policy);
        let third = spawn(
            &c_context,
            json!({"name": "D", "instructions": "go", "mode": "background"}),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = third else {
            panic!("expected active cap ToolError, got {third:?}");
        };
        assert!(
            message.contains("max_active_descendant_tasks is 2"),
            "got: {message}"
        );
        assert!(message.contains("root session"), "got: {message}");
    }

    #[tokio::test]
    async fn spawn_agent_subagent_total_descendant_cap_counts_terminal_tasks() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry)).with_subagent_nesting_policy(
            everruns_core::delegation_services::SubagentNestingPolicy::default()
                .with_agent_task_caps_override(Some(16), Some(1)),
        );

        let first = spawn(
            &context,
            json!({"name": "First", "instructions": "go", "mode": "foreground"}),
        )
        .await;
        assert!(
            matches!(first, ToolExecutionResult::Success(_)),
            "expected first spawn success, got {first:?}"
        );

        let second = spawn(
            &context,
            json!({"name": "Second", "instructions": "go", "mode": "foreground"}),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = second else {
            panic!("expected total cap ToolError, got {second:?}");
        };
        assert!(
            message.contains("max_total_descendant_tasks is 1"),
            "got: {message}"
        );
        assert!(
            message.contains("2 descendant task records"),
            "got: {message}"
        );
    }

    #[test]
    fn subagents_config_validates_descendant_task_caps() {
        let capability = SubagentCapability;
        assert!(
            capability
                .validate_config(&json!({
                    "max_active_descendant_tasks": 16,
                    "max_total_descendant_tasks": 200
                }))
                .is_ok()
        );
        assert_eq!(
            capability
                .validate_config(&json!({"max_active_descendant_tasks": 1025}))
                .unwrap_err(),
            "max_active_descendant_tasks must be <= 1024"
        );
        assert_eq!(
            capability
                .validate_config(&json!({"max_total_descendant_tasks": 10001}))
                .unwrap_err(),
            "max_total_descendant_tasks must be <= 10000"
        );
    }

    #[tokio::test]
    async fn spawn_agent_subagent_stores_result_schema_on_task() {
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
                    "mode": "foreground",
                    "result_schema": {
                        "type": "object",
                        "properties": {"answer": {"type": "string"}},
                        "required": ["answer"],
                        "additionalProperties": false
                    }
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
        assert_eq!(task.spec["result_schema"]["required"], json!(["answer"]));
        assert_eq!(task.state, SessionTaskState::Failed);
        assert_eq!(
            task.error.as_ref().map(|e| e.kind.as_str()),
            Some("no_result")
        );
    }

    #[tokio::test]
    async fn report_result_writes_result_file_and_updates_task() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let file_store = Arc::new(MemoryFileStore::default());
        let parent_session_id = everruns_core::typed_id::SessionId::new();
        let parent_workspace_id =
            everruns_core::typed_id::WorkspaceId::from_uuid(parent_session_id.uuid());
        let child_session_id = everruns_core::typed_id::SessionId::new();
        let task = registry
            .create(CreateSessionTask {
                session_id: parent_session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Runner".to_string(),
                spec: json!({
                    "result_schema": {
                        "type": "object",
                        "properties": {"answer": {"type": "string"}},
                        "required": ["answer"],
                        "additionalProperties": false
                    }
                }),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_session_id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let tool = ReportResultTool::new(
            parent_session_id,
            parent_workspace_id,
            child_session_id,
            task.id.clone(),
            task.spec["result_schema"].clone(),
        )
        .with_file_store(file_store.clone());
        let mut context = ToolContext::new(child_session_id);
        context.session_task_registry = Some(registry.clone());

        let result = tool
            .execute_with_context(json!({"answer": "done"}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        assert_eq!(value["result_path"], task_result_path(&task.id));

        let task = registry
            .get(parent_session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        let result_path = task.result_path.as_deref().expect("result_path");
        let file = file_store
            .read_file(
                SessionId::from_uuid(parent_workspace_id.uuid()),
                result_path,
            )
            .await
            .unwrap()
            .expect("result file");
        assert_eq!(
            serde_json::from_str::<Value>(file.content.as_deref().unwrap()).unwrap(),
            json!({"answer": "done"})
        );
    }

    #[tokio::test]
    async fn report_result_rejects_terminal_task_without_overwriting_result() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let file_store = Arc::new(MemoryFileStore::default());
        let parent_session_id = everruns_core::typed_id::SessionId::new();
        let parent_workspace_id =
            everruns_core::typed_id::WorkspaceId::from_uuid(parent_session_id.uuid());
        let child_session_id = everruns_core::typed_id::SessionId::new();
        let task = registry
            .create(CreateSessionTask {
                session_id: parent_session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Runner".to_string(),
                spec: json!({
                    "result_schema": {
                        "type": "object",
                        "properties": {"answer": {"type": "string"}},
                        "required": ["answer"],
                        "additionalProperties": false
                    }
                }),
                state: SessionTaskState::Succeeded,
                links: TaskLinks {
                    child_session_id: Some(child_session_id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        let existing_path = task_result_path(&task.id);
        registry
            .update(
                parent_session_id,
                &task.id,
                SessionTaskUpdate {
                    result_path: Some(existing_path.clone()),
                    summary: Some("original".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        file_store
            .write_file(
                SessionId::from_uuid(parent_workspace_id.uuid()),
                &existing_path,
                "{\n  \"answer\": \"original\"\n}",
                "utf-8",
            )
            .await
            .unwrap();

        let tool = ReportResultTool::new(
            parent_session_id,
            parent_workspace_id,
            child_session_id,
            task.id.clone(),
            task.spec["result_schema"].clone(),
        )
        .with_file_store(file_store.clone());
        let mut context = ToolContext::new(child_session_id);
        context.session_task_registry = Some(registry.clone());

        let result = tool
            .execute_with_context(json!({"answer": "tampered"}), &context)
            .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected terminal rejection, got {result:?}");
        };
        assert!(message.contains("terminal"), "got: {message}");

        let file = file_store
            .read_file(
                SessionId::from_uuid(parent_workspace_id.uuid()),
                &existing_path,
            )
            .await
            .unwrap()
            .expect("result file");
        assert!(
            file.content.as_deref().unwrap().contains("original"),
            "file was overwritten: {file:?}"
        );
    }

    #[tokio::test]
    async fn report_result_rejects_invalid_result_schema_payload() {
        let tool = ReportResultTool::new(
            everruns_core::typed_id::SessionId::new(),
            everruns_core::typed_id::WorkspaceId::from_uuid(uuid::Uuid::new_v4()),
            everruns_core::typed_id::SessionId::new(),
            "task_test".to_string(),
            json!({
                "type": "object",
                "properties": {"answer": {"type": "string"}},
                "required": ["answer"],
                "additionalProperties": false
            }),
        );
        let result = tool
            .execute_with_context(json!({"extra": true}), &ToolContext::new(SessionId::new()))
            .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected validation error, got {result:?}");
        };
        assert!(
            message.contains("answer") && message.contains("required"),
            "got: {message}"
        );
        assert!(
            message.contains("extra")
                && (message.contains("additional") || message.contains("not allowed")),
            "got: {message}"
        );
    }

    #[tokio::test]
    async fn spawn_agent_subagent_stores_message_schema_and_wakes_on_activity() {
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
                    "message_schema": {
                        "type": "object",
                        "properties": {"step": {"type": "string"}},
                        "required": ["step"],
                        "additionalProperties": false
                    }
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
        assert_eq!(task.spec["message_schema"]["required"], json!(["step"]));
        assert_eq!(task.wake_policy, TaskWakePolicy::OnActivity);
    }

    #[tokio::test]
    async fn report_task_progress_posts_structured_outbound_message() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let parent_session_id = everruns_core::typed_id::SessionId::new();
        let task = registry
            .create(CreateSessionTask {
                session_id: parent_session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Runner".to_string(),
                spec: json!({
                    "message_schema": {
                        "type": "object",
                        "properties": {"step": {"type": "string"}},
                        "required": ["step"],
                        "additionalProperties": false
                    }
                }),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::OnActivity,
            })
            .await
            .unwrap();

        let tool = ReportTaskProgressTool::new(
            parent_session_id,
            task.id.clone(),
            task.attempt,
            task.spec["message_schema"].clone(),
        );
        assert_eq!(tool.name(), "report_task_progress");
        let mut context = ToolContext::new(everruns_core::typed_id::SessionId::new());
        context.session_task_registry = Some(registry.clone());

        let result = tool
            .execute_with_context(json!({"step": "tests-running"}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        assert_eq!(value["status"], "posted");

        let messages = registry
            .list_messages(parent_session_id, &task.id, None, None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].direction, TaskMessageDirection::Outbound);
        assert_eq!(
            messages[0].content,
            vec![TaskMessagePart::Data {
                data: json!({"step": "tests-running"})
            }]
        );
    }

    #[tokio::test]
    async fn report_task_progress_rejects_stale_task_attempt() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let parent_session_id = everruns_core::typed_id::SessionId::new();
        let task = registry
            .create(CreateSessionTask {
                session_id: parent_session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Runner".to_string(),
                spec: json!({"message_schema": {"type": "object"}}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::OnActivity,
            })
            .await
            .unwrap();
        let tool = ReportTaskProgressTool::new(
            parent_session_id,
            task.id.clone(),
            task.attempt,
            task.spec["message_schema"].clone(),
        );

        // Supersede the attempt the tool captured at construction.
        registry
            .update(
                parent_session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Failed),
                    increment_attempt: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let mut context = ToolContext::new(everruns_core::typed_id::SessionId::new());
        context.session_task_registry = Some(registry.clone());
        let result = tool
            .execute_with_context(json!({"step": "late"}), &context)
            .await;
        assert!(
            matches!(result, ToolExecutionResult::InternalError(_)),
            "stale progress must be rejected, got {result:?}"
        );
        let messages = registry
            .list_messages(parent_session_id, &task.id, None, None)
            .await
            .unwrap();
        assert!(
            messages.is_empty(),
            "stale progress must not append messages"
        );
    }

    #[tokio::test]
    async fn report_task_progress_rejects_invalid_message_schema_payload() {
        let tool = ReportTaskProgressTool::new(
            everruns_core::typed_id::SessionId::new(),
            "task_test".to_string(),
            1,
            json!({
                "type": "object",
                "properties": {"step": {"type": "string"}},
                "required": ["step"],
                "additionalProperties": false
            }),
        );
        let result = tool
            .execute_with_context(
                json!({"step": 42, "extra": true}),
                &ToolContext::new(SessionId::new()),
            )
            .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected validation error, got {result:?}");
        };
        assert!(
            message.contains("step") && message.contains("string"),
            "got: {message}"
        );
        assert!(
            message.contains("extra")
                && (message.contains("additional") || message.contains("not allowed")),
            "got: {message}"
        );
    }

    #[test]
    fn subagent_and_channel_progress_tools_have_distinct_names() {
        // EVE-727: the subagent interim-progress tool must not share a wire name
        // with the channel-facing `report_progress` tool. `ToolRegistry` is keyed
        // by name, so a collision would silently drop one tool. This guards the
        // rename against regressions: both tools must coexist in one registry.
        use everruns_core::progress_reporting::{
            REPORT_PROGRESS_TOOL_NAME, ReportProgressTool as ChannelReportProgressTool,
        };
        use everruns_core::tools::ToolRegistry;

        let subagent = ReportTaskProgressTool::new(
            everruns_core::typed_id::SessionId::new(),
            "task_test".to_string(),
            1,
            json!({"type": "object"}),
        );
        assert_eq!(subagent.name(), "report_task_progress");
        assert_eq!(REPORT_PROGRESS_TOOL_NAME, "report_progress");
        assert_ne!(subagent.name(), REPORT_PROGRESS_TOOL_NAME);

        let mut registry = ToolRegistry::new();
        registry.register(ChannelReportProgressTool);
        registry.register(subagent);
        assert!(
            registry.has("report_progress"),
            "channel report_progress tool must survive"
        );
        assert!(
            registry.has("report_task_progress"),
            "subagent report_task_progress tool must survive"
        );
    }

    #[tokio::test]
    async fn explicit_background_without_registry_errors() {
        let context = ToolContext::new(everruns_core::typed_id::SessionId::new());
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
    async fn background_spawn_rejects_when_session_active_run_limit_reached() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "paused".to_string();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = spawn_context(&store, Some(registry));

        for index in 0..crate::background_run::MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION {
            let result = spawn(
                &context,
                json!({
                    "name": format!("Runner {index}"),
                    "instructions": "go",
                }),
            )
            .await;
            let ToolExecutionResult::Success(value) = result else {
                panic!("background spawn below the session limit should start: {result:?}");
            };
            assert_eq!(value["status"], "running");
        }

        let result = spawn(
            &context,
            json!({
                "name": "Runner over limit",
                "instructions": "go",
            }),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("background spawn should reject once the session limit is reached: {result:?}");
        };
        assert!(message.contains("active background runs per session"));
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

        let child_id = everruns_core::typed_id::SessionId::new();
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
                    child_session_id: Some(everruns_core::typed_id::SessionId::new()),
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
