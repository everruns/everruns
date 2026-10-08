// Coordination capability (knowledge/runtime-resources/coordination.md)
//
// A coordinator session hands work to threads and keeps the record of who is
// doing what. The person talks to the coordinator; the coordinator starts a
// thread per piece of work, relays follow-ups, and reports results back.
//
// Decision: a thread is a child session (parent_session_id = coordinator). It
//   lives across many units of work; each unit is one `assignment` task owned
//   by the coordinator and linked to the thread through `links.child_session_id`.
//   A thread runs at most one open assignment at a time.
// Decision: a thread finishes an assignment explicitly with
//   `complete_assignment`. A turn that ends without it is not a result: the
//   server's thread turn listener (crates/server/src/services/coordination.rs)
//   flags the assignment as needing attention, which wakes the coordinator.
// Decision: assignment tasks carry no heartbeat and no watcher. The reaper
//   ignores tasks without a heartbeat, so a thread can wait on the person for
//   days without being failed as orphaned, and threads use no background slot.
// Decision: threads talk to each other only through the coordinator (user
//   decision, 2026-10-08), so all routing has one owner and one record.
// Decision: worker tools (`update_checklist`, `complete_assignment`,
//   `ask_decision`, `report_to_coordinator`, `redirect_to_coordinator`) are
//   injected per turn by `PlatformToolAugmentor` into a session that has an
//   open assignment, the way `report_result` reaches subagents. Coordinator
//   tools are hidden inside threads: a thread does not start threads.
// Decision: relays reach a thread as plain user-role text with a framing line.
//   A tool-based reply channel is being designed separately; keeping relay text
//   in `frame_*` helpers keeps that swap local to this file.

use super::delegation_result::truncate_summary;
use super::util::{get_subagent_delegate, require_str_nonblank};
use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel};
use async_trait::async_trait;
use everruns_contracts::tool_types::{ToolDefinition, ToolHints};
use everruns_contracts::typed_id::{AgentId, SessionId};
use everruns_core::background::{BackgroundProgress, ProgressStep, ProgressStepStatus};
use everruns_core::execution_loading::SessionStore;
use everruns_core::localization::{BackendLocale, resolve_backend_locale};
use everruns_core::session::SessionSeedMode;
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TASK_KIND_ASSIGNMENT, TaskArtifact, TaskError,
    TaskExecutor, TaskExecutorPlugin, TaskInputRequest, TaskLinks, TaskMessage,
    TaskMessageDirection, TaskMessagePart, TaskWakePolicy, task_message_text,
};
use everruns_core::subagent_delegation::PlatformCreateSessionRequest;
use everruns_core::tool_context::{ToolContext, ToolContextService};
use everruns_core::tool_narration::{ToolNarrationPhase, labeled_phrase, safe_arg_str, truncate};
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

pub const COORDINATION_CAPABILITY_ID: &str = "coordination";

/// `state_detail` on a thread's latest assignment once the coordinator
/// resolved the thread. Reopening clears it.
pub const THREAD_RESOLVED_DETAIL: &str = "resolved";
const THREAD_REOPENED_DETAIL: &str = "reopened";

/// Coordinator tools, hidden from a session that is itself a thread.
pub const COORDINATOR_TOOL_NAMES: [&str; 5] = [
    "start_thread",
    "message_thread",
    "list_threads",
    "get_thread",
    "resolve_thread",
];
/// Worker tools, injected into a thread that has an open assignment.
pub const WORKER_TOOL_NAMES: [&str; 5] = [
    "update_checklist",
    "complete_assignment",
    "ask_decision",
    "report_to_coordinator",
    "redirect_to_coordinator",
];

const DEFAULT_MAX_ACTIVE_THREADS: u64 = 8;
const DEFAULT_MAX_TOTAL_THREADS: u64 = 100;
const MAX_TITLE_BYTES: usize = 200;
const MAX_BRIEF_BYTES: usize = 32 * 1024;
const MAX_REPORT_BYTES: usize = 8 * 1024;
const MAX_STEPS: usize = 20;
const MAX_ARTIFACTS: usize = 16;
const MAX_OPTIONS: usize = 4;
/// Coordinator relays into one assignment before the coordinator must ask the
/// person instead. Bounds a coordinator and a thread bouncing messages.
const MAX_RELAYS_PER_ASSIGNMENT: usize = 20;
const GET_THREAD_MESSAGE_LIMIT: usize = 20;

