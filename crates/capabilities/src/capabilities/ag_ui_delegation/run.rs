//! One AG-UI delegation run: its persisted record, the streaming loop, and
//! the session-task executor behind `message_task` / `cancel_task`.

use super::{AG_UI_TARGET_TYPE, AgUiAgentConfig};
use crate::capabilities::SpawnMode;
use crate::capabilities::delegation_result::{schema_validation_errors, write_task_result_value};
use async_trait::async_trait;
use everruns_ag_ui::client::AgUiClient;
use everruns_ag_ui::consumer::{RunOutcome, RunResult, merge_usage};
use everruns_ag_ui::{Interrupt, Message, ResumeBuilder, ResumeEntry, RunAgentInput, TokenUsage};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::url_validation::validate_url_dns_pinned;
use everruns_core::network_access::NetworkAccessList;
use everruns_core::session_task::{
    SessionTask, SessionTaskState, SessionTaskUpdate, TASK_KIND_EXTERNAL_AG_UI, TaskError,
    TaskExecutor, TaskExecutorPlugin, TaskInputRequest, TaskLinks, TaskMessage, task_message_text,
};
use everruns_core::tool_context::ToolContext;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::{Instant, interval, sleep_until};

use crate::capabilities::AGENT_RUN_KEY_PREFIX;

/// Bounds on what a remote agent can make us store (TM-AGENT-031).
const MAX_RESULT_CHARS: usize = 8_192;
const MAX_HISTORY_MESSAGES: usize = 40;
const MAX_INTERRUPTS: usize = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the streaming loop checks for a cancel request.
const CANCEL_POLL: Duration = Duration::from_secs(1);
/// Heartbeat every this many cancel polls.
const HEARTBEAT_EVERY: u32 = 10;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum AgUiRunStatus {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Failed,
    Canceled,
}

impl AgUiRunStatus {
    pub(super) fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }

    fn task_state(self) -> SessionTaskState {
        match self {
            Self::Submitted => SessionTaskState::Queued,
            Self::Working => SessionTaskState::Running,
            Self::InputRequired => SessionTaskState::AwaitingInput,
            Self::Completed => SessionTaskState::Succeeded,
            Self::Failed => SessionTaskState::Failed,
            Self::Canceled => SessionTaskState::Canceled,
        }
    }
}

impl std::fmt::Display for AgUiRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Submitted => "submitted",
            Self::Working => "working",
            Self::InputRequired => "input_required",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        })
    }
}

/// Persisted state of one delegation, across the runs of its thread (the
/// first run and each run that resumes an interrupt).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AgUiRunRecord {
    pub(super) run_id: String,
    kind: String,
    external_agent_id: String,
    external_agent_name: String,
    instructions: String,
    mode: SpawnMode,
    pub(super) status: AgUiRunStatus,
    /// The AG-UI thread every run of this delegation shares.
    thread_id: String,
    /// The last remote run, sent as `parentRunId` by the run that follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_run_id: Option<String>,
    /// Conversation sent with each run; AG-UI agents are stateless per
    /// request. Bounded to the last `MAX_HISTORY_MESSAGES`.
    #[serde(default)]
    history: Vec<Message>,
    /// Interrupts the last run left open.
    #[serde(default)]
    pub(super) interrupts: Vec<Interrupt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    structured_result: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    usage: Vec<TokenUsage>,
    wake_on_completion: bool,
    /// Stream deadline of each run, so a resume gets the one the spawn asked for.
    #[serde(default = "default_timeout_secs")]
    pub(super) timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) task_id: Option<String>,
    /// Config snapshot so the executor can resume without the capability
    /// config. Holds the secret's name, never its value.
    agent_config: AgUiAgentConfig,
    /// Merged network policy captured at spawn time, restored for resumes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) network_access: Option<NetworkAccessList>,
}

impl AgUiRunRecord {
    pub(super) fn new(
        agent: &AgUiAgentConfig,
        instructions: String,
        mode: SpawnMode,
        wake_on_completion: bool,
        result_schema: Option<Value>,
    ) -> Self {
        Self {
            run_id: format!("agrun_{}", uuid::Uuid::now_v7().simple()),
            kind: AG_UI_TARGET_TYPE.to_string(),
            external_agent_id: agent.id.clone(),
            external_agent_name: agent.name.clone(),
            instructions,
            mode,
            status: AgUiRunStatus::Submitted,
            thread_id: format!("thread_{}", uuid::Uuid::now_v7().simple()),
            remote_run_id: None,
            history: Vec::new(),
            interrupts: Vec::new(),
            result: None,
            result_path: None,
            error: None,
            error_kind: None,
            result_schema,
            structured_result: None,
            usage: Vec::new(),
            wake_on_completion,
            timeout_secs: super::DEFAULT_WAIT_TIMEOUT_SECS,
            task_id: None,
            agent_config: agent.clone(),
            network_access: None,
        }
    }

