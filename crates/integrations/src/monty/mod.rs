//! Python code mode over disposable Monty subprocesses.
//!
//! No host mounts, raw networking, environment, or shell fallback. All effects
//! use the turn's authorized tools. Private checkpoints preserve suspended
//! programs; an unsettled nested effect is never automatically repeated.
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use everruns_contracts::{
    runtime::{
        capabilities::{Capability, CapabilityStatus, RiskLevel},
        tool_context::ToolContext,
        tools::Tool,
    },
    tool_types::ToolHints,
    tools::ToolExecutionResult,
};
use monty_pool::{Pool, PoolConfig, ReplConfig, ResumeValue, TurnEvent, on_print_sync};
use monty_types::{MontyObject, ResourceLimits};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::OnceCell;

mod journal;
mod nested;
mod value;

pub const PYTHON_CODE_MODE_CAPABILITY_ID: &str = "python_code_mode";
const MAX_SOURCE: usize = 64 * 1024;
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_SNAPSHOT: usize = 1024 * 1024;
const MAX_SUSPENSIONS: usize = 128;
const MAX_ACTIVE_MS: u64 = 60_000;
static WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
const VERSION: &str = "everruns-monty-1.5.0-v1";

/// Operator configuration, never accepted from model tool arguments.
#[derive(Clone, Debug)]
pub struct MontyConfig {
    /// Absolute path to the pinned Monty 1.5.0 runtime (or a hardened launcher).
    pub binary: PathBuf,
    /// Private persistent fallback for hosts without an injected journal.
    /// Pre-create with mode 0700, outside every agent workspace.
    pub state_directory: Option<PathBuf>,
    /// Concurrent subprocesses, clamped to 1..=32.
    pub max_processes: usize,
}
impl Default for MontyConfig {
    fn default() -> Self {
        Self {
            binary: std::env::var_os("EVERRUNS_MONTY_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/usr/local/bin/everruns-monty-worker")),
            state_directory: std::env::var_os("EVERRUNS_MONTY_STATE_DIR").map(PathBuf::from),
            max_processes: 4,
        }
    }
}

/// Select Python code mode on a Framework agent. Runtime paths are host
/// configuration, deliberately absent from the agent's capability config.
#[derive(Clone, Copy, Debug, Default)]
pub struct PythonCodeMode;
impl everruns_contracts::IntoCapability for PythonCodeMode {
    fn into_capability(self) -> everruns_contracts::CapabilitySpec {
        everruns_contracts::CapabilityRef::new(PYTHON_CODE_MODE_CAPABILITY_ID).into()
    }
}

/// Optional execution capability shared by Framework, Server and agentd.
#[derive(Default)]
pub struct PythonCodeModeCapability {
    config: MontyConfig,
}
impl PythonCodeModeCapability {
    pub fn new(config: MontyConfig) -> Self {
        Self { config }
    }
}
impl Capability for PythonCodeModeCapability {
    fn id(&self) -> &str {
        PYTHON_CODE_MODE_CAPABILITY_ID
    }
    fn name(&self) -> &str {
        "Python Code Mode"
    }
    fn description(&self) -> &str {
        "Use Python as an agent execution interface through isolated Monty subprocesses, authorized tool calls and durable continuations. Bashkit remains a separate option with its own Python runtime."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn icon(&self) -> Option<&str> {
        Some("code")
    }
    fn category(&self) -> Option<&str> {
        Some("Execution")
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(PythonTool::new(self.config.clone()))]
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Python code mode is available. Use python for Python programs that orchestrate authorized tools with tool(name, arguments); tools(query) discovers their schemas. tool() returns {result, error}. Files and networking use those tools, not Python OS APIs. Direct tools remain available. When Python pauses for approval, after the decision call python with the returned resume token; do not rerun the source or perform the pending operation separately.",
        )
    }
}