// =============================================================================
// Config
// =============================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    /// Starts and steers threads.
    #[default]
    Coordinator,
    /// Only works on threads another coordinator starts: no coordinator tools.
    Worker,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkerRef {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoordinationConfig {
    #[serde(default)]
    role: Role,
    /// Who may work a thread: `self` (this agent), `any` (any agent the person
    /// may run), or an agent id. Defaults to `self` only.
    #[serde(default = "default_workers")]
    workers: Vec<WorkerRef>,
    #[serde(default)]
    max_active_threads: Option<u64>,
    #[serde(default)]
    max_total_threads: Option<u64>,
}

impl Default for CoordinationConfig {
    fn default() -> Self {
        Self {
            role: Role::default(),
            workers: default_workers(),
            max_active_threads: None,
            max_total_threads: None,
        }
    }
}

fn default_workers() -> Vec<WorkerRef> {
    vec![WorkerRef {
        id: "self".to_string(),
    }]
}

impl CoordinationConfig {
    fn parse(config: &Value) -> Result<Self, String> {
        if config.is_null() {
            return Ok(Self::default());
        }
        serde_json::from_value(config.clone()).map_err(|error| error.to_string())
    }

    fn allows_self(&self) -> bool {
        self.workers.iter().any(|worker| worker.id == "self")
    }

    fn allows_agent(&self, agent_id: &str) -> bool {
        self.workers
            .iter()
            .any(|worker| worker.id == "any" || worker.id == agent_id)
    }

    fn max_active(&self) -> u64 {
        self.max_active_threads
            .unwrap_or(DEFAULT_MAX_ACTIVE_THREADS)
    }

    fn max_total(&self) -> u64 {
        self.max_total_threads.unwrap_or(DEFAULT_MAX_TOTAL_THREADS)
    }
}

// =============================================================================
// Capability
// =============================================================================

/// Lets an agent coordinate work across threads.
pub struct CoordinationCapability;

impl Capability for CoordinationCapability {
    fn id(&self) -> &str {
        COORDINATION_CAPABILITY_ID
    }

    fn narrate(
        &self,
        _tool_def: Option<&ToolDefinition>,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        narrate_coordination(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    fn name(&self) -> &str {
        "Coordination"
    }

    fn description(&self) -> &str {
        "Coordinate work across threads: start a thread per piece of work, relay follow-ups, and track each thread's checklist and result."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Координація",
            "Координуйте роботу в гілках: запускайте окрему гілку для кожного завдання, передавайте уточнення та відстежуйте чекліст і результат кожної гілки.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("network")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn risk_level(&self) -> RiskLevel {
        // Threads are sessions that spend: same footing as subagents.
        RiskLevel::High
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["coordination"]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "role": {
                    "type": "string",
                    "enum": ["coordinator", "worker"],
                    "default": "coordinator",
                    "description": "coordinator starts and steers threads; worker only works on threads another coordinator starts."
                },
                "workers": {
                    "type": "array",
                    "description": "Who may work a thread: {\"id\": \"self\"} for this agent, {\"id\": \"any\"} for any agent the person may run, or {\"id\": \"agent_...\"} for one agent. Defaults to self.",
                    "items": {
                        "type": "object",
                        "properties": { "id": { "type": "string" } },
                        "required": ["id"],
                        "additionalProperties": false
                    }
                },
                "max_active_threads": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 64,
                    "default": DEFAULT_MAX_ACTIVE_THREADS,
                    "description": "Threads that may be working at once."
                },
                "max_total_threads": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1000,
                    "default": DEFAULT_MAX_TOTAL_THREADS,
                    "description": "Threads one coordinator session may start in total."
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        let parsed = CoordinationConfig::parse(config)?;
        if parsed.workers.is_empty() {
            return Err("workers must name at least one worker".to_string());
        }
        for worker in &parsed.workers {
            if worker.id != "self" && worker.id != "any" && worker.id.parse::<AgentId>().is_err() {
                return Err(format!(
                    "workers[].id must be \"self\", \"any\", or an agent id, got \"{}\"",
                    worker.id
                ));
            }
        }
        if parsed.max_active().clamp(1, 64) != parsed.max_active() {
            return Err("max_active_threads must be between 1 and 64".to_string());
        }
        if parsed.max_total().clamp(1, 1000) != parsed.max_total() {
            return Err("max_total_threads must be between 1 and 1000".to_string());
        }
        Ok(())
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(COORDINATOR_SYSTEM_PROMPT)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        let config = CoordinationConfig::parse(config).unwrap_or_default();
        if config.role == Role::Worker {
            return Vec::new();
        }
        let config = Arc::new(config);
        vec![
            Box::new(StartThreadTool(config.clone())),
            Box::new(MessageThreadTool),
            Box::new(ListThreadsTool),
            Box::new(GetThreadTool),
            Box::new(ResolveThreadTool),
        ]
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }
}

/// Backend-authored narration for the coordinator and worker tools. Titles
/// are the model's own short labels; briefs and reports are never echoed.
fn narrate_coordination(
    tool_name: &str,
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> Option<String> {
    let uk = resolve_backend_locale(locale) == BackendLocale::Uk;
    let (en, ua): ((&str, &str, &str), (&str, &str, &str)) = match tool_name {
        "start_thread" => (
            (
                "Starting thread",
                "Started thread",
                "Could not start thread",
            ),
            (
                "Запускаю гілку",
                "Запустив гілку",
                "Не вдалося запустити гілку",
            ),
        ),
        "message_thread" => (
            (
                "Messaging thread",
                "Messaged thread",
                "Could not message thread",
            ),
            (
                "Пишу в гілку",
                "Написав у гілку",
                "Не вдалося написати в гілку",
            ),
        ),
        "list_threads" => (
            (
                "Listing threads",
                "Listed threads",
                "Could not list threads",
            ),
            (
                "Переглядаю гілки",
                "Переглянув гілки",
                "Не вдалося переглянути гілки",
            ),
        ),
        "get_thread" => (
            ("Reading thread", "Read thread", "Could not read thread"),
            (
                "Читаю гілку",
                "Прочитав гілку",
                "Не вдалося прочитати гілку",
            ),
        ),
        "resolve_thread" => (
            (
                "Resolving thread",
                "Resolved thread",
                "Could not resolve thread",
            ),
            ("Закриваю гілку", "Закрив гілку", "Не вдалося закрити гілку"),
        ),
        "update_checklist" => (
            (
                "Updating checklist",
                "Updated checklist",
                "Could not update checklist",
            ),
            ("Оновлюю план", "Оновив план", "Не вдалося оновити план"),
        ),
        "complete_assignment" => (
            (
                "Completing assignment",
                "Completed assignment",
                "Could not complete assignment",
            ),
            (
                "Завершую завдання",
                "Завершив завдання",
                "Не вдалося завершити завдання",
            ),
        ),
        "ask_decision" => (
            (
                "Asking for a decision",
                "Asked for a decision",
                "Could not ask for a decision",
            ),
            (
                "Прошу рішення",
                "Попросив рішення",
                "Не вдалося попросити рішення",
            ),
        ),
        "report_to_coordinator" => (
            (
                "Reporting progress",
                "Reported progress",
                "Could not report progress",
            ),
            ("Звітую про хід", "Звітував про хід", "Не вдалося звітувати"),
        ),
        "redirect_to_coordinator" => (
            (
                "Redirecting request",
                "Redirected request",
                "Could not redirect request",
            ),
            (
                "Перенаправляю запит",
                "Перенаправив запит",
                "Не вдалося перенаправити запит",
            ),
        ),
        _ => return None,
    };
    let verbs = if uk { ua } else { en };
    let title = safe_arg_str(arguments, &["title"]).map(|title| truncate(title, 60));
    Some(labeled_phrase(verbs.0, verbs.1, verbs.2, title, phase))
}

const COORDINATOR_SYSTEM_PROMPT: &str = "You coordinate work through threads. Answer quick questions yourself. For real work (research, a change, a report, anything multi-step) call start_thread with a short title and a complete brief; the thread works in the background and you are told when it finishes, asks something, or stops. One thread per piece of work: route a follow-up about existing work to its thread with message_thread instead of starting a new one, and check list_threads when unsure. Threads only talk to each other through you. Notices marked as automatic task updates come from the platform, not the person: relay what matters to the person in a sentence or two and link nothing you did not read. Never answer a thread's question on the person's behalf unless the person already told you the answer. Resolve a thread with resolve_thread when the person is done with it.";

// =============================================================================
// Shared helpers
// =============================================================================

fn require_registry(
    context: &ToolContext,
) -> Result<&Arc<dyn SessionTaskRegistry>, ToolExecutionResult> {
    context
        .session_task_registry
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Coordination requires session tasks"))
}

fn require_session_store(context: &ToolContext) -> Result<&dyn SessionStore, ToolExecutionResult> {
    context
        .session_store
        .as_deref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Coordination requires a session store"))
}

fn bounded_text<'a>(
    arguments: &'a Value,
    key: &str,
    max_bytes: usize,
) -> Result<&'a str, ToolExecutionResult> {
    let value = require_str_nonblank(arguments, key)?.trim();
    if value.len() > max_bytes {
        return Err(ToolExecutionResult::tool_error(format!(
            "{key} is too long: {} bytes, the limit is {max_bytes}",
            value.len()
        )));
    }
    Ok(value)
}

fn optional_bounded_text<'a>(
    arguments: &'a Value,
    key: &str,
    max_bytes: usize,
) -> Result<Option<&'a str>, ToolExecutionResult> {
    match arguments.get(key).and_then(Value::as_str).map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) if value.len() > max_bytes => Err(ToolExecutionResult::tool_error(format!(
            "{key} is too long: {} bytes, the limit is {max_bytes}",
            value.len()
        ))),
        Some(value) => Ok(Some(value)),
    }
}