    pub(super) fn public_json(&self) -> Value {
        json!({
            "agent_run_id": self.run_id,
            "kind": self.kind,
            "external_agent_id": self.external_agent_id,
            "external_agent_name": self.external_agent_name,
            "instructions": self.instructions,
            "mode": self.mode,
            "status": self.status,
            "thread_id": self.thread_id,
            "remote_run_id": self.remote_run_id,
            "interrupts": self.interrupts.iter().map(|i| json!({
                "id": i.id,
                "reason": i.reason,
                "message": i.message,
            })).collect::<Vec<_>>(),
            "result": self.result,
            "result_path": self.result_path,
            "error": self.error,
            "wake_on_completion": self.wake_on_completion,
            "task_id": self.task_id,
        })
    }

    fn fail(&mut self, kind: &str, message: impl Into<String>) {
        self.status = AgUiRunStatus::Failed;
        self.error_kind = Some(kind.to_string());
        self.error = Some(truncate(message.into()));
    }

    /// The prompt shown on the task while it awaits input.
    fn input_prompt(&self) -> String {
        let asks = self
            .interrupts
            .iter()
            .map(|i| match &i.message {
                Some(message) => format!("[{}] {message}", i.id),
                None => format!("[{}] {}", i.id, i.reason),
            })
            .collect::<Vec<_>>();
        if asks.is_empty() {
            "External AG-UI agent requires additional input".to_string()
        } else {
            truncate(format!("External AG-UI agent asks:\n{}", asks.join("\n")))
        }
    }
}

fn default_timeout_secs() -> u64 {
    super::DEFAULT_WAIT_TIMEOUT_SECS
}

pub(super) fn message_id() -> String {
    format!("msg_{}", uuid::Uuid::now_v7().simple())
}

fn truncate(value: String) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(MAX_RESULT_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}\n[truncated]")
    } else {
        truncated
    }
}

fn run_key(run_id: &str) -> String {
    format!("{AGENT_RUN_KEY_PREFIX}{run_id}")
}

pub(super) async fn save_run(context: &ToolContext, record: &AgUiRunRecord) -> Result<()> {
    mirror_run_to_task(context, record).await;
    let Some(storage) = &context.storage_store else {
        return Ok(());
    };
    let serialized = serde_json::to_string(record)
        .map_err(|e| AgentLoopError::store(format!("failed to serialize agent run: {e}")))?;
    storage
        .set_value(context.session_id, &run_key(&record.run_id), &serialized)
        .await
}

pub(super) async fn load_run(
    context: &ToolContext,
    run_id: &str,
) -> std::result::Result<AgUiRunRecord, String> {
    let Some(storage) = &context.storage_store else {
        return Err("AG-UI delegation requires storage_store context".to_string());
    };
    let serialized = storage
        .get_value(context.session_id, &run_key(run_id))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No agent run found with id: {run_id}"))?;
    serde_json::from_str(&serialized).map_err(|e| format!("Invalid agent run record: {e}"))
}

async fn load_run_for_task(
    context: &ToolContext,
    task: &SessionTask,
) -> std::result::Result<AgUiRunRecord, String> {
    let run_id = task
        .spec
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("task {} has no run_id", task.id))?;
    load_run(context, run_id).await
}

