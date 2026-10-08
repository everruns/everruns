//! Immediate and scheduled background tool runs.
//!
//! `spawn_background` detaches a background-capable tool, mirrors its lifecycle
//! onto a session task, and streams progress back through the session's event
//! sink. Scheduled variants create a session schedule and a monitor task
//! instead of running immediately.
//!
//! This lives beside the `background_execution` capability rather than in
//! `everruns-core` (EVE-888). Creating session tasks and schedules is hosted
//! behaviour backed by hosted services — the execution kernel owns the neutral
//! `BackgroundExecutableTool`/`BackgroundEventSink` contracts it runs against,
//! not the tool that drives them. Keeping the tool in core also pinned the
//! session task and schedule records there, since core named them at execution
//! time.
//!
//! Admission control moved with it: the worker-wide and per-session semaphores
//! bound concurrent detached runs, and `subagents` shares the same permits so
//! every background path goes through one gate.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::tool_types::{ToolCall, ToolDefinition, ToolResult};
use everruns_contracts::typed_id::SessionId;
use everruns_core::background::{BackgroundEventSink, BackgroundOutcome, BackgroundProgress};
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_hooks::NestedToolPolicy;
use everruns_core::tools::{Tool, ToolExecutionResult, ToolRegistry, validate_tool_arguments};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Maximum active immediate background runs allowed for a single session.
///
/// This mirrors the scheduled monitor cap so model-visible background execution
/// cannot be used to queue unbounded active worker jobs for one session.
pub const MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION: usize = 5;

/// Maximum active immediate background runs allowed in this worker process.
///
/// The per-session semaphore limits tenant/session abuse; this process-wide
/// semaphore keeps concurrent sessions from exhausting worker-local resources.
const MAX_ACTIVE_BACKGROUND_RUNS_PER_WORKER: usize = 64;
static ACTIVE_BACKGROUND_RUNS_PER_WORKER: Semaphore =
    Semaphore::const_new(MAX_ACTIVE_BACKGROUND_RUNS_PER_WORKER);

/// Per-session semaphores that enforce `MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION`.
///
/// Using a semaphore rather than a DB count makes the check atomic: concurrent
/// `spawn_background` calls for the same session cannot both slip through the
/// guard (check-then-act race) because `try_acquire` is inherently atomic.
static SESSION_BACKGROUND_PERMITS: OnceLock<std::sync::Mutex<HashMap<SessionId, Arc<Semaphore>>>> =
    OnceLock::new();

struct SessionBackgroundPermit {
    session_id: SessionId,
    semaphore: Arc<Semaphore>,
    permit: Option<OwnedSemaphorePermit>,
}

/// Active background run permit held for the lifetime of detached work.
///
/// This combines the worker-wide guard with the per-session guard so all
/// background execution paths share the same admission control.
pub struct BackgroundRunPermit {
    _worker: tokio::sync::SemaphorePermit<'static>,
    _session: SessionBackgroundPermit,
}

impl Drop for SessionBackgroundPermit {
    fn drop(&mut self) {
        drop(self.permit.take());

        let Some(permits) = SESSION_BACKGROUND_PERMITS.get() else {
            return;
        };
        let mut permits = permits.lock().unwrap();
        let should_remove = permits.get(&self.session_id).is_some_and(|current| {
            Arc::ptr_eq(current, &self.semaphore)
                && self.semaphore.available_permits() == MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION
                && Arc::strong_count(&self.semaphore) == 2
        });
        if should_remove {
            permits.remove(&self.session_id);
        }
    }
}

fn try_acquire_session_background_permit(
    session_id: SessionId,
) -> std::result::Result<SessionBackgroundPermit, tokio::sync::TryAcquireError> {
    let permits = SESSION_BACKGROUND_PERMITS.get_or_init(Default::default);
    let mut permits = permits.lock().unwrap();
    let semaphore = permits
        .entry(session_id)
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION)))
        .clone();
    let permit = semaphore.clone().try_acquire_owned()?;

    Ok(SessionBackgroundPermit {
        session_id,
        semaphore,
        permit: Some(permit),
    })
}

/// Acquire the shared worker- and session-scoped admission permit used by
/// detached tool implementations.
pub fn try_acquire_background_run_permit(
    session_id: SessionId,
) -> std::result::Result<BackgroundRunPermit, String> {
    let worker = ACTIVE_BACKGROUND_RUNS_PER_WORKER.try_acquire().map_err(|_| {
        format!(
            "Worker is already running the maximum {MAX_ACTIVE_BACKGROUND_RUNS_PER_WORKER} active background runs. Try again after an existing run finishes."
        )
    })?;

    let session = match try_acquire_session_background_permit(session_id) {
        Ok(permit) => permit,
        Err(_) => {
            drop(worker);
            return Err(format!(
                "Maximum {MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION} active background runs per session. Wait for an existing run to finish before starting another."
            ));
        }
    };

    Ok(BackgroundRunPermit {
        _worker: worker,
        _session: session,
    })
}

#[cfg(test)]
fn has_session_background_permits(session_id: SessionId) -> bool {
    SESSION_BACKGROUND_PERMITS
        .get()
        .and_then(|permits| permits.lock().unwrap().get(&session_id).cloned())
        .is_some()
}

/// Spawn a background-capable tool and return immediately with a run handle.
pub struct SpawnBackgroundTool;