fn parse_thread_id(arguments: &Value) -> Result<SessionId, ToolExecutionResult> {
    let raw = require_str_nonblank(arguments, "thread_id")?.trim();
    raw.parse()
        .map_err(|_| ToolExecutionResult::tool_error(format!("Invalid thread_id: {raw}")))
}

/// Assignment tasks owned by `coordinator`, grouped by thread, each group
/// oldest first.
async fn assignments_by_thread(
    registry: &dyn SessionTaskRegistry,
    coordinator: SessionId,
) -> Result<HashMap<SessionId, Vec<SessionTask>>, ToolExecutionResult> {
    let tasks = registry
        .list(
            coordinator,
            Some(&SessionTaskFilter {
                kind: Some(TASK_KIND_ASSIGNMENT.to_string()),
                state: None,
            }),
        )
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    let mut threads: HashMap<SessionId, Vec<SessionTask>> = HashMap::new();
    for task in tasks {
        if let Some(thread) = task.links.child_session_id {
            threads.entry(thread).or_default().push(task);
        }
    }
    for assignments in threads.values_mut() {
        assignments.sort_by_key(|task| task.created_at);
    }
    Ok(threads)
}

async fn thread_assignments(
    registry: &dyn SessionTaskRegistry,
    coordinator: SessionId,
    thread: SessionId,
) -> Result<Vec<SessionTask>, ToolExecutionResult> {
    let mut threads = assignments_by_thread(registry, coordinator).await?;
    threads.remove(&thread).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!(
            "{thread} is not a thread of this session. Use list_threads to see your threads."
        ))
    })
}