/// Mirror the record onto its session task (best effort).
async fn mirror_run_to_task(context: &ToolContext, record: &AgUiRunRecord) {
    let (Some(registry), Some(task_id)) = (&context.session_task_registry, &record.task_id) else {
        return;
    };
    let state = record.status.task_state();
    let input_request = (state == SessionTaskState::AwaitingInput).then(|| TaskInputRequest {
        id: format!("inreq_{}", uuid::Uuid::now_v7().simple()),
        prompt: record.input_prompt(),
        expected: Some(json!({
            "interrupts": record.interrupts.iter().map(|i| json!({
                "id": i.id,
                "reason": i.reason,
                "response_schema": i.response_schema,
            })).collect::<Vec<_>>(),
        })),
    });
    let error = (record.status == AgUiRunStatus::Failed).then(|| TaskError {
        kind: record
            .error_kind
            .clone()
            .unwrap_or_else(|| "remote_failed".to_string()),
        message: record
            .error
            .clone()
            .unwrap_or_else(|| "External AG-UI agent run failed".to_string()),
    });
    let _ = registry
        .update(
            context.session_id,
            task_id,
            SessionTaskUpdate {
                state: Some(state),
                input_request,
                summary: record.result.clone(),
                result_path: record.result_path.clone(),
                error,
                links: record.remote_run_id.clone().map(|remote| TaskLinks {
                    remote_task_id: Some(remote),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await;
}

/// Adds the first user message and returns the first run's input.
pub(super) fn first_input(record: &mut AgUiRunRecord, message: Message) -> RunAgentInput {
    record.history.push(message);
    next_input(record, Vec::new())
}

fn next_input(record: &AgUiRunRecord, resume: Vec<ResumeEntry>) -> RunAgentInput {
    RunAgentInput {
        thread_id: record.thread_id.clone(),
        run_id: format!("run_{}", uuid::Uuid::now_v7().simple()),
        parent_run_id: record.remote_run_id.clone(),
        messages: record.history.clone(),
        resume,
        ..RunAgentInput::default()
    }
    .with_protocol_version()
}

/// The resume list a `message_task` answer stands for. A JSON object keyed
/// by open interrupt ids answers each listed one and abandons the rest;
/// anything else (JSON or plain text) resolves every open interrupt with
/// that payload.
pub(super) fn resume_from_answer(
    interrupts: &[Interrupt],
    answer: &str,
) -> std::result::Result<Vec<ResumeEntry>, String> {
    let parsed = serde_json::from_str::<Value>(answer).ok();
    let mut resume = ResumeBuilder::new(interrupts);
    match &parsed {
        Some(Value::Object(map))
            if !map.is_empty()
                && map
                    .keys()
                    .all(|key| interrupts.iter().any(|open| &open.id == key)) =>
        {
            for (id, payload) in map {
                resume
                    .resolve(id, payload.clone())
                    .map_err(|e| e.to_string())?;
            }
            resume.cancel_remaining();
        }
        _ => {
            let payload = parsed.unwrap_or_else(|| Value::String(answer.to_string()));
            for open in interrupts {
                resume
                    .resolve(&open.id, payload.clone())
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    resume.build().map_err(|e| e.to_string())
}

/// Builds the client for one request. Runs before every request, including
/// resumes, so a policy or secret change applies to the next run.
async fn build_client(
    agent: &AgUiAgentConfig,
    context: &ToolContext,
) -> std::result::Result<AgUiClient, String> {
    // THREAT[TM-AGENT-030]: the session's merged network ACL gates every
    // request, and private/metadata addresses are refused with the resolved
    // address pinned, so DNS cannot be rebound between check and connect.
    if let Some(acl) = &context.network_access
        && !acl.is_url_allowed(&agent.url)
    {
        return Err(format!(
            "AG-UI endpoint blocked by network access policy: {}",
            agent.url
        ));
    }
    agent.validate()?;
    // THREAT[TM-AGENT-030]: no redirects, so a 30x cannot move the request
    // (and its bearer token) to a host the checks above never saw.
    let mut http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT);
    if !agent.allow_local_urls {
        let (url, addrs) = validate_url_dns_pinned(&agent.url)
            .await
            .map_err(|e| format!("AG-UI agent {} has unsafe url: {e}", agent.id))?;
        // An IP literal comes back with no addresses: the static check
        // already validated it and there is nothing to pin.
        if let (Some(host), false) = (url.host_str(), addrs.is_empty()) {
            http = http.resolve_to_addrs(host, &addrs);
        }
    }
    let http = http
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
    let mut client = AgUiClient::new(agent.url.clone()).with_http_client(http);
    for (name, value) in &agent.headers {
        client = client
            .with_header(name, value)
            .map_err(|e| format!("AG-UI agent {} header {name}: {e}", agent.id))?;
    }
    // THREAT[TM-AGENT-032]: the token is read from the session secret store
    // per request and handed to the client as a sensitive header; it is never
    // written to the run record, task, or tool result.
    if let Some(secret) = &agent.bearer_token_secret {
        let storage = context
            .storage_store
            .as_ref()
            .ok_or("AG-UI delegation requires storage_store context")?;
        let token = storage
            .get_secret(context.session_id, secret)
            .await
            .map_err(|e| format!("Failed to read secret {secret}: {e}"))?
            .ok_or_else(|| {
                format!(
                    "AG-UI agent {} needs session secret {secret}, which is not set",
                    agent.id
                )
            })?;
        client = client.with_bearer_token(token);
    }
    Ok(client)
}

/// Fold a finished stream into the record.
fn apply_result(record: &mut AgUiRunRecord, result: RunResult) {
    // THREAT[TM-AGENT-031]: remote text is untrusted; it is stored bounded
    // and reaches the model only as a tool result.
    record.remote_run_id = result.run_id.clone().or(record.remote_run_id.take());
    merge_usage(&mut record.usage, &result.usage);
    for message in result
        .messages
        .iter()
        .filter(|m| m.subagent_run_id.is_none() && !m.content.is_empty())
    {
        record.history.push(Message::assistant(
            message.id.clone(),
            truncate(message.content.clone()),
        ));
    }
    let excess = record.history.len().saturating_sub(MAX_HISTORY_MESSAGES);
    record.history.drain(..excess);
    let text = result.text();
    record.result = (!text.is_empty()).then(|| truncate(text));
    record.interrupts.clear();
    match result.outcome {
        RunOutcome::Success { .. } => {
            record.status = AgUiRunStatus::Completed;
            record.structured_result = result.result;
        }
        RunOutcome::Interrupted => {
            record.status = AgUiRunStatus::InputRequired;
            record.interrupts = result
                .interrupts
                .into_iter()
                .take(MAX_INTERRUPTS)
                .map(|mut interrupt| {
                    interrupt.message = interrupt.message.map(truncate);
                    interrupt
                })
                .collect();
        }
        RunOutcome::Cancelled => record.status = AgUiRunStatus::Canceled,
        RunOutcome::Failed { message, code } => {
            let message = match code {
                Some(code) => format!("{message} ({code})"),
                None => message,
            };
            record.fail("remote_failed", message);
        }
        RunOutcome::Pending => record.fail("protocol", "the stream ended without an outcome"),
    }
}

async fn write_result_artifact(context: &ToolContext, record: &mut AgUiRunRecord) -> Result<()> {
    if record.status != AgUiRunStatus::Completed {
        return Ok(());
    }
    if let Some(schema) = record.result_schema.clone() {
        let Some(value) = record.structured_result.clone() else {
            record.fail(
                "no_result",
                "External AG-UI agent finished without a RUN_FINISHED result",
            );
            return Ok(());
        };
        let errors = schema_validation_errors(&schema, &value);
        if !errors.is_empty() {
            record.fail(
                "schema_mismatch",
                format!(
                    "External AG-UI agent result did not match result_schema: {}",
                    errors.join("; ")
                ),
            );
            return Ok(());
        }
        let Some(task_id) = record.task_id.clone() else {
            record.fail(
                "result_write_failed",
                "Structured result has no local session task",
            );
            return Ok(());
        };
        match write_task_result_value(context, &task_id, &value).await? {
            Some(path) => {
                record.result_path = Some(path);
                record.result = Some(truncate(value.to_string()));
            }
            None => record.fail(
                "result_write_failed",
                "Structured result could not be persisted",
            ),
        }
        return Ok(());
    }
    // Runtime-owned record: written through the artifact store so a read-only
    // model-facing workspace policy does not deny it.
    let Some(file_store) = context.runtime_artifact_file_store() else {
        return Ok(());
    };
    let dir = format!("/.agent-runs/{}", record.run_id);
    let path = format!("{dir}/result.json");
    let _ = file_store
        .create_directory(context.session_id, "/.agent-runs")
        .await;
    let _ = file_store.create_directory(context.session_id, &dir).await;
    record.result_path = Some(path.clone());
    let body = serde_json::to_string_pretty(&record.public_json())
        .unwrap_or_else(|_| record.public_json().to_string());
    file_store
        .write_file(context.session_id, &path, &body, "utf-8")
        .await?;
    Ok(())
}

/// Live streams in this process, so `cancel_task` can close one directly.
/// A stream in another worker notices the task's `cancel_requested_at`
/// within `CANCEL_POLL` instead.
static ACTIVE_STREAMS: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(Default::default);

struct ActiveStream(String);

impl ActiveStream {
    fn register(run_id: &str) -> (Self, watch::Receiver<bool>) {
        let (tx, rx) = watch::channel(false);
        if let Ok(mut active) = ACTIVE_STREAMS.lock() {
            active.insert(run_id.to_string(), tx);
        }
        (Self(run_id.to_string()), rx)
    }

    fn signal(run_id: &str) -> bool {
        ACTIVE_STREAMS
            .lock()
            .ok()
            .and_then(|active| active.get(run_id).map(|tx| tx.send(true).is_ok()))
            .unwrap_or(false)
    }
}

impl Drop for ActiveStream {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_STREAMS.lock() {
            active.remove(&self.0);
        }
    }
}

/// How a streamed run ended for the caller. The record is already saved.
pub(super) enum DriveOutcome {
    Finished(Box<AgUiRunRecord>),
    TimedOut(Box<AgUiRunRecord>),
}

enum StreamEnd {
    Done,
    Canceled,
    TimedOut,
    Failed(String),
}

/// Whether someone asked for the task to stop; heartbeats on the way.
async fn cancel_requested(context: &ToolContext, task_id: Option<&str>, heartbeat: bool) -> bool {
    let (Some(registry), Some(task_id)) = (&context.session_task_registry, task_id) else {
        return false;
    };
    let task = if heartbeat {
        registry
            .update(
                context.session_id,
                task_id,
                SessionTaskUpdate {
                    heartbeat_at: Some(chrono::Utc::now()),
                    ..Default::default()
                },
            )
            .await
    } else {
        registry.get(context.session_id, task_id).await
    };
    matches!(task, Ok(Some(task)) if task.cancel_requested_at.is_some() || task.state.is_terminal())
}

/// Stream one run to its end, a cancel, or the deadline, and persist the
/// outcome. Dropping the stream closes the connection, which is how the
/// remote run is cancelled.
pub(super) async fn drive_run(
    context: &ToolContext,
    agent: &AgUiAgentConfig,
    mut record: AgUiRunRecord,
    input: RunAgentInput,
    timeout_secs: u64,
) -> DriveOutcome {
    let (_active, mut cancel_rx) = ActiveStream::register(&record.run_id);
    record.status = AgUiRunStatus::Working;
    record.interrupts.clear();
    let _ = save_run(context, &record).await;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let end = match build_client(agent, context).await {
        Err(error) => StreamEnd::Failed(error),
        Ok(client) => match tokio::time::timeout_at(deadline, client.run(&input)).await {
            Err(_) => StreamEnd::TimedOut,
            Ok(Err(error)) => StreamEnd::Failed(format!("AG-UI request failed: {error}")),
            Ok(Ok(mut stream)) => {
                let mut ticks = interval(CANCEL_POLL);
                let mut tick_count: u32 = 0;
                let end = loop {
                    tokio::select! {
                        event = stream.next() => match event {
                            None => break StreamEnd::Done,
                            Some(Ok(_)) => {}
                            Some(Err(error)) => {
                                break StreamEnd::Failed(format!("AG-UI stream failed: {error}"));
                            }
                        },
                        _ = cancel_rx.changed() => break StreamEnd::Canceled,
                        _ = ticks.tick() => {
                            tick_count = tick_count.wrapping_add(1);
                            let heartbeat = tick_count % HEARTBEAT_EVERY == 1;
                            if cancel_requested(context, record.task_id.as_deref(), heartbeat).await {
                                break StreamEnd::Canceled;
                            }
                        }
                        _ = sleep_until(deadline) => break StreamEnd::TimedOut,
                    }
                };
                if matches!(end, StreamEnd::Done) {
                    match stream.result().cloned() {
                        Some(result) => apply_result(&mut record, result),
                        None => record.fail("protocol", "the stream ended without a result"),
                    }
                }
                end
            }
        },
    };

    // A cancel or failure written elsewhere while we streamed wins.
    if let Ok(current) = load_run(context, &record.run_id).await
        && current.status.is_terminal()
    {
        return DriveOutcome::Finished(Box::new(current));
    }
    let timed_out = matches!(end, StreamEnd::TimedOut);
    match end {
        StreamEnd::Done => {}
        StreamEnd::Canceled => record.status = AgUiRunStatus::Canceled,
        StreamEnd::TimedOut => record.fail(
            "timeout",
            format!("No AG-UI run outcome within {timeout_secs}s; the stream was closed"),
        ),
        StreamEnd::Failed(error) => record.fail("remote_failed", error),
    }
    if let Err(error) = write_result_artifact(context, &mut record).await {
        record.fail("result_write_failed", error.to_string());
    }
    let _ = save_run(context, &record).await;
    if timed_out {
        DriveOutcome::TimedOut(Box::new(record))
    } else {
        DriveOutcome::Finished(Box::new(record))
    }
}

/// Stream a run in the background. The outcome reaches the parent through
/// the task itself: under the `on_activity` wake policy the transition into
/// `awaiting_input` or a terminal state is the wake, carrying the prompt or
/// the summary. No outbound task message is posted, since under that policy
/// it would wake the parent a second time.
pub(super) fn spawn_background(
    context: ToolContext,
    agent: AgUiAgentConfig,
    record: AgUiRunRecord,
    input: RunAgentInput,
    timeout_secs: u64,
) {
    tokio::spawn(async move {
        drive_run(&context, &agent, record, input, timeout_secs).await;
    });
}

/// Context for work on a stored run: restores the spawn-time network policy.
fn run_context(record: &AgUiRunRecord, context: &ToolContext) -> ToolContext {
    context.clone().with_network_access(
        record
            .network_access
            .clone()
            .or_else(|| context.network_access.clone()),
    )
}

/// Control plane for `external_ag_ui` session tasks, registered through
/// [`TaskExecutorPlugin`].
///
/// - `deliver` answers the open interrupts of a run awaiting input by
///   starting the resuming run on the same AG-UI thread; any other state
///   refuses the message.
/// - `cancel` closes the stream when this process holds it and marks the run
///   canceled; a stream held by another worker sees the task's cancel request
///   within a second.
/// - `can_reattach` is false: AG-UI has no "get run", so a stream lost with
///   its worker is failed as orphaned by the reaper.
pub struct AgUiAgentTaskExecutor;

#[async_trait]
impl TaskExecutor for AgUiAgentTaskExecutor {
    fn kind(&self) -> &str {
        TASK_KIND_EXTERNAL_AG_UI
    }

    /// A finished run is mirrored; a live stream lost with its worker cannot
    /// be picked up again (AG-UI has no "get run"), so it is refused and the
    /// reaper fails the task as orphaned.
    async fn start(&self, task: &SessionTask, context: &ToolContext) -> Result<()> {
        let record = load_run_for_task(context, task)
            .await
            .map_err(AgentLoopError::tool)?;
        if record.status.is_terminal() || record.status == AgUiRunStatus::InputRequired {
            mirror_run_to_task(context, &record).await;
            return Ok(());
        }
        Err(AgentLoopError::tool(format!(
            "{TASK_KIND_EXTERNAL_AG_UI} task {} lost its stream; AG-UI runs cannot be re-attached",
            task.id
        )))
    }

    /// Answers the interrupts of a run awaiting input by starting the
    /// resuming run on the same thread.
    async fn deliver(
        &self,
        task: &SessionTask,
        message: &TaskMessage,
        context: &ToolContext,
    ) -> Result<()> {
        let record = load_run_for_task(context, task)
            .await
            .map_err(AgentLoopError::tool)?;
        if record.status != AgUiRunStatus::InputRequired {
            return Err(AgentLoopError::tool(format!(
                "AG-UI run {} is {}; only a run waiting for input accepts a message",
                record.run_id, record.status
            )));
        }
        let resume = resume_from_answer(&record.interrupts, &task_message_text(&message.content))
            .map_err(AgentLoopError::tool)?;
        let input = next_input(&record, resume);
        let agent = record.agent_config.clone();
        let timeout_secs = record.timeout_secs;
        let mut record = record;
        // Leave awaiting_input before returning, so a wait_task issued right
        // after message_task waits for the resumed run, not the old ask.
        record.status = AgUiRunStatus::Working;
        record.interrupts.clear();
        save_run(context, &record).await?;
        spawn_background(
            run_context(&record, context),
            agent,
            record,
            input,
            timeout_secs,
        );
        Ok(())
    }

    async fn cancel(&self, task: &SessionTask, context: &ToolContext) -> Result<()> {
        let mut record = load_run_for_task(context, task)
            .await
            .map_err(AgentLoopError::tool)?;
        // Close a stream this process holds; one held elsewhere sees the
        // task's cancel request on its next poll.
        ActiveStream::signal(&record.run_id);
        if !record.status.is_terminal() {
            record.status = AgUiRunStatus::Canceled;
            record.interrupts.clear();
            save_run(context, &record).await?;
        }
        Ok(())
    }

    async fn reconcile(&self, task: &SessionTask, context: &ToolContext) -> Result<()> {
        let record = load_run_for_task(context, task)
            .await
            .map_err(AgentLoopError::tool)?;
        if record.status.is_terminal() {
            mirror_run_to_task(context, &record).await;
        }
        Ok(())
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(AgUiAgentTaskExecutor),
    }
}