/// Monty's host driver. A clone shares process admission, never interpreter state.
#[derive(Clone)]
pub struct PythonTool {
    config: MontyConfig,
    pool: Arc<OnceCell<Pool>>,
}
impl PythonTool {
    pub fn new(config: MontyConfig) -> Self {
        Self {
            config,
            pool: Arc::new(OnceCell::new()),
        }
    }
    async fn pool(&self) -> Result<&Pool, String> {
        self.pool.get_or_try_init(|| async {
            if !self.config.binary.is_absolute() { return Err("Monty runtime must be an absolute operator-configured path".into()); }
            let mut config = PoolConfig::subprocess(&self.config.binary);
            config.min_processes = 0;
            config.max_processes = self.config.max_processes.clamp(1, 32);
            config.max_checkouts_per_worker = Some(1);
            config.checkout_timeout = Some(Duration::from_secs(5));
            config.request_timeout = Some(Duration::from_secs(12));
            config.max_decode_bytes = 4 * 1024 * 1024;
            config.auto_resume = false;
            Pool::new(config).await.map_err(|error| { tracing::error!(%error, "Monty pool initialization failed"); "Monty runtime unavailable; install the pinned worker and configure EVERRUNS_MONTY_BIN".into() })
        }).await
    }
}

#[async_trait]
impl Tool for PythonTool {
    fn name(&self) -> &str {
        "python"
    }
    fn display_name(&self) -> Option<&str> {
        Some("Python")
    }
    fn description(&self) -> &str {
        "Execute a bounded Python program in Monty. tools(query) lists authorized tool schemas. tool(name, arguments) invokes one and returns {'result': ..., 'error': ...}. Final expression is the result. No pip, host files, subprocesses or sockets; use authorized workspace/network tools. On approval, resume the saved program with the returned token instead of repeating its source. Supply exactly one of script or resume."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"script":{"type":"string","maxLength":MAX_SOURCE},"resume":{"type":"string","maxLength":80}},"additionalProperties":false,"oneOf":[{"required":["script"]},{"required":["resume"]}]})
    }
    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_idempotent(true)
            .with_persist_output(true)
            .with_concurrency_class("session_workspace")
            .with_stays_direct(true)
    }
    fn requires_context(&self) -> bool {
        true
    }
    async fn execute(&self, _: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Python requires an agent tool context")
    }
    async fn execute_with_context(
        &self,
        arguments: Value,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        let run = self.run(arguments, ctx);
        let result = if let Some(cancel) = &ctx.cancellation {
            tokio::select! { result = tokio::time::timeout(Duration::from_millis(MAX_ACTIVE_MS), run) => result, _ = cancel.cancelled() => return ToolExecutionResult::tool_error("Python execution cancelled; reconcile any interrupted tool effect before retrying") }
        } else {
            tokio::time::timeout(Duration::from_millis(MAX_ACTIVE_MS), run).await
        };
        match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => ToolExecutionResult::tool_error(error),
            Err(_) => ToolExecutionResult::tool_error(
                "Python active wall-time limit exceeded; any unsettled tool effect will not be repeated",
            ),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    version: String,
    snapshot: String,
    output: String,
    suspensions: usize,
    active_ms: u64,
}