#[derive(Debug, Clone)]
struct BackgroundScheduleRequest {
    cron_expression: Option<String>,
    scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
    timezone: String,
}

fn parse_background_schedule(
    arguments: &Value,
) -> std::result::Result<Option<BackgroundScheduleRequest>, String> {
    let Some(schedule) = arguments.get("schedule") else {
        return Ok(None);
    };
    let Some(schedule) = schedule.as_object() else {
        return Err("schedule must be an object".to_string());
    };

    let cron_expression = schedule
        .get("cron_expression")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let scheduled_at = match schedule.get("scheduled_at").and_then(Value::as_str) {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(
                    chrono::DateTime::parse_from_rfc3339(value)
                        .map_err(|_| "scheduled_at must be RFC3339".to_string())?
                        .with_timezone(&chrono::Utc),
                )
            }
        }
        None => None,
    };

    match (cron_expression.is_some(), scheduled_at.is_some()) {
        (false, false) => {
            return Err(
                "schedule must include exactly one of cron_expression (recurring) or scheduled_at (one-shot)"
                    .to_string(),
            );
        }
        (true, true) => {
            return Err(
                "schedule must not include both cron_expression and scheduled_at; provide exactly one"
                    .to_string(),
            );
        }
        _ => {}
    }

    let timezone = schedule
        .get("timezone")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("UTC")
        .to_string();

    Ok(Some(BackgroundScheduleRequest {
        cron_expression,
        scheduled_at,
        timezone,
    }))
}

fn build_background_schedule_description(
    tool_name: &str,
    tool_args: &Value,
    title: &str,
    signal_on_completion: bool,
) -> String {
    let payload = json!({
        "tool": tool_name,
        "title": title,
        "signal_on_completion": signal_on_completion,
        "args": tool_args,
    });
    let payload_json =
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());

    format!(
        "Monitor: {title}\n\n\
This scheduled monitor fired. Start the background run now.\n\n\
spawn_background payload:\n{payload_json}"
    )
}

/// Run the target call through the turn's pre-tool chain and the target's
/// own schema, returning the call that may run or the outcome to record.
///
/// A refusal is the chain's own outcome, unchanged: a blocked call reads as
/// it would when called directly, and a deferral (a hosted approval request)
/// parks the turn for the target call, whose approval a retried
/// `spawn_background` with the same arguments then finds.
async fn authorize_target_call(
    policy: &dyn NestedToolPolicy,
    tool: &dyn Tool,
    tool_def: &ToolDefinition,
    requested: ToolCall,
    context: &ToolContext,
) -> std::result::Result<ToolCall, ToolExecutionResult> {
    let target_name = requested.name.clone();
    let authorized = policy
        .authorize(requested, tool_def, context)
        .await
        .map_err(|outcome| ToolExecutionResult::PolicyOutcome(Box::new(outcome)))?;
    // The decision covers this tool; a hook that retargets the call would
    // run something no gate decided on as that tool.
    if authorized.name != target_name {
        return Err(ToolExecutionResult::tool_error(format!(
            "A pre-tool hook changed the background target from {target_name} to {}; refusing to run it.",
            authorized.name
        )));
    }
    match validate_tool_arguments(tool, &authorized) {
        Ok(None) => Ok(authorized),
        Ok(Some(invalid)) => Err(ToolExecutionResult::tool_error(invalid)),
        Err(error) => Err(ToolExecutionResult::internal_error(error)),
    }
}

/// Text the run's log keeps when a post-tool hook withheld the output.
const WITHHELD_OUTPUT_LOG: &str = "[output withheld by tool policy]\n";