/// Where a thread stands, in the words the person sees.
fn thread_status(latest: &SessionTask) -> &'static str {
    if latest.state_detail.as_deref() == Some(THREAD_RESOLVED_DETAIL) {
        return "resolved";
    }
    match latest.state {
        SessionTaskState::AwaitingInput | SessionTaskState::Failed => "needs_attention",
        SessionTaskState::Queued | SessionTaskState::Running => "working",
        SessionTaskState::Succeeded => "ready_for_review",
        SessionTaskState::Canceled => "open",
    }
}

fn checklist_summary(task: &SessionTask) -> Option<String> {
    let steps = &task.progress.as_ref()?.steps;
    if steps.is_empty() {
        return None;
    }
    let done = steps
        .iter()
        .filter(|step| step.status == ProgressStepStatus::Done)
        .count();
    Some(format!("{done}/{} steps done", steps.len()))
}

fn assignment_view(task: &SessionTask) -> Value {
    json!({
        "assignment_id": task.id,
        "title": task.display_name,
        "state": task.state.to_string(),
        "checklist": task.progress.as_ref().map(|progress| &progress.steps),
        "question": task.input_request.as_ref().map(|request| &request.prompt),
        "summary": task.summary,
        "error": task.error.as_ref().map(|error| &error.message),
        "artifacts": task.artifacts,
    })
}

/// First message a thread receives for a new assignment.
fn frame_brief(title: &str, brief: &str) -> String {
    format!(
        "New assignment from the coordinator: {title}\n\n{brief}\n\nKeep your checklist current with update_checklist. When the work is done, call complete_assignment with a short summary and how you checked it. If you need a decision from the person, call ask_decision. If this request belongs to someone else, call redirect_to_coordinator."
    )
}

/// A follow-up relayed into a thread with an open assignment.
fn frame_relay(message: &str) -> String {
    format!("Message from the coordinator:\n\n{message}")
}

/// Send to a thread off the tool's call path: embedded hosts run the thread's
/// turn inside `send_message`, which would otherwise block the coordinator.
fn send_to_thread(context: &ToolContext, thread: SessionId, text: String, task_id: String) {
    let context = context.clone();
    tokio::spawn(async move {
        let Some(delegate) = context.subagent_delegate.clone() else {
            return;
        };
        if let Err(error) = delegate.send_message(thread, &text).await {
            tracing::warn!(%thread, %error, "coordination: message to thread failed");
            if let Some(registry) = context.session_task_registry.as_ref() {
                let _ = registry
                    .update(
                        context.session_id,
                        &task_id,
                        SessionTaskUpdate {
                            state: Some(SessionTaskState::Failed),
                            error: Some(TaskError {
                                kind: "delivery_failed".to_string(),
                                message: error.to_string(),
                            }),
                            ..Default::default()
                        },
                    )
                    .await;
            }
        }
    });
}