impl PythonTool {
    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolExecutionResult, String> {
        let object = args
            .as_object()
            .ok_or("Python arguments must be an object")?;
        if object.keys().any(|key| key != "script" && key != "resume")
            || object.values().any(|value| !value.is_string())
        {
            return Err("Python accepts only string script or resume arguments".into());
        }
        let script = args.get("script").and_then(Value::as_str);
        let resume = args.get("resume").and_then(Value::as_str);
        if script.is_some() == resume.is_some() {
            return Err("Supply exactly one of script or resume".into());
        }
        if script.is_some_and(|s| s.len() > MAX_SOURCE) {
            return Err("Python source exceeds 64 KiB".into());
        }
        let journal = journal::resolve(ctx, self.config.state_directory.as_deref());
        let (root, index) = if let Some(token) = resume {
            parse_token(token)?
        } else {
            (
                journal::digest(
                    ctx.idempotency_key
                        .as_deref()
                        .unwrap_or(&uuid::Uuid::new_v4().to_string()),
                ),
                0,
            )
        };
        let mut output = String::new();
        let mut suspensions = 0;
        let mut active_ms = 0;
        let mut checkpoint_index = index;
        let checkpoint = if let Ok(journal) = &journal {
            if let Some(script) = script {
                journal::save(
                    journal,
                    &format!("{root}:source"),
                    json!({"sha256":journal::digest(script)}),
                )
                .await?;
            }
            // Complete results are immutable; resuming a completed continuation
            // returns its recorded result and never executes the program again.
            if let Some(result) = journal::load(journal, &format!("{root}:complete")).await? {
                return Ok(ToolExecutionResult::success(result));
            }
            journal::load(journal, &format!("{root}:checkpoint:{index}")).await?
        } else {
            None
        };
        if resume.is_some() && checkpoint.is_none() {
            return Err("Python continuation not found in this session and turn".into());
        }
        // Capability registries can create multiple tool instances. Admission
        // is also process-wide, so many agents cannot multiply the worker cap.
        let _admission = WORKERS
            .acquire()
            .await
            .map_err(|_| "Monty admission closed")?;
        let repl = ReplConfig {
            limits: Some(ResourceLimits {
                max_feed_duration: Some(Duration::from_secs(10)),
                max_turn_duration: Some(Duration::from_secs(5)),
                max_memory: Some(64 * 1024 * 1024),
                max_recursion_depth: 200,
                max_suspensions: MAX_SUSPENSIONS,
                max_total_sleep: Some(Duration::ZERO),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut worker = self
            .pool()
            .await?
            .checkout(&repl)
            .await
            .map_err(runtime_error)?;
        let mut started = Instant::now();
        let mut event = if let Some(value) = checkpoint {
            let saved: Checkpoint =
                serde_json::from_value(value).map_err(|_| "invalid private Python checkpoint")?;
            if saved.version != VERSION || saved.snapshot.len() > MAX_SNAPSHOT * 4 / 3 + 4 {
                return Err("Python checkpoint version or size mismatch".into());
            }
            output = saved.output;
            suspensions = saved.suspensions;
            active_ms = saved.active_ms;
            let snapshot = STANDARD
                .decode(saved.snapshot)
                .map_err(|_| "invalid private Python checkpoint")?;
            let mut print = on_print_sync(|_, text| append_output(&mut output, text));
            worker
                .restore(snapshot, vec![], &mut print)
                .await
                .map_err(runtime_error)?
                .0
                .ok_or("Python checkpoint is not suspended")?
        } else {
            let mut print = on_print_sync(|_, text| append_output(&mut output, text));
            worker
                .feed(
                    script.ok_or("missing Python source")?,
                    vec![
                        ("tool".into(), MontyObject::function("tool", None)),
                        ("tools".into(), MontyObject::function("tools", None)),
                    ],
                    vec![],
                    false,
                    &mut print,
                )
                .await
                .map_err(runtime_error)?
        };
        loop {
            if active_ms.saturating_add(started.elapsed().as_millis() as u64) > MAX_ACTIVE_MS {
                return Err("Python cumulative active-time limit exceeded".into());
            }
            if suspensions >= MAX_SUSPENSIONS && !matches!(&event, TurnEvent::Complete(_)) {
                return Err("Python cumulative suspension limit exceeded".into());
            }
            let answer = match event {
                TurnEvent::Complete(value) => {
                    let result = json!({"result":value::from_python(value.as_ref())?,"stdout":output,"stdout_truncated":output.len() >= MAX_OUTPUT});
                    if let Ok(journal) = &journal { journal::save(journal, &format!("{root}:complete"), result.clone()).await?; }
                    return Ok(ToolExecutionResult::success(result));
                }
                TurnEvent::FunctionCall { function_name, args, object_id: None, .. } if function_name == "tools" => {
                    if args.args().len() > 1 || args.kwargs().next().is_some() { return Err("tools accepts one optional positional query".into()); }
                    let query = args.arg(0).map(|v| v.as_str().ok_or("tools query must be a string")).transpose()?.unwrap_or("");
                    ResumeValue::Return(value::to_python(&nested::catalog(ctx, query)?)?)
                }
                TurnEvent::FunctionCall { function_name, args, object_id: None, .. } if function_name == "tool" => {
                    if args.args().len() != 2 || args.kwargs().next().is_some() { return Err("tool requires two positional arguments: name, arguments".into()); }
                    let name = args.arg(0).and_then(|v| v.as_str()).ok_or("tool name must be a string")?;
                    let arguments = value::from_python(args.arg(1).ok_or("missing arguments")?)?;
                    if script.is_some() && ctx.idempotency_key.is_none() {
                        return Err("Python tool effects require a stable host invocation idempotency key".into());
                    }
                    let journal = journal.as_ref().map_err(Clone::clone)?;
                    let token = format!("{root}:{checkpoint_index}");
                    let checkpoint_key = format!("{root}:checkpoint:{checkpoint_index}");
                    if journal::load(journal, &checkpoint_key).await?.is_none() {
                        let snapshot = worker.dump().await.map_err(runtime_error)?;
                        if snapshot.len() > MAX_SNAPSHOT { return Err("Python checkpoint exceeds 1 MiB".into()); }
                        active_ms = active_ms.saturating_add(started.elapsed().as_millis() as u64);
                        started = Instant::now();
                        let saved = Checkpoint { version: VERSION.into(), snapshot:STANDARD.encode(snapshot), output:output.clone(), suspensions, active_ms };
                        journal::save(journal, &checkpoint_key, serde_json::to_value(saved).map_err(|_| "invalid Python checkpoint")?).await?;
                    }
                    match nested::invoke(ctx, journal, &format!("{root}:effect:{checkpoint_index}"), name, arguments).await? {
                        nested::Outcome::Result(result) => { checkpoint_index += 1; ResumeValue::Return(value::to_python(&result)?) }
                        nested::Outcome::Pause(mut result) => {
                            if let Some(value) = result.result.as_mut().and_then(Value::as_object_mut) {
                                value.insert("python_resume".into(), Value::String(token.clone()));
                                value.insert("python_instruction".into(), Value::String("After the decision, call python with {\"resume\": token}. Do not rerun the source.".into()));
                            }
                            result.error = Some(format!("{}; Python suspended. Resume with {{\"resume\":\"{token}\"}} after the decision.", result.error.as_deref().unwrap_or("Tool policy blocked execution")));
                            return Ok(ToolExecutionResult::PolicyOutcome(Box::new(result)));
                        }
                    }
                }
                TurnEvent::OsCall { .. } => ResumeValue::NotHandled,
                TurnEvent::NameLookup { .. } => return Err("Unknown Python name; only tool(name, arguments) and tools(query) are host functions".into()),
                _ => return Err("Unsupported Python host call; use the authorized tool bridge".into()),
            };
            suspensions += 1;
            let mut print = on_print_sync(|_, text| append_output(&mut output, text));
            event = worker
                .resume(answer, &mut print)
                .await
                .map_err(runtime_error)?;
        }
    }
}
fn parse_token(token: &str) -> Result<(String, usize), String> {
    let (root, index) = token.split_once(':').ok_or("invalid Python resume token")?;
    if root.len() != 64 || !root.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid Python resume token".into());
    }
    let index = index
        .parse::<usize>()
        .map_err(|_| "invalid Python resume token")?;
    if index > MAX_SUSPENSIONS {
        return Err("invalid Python resume token".into());
    }
    Ok((root.to_owned(), index))
}
fn append_output(output: &mut String, text: &str) {
    let mut end = text.len().min(MAX_OUTPUT.saturating_sub(output.len()));
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&text[..end]);
}
fn runtime_error(error: monty_pool::PoolError) -> String {
    // Runtime exceptions contain agent code; infrastructure errors may contain
    // host paths. Never report worker internals or snapshot bytes to the model.
    match error {
        monty_pool::PoolError::Runtime(exception) => {
            let mut message = String::from("Python: ");
            append_output(&mut message, &exception.to_string());
            message
        }
        _ => {
            tracing::warn!("Monty worker failed or exceeded a limit; checkout discarded");
            "Python worker failed or exceeded its limits; no shell fallback was attempted".into()
        }
    }
}

#[cfg(test)]
mod tests;
