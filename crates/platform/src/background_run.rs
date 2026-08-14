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
use everruns_core::background::{BackgroundEventSink, BackgroundOutcome, BackgroundProgress};
use everruns_core::error::Result;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult, ToolRegistry};
use everruns_core::typed_id::SessionId;
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

#[async_trait]
impl Tool for SpawnBackgroundTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
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
        let tool_args = match arguments.get("args") {
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
        if context.file_store.is_none() {
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
                let message = match err {
                    ToolExecutionResult::ToolError(msg) => msg,
                    ToolExecutionResult::InternalError(inner) => inner.message,
                    ToolExecutionResult::ConnectionRequired { provider } => {
                        format!("Background tool requires connection setup: {provider}")
                    }
                    ToolExecutionResult::Success(_)
                    | ToolExecutionResult::SuccessWithImages { .. } => {
                        "Background run ended unexpectedly".to_string()
                    }
                };
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
                 the session will not be woken (see everruns-local's wake routing)"
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
        let file_store = self.context.file_store.as_ref().ok_or_else(|| {
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
        if !state.output_log_truncated {
            return state.output_log.clone();
        }

        format!(
            "{}\n[system] background output truncated at {} characters\n",
            state.output_log, MAX_BACKGROUND_OUTPUT_LOG_CHARS
        )
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
/// - `context.file_store` or `context.session_task_registry` is absent
/// - `spec["tool"]` is absent or empty
/// - the tool is not in the built-in default registry
/// - the tool does not implement `BackgroundExecutable`
/// - the tool's current hints are not `idempotent` or `readonly`
/// - per-worker or per-session background concurrency caps are exhausted
pub async fn reattach_background_run(
    task: &everruns_core::session_task::SessionTask,
    context: &everruns_core::tool_context::ToolContext,
) -> everruns_core::error::Result<()> {
    // Fail fast before spawning a tokio task so the reaper can fall back to
    // orphaned-fail rather than leaving the task stuck in Running forever.
    if context.file_store.is_none() {
        return Err(everruns_core::error::AgentLoopError::tool(
            "file store not available; cannot re-attach background run",
        ));
    }
    if context.session_task_registry.is_none() {
        return Err(everruns_core::error::AgentLoopError::tool(
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
            everruns_core::error::AgentLoopError::tool(
                "background_tool spec missing 'tool' field; cannot re-attach",
            )
        })?;

    let tool_args = task
        .spec
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

    let registry = std::sync::Arc::new(ToolRegistry::with_defaults());

    let Some(tool) = registry.get(&tool_name).cloned() else {
        return Err(everruns_core::error::AgentLoopError::tool(format!(
            "tool '{tool_name}' not found in built-in registry; cannot re-attach"
        )));
    };

    if tool.as_background_executable().is_none() {
        return Err(everruns_core::error::AgentLoopError::tool(format!(
            "tool '{tool_name}' does not support background execution; cannot re-attach"
        )));
    }

    // Re-verify tool hints from the live registry rather than trusting
    // spec["reattachable"], which could be forged via task creation APIs.
    let hints = tool.hints();
    if !hints.idempotent.unwrap_or(false) && !hints.readonly.unwrap_or(false) {
        return Err(everruns_core::error::AgentLoopError::tool(format!(
            "tool '{tool_name}' is not idempotent or readonly; re-attach declined",
        )));
    }

    // Enforce the same concurrency caps as spawn_background so many concurrent
    // re-attaches cannot exhaust worker or session limits.
    let background_run_permit = try_acquire_background_run_permit(task.session_id)
        .map_err(everruns_core::error::AgentLoopError::tool)?;

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
    session_id: everruns_core::SessionId,
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
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_core::background::BackgroundExecutableTool;
    use everruns_core::session_file::{FileInfo, FileStat, SessionFile};
    use everruns_core::session_task::SessionTaskRegistry;
    use everruns_core::subagent_delegation::SubagentSessionDelegate;
    use everruns_core::tool_types::ToolHints;
    use everruns_core::typed_id::HarnessId;
    use everruns_core::{session_files::SessionFileSystem, session_services::SessionScheduleStore};
    use everruns_core::{session_services::KeyInfo, session_services::SecretInfo};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc as StdArc, Mutex};

    #[derive(Default)]
    struct TestFailingBackgroundTool;

    #[async_trait]
    impl BackgroundExecutableTool for TestFailingBackgroundTool {
        async fn execute_background(
            &self,
            _arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            sink.status("Running failing test")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            sink.output("stderr", "background failed")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            Err(ToolExecutionResult::tool_error("boom"))
        }
    }

    #[async_trait]
    impl Tool for TestFailingBackgroundTool {
        fn name(&self) -> &str {
            "test_background_fail"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background Fail")
        }

        fn description(&self) -> &str {
            "failing background test tool"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    #[derive(Default)]
    struct TestLargeOutputBackgroundTool;

    #[async_trait]
    impl BackgroundExecutableTool for TestLargeOutputBackgroundTool {
        async fn execute_background(
            &self,
            _arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            let large_chunk = "x".repeat(MAX_BACKGROUND_OUTPUT_LOG_CHARS + 4096);
            sink.output("stdout", &large_chunk)
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            Ok(BackgroundOutcome {
                summary: "large output complete".to_string(),
                result: json!({"ok": true}),
                raw_output: None,
            })
        }
    }

    #[async_trait]
    impl Tool for TestLargeOutputBackgroundTool {
        fn name(&self) -> &str {
            "test_background_large_output"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background Large Output")
        }

        fn description(&self) -> &str {
            "background test tool with huge output"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    struct BlockingBackgroundTool {
        release: StdArc<AtomicBool>,
    }

    #[async_trait]
    impl BackgroundExecutableTool for BlockingBackgroundTool {
        async fn execute_background(
            &self,
            _arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            sink.status("Blocking until released")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            while !self.release.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Ok(BackgroundOutcome {
                summary: "released".to_string(),
                result: json!({"ok": true}),
                raw_output: None,
            })
        }
    }

    #[async_trait]
    impl Tool for BlockingBackgroundTool {
        fn name(&self) -> &str {
            "test_background_blocking"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background Blocking")
        }

        fn description(&self) -> &str {
            "background test tool that waits for test release"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    /// A background tool that sleeps indefinitely, allowing the test to exercise
    /// cancel via the cancel-watcher without actually waiting forever.
    #[derive(Default)]
    struct SleepingBackgroundTool;

    #[async_trait]
    impl BackgroundExecutableTool for SleepingBackgroundTool {
        async fn execute_background(
            &self,
            _arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            sink.status("Sleeping forever")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            // Sleep for a very long time; the cancel-watcher will win the select.
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            Ok(BackgroundOutcome {
                summary: "should not reach here".to_string(),
                result: json!({}),
                raw_output: None,
            })
        }
    }

    #[async_trait]
    impl Tool for SleepingBackgroundTool {
        fn name(&self) -> &str {
            "test_background_sleeping"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background Sleeping")
        }

        fn description(&self) -> &str {
            "background test tool that sleeps indefinitely"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    #[tokio::test]
    async fn test_spawn_background_executes_and_signals_session() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let platform_store = Arc::new(TestSubagentDelegate::default());
        let storage_store = Arc::new(NoopStorageStore);
        let task_registry = Arc::new(InMemoryTaskRegistry::default());
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();

        let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_subagent_delegate(platform_store.clone())
            .with_session_task_registry(task_registry.clone());

        let tool = SpawnBackgroundTool;
        let result = tool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": { "summary": "Background complete" }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should succeed");
        };
        let run_id = value["run_id"].as_str().unwrap().to_string();
        let task_id = value["task_id"].as_str().unwrap().to_string();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                    && task.state == everruns_core::session_task::SessionTaskState::Succeeded
                {
                    break task;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background run should complete");
        let _ = run_id; // still available in result json

        let messages = platform_store.sent_messages.lock().unwrap().clone();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("Background run completed"));

        let log_file = file_store
            .read_file(session_id, &format!("/.background/{run_id}/output.log"))
            .await
            .unwrap()
            .expect("log file");
        assert!(
            log_file
                .content
                .as_deref()
                .unwrap_or_default()
                .contains("hello from background")
        );
    }

    #[tokio::test]
    async fn test_spawn_background_persists_failure_artifacts() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let task_registry = Arc::new(InMemoryTaskRegistry::default());
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestFailingBackgroundTool)
            .build();

        let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_task_registry(task_registry.clone());

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background_fail",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should succeed");
        };
        let run_id = value["run_id"].as_str().unwrap().to_string();
        let task_id = value["task_id"].as_str().unwrap().to_string();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                    && task.state == everruns_core::session_task::SessionTaskState::Failed
                {
                    break task;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background run should fail");
        let _ = run_id;

        let log_file = file_store
            .read_file(session_id, &format!("/.background/{run_id}/output.log"))
            .await
            .unwrap()
            .expect("log file");
        assert!(
            log_file
                .content
                .as_deref()
                .unwrap_or_default()
                .contains("background failed")
        );

        let result_file = file_store
            .read_file(session_id, &format!("/.background/{run_id}/result.json"))
            .await
            .unwrap()
            .expect("result file");
        let result_json: Value =
            serde_json::from_str(result_file.content.as_deref().unwrap_or_default())
                .expect("valid json");
        assert_eq!(result_json["status"], "failed");
        assert_eq!(result_json["error"], "boom");
    }

    #[tokio::test]
    async fn test_spawn_background_rejects_when_session_active_run_limit_reached() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let task_registry = Arc::new(InMemoryTaskRegistry::default());
        let release = StdArc::new(AtomicBool::new(false));
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(BlockingBackgroundTool {
                release: release.clone(),
            })
            .build();

        let context = ToolContext::with_stores(session_id, file_store, storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_task_registry(task_registry.clone());

        let mut task_ids = Vec::new();
        for _ in 0..MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION {
            let result = SpawnBackgroundTool
                .execute_with_context(
                    json!({
                        "tool": "test_background_blocking",
                        "args": {}
                    }),
                    &context,
                )
                .await;

            let ToolExecutionResult::Success(value) = result else {
                panic!("background run below the session limit should start");
            };
            task_ids.push(value["task_id"].as_str().unwrap().to_string());
        }

        // Wait for all tasks to be running (semaphore acquired).
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let running = task_registry
                    .list(
                        session_id,
                        Some(&everruns_core::session_task::SessionTaskFilter {
                            kind: Some(
                                everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL.to_string(),
                            ),
                            state: Some(everruns_core::session_task::SessionTaskState::Running),
                        }),
                    )
                    .await
                    .unwrap();
                if running.len() == MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background runs should become running");

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background_blocking",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            release.store(true, Ordering::SeqCst);
            panic!("spawn_background should reject once the session limit is reached");
        };
        assert!(message.contains("active background runs per session"));

        release.store(true, Ordering::SeqCst);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            for task_id in task_ids {
                loop {
                    if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                        && task.state == everruns_core::session_task::SessionTaskState::Succeeded
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        })
        .await
        .expect("blocking background runs should complete after release");

        // The permit drop is enqueued in the spawned task after it marks the
        // resource Completed, so the cache entry may still exist briefly once
        // we observe Completed status.  Poll until pruned rather than asserting
        // immediately to avoid a race.
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if !has_session_background_permits(session_id) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("completed background runs should prune their per-session permit cache entry");
    }

    #[tokio::test]
    async fn test_spawn_background_requires_task_registry() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();

        // No task_registry wired — should fail.
        let context = ToolContext::with_stores(session_id, file_store, storage_store)
            .with_tool_registry(Arc::new(tool_registry));

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            panic!("spawn_background should reject missing task registry");
        };
        assert!(message.contains("Session task registry not available"));
    }

    #[tokio::test]
    async fn test_spawn_background_requires_file_store() {
        let session_id = SessionId::new();
        let storage_store = Arc::new(NoopStorageStore);
        let task_registry = Arc::new(InMemoryTaskRegistry::default());
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();

        // No file_store wired — should fail.
        let context = ToolContext::with_storage_store(session_id, storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_task_registry(task_registry);

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            panic!("spawn_background should reject missing file store");
        };
        assert!(message.contains("Session file store not available"));
    }

    #[tokio::test]
    async fn test_spawn_background_caps_output_log_size() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let task_registry = Arc::new(InMemoryTaskRegistry::default());
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestLargeOutputBackgroundTool)
            .build();

        let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_task_registry(task_registry.clone());

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background_large_output",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should succeed");
        };
        let run_id = value["run_id"].as_str().unwrap().to_string();
        let task_id = value["task_id"].as_str().unwrap().to_string();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                    && task.state == everruns_core::session_task::SessionTaskState::Succeeded
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background run should complete");
        let _ = run_id;

        let log_content = file_store
            .read_file(session_id, &format!("/.background/{run_id}/output.log"))
            .await
            .unwrap()
            .expect("log file")
            .content
            .unwrap_or_default();

        assert!(log_content.contains("[system] background output truncated"));
        assert!(log_content.chars().count() <= MAX_BACKGROUND_OUTPUT_LOG_CHARS + 128);
    }

    #[tokio::test]
    async fn test_spawn_background_can_create_scheduled_monitor() {
        let session_id = SessionId::new();
        let schedule_store = Arc::new(TestScheduleStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();

        let context = ToolContext::with_storage_store(session_id, storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_schedule_store(schedule_store.clone());

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "title": "Watch PR 1319",
                    "args": { "summary": "Background complete" },
                    "schedule": {
                        "cron_expression": "*/10 * * * *",
                        "timezone": "America/Chicago"
                    }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should create a schedule: {result:?}");
        };

        assert_eq!(value["status"], "scheduled");
        assert_eq!(value["title"], "Watch PR 1319");
        assert_eq!(value["cron_expression"], "*/10 * * * *");
        assert_eq!(value["timezone"], "America/Chicago");

        let schedules = schedule_store.list_schedules(session_id).await.unwrap();
        assert_eq!(schedules.len(), 1);
        assert_eq!(
            schedules[0].cron_expression.as_deref(),
            Some("*/10 * * * *")
        );
        assert!(schedules[0].description.contains("Monitor: Watch PR 1319"));
        assert!(
            schedules[0]
                .description
                .contains("\"summary\": \"Background complete\"")
        );
    }

    #[tokio::test]
    async fn test_spawn_background_rejects_invalid_scheduled_at() {
        let session_id = SessionId::new();
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();
        let context = ToolContext::with_storage_store(session_id, storage_store)
            .with_tool_registry(Arc::new(tool_registry));

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": {},
                    "schedule": {
                        "scheduled_at": "tomorrow at noon"
                    }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            panic!("spawn_background should reject invalid scheduled_at");
        };
        assert!(message.contains("scheduled_at must be RFC3339"));
    }

    #[tokio::test]
    async fn test_spawn_background_rejects_ambiguous_schedule_shape() {
        let session_id = SessionId::new();
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();
        let context = ToolContext::with_storage_store(session_id, storage_store)
            .with_tool_registry(Arc::new(tool_registry));

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": {},
                    "schedule": {
                        "cron_expression": "*/10 * * * *",
                        "scheduled_at": "2026-04-16T15:30:00Z"
                    }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            panic!("spawn_background should reject ambiguous schedule shape");
        };
        assert!(message.contains("must not include both cron_expression and scheduled_at"));
    }

    /// End-to-end cancel test: spawn a long-sleeping background tool, then call
    /// `request_cancel` on the task registry, and assert the task ends Canceled.
    #[tokio::test]
    async fn test_cancel_background_run_via_task_registry() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let task_registry = Arc::new(InMemoryTaskRegistry::default());

        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(SleepingBackgroundTool)
            .build();

        let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_task_registry(task_registry.clone());

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background_sleeping",
                    "args": {},
                    "signal_on_completion": false
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should succeed");
        };
        let run_id = value["run_id"].as_str().unwrap().to_string();
        let task_id = value["task_id"].as_str().unwrap().to_string();

        // Wait until the background task is Running (heartbeat loop started).
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                // Wait for a heartbeat to confirm the watcher is live.
                if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                    && task.heartbeat_at.is_some()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("background run should start and send at least one heartbeat");

        // Request cancel.
        task_registry
            .request_cancel(session_id, &task_id)
            .await
            .expect("request_cancel should succeed");

        // Wait for the task to reach Canceled state.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                    && task.state == everruns_core::session_task::SessionTaskState::Canceled
                {
                    break task;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("background task should reach Canceled state");

        // Verify result.json and output.log were written.
        let result_file = file_store
            .read_file(session_id, &format!("/.background/{run_id}/result.json"))
            .await
            .unwrap()
            .expect("result.json should exist");
        let result_json: Value =
            serde_json::from_str(result_file.content.as_deref().unwrap_or_default())
                .expect("valid json");
        assert_eq!(result_json["status"], "canceled");

        let log_file = file_store
            .read_file(session_id, &format!("/.background/{run_id}/output.log"))
            .await
            .unwrap()
            .expect("output.log should exist");
        assert!(
            log_file
                .content
                .as_deref()
                .unwrap_or_default()
                .contains("Canceled by request.")
        );
    }

    #[tokio::test]
    async fn reattach_fails_with_missing_file_store() {
        let session_id = SessionId::new();
        // Context with no file_store — only session_task_registry is wired.
        let task_registry = Arc::new(InMemoryTaskRegistry::default());
        let context = everruns_core::tool_context::ToolContext::new(session_id)
            .with_session_task_registry(task_registry);
        let task = make_reattach_task(serde_json::json!({
            "tool": "get_current_time",
            "arguments": {},
            "reattachable": true,
            "signal_on_completion": true,
        }));
        let err = reattach_background_run(&task, &context)
            .await
            .expect_err("should fail without file store");
        assert!(
            err.to_string().contains("file store"),
            "error should mention file store, got: {err}"
        );
    }

    #[tokio::test]
    async fn reattach_fails_with_missing_task_registry() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        // Context has a file_store but no session_task_registry.
        let context = everruns_core::tool_context::ToolContext::with_stores(
            session_id,
            file_store,
            storage_store,
        );
        let task = make_reattach_task(serde_json::json!({
            "tool": "get_current_time",
            "arguments": {},
            "reattachable": true,
            "signal_on_completion": true,
        }));
        let err = reattach_background_run(&task, &context)
            .await
            .expect_err("should fail without task registry");
        assert!(
            err.to_string().contains("task registry"),
            "error should mention task registry, got: {err}"
        );
    }

    #[tokio::test]
    async fn reattach_fails_with_unknown_tool_name() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let task_registry = Arc::new(InMemoryTaskRegistry::default());
        let context = everruns_core::tool_context::ToolContext::with_stores(
            session_id,
            file_store,
            storage_store,
        )
        .with_session_task_registry(task_registry);
        // "test_background" is not in ToolRegistry::with_defaults().
        let task = make_reattach_task(serde_json::json!({
            "tool": "test_background",
            "arguments": {},
            "reattachable": true,
            "signal_on_completion": true,
        }));
        let err = reattach_background_run(&task, &context)
            .await
            .expect_err("should fail for unknown tool");
        assert!(
            err.to_string().contains("not found in built-in registry"),
            "error should mention built-in registry, got: {err}"
        );
    }

    #[tokio::test]
    async fn reattach_fails_with_missing_tool_spec_field() {
        let session_id = SessionId::new();
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let task_registry = Arc::new(InMemoryTaskRegistry::default());
        let context = everruns_core::tool_context::ToolContext::with_stores(
            session_id,
            file_store,
            storage_store,
        )
        .with_session_task_registry(task_registry);
        // Spec has no "tool" field.
        let task = make_reattach_task(serde_json::json!({ "reattachable": true }));
        let err = reattach_background_run(&task, &context)
            .await
            .expect_err("should fail with missing tool field");
        assert!(
            err.to_string().contains("missing 'tool' field"),
            "error should mention missing tool field, got: {err}"
        );
    }

    #[test]
    fn test_is_canceled_outcome_detects_sentinel() {
        // The sentinel produced by the cancel-watcher branch.
        let sentinel: std::result::Result<BackgroundOutcome, ToolExecutionResult> = Err(
            ToolExecutionResult::ToolError(BACKGROUND_CANCEL_SENTINEL.to_string()),
        );
        assert!(is_canceled_outcome(&sentinel));
    }

    #[test]
    fn test_is_canceled_outcome_does_not_match_other_errors() {
        let other_err: std::result::Result<BackgroundOutcome, ToolExecutionResult> =
            Err(ToolExecutionResult::ToolError("boom".to_string()));
        assert!(!is_canceled_outcome(&other_err));

        let success: std::result::Result<BackgroundOutcome, ToolExecutionResult> =
            Ok(BackgroundOutcome {
                summary: "ok".to_string(),
                result: json!({"ok": true}),
                raw_output: None,
            });
        assert!(!is_canceled_outcome(&success));
    }

    #[derive(Default)]
    struct TestBackgroundTool;

    #[async_trait]
    impl BackgroundExecutableTool for TestBackgroundTool {
        async fn execute_background(
            &self,
            arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            sink.status("Waiting for test result")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            sink.output("stdout", "hello from background")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            sink.progress(BackgroundProgress {
                current: Some(1),
                total: Some(1),
                unit: Some("step".to_string()),
                label: Some("done".to_string()),
            })
            .await
            .map_err(ToolExecutionResult::internal_error)?;

            Ok(BackgroundOutcome {
                summary: arguments["summary"].as_str().unwrap_or("done").to_string(),
                result: json!({"ok": true}),
                raw_output: None,
            })
        }
    }

    #[async_trait]
    impl Tool for TestBackgroundTool {
        fn name(&self) -> &str {
            "test_background"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background")
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string" }
                }
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    #[derive(Default)]
    struct NoopStorageStore;

    #[async_trait]
    impl everruns_core::session_services::SessionStorageStore for NoopStorageStore {
        async fn set_value(
            &self,
            _session_id: SessionId,
            _key: &str,
            _value: &str,
        ) -> everruns_core::Result<()> {
            Ok(())
        }
        async fn get_value(
            &self,
            _session_id: SessionId,
            _key: &str,
        ) -> everruns_core::Result<Option<String>> {
            Ok(None)
        }
        async fn delete_value(
            &self,
            _session_id: SessionId,
            _key: &str,
        ) -> everruns_core::Result<bool> {
            Ok(false)
        }
        async fn list_keys(&self, _session_id: SessionId) -> everruns_core::Result<Vec<KeyInfo>> {
            Ok(Vec::new())
        }
        async fn set_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
            _value: &str,
        ) -> everruns_core::Result<()> {
            Ok(())
        }
        async fn get_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
        ) -> everruns_core::Result<Option<String>> {
            Ok(None)
        }
        async fn delete_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
        ) -> everruns_core::Result<bool> {
            Ok(false)
        }
        async fn list_secrets(
            &self,
            _session_id: SessionId,
        ) -> everruns_core::Result<Vec<SecretInfo>> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct TestFileStore {
        files: Mutex<HashMap<String, SessionFile>>,
    }

    #[async_trait]
    impl everruns_core::session_files::SessionFileSystem for TestFileStore {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            _session_id: SessionId,
            path: &str,
        ) -> everruns_core::Result<Option<SessionFile>> {
            Ok(self.files.lock().unwrap().get(path).cloned())
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> everruns_core::Result<SessionFile> {
            let now = chrono::Utc::now();
            let file = SessionFile {
                id: uuid::Uuid::now_v7(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: now,
                updated_at: now,
            };
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), file.clone());
            Ok(file)
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _recursive: bool,
        ) -> everruns_core::Result<bool> {
            Ok(false)
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> everruns_core::Result<Vec<FileInfo>> {
            Ok(Vec::new())
        }

        async fn stat_file(
            &self,
            _session_id: SessionId,
            path: &str,
        ) -> everruns_core::Result<Option<FileStat>> {
            let file = self.files.lock().unwrap().get(path).cloned();
            Ok(file.map(|entry| FileStat {
                path: entry.path,
                name: entry.name,
                is_directory: entry.is_directory,
                is_readonly: entry.is_readonly,
                size_bytes: entry.size_bytes,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
            }))
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> everruns_core::Result<Vec<everruns_core::session_file::GrepMatch>> {
            Ok(Vec::new())
        }

        async fn create_directory(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> everruns_core::Result<FileInfo> {
            let now = chrono::Utc::now();
            let id = uuid::Uuid::now_v7();
            let dir = SessionFile {
                id,
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                content: None,
                encoding: "text".to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: now,
                updated_at: now,
            };
            self.files.lock().unwrap().insert(path.to_string(), dir);
            Ok(FileInfo {
                id,
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: now,
                updated_at: now,
            })
        }
    }

    #[derive(Default)]
    struct TestSubagentDelegate {
        sent_messages: Mutex<Vec<String>>,
    }

    #[async_trait]
    #[async_trait]
    impl SubagentSessionDelegate for TestSubagentDelegate {
        async fn get_agent_by_id(
            &self,
            _id: everruns_core::typed_id::AgentId,
        ) -> everruns_core::Result<Option<everruns_core::AgentDefinition>> {
            Ok(None)
        }
        async fn add_agent_session_participant(
            &self,
            _session_id: SessionId,
            _agent_id: everruns_core::typed_id::AgentId,
        ) -> everruns_core::Result<everruns_core::typed_id::SessionParticipantId> {
            Err(everruns_core::error::AgentLoopError::tool(
                "test store does not add participants",
            ))
        }
        async fn get_harness(
            &self,
            _id: HarnessId,
        ) -> everruns_core::Result<Option<everruns_core::HarnessDefinition>> {
            Ok(None)
        }
        async fn create_session_with_options(
            &self,
            _request: everruns_core::subagent_delegation::PlatformCreateSessionRequest,
        ) -> everruns_core::Result<everruns_core::ExecutionSession> {
            Err(everruns_core::error::AgentLoopError::tool(
                "test store does not create sessions",
            ))
        }
        async fn get_session_by_id(
            &self,
            _id: SessionId,
        ) -> everruns_core::Result<Option<everruns_core::ExecutionSession>> {
            Ok(None)
        }
        async fn send_message(
            &self,
            _session_id: SessionId,
            content: &str,
        ) -> everruns_core::Result<()> {
            self.sent_messages.lock().unwrap().push(content.to_string());
            Ok(())
        }
        async fn get_messages(
            &self,
            _session_id: SessionId,
            _limit: Option<usize>,
        ) -> everruns_core::Result<Vec<everruns_core::subagent_delegation::PlatformMessage>>
        {
            Ok(Vec::new())
        }
        async fn wait_for_idle(
            &self,
            _session_id: SessionId,
            _timeout_secs: Option<u64>,
        ) -> everruns_core::Result<String> {
            Ok("idle".to_string())
        }
    }

    #[derive(Default)]
    struct InMemoryTaskRegistry {
        tasks: Mutex<HashMap<String, everruns_core::session_task::SessionTask>>,
    }

    #[async_trait]
    impl everruns_core::session_task::SessionTaskRegistry for InMemoryTaskRegistry {
        async fn create(
            &self,
            input: everruns_core::session_task::CreateSessionTask,
        ) -> everruns_core::Result<everruns_core::session_task::SessionTask> {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(id) = &input.id
                && let Some(existing) = tasks.get(id)
            {
                return Ok(existing.clone());
            }
            let task = everruns_core::session_task::new_session_task(input, chrono::Utc::now());
            tasks.insert(task.id.clone(), task.clone());
            Ok(task)
        }

        async fn update(
            &self,
            _session_id: SessionId,
            task_id: &str,
            update: everruns_core::session_task::SessionTaskUpdate,
        ) -> everruns_core::Result<Option<everruns_core::session_task::SessionTask>> {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get_mut(task_id) else {
                return Ok(None);
            };
            everruns_core::session_task::apply_task_update(task, update, chrono::Utc::now());
            Ok(Some(task.clone()))
        }

        async fn get(
            &self,
            _session_id: SessionId,
            task_id: &str,
        ) -> everruns_core::Result<Option<everruns_core::session_task::SessionTask>> {
            Ok(self.tasks.lock().unwrap().get(task_id).cloned())
        }

        async fn list(
            &self,
            _session_id: SessionId,
            filter: Option<&everruns_core::session_task::SessionTaskFilter>,
        ) -> everruns_core::Result<Vec<everruns_core::session_task::SessionTask>> {
            let tasks = self.tasks.lock().unwrap();
            Ok(tasks
                .values()
                .filter(|task| {
                    filter.is_none_or(|f| {
                        f.kind.as_deref().is_none_or(|kind| task.kind == kind)
                            && f.state.is_none_or(|state| task.state == state)
                    })
                })
                .cloned()
                .collect())
        }

        async fn request_cancel(
            &self,
            _session_id: SessionId,
            task_id: &str,
        ) -> everruns_core::Result<Option<everruns_core::session_task::SessionTask>> {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get_mut(task_id) else {
                return Ok(None);
            };
            task.cancel_requested_at
                .get_or_insert_with(chrono::Utc::now);
            task.updated_at = chrono::Utc::now();
            Ok(Some(task.clone()))
        }

        async fn record_message(
            &self,
            _session_id: SessionId,
            task_id: &str,
            message: everruns_core::session_task::NewTaskMessage,
        ) -> everruns_core::Result<everruns_core::session_task::TaskMessage> {
            let tasks = self.tasks.lock().unwrap();
            let _task = tasks
                .get(task_id)
                .ok_or_else(|| everruns_core::AgentLoopError::tool(format!("no task {task_id}")))?;
            Ok(everruns_core::session_task::TaskMessage {
                id: everruns_core::session_task::generate_task_message_id(),
                task_id: task_id.to_string(),
                direction: message.direction,
                content: message.content,
                in_reply_to: message.in_reply_to,
                created_at: chrono::Utc::now(),
            })
        }

        async fn list_messages(
            &self,
            _session_id: SessionId,
            _task_id: &str,
            _limit: Option<u32>,
            _after_id: Option<&str>,
        ) -> everruns_core::Result<Vec<everruns_core::session_task::TaskMessage>> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct TestScheduleStore {
        schedules: Mutex<Vec<everruns_core::session_schedule::SessionSchedule>>,
    }

    #[async_trait]
    impl everruns_core::session_services::SessionScheduleStore for TestScheduleStore {
        async fn create_schedule(
            &self,
            session_id: SessionId,
            description: String,
            cron_expression: Option<String>,
            scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
            timezone: String,
        ) -> everruns_core::Result<everruns_core::session_schedule::SessionSchedule> {
            let schedule = everruns_core::session_schedule::SessionSchedule {
                id: everruns_core::typed_id::ScheduleId::new(),
                session_id,
                owner_principal_id: everruns_core::PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                owner: None,
                effective_owner: None,
                description,
                cron_expression: cron_expression.clone(),
                scheduled_at,
                timezone,
                enabled: true,
                schedule_type: everruns_core::session_schedule::SessionSchedule::derive_type(
                    &cron_expression,
                ),
                next_trigger_at: Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
                last_triggered_at: None,
                trigger_count: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            self.schedules.lock().unwrap().push(schedule.clone());
            Ok(schedule)
        }

        async fn cancel_schedule(
            &self,
            _session_id: SessionId,
            schedule_id: everruns_core::ScheduleId,
        ) -> everruns_core::Result<everruns_core::session_schedule::SessionSchedule> {
            let mut schedules = self.schedules.lock().unwrap();
            let schedule = schedules
                .iter_mut()
                .find(|schedule| schedule.id == schedule_id)
                .ok_or_else(|| {
                    everruns_core::AgentLoopError::tool("Schedule not found".to_string())
                })?;
            schedule.enabled = false;
            Ok(schedule.clone())
        }

        async fn list_schedules(
            &self,
            session_id: SessionId,
        ) -> everruns_core::Result<Vec<everruns_core::session_schedule::SessionSchedule>> {
            Ok(self
                .schedules
                .lock()
                .unwrap()
                .iter()
                .filter(|schedule| schedule.session_id == session_id)
                .cloned()
                .collect())
        }

        async fn count_active_schedules(
            &self,
            session_id: SessionId,
        ) -> everruns_core::Result<u32> {
            Ok(self
                .schedules
                .lock()
                .unwrap()
                .iter()
                .filter(|schedule| schedule.session_id == session_id && schedule.enabled)
                .count() as u32)
        }

        async fn count_active_org_schedules(&self) -> everruns_core::Result<u32> {
            // Test store is single-org; count all enabled schedules.
            Ok(self
                .schedules
                .lock()
                .unwrap()
                .iter()
                .filter(|schedule| schedule.enabled)
                .count() as u32)
        }
    }

    fn make_reattach_task(spec: serde_json::Value) -> everruns_core::session_task::SessionTask {
        use everruns_core::session_task::{SessionTaskState, TaskLinks, TaskWakePolicy};
        everruns_core::session_task::SessionTask {
            id: "t-reattach".to_string(),
            session_id: SessionId::new(),
            root_session_id: None,
            kind: everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL.to_string(),
            display_name: "Reattach test".to_string(),
            spec,
            state: SessionTaskState::Running,
            state_detail: None,
            progress: None,
            input_request: None,
            cancel_requested_at: None,
            summary: None,
            result_path: None,
            artifacts: vec![],
            error: None,
            attempt: 2,
            worker_id: None,
            heartbeat_at: None,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
            created_at: chrono::Utc::now(),
            started_at: None,
            finished_at: None,
            updated_at: chrono::Utc::now(),
        }
    }
}