#[async_trait]
impl Tool for SpawnBackgroundTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_spawn_background(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "spawn_background"
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        // Shapes the turn, so it never runs from a shell script.
        everruns_contracts::tool_types::ToolHints::default().with_stays_direct(true)
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Background")
    }

    fn description(&self) -> &str {
        "Run a background-capable built-in tool asynchronously. Returns immediately and signals the session when the background run completes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "description": "Name of the built-in tool to execute in the background"
                },
                "args": {
                    "type": "object",
                    "description": "Arguments to pass to the target tool"
                },
                "title": {
                    "type": "string",
                    "description": "Optional human-readable label for the background run"
                },
                "schedule": {
                    "type": "object",
                    "description": "Optional session schedule. When provided, this creates a scheduled monitor instead of starting the run immediately.",
                    "properties": {
                        "cron_expression": {
                            "type": "string",
                            "description": "Standard 5-field cron expression for recurring runs (e.g. '*/10 * * * *' for every 10 minutes)"
                        },
                        "scheduled_at": {
                            "type": "string",
                            "description": "ISO 8601 datetime for a one-shot run (e.g. '2026-04-16T15:30:00Z')"
                        },
                        "timezone": {
                            "type": "string",
                            "description": "IANA timezone for the schedule. Default: UTC"
                        }
                    },
                    "additionalProperties": false
                },
                "signal_on_completion": {
                    "type": "boolean",
                    "description": "Send a synthetic user message back to the session when the run completes",
                    "default": true
                }
            },
            "required": ["tool", "args"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "spawn_background requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let tool_name = match arguments.get("tool").and_then(|v| v.as_str()) {
            Some(name) if !name.trim().is_empty() => name.trim(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: tool"),
        };
        let requested_args = match arguments.get("args") {
            Some(args) if args.is_object() => args.clone(),
            _ => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: args (object expected)",
                );
            }
        };
        let signal_on_completion = arguments
            .get("signal_on_completion")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let schedule_request = match parse_background_schedule(&arguments) {
            Ok(schedule) => schedule,
            Err(message) => return ToolExecutionResult::tool_error(message),
        };

        let Some(tool_registry) = &context.tool_registry else {
            return ToolExecutionResult::tool_error(
                "Tool registry not available in this context. spawn_background requires worker-side tool execution.",
            );
        };

        let Some(tool) = tool_registry.get(tool_name).cloned() else {
            return ToolExecutionResult::tool_error(format!("Unknown tool: {tool_name}"));
        };
        if tool_name == self.name() {
            return ToolExecutionResult::tool_error(
                "spawn_background cannot target itself recursively",
            );
        }
        if tool.hints().supports_background != Some(true) {
            return ToolExecutionResult::tool_error(format!(
                "Tool does not support background execution: {tool_name}"
            ));
        }
        if tool.as_background_executable().is_none() {
            return ToolExecutionResult::tool_error(format!(
                "Tool declared background support but has no background executor: {tool_name}"
            ));
        }
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                tool.display_name()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("Background {tool_name}"))
            });

        // THREAT[TM-TOOL-055]: the outer call passed the policy chain as
        // `spawn_background`; the target call has to pass it as itself before
        // anything is scheduled or run (EVE-1186). Only the authorized call's
        // arguments go any further, so a later rewrite cannot outlive the
        // decision.
        let Some(policy) = context.nested_tool_policy.clone() else {
            return ToolExecutionResult::tool_error(
                "spawn_background requires the turn's tool policy and can only run from an agent turn.",
            );
        };
        let target_def = tool.to_definition();
        let target_call = match authorize_target_call(
            policy.as_ref(),
            tool.as_ref(),
            &target_def,
            ToolCall {
                id: context
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| self.name().to_string()),
                name: tool_name.to_string(),
                arguments: requested_args,
            },
            context,
        )
        .await
        {
            Ok(call) => call,
            Err(refused) => return refused,
        };
        let tool_args = target_call.execution_arguments();

        // A schedule fire re-enters `spawn_background` in a later turn and is
        // authorized again then; checking here as well means a call the policy
        // refuses never becomes a schedule or a monitor probe spec.
        if let Some(schedule_request) = schedule_request {
            let Some(schedule_store) = &context.schedule_store else {
                return ToolExecutionResult::tool_error(
                    "Schedule store not available in this context. Scheduled monitors require session schedules.",
                );
            };

            let description = build_background_schedule_description(
                tool_name,
                &tool_args,
                &title,
                signal_on_completion,
            );

            return match schedule_store
                .create_schedule_enforcing_limits(
                    context.session_id,
                    description,
                    schedule_request.cron_expression.clone(),
                    schedule_request.scheduled_at,
                    schedule_request.timezone.clone(),
                )
                .await
            {
                Ok(schedule) => {
                    // Best-effort: create a monitor task linked to this schedule.
                    // The schedule is the source of truth; task creation failure
                    // does not fail the spawn.
                    let mut monitor_task_id: Option<String> = None;
                    if let Some(ref task_registry) = context.session_task_registry {
                        let spec = json!({
                            "tool": tool_name,
                            "arguments": &tool_args,
                            "schedule_id": schedule.id.to_string(),
                            "schedule_type": schedule.schedule_type,
                            "cron_expression": schedule.cron_expression,
                            "scheduled_at": schedule.scheduled_at,
                            "timezone": schedule.timezone,
                            "signal_on_completion": signal_on_completion,
                        });
                        match task_registry
                            .create(everruns_core::session_task::CreateSessionTask {
                                session_id: context.session_id,
                                id: None,
                                kind: everruns_core::session_task::TASK_KIND_MONITOR.to_string(),
                                display_name: title.clone(),
                                spec,
                                state: everruns_core::session_task::SessionTaskState::Running,
                                links: everruns_core::session_task::TaskLinks::default(),
                                // Silent: the schedule's injected prompt message already
                                // wakes the session; a second wake on every fire would be noise.
                                wake_policy: everruns_core::session_task::TaskWakePolicy::Silent,
                            })
                            .await
                        {
                            Ok(task) => {
                                monitor_task_id = Some(task.id);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    session_id = %context.session_id,
                                    schedule_id = %schedule.id,
                                    error = %e,
                                    "Failed to create monitor task for schedule (best-effort)"
                                );
                            }
                        }
                    }
                    ToolExecutionResult::success(json!({
                        "created": true,
                        "status": "scheduled",
                        "title": title,
                        "tool": tool_name,
                        "signal_on_completion": signal_on_completion,
                        "schedule_id": schedule.id.to_string(),
                        "schedule_type": schedule.schedule_type,
                        "cron_expression": schedule.cron_expression,
                        "scheduled_at": schedule.scheduled_at,
                        "timezone": schedule.timezone,
                        "next_trigger_at": schedule.next_trigger_at,
                        "enabled": schedule.enabled,
                        "task_id": monitor_task_id,
                    }))
                }
                Err(everruns_core::session_schedule::ScheduleLimitError::Store(err)) => {
                    ToolExecutionResult::internal_error(err)
                }
                Err(everruns_core::session_schedule::ScheduleLimitError::Rejected(msg)) => {
                    ToolExecutionResult::tool_error(msg)
                }
            };
        }

        let Some(task_registry) = &context.session_task_registry else {
            return ToolExecutionResult::tool_error(
                "Session task registry not available in this context. Background runs require task tracking.",
            );
        };
        if context.runtime_artifact_file_store().is_none() {
            return ToolExecutionResult::tool_error(
                "Session file store not available in this context. spawn_background requires artifact persistence.",
            );
        }

        let background_run_permit = match try_acquire_background_run_permit(context.session_id) {
            Ok(permit) => permit,
            Err(message) => return ToolExecutionResult::tool_error(message),
        };

        let run_id = format!("bg_{}", uuid::Uuid::now_v7().simple());
        let artifact_dir = format!("/.background/{run_id}");
        let log_path = format!("{artifact_dir}/output.log");
        let result_path = format!("{artifact_dir}/result.json");

        // Create the session task tracking this run (knowledge/runtime-resources/session-tasks.md).
        // task_registry is guaranteed Some above.
        let (task_id, task_attempt): (Option<String>, i32) = match task_registry
            .create(everruns_core::session_task::CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL.to_string(),
                display_name: title.clone(),
                spec: json!({
                    "tool": tool_name,
                    "arguments": &tool_args,
                    // Reaper uses this to decide whether to re-run on orphan.
                    // Only idempotent/readonly tools are safe to re-execute.
                    "reattachable": tool.hints().idempotent.unwrap_or(false)
                        || tool.hints().readonly.unwrap_or(false),
                    // Persisted so re-attach can restore the original signaling behavior.
                    "signal_on_completion": signal_on_completion,
                }),
                state: everruns_core::session_task::SessionTaskState::Running,
                links: everruns_core::session_task::TaskLinks::default(),
                wake_policy: everruns_core::session_task::TaskWakePolicy::Silent,
            })
            .await
        {
            Ok(task) => (Some(task.id), task.attempt),
            Err(e) => {
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to create background run task: {e}"
                ));
            }
        };

        let background_context = context.clone().with_tool_registry(tool_registry.clone());
        let sink = Arc::new(SessionBackgroundSink::new(
            background_context.clone(),
            run_id.clone(),
            title.clone(),
            tool_name.to_string(),
            log_path.clone(),
            result_path.clone(),
            signal_on_completion,
            task_id.clone(),
        ));
        let run_id_for_task = run_id.clone();
        let tool_for_task = tool.clone();
        let tool_name_for_task = tool_name.to_string();
        let policy_for_task = policy.clone();
        let target_def_for_task = target_def.clone();

        // Clone registry/ids for the cancel-watcher inside the spawned task.
        let cancel_registry = context.session_task_registry.clone();
        let cancel_session_id = context.session_id;
        let cancel_task_id = task_id.clone();
        // Attempt captured at task creation for stale-attempt fencing.
        let cancel_task_attempt = task_attempt;

        tokio::spawn(async move {
            let _background_run_permit = background_run_permit;
            let _ = sink.status("Starting").await;

            // Run the tool future inside a select! against a cancel-watch loop
            // when a task registry and task_id are wired up.  The watcher sends
            // a heartbeat every ~2 s and checks cancel_requested_at; when set,
            // it wins the select and the tool future is dropped.
            //
            // Rationale: cancel intent is recorded in shared storage so this
            // design works even when cancel_task executes on a different worker.
            let outcome: std::result::Result<BackgroundOutcome, ToolExecutionResult> = match (
                cancel_registry.as_ref(),
                cancel_task_id.as_deref(),
            ) {
                (Some(registry), Some(task_id_str)) => {
                    let registry = registry.clone();
                    let task_id_str = task_id_str.to_string();
                    let tool_fut = async {
                        match tool_for_task.as_background_executable() {
                            Some(background_tool) => {
                                background_tool
                                    .execute_background(tool_args, background_context, sink.clone())
                                    .await
                            }
                            None => Err(ToolExecutionResult::tool_error(format!(
                                "Tool declared background support but has no background executor: {}",
                                tool_name_for_task
                            ))),
                        }
                    };
                    let watch_fut = async {
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            // Send heartbeat with stale-attempt fencing so a
                            // superseded executor's heartbeats are rejected once
                            // the reaper increments the attempt counter.
                            let _ = registry
                                .update(
                                    cancel_session_id,
                                    &task_id_str,
                                    everruns_core::session_task::SessionTaskUpdate {
                                        heartbeat_at: Some(chrono::Utc::now()),
                                        expected_attempt: Some(cancel_task_attempt),
                                        ..Default::default()
                                    },
                                )
                                .await;
                            // Check for cooperative cancel intent.
                            if let Ok(Some(task)) =
                                registry.get(cancel_session_id, &task_id_str).await
                                && task.cancel_requested_at.is_some()
                            {
                                break;
                            }
                        }
                    };
                    tokio::select! {
                        result = tool_fut => result,
                        () = watch_fut => {
                            // Cancel was requested; the tool future is dropped.
                            Err(ToolExecutionResult::ToolError(
                                BACKGROUND_CANCEL_SENTINEL.to_string(),
                            ))
                        }
                    }
                }
                // No registry or no task_id: run without cancel watch (unchanged behaviour).
                _ => match tool_for_task.as_background_executable() {
                    Some(background_tool) => {
                        background_tool
                            .execute_background(tool_args, background_context, sink.clone())
                            .await
                    }
                    None => Err(ToolExecutionResult::tool_error(format!(
                        "Tool declared background support but has no background executor: {}",
                        tool_name_for_task
                    ))),
                },
            };

            // Route canceled outcome through finalize_canceled so file/resource
            // cleanup is consistent with the success/fail paths.
            let finalize_result = if is_canceled_outcome(&outcome) {
                sink.finalize_canceled().await
            } else {
                let outcome = sink
                    .apply_post_tool_policy(
                        policy_for_task.as_ref(),
                        &target_call,
                        &target_def_for_task,
                        outcome,
                    )
                    .await;
                sink.finalize(outcome).await
            };
            if let Err(err) = finalize_result {
                tracing::warn!(
                    run_id = run_id_for_task,
                    error = %err,
                    "Background run finalization failed"
                );
            }
        });

        ToolExecutionResult::success(json!({
            "run_id": run_id,
            "resource_id": run_id,
            "task_id": task_id,
            "title": title,
            "tool": tool_name,
            "status": "running",
            "signal_on_completion": signal_on_completion,
            "artifact_dir": artifact_dir,
            "log_path": log_path,
            "result_path": result_path
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Debug, Default)]
struct SessionBackgroundState {
    status_text: String,
    progress: Option<BackgroundProgress>,
    output_tail: String,
    output_log: String,
    output_log_chars: usize,
    output_log_truncated: bool,
    /// The log as the post-tool chain left it; replaces the streamed log.
    output_log_override: Option<String>,
}

const MAX_BACKGROUND_OUTPUT_LOG_CHARS: usize = 256 * 1024;

struct SessionBackgroundSink {
    context: ToolContext,
    run_id: String,
    display_name: String,
    tool_name: String,
    log_path: String,
    result_path: String,
    signal_on_completion: bool,
    /// Session task mirroring this run; None when no task registry is wired.
    task_id: Option<String>,
    state: tokio::sync::Mutex<SessionBackgroundState>,
}

impl SessionBackgroundSink {
    #[allow(clippy::too_many_arguments)]
    fn new(
        context: ToolContext,
        run_id: String,
        display_name: String,
        tool_name: String,
        log_path: String,
        result_path: String,
        signal_on_completion: bool,
        task_id: Option<String>,
    ) -> Self {
        Self {
            context,
            run_id,
            display_name,
            tool_name,
            log_path,
            result_path,
            signal_on_completion,
            task_id,
            state: tokio::sync::Mutex::new(SessionBackgroundState {
                status_text: "Queued".to_string(),
                ..Default::default()
            }),
        }
    }

    /// Mirror an update onto the session task (best-effort).
    async fn mirror_task(&self, update: everruns_core::session_task::SessionTaskUpdate) {
        let (Some(registry), Some(task_id)) = (&self.context.session_task_registry, &self.task_id)
        else {
            return;
        };
        let _ = registry
            .update(self.context.session_id, task_id, update)
            .await;
    }

    async fn finalize_canceled(&self) -> Result<()> {
        let output_log = {
            let state = self.state.lock().await;
            let mut log = Self::final_output_log(&state);
            log.push_str("\nCanceled by request.\n");
            log
        };
        self.write_text_file(&self.log_path, &output_log).await?;
        let result_json = serde_json::to_string_pretty(&serde_json::json!({"status": "canceled"}))
            .unwrap_or_else(|_| r#"{"status":"canceled"}"#.to_string());
        self.write_text_file(&self.result_path, &result_json)
            .await?;

        let mut state = self.state.lock().await;
        state.status_text = "Canceled".to_string();
        drop(state);

        self.mirror_task(everruns_core::session_task::SessionTaskUpdate {
            state: Some(everruns_core::session_task::SessionTaskState::Canceled),
            summary: Some("Canceled by request.".to_string()),
            result_path: Some(self.result_path.clone()),
            ..Default::default()
        })
        .await;
        if self.signal_on_completion {
            self.signal_session("canceled", "Canceled by request.")
                .await?;
        }
        Ok(())
    }

    async fn finalize(
        &self,
        outcome: std::result::Result<BackgroundOutcome, ToolExecutionResult>,
    ) -> Result<()> {
        match outcome {
            Ok(outcome) => {
                let output_log = if let Some(raw_output) = &outcome.raw_output {
                    raw_output.clone()
                } else {
                    let state = self.state.lock().await;
                    Self::final_output_log(&state)
                };
                self.write_text_file(&self.log_path, &output_log).await?;
                let result_json = serde_json::to_string_pretty(&outcome.result)
                    .unwrap_or_else(|_| outcome.result.to_string());
                self.write_text_file(&self.result_path, &result_json)
                    .await?;

                let mut state = self.state.lock().await;
                state.status_text = "Completed".to_string();
                drop(state);
                self.mirror_task(everruns_core::session_task::SessionTaskUpdate {
                    state: Some(everruns_core::session_task::SessionTaskState::Succeeded),
                    summary: Some(outcome.summary.clone()),
                    result_path: Some(self.result_path.clone()),
                    ..Default::default()
                })
                .await;
                if self.signal_on_completion {
                    self.signal_session("completed", &outcome.summary).await?;
                }
            }
            Err(err) => {
                let message = failure_message(err);
                let output_log = {
                    let state = self.state.lock().await;
                    Self::final_output_log(&state)
                };
                self.write_text_file(&self.log_path, &output_log).await?;
                let error_json = serde_json::to_string_pretty(&json!({
                    "status": "failed",
                    "error": &message,
                }))
                .unwrap_or_else(|_| {
                    json!({
                        "status": "failed",
                        "error": &message,
                    })
                    .to_string()
                });
                self.write_text_file(&self.result_path, &error_json).await?;
                let mut state = self.state.lock().await;
                state.status_text = "Failed".to_string();
                drop(state);
                self.mirror_task(everruns_core::session_task::SessionTaskUpdate {
                    state: Some(everruns_core::session_task::SessionTaskState::Failed),
                    summary: Some(message.clone()),
                    result_path: Some(self.result_path.clone()),
                    error: Some(everruns_core::session_task::TaskError {
                        kind: "error".to_string(),
                        message: message.clone(),
                    }),
                    ..Default::default()
                })
                .await;
                if self.signal_on_completion {
                    self.signal_session("failed", &message).await?;
                }
            }
        }

        Ok(())
    }

    async fn signal_session(&self, status: &str, summary: &str) -> Result<()> {
        // Without a platform store there is nowhere to deliver the completion,
        // and the background run finishes invisibly — the agent that spawned it
        // never learns it ended. Say so: a host that wires `spawn_background`
        // but no store has a hole, not a preference.
        let Some(platform_store) = &self.context.subagent_delegate else {
            tracing::warn!(
                run_id = %self.run_id,
                tool = %self.tool_name,
                session_id = %self.context.session_id,
                %status,
                "background run finished but no platform store is configured; \
                 the session will not be woken (see everruns::local wake routing)"
            );
            return Ok(());
        };
        let message = format!(
            "Background run {status}.\n- run_id: {}\n- title: {}\n- tool: {}\n- summary: {}\n- result_path: {}\n- log_path: {}",
            self.run_id,
            self.display_name,
            self.tool_name,
            summary,
            self.result_path,
            self.log_path
        );
        platform_store
            .send_message(self.context.session_id, &message)
            .await
    }

    async fn write_text_file(&self, path: &str, content: &str) -> Result<()> {
        // Runtime-owned record: see `ToolContext::runtime_artifact_file_store`.
        // The model reads these artifacts but its workspace policy never lets
        // it write them (EVE-1165).
        let file_store = self.context.runtime_artifact_file_store().ok_or_else(|| {
            anyhow::anyhow!(
                "background run {} cannot persist artifact {} because no session file store is configured",
                self.run_id,
                path
            )
        })?;

        ensure_directory(file_store.as_ref(), self.context.session_id, "/.background").await?;
        let run_dir = format!("/.background/{}", self.run_id);
        ensure_directory(file_store.as_ref(), self.context.session_id, &run_dir).await?;
        file_store
            .write_file(self.context.session_id, path, content, "text")
            .await?;
        Ok(())
    }
}

#[async_trait]
impl BackgroundEventSink for SessionBackgroundSink {
    async fn status(&self, message: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        state.status_text = message.to_string();
        drop(state);
        self.mirror_task(everruns_core::session_task::SessionTaskUpdate {
            state_detail: Some(message.to_string()),
            ..Default::default()
        })
        .await;
        Ok(())
    }

    async fn output(&self, stream: &str, delta: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        if !delta.is_empty() {
            let prefix = format!("[{stream}] ");
            state.output_tail.push_str(&prefix);
            state.output_tail.push_str(delta);
            Self::append_to_output_log(&mut state, &prefix, delta);
            if state.output_tail.chars().count() > 2048 {
                state.output_tail = state
                    .output_tail
                    .chars()
                    .rev()
                    .take(2048)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
            }
        }
        Ok(())
    }

    async fn progress(&self, progress: BackgroundProgress) -> Result<()> {
        let mut state = self.state.lock().await;
        state.progress = Some(progress.clone());
        drop(state);
        self.mirror_task(everruns_core::session_task::SessionTaskUpdate {
            progress: Some(progress),
            ..Default::default()
        })
        .await;
        Ok(())
    }
}

impl SessionBackgroundSink {
    /// Run the turn's post-tool chain on the finished run, as for a direct
    /// call of the target tool (EVE-1186).
    ///
    /// Hooks see the run's result, its summary (what the session is told) and
    /// its output log, and what they leave is what gets persisted and
    /// signalled: an output guardrail that withholds the result also withholds
    /// the summary and the log.
    async fn apply_post_tool_policy(
        &self,
        policy: &dyn NestedToolPolicy,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        outcome: std::result::Result<BackgroundOutcome, ToolExecutionResult>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
        let streamed_log = {
            let state = self.state.lock().await;
            Self::final_output_log(&state)
        };
        let failed = outcome.is_err();
        let mut result = match outcome {
            Ok(outcome) => ToolResult {
                tool_call_id: tool_call.id.clone(),
                result: Some(json!({ "summary": outcome.summary, "result": outcome.result })),
                images: None,
                error: None,
                connection_required: None,
                raw_output: Some(outcome.raw_output.unwrap_or(streamed_log)),
            },
            Err(err) => {
                let message = failure_message(err);
                ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: Some(json!({ "error": &message })),
                    images: None,
                    error: Some(message),
                    connection_required: None,
                    raw_output: Some(streamed_log),
                }
            }
        };

        policy
            .after_exec(tool_call, tool_def, &mut result, &self.context)
            .await;

        {
            let mut state = self.state.lock().await;
            state.output_log_override = Some(
                result
                    .raw_output
                    .take()
                    .unwrap_or_else(|| WITHHELD_OUTPUT_LOG.to_string()),
            );
        }

        let value = result.result.take().unwrap_or(Value::Null);
        let text = || match &value {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        if failed || result.error.is_some() {
            let message = result.error.take().unwrap_or_else(text);
            return Err(ToolExecutionResult::ToolError(message));
        }
        match (
            value.get("summary").and_then(Value::as_str),
            value.get("result"),
        ) {
            (Some(summary), Some(inner)) => Ok(BackgroundOutcome {
                summary: summary.to_string(),
                result: inner.clone(),
                raw_output: None,
            }),
            // A hook replaced the whole result (an output guardrail's notice):
            // that replacement is all anyone gets to see.
            _ => Ok(BackgroundOutcome {
                summary: text(),
                result: value.clone(),
                raw_output: None,
            }),
        }
    }

    fn append_to_output_log(state: &mut SessionBackgroundState, prefix: &str, delta: &str) {
        if state.output_log_chars >= MAX_BACKGROUND_OUTPUT_LOG_CHARS {
            state.output_log_truncated = true;
            return;
        }

        let chunk = format!("{prefix}{delta}");
        let remaining = MAX_BACKGROUND_OUTPUT_LOG_CHARS - state.output_log_chars;
        let chunk_chars = chunk.chars().count();

        if chunk_chars <= remaining {
            state.output_log.push_str(&chunk);
            state.output_log_chars += chunk_chars;
            return;
        }

        let truncated_chunk: String = chunk.chars().take(remaining).collect();
        state.output_log.push_str(&truncated_chunk);
        state.output_log_chars += truncated_chunk.chars().count();
        state.output_log_truncated = true;
    }

    fn final_output_log(state: &SessionBackgroundState) -> String {
        if let Some(log) = &state.output_log_override {
            return log.clone();
        }
        if !state.output_log_truncated {
            return state.output_log.clone();
        }

        format!(
            "{}\n[system] background output truncated at {} characters\n",
            state.output_log, MAX_BACKGROUND_OUTPUT_LOG_CHARS
        )
    }
}

/// What a failed run reports to its result file and the session.
fn failure_message(err: ToolExecutionResult) -> String {
    match err {
        ToolExecutionResult::ToolError(msg) => msg,
        ToolExecutionResult::InternalError(inner) => inner.message,
        ToolExecutionResult::ConnectionRequired { provider, .. } => {
            format!("Background tool requires connection setup: {provider}")
        }
        ToolExecutionResult::PolicyOutcome(result) => result
            .error
            .unwrap_or_else(|| "Background run was not allowed by policy".to_string()),
        ToolExecutionResult::Success(_) | ToolExecutionResult::SuccessWithImages { .. } => {
            "Background run ended unexpectedly".to_string()
        }
    }
}

/// Internal sentinel injected by the cancel-watch select branch. Namespaced
/// so a background tool that legitimately fails with "canceled" is never
/// misclassified as a cooperative cancel.
const BACKGROUND_CANCEL_SENTINEL: &str = "__everruns_background_cancel__";

/// Returns true when an outcome from the cancel-watch select is the sentinel
/// cancel signal.  Factored out so the condition is testable without a
/// running async runtime.
fn is_canceled_outcome(
    outcome: &std::result::Result<BackgroundOutcome, ToolExecutionResult>,
) -> bool {
    matches!(outcome, Err(ToolExecutionResult::ToolError(msg)) if msg == BACKGROUND_CANCEL_SENTINEL)
}

/// Re-attach a `background_tool` task after worker loss.
///
/// Called by a host-owned task executor when the reaper decides the task is
/// safe to restart. Reads `spec["tool"]` and `spec["arguments"]` from
/// the task, looks up the tool in the built-in default registry, and spawns a
/// fresh background run with `task.attempt` as the heartbeat fence. New artifact
/// paths are generated so old partial artifacts do not conflict.
///
/// Returns an error (→ reaper falls back to orphaned-fail) when:
/// - `context.runtime_artifact_file_store()` or `context.session_task_registry` is absent
/// - `spec["tool"]` is absent or empty
/// - the tool is not in the built-in default registry
/// - the tool does not implement `BackgroundExecutable`
/// - the tool's current hints are not `idempotent` or `readonly`
/// - per-worker or per-session background concurrency caps are exhausted
pub async fn reattach_background_run(
    task: &everruns_core::session_task::SessionTask,
    context: &everruns_core::tool_context::ToolContext,
) -> everruns_contracts::error::Result<()> {
    // Fail fast before spawning a tokio task so the reaper can fall back to
    // orphaned-fail rather than leaving the task stuck in Running forever.
    if context.runtime_artifact_file_store().is_none() {
        return Err(AgentLoopError::tool(
            "file store not available; cannot re-attach background run",
        ));
    }
    if context.session_task_registry.is_none() {
        return Err(AgentLoopError::tool(
            "task registry not available; cannot re-attach background run",
        ));
    }

    let tool_name: String = task
        .spec
        .get("tool")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            AgentLoopError::tool("background_tool spec missing 'tool' field; cannot re-attach")
        })?;

    let tool_args = task
        .spec
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

    let registry = std::sync::Arc::new(ToolRegistry::with_defaults());

    let Some(tool) = registry.get(&tool_name).cloned() else {
        return Err(AgentLoopError::tool(format!(
            "tool '{tool_name}' not found in built-in registry; cannot re-attach"
        )));
    };

    if tool.as_background_executable().is_none() {
        return Err(AgentLoopError::tool(format!(
            "tool '{tool_name}' does not support background execution; cannot re-attach"
        )));
    }

    // Re-verify tool hints from the live registry rather than trusting
    // spec["reattachable"], which could be forged via task creation APIs.
    let hints = tool.hints();
    if !hints.idempotent.unwrap_or(false) && !hints.readonly.unwrap_or(false) {
        return Err(AgentLoopError::tool(format!(
            "tool '{tool_name}' is not idempotent or readonly; re-attach declined",
        )));
    }

    // Enforce the same concurrency caps as spawn_background so many concurrent
    // re-attaches cannot exhaust worker or session limits.
    let background_run_permit =
        try_acquire_background_run_permit(task.session_id).map_err(AgentLoopError::tool)?;

    // Restore original signaling behavior; default true for tasks created before
    // this field was persisted.
    let signal_on_completion = task
        .spec
        .get("signal_on_completion")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let run_id = format!("bg_{}", uuid::Uuid::now_v7().simple());
    let artifact_dir = format!("/.background/{run_id}");
    let log_path = format!("{artifact_dir}/output.log");
    let result_path = format!("{artifact_dir}/result.json");

    let task_id = task.id.clone();
    let task_attempt = task.attempt;
    let session_id = task.session_id;

    let sink_context = context.clone().with_tool_registry(registry);
    let sink = std::sync::Arc::new(SessionBackgroundSink::new(
        sink_context.clone(),
        run_id.clone(),
        task.display_name.clone(),
        tool_name.to_string(),
        log_path,
        result_path,
        signal_on_completion,
        Some(task_id.clone()),
    ));

    let cancel_registry = context.session_task_registry.clone();
    let run_id_for_log = run_id.clone();

    tokio::spawn(async move {
        // Hold permits for the duration of the re-attached run.
        let _background_run_permit = background_run_permit;
        let _ = sink.status("Re-attaching").await;

        let outcome: std::result::Result<BackgroundOutcome, ToolExecutionResult> =
            match (cancel_registry.as_ref(), Some(task_id.as_str())) {
                (Some(registry), Some(task_id_str)) => {
                    let registry = registry.clone();
                    let task_id_str = task_id_str.to_string();
                    let tool_fut = async {
                        match tool.as_background_executable() {
                            Some(bg) => {
                                bg.execute_background(tool_args, sink_context.clone(), sink.clone())
                                    .await
                            }
                            None => Err(ToolExecutionResult::tool_error(format!(
                                "tool '{tool_name}' lost background support during re-attach"
                            ))),
                        }
                    };
                    let watch_fut = async {
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            let _ = registry
                                .update(
                                    session_id,
                                    &task_id_str,
                                    everruns_core::session_task::SessionTaskUpdate {
                                        heartbeat_at: Some(chrono::Utc::now()),
                                        expected_attempt: Some(task_attempt),
                                        ..Default::default()
                                    },
                                )
                                .await;
                            if let Ok(Some(t)) = registry.get(session_id, &task_id_str).await
                                && t.cancel_requested_at.is_some()
                            {
                                break;
                            }
                        }
                    };
                    tokio::select! {
                        result = tool_fut => result,
                        () = watch_fut => Err(ToolExecutionResult::ToolError(
                            BACKGROUND_CANCEL_SENTINEL.to_string(),
                        )),
                    }
                }
                _ => match tool.as_background_executable() {
                    Some(bg) => {
                        bg.execute_background(tool_args, sink_context, sink.clone())
                            .await
                    }
                    None => Err(ToolExecutionResult::tool_error(format!(
                        "tool '{tool_name}' lost background support during re-attach"
                    ))),
                },
            };

        let finalize_result = if is_canceled_outcome(&outcome) {
            sink.finalize_canceled().await
        } else {
            sink.finalize(outcome).await
        };
        if let Err(err) = finalize_result {
            tracing::warn!(
                run_id = run_id_for_log,
                error = %err,
                "Background run re-attach finalization failed"
            );
        }
    });

    Ok(())
}

async fn ensure_directory(
    file_store: &dyn everruns_core::session_files::SessionFileSystem,
    session_id: everruns_contracts::typed_id::SessionId,
    path: &str,
) -> Result<()> {
    if let Some(entry) = file_store.stat_file(session_id, path).await? {
        if entry.is_directory {
            return Ok(());
        }
        return Err(anyhow::anyhow!("path exists but is not a directory: {path}").into());
    }
    let _ = file_store.create_directory(session_id, path).await?;
    Ok(())
}

#[cfg(test)]
#[path = "background_run_policy_tests.rs"]
mod policy_tests;

#[cfg(test)]
#[path = "background_run_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "background_run_tests.rs"]
mod tests;
