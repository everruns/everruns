// Threads capability (knowledge/runtime-resources/threads.md)
//
// A thread is the role a session plays when another agent opened it for
// followed, continued work. The parent starts a thread per piece of work,
// relays follow-ups into it, and reads its checklist and result; the person
// can open the thread and talk to it directly.
//
// Decision: a thread is a child session (parent_session_id = parent). It
//   lives across many units of work; each unit is one `assignment` task owned
//   by the parent and linked to the thread through `links.child_session_id`.
//   A thread runs at most one open assignment at a time.
// Decision: a thread finishes an assignment explicitly with `finish`. A turn
//   that ends without it is not a result: the server's thread turn listener
//   (crates/server/src/listeners/coordination/mod.rs), or on data plane v2
//   the shard's (crates/agentd/src/thread_turns.rs), flags the assignment as
//   needing attention, which wakes the parent.
// Decision: assignment tasks carry no heartbeat and no watcher. The reaper
//   ignores tasks without a heartbeat, so a thread can wait on the person for
//   days without being failed as orphaned, and threads use no background slot.
// Decision: threads talk to each other only through their parent unless the
//   agent turns on `sibling_communication` (user decisions, 2026-10-08 and
//   2026-11-25), so routing has one owner and one record by default.
// Decision: parent tools (`report_progress`, `ask_parent`, `finish`,
//   `hand_back`) are injected per turn by `PlatformToolAugmentor` into a
//   session that has an open assignment (`parent_communication`). Threads and
//   Coordinator are hidden from every child session (`PlatformToolAugmentor::
//   hidden_reason_capabilities`): a thread does not start threads.
// Decision: who may work a thread is this capability's own list: `self`,
//   `agents` (named, by agent id; names are filled on save), and `any_agent`.
//   Threads supersedes Spawn agents, so `spawn_agent` never competes with
//   `start_thread`: a model given both called spawn_agent with a bad target 36
//   times instead of starting a thread. Validation refuses both on one agent.
// Decision: coordinator behaviour (answer quick questions, route real work to
//   threads) is the separate `coordinator` capability, a prompt that needs
//   this one. Threads alone gives the tools without the persona.
// Decision: relays reach a thread as plain user-role text with a framing line,
//   kept in `frame_*` helpers so a tool-based channel stays a local swap.

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

pub use everruns_contracts::capability::delegation::THREADS_CAPABILITY_ID;

/// `state_detail` on a thread's latest assignment once the coordinator
/// resolved the thread. Reopening clears it.
pub const THREAD_RESOLVED_DETAIL: &str = "resolved";
const THREAD_REOPENED_DETAIL: &str = "reopened";

/// Thread tools, hidden from a session that is itself a thread.
pub const THREAD_TOOL_NAMES: [&str; 5] = [
    "start_thread",
    "message_thread",
    "list_threads",
    "get_thread",
    "close_thread",
];
/// Parent tools (`parent_communication`), injected into a thread that has an
/// open assignment.
pub const PARENT_TOOL_NAMES: [&str; 4] = ["report_progress", "ask_parent", "finish", "hand_back"];

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

/// An agent that may work threads. Names are filled from the agent on save.
#[derive(Debug, Clone, Deserialize)]
struct ThreadAgent {
    agent_id: AgentId,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
}

impl ThreadAgent {
    /// The name the model addresses: the agent's name, else its id.
    fn label(&self) -> String {
        [&self.id, &self.name]
            .into_iter()
            .find(|value| !value.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| self.agent_id.to_string())
    }