/// Best-effort note to a thread. Spawned like `send_to_thread`: an embedded
/// host's delegate runs the thread's turn inside `send_message`, which must not
/// hold the coordinator's own turn.
fn notify_thread(context: &ToolContext, thread: SessionId, text: &'static str) {
    let Some(delegate) = context.subagent_delegate.clone() else {
        return;
    };
    tokio::spawn(async move {
        let _ = delegate.send_message(thread, text).await;
    });
}

async fn create_assignment(
    registry: &dyn SessionTaskRegistry,
    coordinator: SessionId,
    thread: SessionId,
    title: &str,
    brief: &str,
    worker: &str,
) -> Result<SessionTask, ToolExecutionResult> {
    registry
        .create(CreateSessionTask {
            session_id: coordinator,
            id: None,
            kind: TASK_KIND_ASSIGNMENT.to_string(),
            display_name: title.to_string(),
            spec: json!({ "title": title, "brief": brief, "worker": worker }),
            state: SessionTaskState::Running,
            links: TaskLinks {
                child_session_id: Some(thread),
                ..Default::default()
            },
            // Wake the coordinator on every result, question, and report.
            wake_policy: TaskWakePolicy::OnActivity,
        })
        .await
        .map_err(ToolExecutionResult::internal_error)
}

// =============================================================================
// Coordinator tools
// =============================================================================

macro_rules! context_tool {
    ($name:literal, $display:literal) => {
        fn name(&self) -> &str {
            $name
        }

        fn display_name(&self) -> Option<&str> {
            Some($display)
        }

        fn requires_context(&self) -> bool {
            true
        }

        fn required_context_services(&self) -> &'static [ToolContextService] {
            &[ToolContextService::SessionTaskRegistry]
        }
    };
}

struct StartThreadTool(Arc<CoordinationConfig>);

#[async_trait]
impl Tool for StartThreadTool {
    context_tool!("start_thread", "Start Thread");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("start_thread requires session context.")
    }

    fn description(&self) -> &str {
        "Start a thread: a background session that works one assignment and reports back. Give a short title in the person's words and a complete brief (the thread sees nothing else). Returns the thread_id."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "A few plain words naming the work, in the person's vocabulary." },
                "brief": { "type": "string", "description": "Everything the thread needs: the ask in the person's words, relevant decisions, and what done looks like." },
                "worker": { "type": "string", "description": "\"self\" (default) runs the thread on this agent; an agent id runs it on that agent, when allowed." }
            },
            "required": ["title", "brief"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        // Serialized with spawns so concurrent calls cannot overrun the caps.
        ToolHints::default().with_concurrency_class(super::SPAWN_AGENT_CONCURRENCY_CLASS)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        start_thread(&self.0, arguments, context)
            .await
            .unwrap_or_else(|error| error)
    }
}