    fn matches(&self, wanted: &str) -> bool {
        self.id == wanted
            || self.name.eq_ignore_ascii_case(wanted)
            || self.agent_id.to_string() == wanted
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadsConfig {
    /// This agent works threads itself.
    #[serde(rename = "self", default = "default_true")]
    runs_self: bool,
    /// Named agents that may work threads.
    #[serde(default)]
    agents: Vec<ThreadAgent>,
    /// Any agent the person may run, by agent id.
    #[serde(default)]
    any_agent: bool,
    #[serde(default)]
    max_open: Option<u64>,
    #[serde(default)]
    max_total: Option<u64>,
}

fn default_true() -> bool {
    true
}

impl Default for ThreadsConfig {
    fn default() -> Self {
        Self {
            runs_self: true,
            agents: Vec::new(),
            any_agent: false,
            max_open: None,
            max_total: None,
        }
    }
}

impl ThreadsConfig {
    fn parse(config: &Value) -> Result<Self, String> {
        if config.is_null() {
            return Ok(Self::default());
        }
        serde_json::from_value(config.clone()).map_err(|error| error.to_string())
    }

    fn agent(&self, wanted: &str) -> Option<&ThreadAgent> {
        self.agents.iter().find(|agent| agent.matches(wanted))
    }

    fn max_active(&self) -> u64 {
        self.max_open.unwrap_or(DEFAULT_MAX_ACTIVE_THREADS)
    }

    fn max_total(&self) -> u64 {
        self.max_total.unwrap_or(DEFAULT_MAX_TOTAL_THREADS)
    }
}

// =============================================================================
// Capability
// =============================================================================

/// Lets an agent open threads: followed, continued work in child sessions.
pub struct ThreadsCapability;

impl Capability for ThreadsCapability {
    fn id(&self) -> &str {
        THREADS_CAPABILITY_ID
    }

    /// Retired id, rewritten on read (`capability::delegation`).
    fn aliases(&self) -> Vec<&'static str> {
        vec!["coordination"]
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
        "Threads"
    }

    fn description(&self) -> &str {
        "Open a thread per piece of work: it works in the background, keeps a checklist, takes follow-ups, and reports back. The person can open a thread and talk to it."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Гілки",
            "Відкривайте окрему гілку для кожного завдання: вона працює у фоні, веде чекліст, приймає уточнення й звітує. Людина може відкрити гілку й писати в неї напряму.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("network")
    }

    fn category(&self) -> Option<&str> {
        Some("Orchestration")
    }

    fn risk_level(&self) -> RiskLevel {
        // Threads are sessions that spend: same footing as spawned agents.
        RiskLevel::High
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["threads"]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "self": {
                    "type": "boolean",
                    "default": true,
                    "title": "This agent",
                    "description": "This agent works threads itself."
                },
                "agents": {
                    "type": "array",
                    "title": "Agents",
                    "description": "Other agents that may work threads, each as itself. The model passes the agent's name.",
                    "items": {
                        "type": "object",
                        "properties": { "agent_id": { "type": "string" } },
                        "required": ["agent_id"]
                    }
                },
                "any_agent": {
                    "type": "boolean",
                    "default": false,
                    "title": "Any agent",
                    "description": "Any agent the person may run can work a thread, by agent id."
                },
                "max_open": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 64,
                    "default": DEFAULT_MAX_ACTIVE_THREADS,
                    "description": "Threads that may be working at once."
                },
                "max_total": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1000,
                    "default": DEFAULT_MAX_TOTAL_THREADS,
                    "description": "Threads one session may open in total."
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        let parsed = ThreadsConfig::parse(config)?;
        if !parsed.runs_self && parsed.agents.is_empty() && !parsed.any_agent {
            return Err(
                "Threads needs at least one agent to work them: set self, list agents, or allow any_agent"
                    .to_string(),
            );
        }
        if parsed.max_active().clamp(1, 64) != parsed.max_active() {
            return Err("max_open must be between 1 and 64".to_string());
        }
        if parsed.max_total().clamp(1, 1000) != parsed.max_total() {
            return Err("max_total must be between 1 and 1000".to_string());
        }
        Ok(())
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(THREADS_SYSTEM_PROMPT)
    }

    /// One way to start work per agent: Threads wins over a harness default
    /// of Spawn agents (validation refuses both set on the agent itself).
    fn supersedes(&self) -> Vec<&'static str> {
        vec![everruns_contracts::capability::delegation::SPAWN_AGENTS_CAPABILITY_ID]
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        let config = Arc::new(ThreadsConfig::parse(config).unwrap_or_default());
        vec![
            Box::new(StartThreadTool(config.clone())),
            Box::new(MessageThreadTool),
            Box::new(ListThreadsTool(config)),
            Box::new(GetThreadTool),
            Box::new(CloseThreadTool),
        ]
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }
}

/// Coordinator behaviour: a prompt that makes the agent answer quick questions
/// and route real work to threads. Needs Threads (pulled in as a dependency).
pub struct CoordinatorCapability;

impl Capability for CoordinatorCapability {
    fn id(&self) -> &str {
        everruns_contracts::capability::delegation::COORDINATOR_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Coordinator"
    }

    fn description(&self) -> &str {
        "Behave as a coordinator: answer quick questions, start a thread for real work, relay follow-ups and results. Needs Threads."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Координатор",
            "Поводиться як координатор: відповідає на швидкі запитання, відкриває гілку для справжньої роботи, передає уточнення й результати. Потребує гілок.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("network")
    }