async fn start_thread(
    config: &CoordinationConfig,
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let title = bounded_text(&arguments, "title", MAX_TITLE_BYTES)?;
    let brief = bounded_text(&arguments, "brief", MAX_BRIEF_BYTES)?;
    let worker = arguments
        .get("worker")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|worker| !worker.is_empty())
        .unwrap_or("self");
    let registry = require_registry(context)?;
    let delegate = get_subagent_delegate(context)?;
    let session_store = require_session_store(context)?;

    let coordinator = session_store
        .get_session(context.session_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
        .ok_or_else(|| ToolExecutionResult::tool_error("Current session not found"))?;
    if coordinator.parent_session_id.is_some() {
        return Err(ToolExecutionResult::tool_error(
            "This session is a thread, and threads do not start threads. Report to your coordinator with report_to_coordinator instead.",
        ));
    }

    let threads = assignments_by_thread(registry.as_ref(), context.session_id).await?;
    let active = threads
        .values()
        .filter(|assignments| assignments.iter().any(|task| !task.state.is_terminal()))
        .count() as u64;
    if active >= config.max_active() {
        return Err(ToolExecutionResult::tool_error(format!(
            "{active} threads are already working, the limit is {}. Wait for one to finish, or route this to an existing thread with message_thread.",
            config.max_active()
        )));
    }
    if threads.len() as u64 >= config.max_total() {
        return Err(ToolExecutionResult::tool_error(format!(
            "This session already started {} threads, the limit is {}. Route new work to an existing thread with message_thread.",
            threads.len(),
            config.max_total()
        )));
    }

    let (harness_id, agent_id) = if worker == "self" {
        if !config.allows_self() {
            return Err(ToolExecutionResult::tool_error(
                "This agent is not configured to work threads itself. Pass an allowed agent id as worker.",
            ));
        }
        (coordinator.harness_id, coordinator.agent_id)
    } else {
        let agent_id: AgentId = worker.parse().map_err(|_| {
            ToolExecutionResult::tool_error(format!(
                "worker must be \"self\" or an agent id, got \"{worker}\""
            ))
        })?;
        if !config.allows_agent(worker) {
            return Err(ToolExecutionResult::tool_error(format!(
                "Agent {worker} is not an allowed worker for this coordinator."
            )));
        }
        // THREAT[TM-AUTHZ]: the delegate runs as the person, so an agent they
        // may not see resolves to None and is refused here.
        if delegate
            .get_agent_by_id(agent_id)
            .await
            .map_err(ToolExecutionResult::internal_error)?
            .is_none()
        {
            return Err(ToolExecutionResult::tool_error(format!(
                "Agent {worker} was not found."
            )));
        }
        let harness_id = delegate
            .get_agent_harness_id(agent_id)
            .await
            .map_err(ToolExecutionResult::internal_error)?
            .unwrap_or(coordinator.harness_id);
        (harness_id, Some(agent_id))
    };

    let thread = delegate
        .create_session_with_options(PlatformCreateSessionRequest {
            harness_id,
            agent_id,
            title: Some(title.to_string()),
            goal: Some(title.to_string()),
            locale: coordinator.locale.clone(),
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: Some(context.session_id),
            forked_from_session_id: None,
            budget_root_session_id: None,
            seed: SessionSeedMode::Fresh,
        })
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    let task = create_assignment(
        registry.as_ref(),
        context.session_id,
        thread.id,
        title,
        brief,
        worker,
    )
    .await?;
    send_to_thread(
        context,
        thread.id,
        frame_brief(title, brief),
        task.id.clone(),
    );

    Ok(ToolExecutionResult::success(json!({
        "thread_id": thread.id.to_string(),
        "assignment_id": task.id,
        "title": title,
        "worker": worker,
        "status": "working",
    })))
}

struct MessageThreadTool;

#[async_trait]
impl Tool for MessageThreadTool {
    context_tool!("message_thread", "Message Thread");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("message_thread requires session context.")
    }

    fn description(&self) -> &str {
        "Send a follow-up to an existing thread. While the thread works, the message joins its current assignment (and answers its open question, if any). When the thread is idle, the message becomes a new assignment; pass title to name it."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "message": { "type": "string", "description": "What the thread should know or do, complete on its own." },
                "title": { "type": "string", "description": "Title for a new assignment when the thread is idle." }
            },
            "required": ["thread_id", "message"],
            "additionalProperties": false
        })
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        message_thread(arguments, context)
            .await
            .unwrap_or_else(|error| error)
    }
}

async fn message_thread(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let thread = parse_thread_id(&arguments)?;
    let message = bounded_text(&arguments, "message", MAX_BRIEF_BYTES)?;
    let title = optional_bounded_text(&arguments, "title", MAX_TITLE_BYTES)?;
    let registry = require_registry(context)?;
    get_subagent_delegate(context)?;
    let assignments = thread_assignments(registry.as_ref(), context.session_id, thread).await?;
    let latest = assignments.last().expect("thread has an assignment");

    if !latest.state.is_terminal() {
        let relays = registry
            .list_messages(context.session_id, &latest.id, Some(200), None)
            .await
            .map_err(ToolExecutionResult::internal_error)?
            .into_iter()
            .filter(|message| message.direction == TaskMessageDirection::Inbound)
            .count();
        if relays >= MAX_RELAYS_PER_ASSIGNMENT {
            return Err(ToolExecutionResult::tool_error(format!(
                "You already sent this assignment {relays} messages. Stop relaying and ask the person how to proceed."
            )));
        }
        // An answer clears the thread's open question and puts it back to work.
        let in_reply_to = latest
            .input_request
            .as_ref()
            .map(|request| request.id.clone());
        registry
            .record_message(
                context.session_id,
                &latest.id,
                NewTaskMessage {
                    direction: TaskMessageDirection::Inbound,
                    content: vec![TaskMessagePart::text(message)],
                    in_reply_to,
                    expected_attempt: None,
                },
            )
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        send_to_thread(context, thread, frame_relay(message), latest.id.clone());
        return Ok(ToolExecutionResult::success(json!({
            "thread_id": thread.to_string(),
            "assignment_id": latest.id,
            "delivered": "current_assignment",
        })));
    }

    let title = title
        .map(str::to_string)
        .unwrap_or_else(|| follow_up_title(&latest.display_name));
    if latest.state_detail.as_deref() == Some(THREAD_RESOLVED_DETAIL) {
        reopen(context, thread, latest).await;
    }
    let worker = latest
        .spec
        .get("worker")
        .and_then(Value::as_str)
        .unwrap_or("self")
        .to_string();
    let task = create_assignment(
        registry.as_ref(),
        context.session_id,
        thread,
        &title,
        message,
        &worker,
    )
    .await?;
    send_to_thread(
        context,
        thread,
        frame_brief(&title, message),
        task.id.clone(),
    );
    Ok(ToolExecutionResult::success(json!({
        "thread_id": thread.to_string(),
        "assignment_id": task.id,
        "title": title,
        "delivered": "new_assignment",
    })))
}

fn follow_up_title(previous: &str) -> String {
    let mut title = format!("Follow-up: {previous}");
    if title.len() > MAX_TITLE_BYTES {
        let mut end = MAX_TITLE_BYTES;
        while !title.is_char_boundary(end) {
            end -= 1;
        }
        title.truncate(end);
    }
    title
}

async fn reopen(context: &ToolContext, thread: SessionId, latest: &SessionTask) {
    if let Some(registry) = context.session_task_registry.as_ref() {
        let _ = registry
            .update(
                context.session_id,
                &latest.id,
                SessionTaskUpdate {
                    state_detail: Some(THREAD_REOPENED_DETAIL.to_string()),
                    ..Default::default()
                },
            )
            .await;
    }
    if let Some(delegate) = context.subagent_delegate.as_ref()
        && let Err(error) = delegate.set_session_archived(thread, false).await
    {
        tracing::debug!(%thread, %error, "coordination: unarchive failed");
    }
}

struct ListThreadsTool;

#[async_trait]
impl Tool for ListThreadsTool {
    context_tool!("list_threads", "List Threads");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_threads requires session context.")
    }

    fn description(&self) -> &str {
        "List this session's threads, newest activity first, with each thread's status (working, needs_attention, ready_for_review, open, resolved) and current assignment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "include_resolved": { "type": "boolean", "description": "Include resolved threads. Default false." }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let include_resolved = arguments
            .get("include_resolved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let registry = match require_registry(context) {
            Ok(registry) => registry,
            Err(error) => return error,
        };
        let threads = match assignments_by_thread(registry.as_ref(), context.session_id).await {
            Ok(threads) => threads,
            Err(error) => return error,
        };
        let mut rows: Vec<(chrono::DateTime<chrono::Utc>, Value)> = threads
            .iter()
            .filter_map(|(thread, assignments)| {
                let latest = assignments.last()?;
                let status = thread_status(latest);
                if status == "resolved" && !include_resolved {
                    return None;
                }
                Some((
                    latest.updated_at,
                    json!({
                        "thread_id": thread.to_string(),
                        "title": assignments.first().map(|task| task.display_name.as_str()),
                        "status": status,
                        "assignments": assignments.len(),
                        "checklist": checklist_summary(latest),
                        "current": assignment_view(latest),
                    }),
                ))
            })
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.0));
        ToolExecutionResult::success(json!({
            "threads": rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>(),
        }))
    }
}

struct GetThreadTool;

#[async_trait]
impl Tool for GetThreadTool {
    context_tool!("get_thread", "Get Thread");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("get_thread requires session context.")
    }

    fn description(&self) -> &str {
        "Read one thread: its assignments with checklists and results, and its most recent messages."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "messages": { "type": "integer", "minimum": 0, "maximum": GET_THREAD_MESSAGE_LIMIT, "description": "Recent messages to include. Default 6." }
            },
            "required": ["thread_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        get_thread(arguments, context)
            .await
            .unwrap_or_else(|error| error)
    }
}

async fn get_thread(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let thread = parse_thread_id(&arguments)?;
    let limit = arguments
        .get("messages")
        .and_then(Value::as_u64)
        .map(|limit| limit.min(GET_THREAD_MESSAGE_LIMIT as u64) as usize)
        .unwrap_or(6);
    let registry = require_registry(context)?;
    let assignments = thread_assignments(registry.as_ref(), context.session_id, thread).await?;
    let latest = assignments.last().expect("thread has an assignment");
    let messages = if limit == 0 {
        Vec::new()
    } else {
        get_subagent_delegate(context)?
            .get_messages(thread, Some(limit))
            .await
            .map_err(ToolExecutionResult::internal_error)?
            .into_iter()
            .map(|message| {
                json!({
                    "role": message.role,
                    "content": truncate_summary(&message.content),
                    "created_at": message.created_at,
                })
            })
            .collect()
    };
    Ok(ToolExecutionResult::success(json!({
        "thread_id": thread.to_string(),
        "status": thread_status(latest),
        "assignments": assignments.iter().map(assignment_view).collect::<Vec<_>>(),
        "recent_messages": messages,
    })))
}