    fn category(&self) -> Option<&str> {
        Some("Orchestration")
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec![THREADS_CAPABILITY_ID]
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(COORDINATOR_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        Vec::new()
    }
}

/// Backend-authored narration for the thread and parent tools. Titles
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
        "close_thread" => (
            ("Closing thread", "Closed thread", "Could not close thread"),
            ("Закриваю гілку", "Закрив гілку", "Не вдалося закрити гілку"),
        ),
        "report_progress" => (
            (
                "Reporting progress",
                "Reported progress",
                "Could not report progress",
            ),
            ("Звітую про хід", "Звітував про хід", "Не вдалося звітувати"),
        ),
        "finish" => (
            ("Finishing", "Finished", "Could not finish"),
            (
                "Завершую завдання",
                "Завершив завдання",
                "Не вдалося завершити завдання",
            ),
        ),
        "ask_parent" => (
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
        "hand_back" => (
            (
                "Handing a request back",
                "Handed a request back",
                "Could not hand the request back",
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

const THREADS_SYSTEM_PROMPT: &str = "Use threads for work that is followed and continued: start_thread with a short title and a complete brief (the thread sees nothing else; to run it on one of your agents, pass the agent's name as agent, and list_threads lists them). A thread works in the background, keeps a checklist, and you are told when it finishes, asks something, or stops. A follow-up to existing work goes to that thread with message_thread, even after it finished; start a new thread only for new work. Read a thread with get_thread, and close_thread when the person is done with it. Every thread in this run shares the shared/ folder: pass file paths instead of pasting long content.";

/// The coordinator persona (the `coordinator` capability): answer quick
/// questions, route real work to threads, relay results.
pub const COORDINATOR_SYSTEM_PROMPT: &str = "You coordinate work through threads. Answer quick questions yourself. For real work (research, a change, a report, anything multi-step) start a thread; once work is in a thread, do not do it yourself as well: tell the person in a sentence where it went, and give the result when the automatic update arrives. One thread per piece of work: a follow-up that extends, corrects or asks about existing work (\"also add the dates\", \"make it shorter\") goes to that thread with message_thread; check list_threads when unsure. Notices marked as automatic task updates come from the platform, not the person: relay what matters in a sentence or two and link nothing you did not read. Never answer a thread's question on the person's behalf unless the person already told you the answer; when the answer is already in what the person asked for, say so to the thread. Close a thread with close_thread when the person is done with it.";

// =============================================================================
// Shared helpers
// =============================================================================

fn require_registry(
    context: &ToolContext,
) -> Result<&Arc<dyn SessionTaskRegistry>, ToolExecutionResult> {
    context
        .session_task_registry
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Threads require session tasks"))
}

fn require_session_store(context: &ToolContext) -> Result<&dyn SessionStore, ToolExecutionResult> {
    context
        .session_store
        .as_deref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Threads require a session store"))
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

// Instructions appended to every brief. The UI shows the brief without them by
// matching this text and the two headers below (chat-thread-messages.ts in the
// UI), so change them together.
const BRIEF_INSTRUCTIONS: &str = "Keep your checklist current with report_progress. When the work is done, call finish with a short summary and how you checked it. The brief is the person's request, so do not ask for approval to do what it asks; call ask_parent only for a real choice the brief leaves open. If this request belongs to someone else, call hand_back.";

/// First message a thread receives for a new assignment.
fn frame_brief(title: &str, brief: &str) -> String {
    format!("New assignment from your parent: {title}\n\n{brief}\n\n{BRIEF_INSTRUCTIONS}")
}

/// A follow-up relayed into a thread with an open assignment.
fn frame_relay(message: &str) -> String {
    format!("Message from your parent:\n\n{message}")
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

struct StartThreadTool(Arc<ThreadsConfig>);

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
                "agent": { "type": "string", "description": "\"self\" (default) runs the thread on this agent; an agent's name from list_threads runs it on that agent." }
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
    config: &ThreadsConfig,
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let title = bounded_text(&arguments, "title", MAX_TITLE_BYTES)?;
    let brief = bounded_text(&arguments, "brief", MAX_BRIEF_BYTES)?;
    let worker = arguments
        .get("agent")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
        .unwrap_or("self");
    let registry = require_registry(context)?;
    let delegate = get_subagent_delegate(context)?;
    let session_store = require_session_store(context)?;

    let parent = session_store
        .get_session(context.session_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
        .ok_or_else(|| ToolExecutionResult::tool_error("Current session not found"))?;
    if parent.parent_session_id.is_some() {
        return Err(ToolExecutionResult::tool_error(
            "This session is a thread, and threads do not start threads. Tell your parent with report_progress instead.",
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
        if !config.runs_self {
            return Err(ToolExecutionResult::tool_error(
                "This agent does not work threads itself. Pass one of its agents by name (list_threads lists them).",
            ));
        }
        // The thread runs on the agent's current harness, not the parent
        // session's: a long-lived Chat can predate a harness move (migration
        // 165 left old Chats on `generic`), and session creation validates
        // against the agent's binding.
        let harness_id = match parent.agent_id {
            Some(agent_id) => delegate
                .get_agent_harness_id(agent_id)
                .await
                .map_err(ToolExecutionResult::internal_error)?
                .unwrap_or(parent.harness_id),
            None => parent.harness_id,
        };
        (harness_id, parent.agent_id)
    } else {
        let agent_id: AgentId = match config.agent(worker) {
            Some(agent) => agent.agent_id,
            None if config.any_agent => worker.parse().map_err(|_| {
                ToolExecutionResult::tool_error(format!(
                    "agent must be \"self\", one of your agents' names, or an agent id, got \"{worker}\". list_threads lists your agents."
                ))
            })?,
            None => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "\"{worker}\" is not one of the agents that may work threads here. list_threads lists them."
                )));
            }
        };
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
            .unwrap_or(parent.harness_id);
        (harness_id, Some(agent_id))
    };

    let thread = delegate
        .create_session_with_options(PlatformCreateSessionRequest {
            harness_id,
            agent_id,
            title: Some(title.to_string()),
            goal: Some(title.to_string()),
            locale: parent.locale.clone(),
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: Some(context.session_id),
            forked_from_session_id: None,
            budget_root_session_id: None,
            seed: SessionSeedMode::Fresh,
            reason: crate::platform_store::tool_change_reason(
                "start_thread",
                &format!("start thread \"{title}\""),
            ),
            helper_model: None,
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
        "agent": worker,
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
        && let Err(error) = delegate
            .set_session_archived(
                thread,
                false,
                &crate::platform_store::tool_change_reason("threads", "reopen thread"),
            )
            .await
    {
        tracing::debug!(%thread, %error, "coordination: unarchive failed");
    }
}

struct ListThreadsTool(Arc<ThreadsConfig>);

#[async_trait]
impl Tool for ListThreadsTool {
    context_tool!("list_threads", "List Threads");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_threads requires session context.")
    }

    fn description(&self) -> &str {
        "List this session's threads, newest activity first, with each thread's status (working, needs_attention, ready_for_review, open, resolved) and current assignment, and the agents that may work threads."
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
        let mut agents: Vec<Value> = Vec::new();
        if self.0.runs_self {
            agents.push(json!({ "agent": "self", "description": "This agent." }));
        }
        agents.extend(self.0.agents.iter().map(|agent| {
            json!({
                "agent": agent.label(),
                "description": agent.description,
            })
        }));
        ToolExecutionResult::success(json!({
            "threads": rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>(),
            "agents": agents,
            "any_agent": self.0.any_agent,
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

struct CloseThreadTool;

#[async_trait]
impl Tool for CloseThreadTool {
    context_tool!("close_thread", "Close Thread");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("close_thread requires session context.")
    }

    fn description(&self) -> &str {
        "Close a thread once the person is done with it, or reopen it with reopen=true. A thread that is still working is refused unless cancel=true, which stops its assignment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "reopen": { "type": "boolean" },
                "cancel": { "type": "boolean", "description": "Stop a working assignment and close the thread." }
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
        close_thread(arguments, context)
            .await
            .unwrap_or_else(|error| error)
    }
}

async fn close_thread(
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
            "This thread is still working. Wait for it, or pass cancel=true to stop it and close it.",
        ));
    }
    if !latest.state.is_terminal() {
        notify_thread(
            context,
            thread,
            "Your parent closed this thread. Stop work; do not start anything new.",
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
        Some(delegate) => delegate
            .set_session_archived(
                thread,
                true,
                &crate::platform_store::tool_change_reason("close_thread", "close thread"),
            )
            .await
            .is_ok(),
        None => false,
    };
    Ok(ToolExecutionResult::success(json!({
        "thread_id": thread.to_string(),
        "status": "resolved",
        "archived": archived,
    })))
}

#[path = "threads_parent.rs"]
mod worker;
#[cfg(test)]
use worker::{AskParentTool, HandBackTool};
pub use worker::{
    ThreadAssignment, ThreadTurn, open_assignment_for_thread, parent_tool_definitions,
    parent_tools, settle_thread_turn,
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
                "Your parent canceled this assignment. Stop work and reply with a one-line note on where you got to.",
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
#[path = "threads_tests.rs"]
mod tests;