struct ResolveThreadTool;

#[async_trait]
impl Tool for ResolveThreadTool {
    context_tool!("resolve_thread", "Resolve Thread");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("resolve_thread requires session context.")
    }

    fn description(&self) -> &str {
        "Mark a thread resolved once the person is done with it, or reopen it with reopen=true. A thread that is still working is refused unless cancel=true, which stops its assignment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "reopen": { "type": "boolean" },
                "cancel": { "type": "boolean", "description": "Stop a working assignment and resolve the thread." }
            },
            "required": ["thread_id"],
            "additionalProperties": false
        })
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        resolve_thread(arguments, context)
            .await
            .unwrap_or_else(|error| error)
    }
}

async fn resolve_thread(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let thread = parse_thread_id(&arguments)?;
    let reopen_requested = arguments
        .get("reopen")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cancel = arguments
        .get("cancel")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let registry = require_registry(context)?;
    let assignments = thread_assignments(registry.as_ref(), context.session_id, thread).await?;
    let latest = assignments.last().expect("thread has an assignment");

    if reopen_requested {
        reopen(context, thread, latest).await;
        return Ok(ToolExecutionResult::success(json!({
            "thread_id": thread.to_string(),
            "status": "open",
        })));
    }
    if !latest.state.is_terminal() && !cancel {
        return Err(ToolExecutionResult::tool_error(
            "This thread is still working. Wait for it, or pass cancel=true to stop it and resolve.",
        ));
    }
    if !latest.state.is_terminal() {
        notify_thread(
            context,
            thread,
            "The coordinator resolved this thread. Stop work; do not start anything new.",
        );
    }
    registry
        .update(
            context.session_id,
            &latest.id,
            SessionTaskUpdate {
                state: (!latest.state.is_terminal()).then_some(SessionTaskState::Canceled),
                state_detail: Some(THREAD_RESOLVED_DETAIL.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    let archived = match context.subagent_delegate.as_ref() {
        Some(delegate) => delegate.set_session_archived(thread, true).await.is_ok(),
        None => false,
    };
    Ok(ToolExecutionResult::success(json!({
        "thread_id": thread.to_string(),
        "status": "resolved",
        "archived": archived,
    })))
}

#[path = "coordination_worker.rs"]
mod worker;
#[cfg(test)]
use worker::{AskDecisionTool, RedirectToCoordinatorTool};
pub use worker::{
    ThreadAssignment, ThreadTurn, open_assignment_for_thread, settle_thread_turn,
    worker_tool_definitions, worker_tools,
};

// =============================================================================
// Task executor
// =============================================================================

/// Control plane for `assignment` tasks: `message_task` relays into the thread,
/// `cancel_task` stops the assignment and tells the thread.
pub struct AssignmentTaskExecutor;

#[async_trait]
impl TaskExecutor for AssignmentTaskExecutor {
    fn kind(&self) -> &str {
        TASK_KIND_ASSIGNMENT
    }

    async fn deliver(
        &self,
        task: &SessionTask,
        message: &TaskMessage,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        let (Some(delegate), Some(thread)) = (
            context.subagent_delegate.as_ref(),
            task.links.child_session_id,
        ) else {
            return Err(everruns_contracts::error::AgentLoopError::tool(
                "assignment delivery requires a thread link and the delegation service",
            ));
        };
        delegate
            .send_message(thread, &frame_relay(&task_message_text(&message.content)))
            .await
    }

    async fn cancel(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        if let Some(thread) = task.links.child_session_id {
            notify_thread(
                context,
                thread,
                "The coordinator canceled this assignment. Stop work and reply with a one-line note on where you got to.",
            );
        }
        // No watcher settles assignments, so cancel settles it here.
        if let Some(registry) = context.session_task_registry.as_ref() {
            registry
                .update(
                    task.session_id,
                    &task.id,
                    SessionTaskUpdate {
                        state: Some(SessionTaskState::Canceled),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Ok(())
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(AssignmentTaskExecutor),
    }
}

#[cfg(test)]
#[path = "coordination_tests.rs"]
mod tests;
