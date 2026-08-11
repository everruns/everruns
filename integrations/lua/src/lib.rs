//! Sandboxed Lua execution and code-mode integration for Everruns agents.
//!
//! Scripts run in a fresh vendored Lua 5.4 VM with bounded memory,
//! instructions, time, output, and standard-library access. Filesystem and tool
//! calls cross host-provided contracts rather than reaching the machine.
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and is an
//! opt-in high-risk integration for `everruns-host`.
//!
//! # Example
//!
//! ```
//! use everruns_core::Capability;
//! use everruns_integrations_lua::LuaCapability;
//!
//! assert_eq!(LuaCapability.id(), "lua");
//! ```

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel,
};
use everruns_core::*;
use std::result::Result;

mod code_mode;
use crate::exec_tool_result::ExecToolResultPayload;
use crate::session_file::SessionFile;
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SessionFileSystem, ToolContext};
use crate::typed_id::SessionId;
use async_trait::async_trait;
pub use code_mode::{LUA_CODE_MODE_CAPABILITY_ID, LuaCodeModeCapability};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const LUA_CAPABILITY_ID: &str = "lua";

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 60_000;
/// Hard memory cap for the VM (TM-LUA-003).
const MEMORY_LIMIT_BYTES: usize = 32 * 1024 * 1024;
/// Instruction budget before the VM is interrupted (TM-LUA-002).
const MAX_INSTRUCTIONS: u64 = 50_000_000;
/// Cap on captured `print` output before further writes are dropped (TM-LUA-008).
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

const TOOL_DESCRIPTION: &str = r#"Execute a Lua 5.4 script in an isolated, sandboxed environment.

The session filesystem is available through the `fs` table (rooted at /workspace),
and `json.encode` / `json.decode` are available for structured data. Use `print(...)`
for output and `return <value>` to send a JSON-serializable result back.

No network, no host process access, no `io`/`os.execute`. CPU, memory, and runtime
are bounded."#;

const SYSTEM_PROMPT: &str = r#"You can run Lua 5.4 scripts via the `lua` tool for logic, math, and structured
data processing over the session workspace.

Host API:
- `fs.read(path)`, `fs.write(path, s)`, `fs.append(path, s)`, `fs.exists(path)`
- `fs.list(path)`, `fs.stat(path)`, `fs.remove(path[, recursive])`, `fs.mkdir(path)`
- `fs.grep(pattern[, path])` (indexed search)
- `json.encode(value)`, `json.decode(string)`
- `base64.encode(string)`, `base64.decode(string)`
- `print(...)` for output; `return value` to return a JSON-serializable result.

The full Lua 5.4 standard library is available (`string.*` incl. `format`/
`find`/`match`/`gsub`, `table.*` incl. `sort`, `math.*`, `os.time`/`os.date`).

When enabled by the environment (otherwise these globals are nil):
- `http.get(url)` / `http.post(url, body)` -> `{ status, body }`, allow-listed
  hosts only.
- `tools.<name>(args_table)` -> result, to call other available tools.

Disabled for sandboxing (do not use — nil): `io`, `os.execute`/`os.getenv`,
`require`/`package`, `load`/`dofile`. Use `fs` for files; raw sockets are not
available (use `http` when present)."#;

// ============================================================================
// Capability
// ============================================================================

/// Lua execution capability — sandboxed scripting over the session VFS.
pub struct LuaCapability;

impl Capability for LuaCapability {
    fn id(&self) -> &str {
        LUA_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Lua"
    }

    fn description(&self) -> &str {
        r#"Execute Lua scripts in an isolated, sandboxed environment.

> [!NOTE]
> Scripts run in a virtual environment with no host or network access. The
> session filesystem is available via the `fs` table and `json` is available
> for structured data."#
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Lua",
            r#"Виконуйте Lua-скрипти в ізольованому середовищі-пісочниці.

> [!NOTE]
> Скрипти виконуються у віртуальному середовищі без доступу до хоста чи мережі.
> Файлова система сесії доступна через таблицю `fs`, а для структурованих даних
> доступний `json`."#,
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        // Scripted code execution + LLM-driven invocation: same trust elevation
        // as bashkit_shell. Admin-gated assignment (TM-LUA-001).
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("code")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(LuaTool)]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

// ============================================================================
// Tool
// ============================================================================

/// Tool to execute a Lua script in the sandbox.
pub struct LuaTool;

#[async_trait]
impl Tool for LuaTool {
    fn name(&self) -> &str {
        "lua"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Lua")
    }

    fn description(&self) -> &str {
        TOOL_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "Lua 5.4 source to execute."
                },
                "working_dir": {
                    "type": "string",
                    "default": crate::session_path::WORKSPACE_PREFIX,
                    "description": "Working directory (informational; `fs.*` paths default to /workspace and resolve through the session filesystem)."
                },
                "timeout_ms": {
                    "type": "integer",
                    "default": DEFAULT_TIMEOUT_MS,
                    "description": "Wall-clock timeout in milliseconds (capped at 60000)."
                },
                "output": crate::tool_output_sanitizer::output_verbosity_schema(),
            },
            "required": ["script"],
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_persist_output(true)
            // Mutates the shared session workspace: serialize concurrent lua/bash
            // calls in a batch so they don't race. Runs an in-process interpreter,
            // so offload to its own task.
            .with_concurrency_class("session_workspace")
            .with_cpu_bound(true)
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "lua requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let script = match arguments.get("script").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolExecutionResult::tool_error("Missing required parameter: script"),
        };

        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let output_mode = arguments
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .to_string();

        let file_store = match &context.file_store {
            Some(store) => store.clone(),
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        let vfs = Arc::new(LuaVfs::new(context.session_id, file_store));
        let limits = LuaLimits {
            memory_bytes: MEMORY_LIMIT_BYTES,
            max_instructions: MAX_INSTRUCTIONS,
            timeout: Duration::from_millis(timeout_ms),
            max_output_bytes: MAX_OUTPUT_BYTES,
        };

        // Code mode (`tools.<name>`): expose only Auto, non-destructive,
        // non-execution sibling tools. Excludes approval/client-side tools and
        // the execution tools themselves (no `lua`/`bash` re-entry).
        let allowed_tools = gated_code_mode_tools(context);

        // HTTP (`http.*`) is fail-closed: needs a host egress service AND a
        // non-empty network allow-list (the per-URL check happens at call time).
        let http_enabled = context.egress_service.is_some()
            && context
                .network_access
                .as_ref()
                .is_some_and(|a| !a.allowed.is_empty());

        // Stream captured `print` output as tool.output.delta events.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let emit_context = context.clone();
        let emit_task = tokio::spawn(async move {
            while let Some(chunk) = rx.recv().await {
                if !chunk.is_empty() {
                    emit_context.emit_tool_output("lua", &chunk, "stdout").await;
                }
            }
        });

        // engine::run enforces its own deadline; the outer timeout is a backstop.
        let outcome = tokio::time::timeout(
            limits.timeout + Duration::from_secs(2),
            engine::run(
                &script,
                vfs,
                context.clone(),
                allowed_tools,
                http_enabled,
                &limits,
                tx,
            ),
        )
        .await;

        let _ = emit_task.await;

        let outcome = match outcome {
            Ok(o) => o,
            Err(_) => {
                return ToolExecutionResult::tool_error(format!(
                    "Lua execution timed out after {}ms",
                    timeout_ms
                ));
            }
        };

        if let Some(err) = outcome.error {
            let clean = crate::tool_output_sanitizer::clean_exec_output(&outcome.stdout);
            return ToolExecutionResult::tool_error(if clean.is_empty() {
                format!("Lua error: {err}")
            } else {
                format!("Lua error: {err}\n--- output ---\n{clean}")
            });
        }

        // Shape stdout exactly like the exec tools so UI/persistence stay uniform.
        let payload = ExecToolResultPayload::new(&outcome.stdout, "", 0, &output_mode);
        let ExecToolResultPayload {
            stdout,
            truncated,
            total_lines,
            raw_output,
            ..
        } = payload;

        ToolExecutionResult::success_with_raw_output(
            json!({
                "stdout": stdout,
                "result": outcome.return_value,
                "success": true,
                "truncated": truncated,
                "total_lines": total_lines,
            }),
            raw_output,
        )
    }
}

/// Whether a tool is eligible to be driven through Lua "code mode" (the
/// `tools.<name>` table) rather than called directly.
///
/// This is the single source of truth shared by two call sites so they can
/// never drift apart: the engine's code-mode exposure
/// ([`gated_code_mode_tools`]) and the `lua_code_mode` capability's
/// tool-definition filter (which *hides* exactly this set from the model). If
/// the two used different predicates a tool could be hidden from the model yet
/// not re-exposed in Lua — an unreachable tool.
///
/// Conservative gate (TM-LUA-009): `Auto` policy only (never approval- or
/// client-side tools), non-destructive, non-`cpu_bound`, and never the
/// execution tools themselves (`lua`/`bash` are excluded so code mode cannot
/// re-enter a shell or itself).
pub fn is_code_mode_eligible(
    name: &str,
    policy: &crate::tool_types::ToolPolicy,
    hints: &ToolHints,
) -> bool {
    use crate::tool_types::ToolPolicy;
    name != "lua"
        && name != "bash"
        && *policy == ToolPolicy::Auto
        && hints.destructive != Some(true)
        && hints.cpu_bound != Some(true)
}

/// Sibling tools exposed to Lua "code mode" via the `tools` table.
///
/// Filters the live tool registry through [`is_code_mode_eligible`]. Returns an
/// empty list when no tool registry is present.
fn gated_code_mode_tools(context: &ToolContext) -> Vec<String> {
    let Some(reg) = context.tool_registry.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for name in reg.tool_names() {
        let Some(tool) = reg.get(name) else { continue };
        if is_code_mode_eligible(name, &tool.policy(), &tool.hints()) {
            out.push(name.to_string());
        }
    }
    out.sort();
    out
}

// ============================================================================
// Limits & outcome (engine-agnostic)
// ============================================================================

/// Resource limits enforced by the engine (TM-LUA-002/003/008).
///
#[derive(Debug, Clone)]
pub struct LuaLimits {
    pub memory_bytes: usize,
    pub max_instructions: u64,
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

/// Result of one script execution.
#[derive(Debug, Default)]
pub struct LuaOutcome {
    /// Captured `print` output.
    pub stdout: String,
    /// The script's `return` value, JSON-serialized (if any).
    pub return_value: Option<Value>,
    /// Set when the script raised an error or hit a limit.
    pub error: Option<String>,
}

impl LuaOutcome {
    fn engine_error(msg: impl Into<String>) -> Self {
        Self {
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

// ============================================================================
// LuaVfs — the only seam to the (session-scoped) session filesystem (TM-LUA-004)
// ============================================================================

/// Entry returned by `fs.list` / `fs.stat`.
#[derive(Debug, Clone)]
pub struct VfsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
}

/// A grep hit returned by `fs.grep`.
#[derive(Debug, Clone)]
pub struct VfsGrepHit {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

/// Bridges the Lua `fs` table to `SessionFileSystem`, scoped to one session.
pub struct LuaVfs {
    session_id: SessionId,
    store: Arc<dyn SessionFileSystem>,
}

impl LuaVfs {
    pub fn new(session_id: SessionId, store: Arc<dyn SessionFileSystem>) -> Self {
        Self { session_id, store }
    }

    pub async fn read(&self, path: &str) -> Result<String, String> {
        let sp = crate::session_path::to_session_path(path);
        match self.store.read_file(self.session_id, &sp).await {
            Ok(Some(file)) => {
                let content = file.content.unwrap_or_default();
                let bytes = SessionFile::decode_content(&content, &file.encoding)
                    .map_err(|e| e.to_string())?;
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            }
            Ok(None) => Err(format!("file not found: {path}")),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn write(&self, path: &str, content: &str) -> Result<(), String> {
        let sp = crate::session_path::to_session_path(path);
        let (encoded, encoding) = SessionFile::encode_content(content.as_bytes());
        self.store
            .write_file(self.session_id, &sp, &encoded, &encoding)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub async fn append(&self, path: &str, content: &str) -> Result<(), String> {
        // Append semantics: treat a missing file as empty.
        let mut existing = self.read(path).await.unwrap_or_default();
        existing.push_str(content);
        self.write(path, &existing).await
    }

    pub async fn exists(&self, path: &str) -> Result<bool, String> {
        let sp = crate::session_path::to_session_path(path);
        if matches!(
            self.store.read_file(self.session_id, &sp).await,
            Ok(Some(_))
        ) {
            return Ok(true);
        }
        Ok(self
            .store
            .list_directory(self.session_id, &sp)
            .await
            .is_ok())
    }

    pub async fn stat(&self, path: &str) -> Result<Option<VfsEntry>, String> {
        let sp = crate::session_path::to_session_path(path);
        match self.store.stat_file(self.session_id, &sp).await {
            Ok(Some(stat)) => Ok(Some(VfsEntry {
                name: stat.name,
                is_dir: stat.is_directory,
                size: stat.size_bytes,
            })),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn list(&self, path: &str) -> Result<Vec<VfsEntry>, String> {
        let sp = crate::session_path::to_session_path(path);
        self.store
            .list_directory(self.session_id, &sp)
            .await
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|e| VfsEntry {
                        name: e.name,
                        is_dir: e.is_directory,
                        size: e.size_bytes,
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }

    pub async fn remove(&self, path: &str, recursive: bool) -> Result<bool, String> {
        let sp = crate::session_path::to_session_path(path);
        self.store
            .delete_file(self.session_id, &sp, recursive)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn mkdir(&self, path: &str) -> Result<(), String> {
        let sp = crate::session_path::to_session_path(path);
        self.store
            .create_directory(self.session_id, &sp)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub async fn grep(&self, pattern: &str, path: Option<&str>) -> Result<Vec<VfsGrepHit>, String> {
        // None path => whole workspace; otherwise translate the scope.
        let scope = match path {
            Some(p) => {
                let sp = crate::session_path::to_session_path(p);
                if sp == "/" { None } else { Some(sp) }
            }
            None => None,
        };
        self.store
            .grep_files(self.session_id, pattern, scope.as_deref())
            .await
            .map(|matches| {
                matches
                    .into_iter()
                    .map(|m| VfsGrepHit {
                        // Re-root to the Lua VFS namespace.
                        path: crate::session_path::to_display_path(&m.path),
                        line_number: m.line_number,
                        line: m.line,
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }
}

// ============================================================================
// Engine (mlua)
// ============================================================================

// The engine: mlua (vendored Lua 5.4, never LuaJIT). mlua loads the full stdlib,
// so the sandbox works by *scrubbing* the dangerous surface
// (io/os.execute/package/require/load/dofile/string.dump) rather than by
// omission. Hardening (TM-LUA-001..009), all on by default, no configuration:
// - memory: `set_memory_limit` hard cap;
// - CPU/time: instruction-count hook + wall-clock deadline;
// - isolation: the VM runs on a dedicated blocking thread; `fs.*`, `http.*`, and
//   `tools.*` calls are marshaled to the async runtime over a channel. A
//   pathological *synchronous* op (e.g. catastrophic Lua pattern in C, which the
//   instruction hook cannot interrupt) therefore occupies one blocking-pool
//   thread instead of stalling a shared runtime worker — multitenant
//   containment. Residual: such an op is not force-killable in-process; the
//   robust fix is out-of-process execution.
// - egress: `http.*` is fail-closed — it requires both a host `EgressService`
//   and a non-empty network allow-list that permits the URL (TM-LUA-005).
// - code mode: `tools.<name>(args)` re-enters only Auto, non-destructive,
//   non-execution tools; the child context has no tool_registry, so code mode
//   cannot recurse (TM-LUA-009).
mod engine {
    use super::{LuaLimits, LuaOutcome, LuaVfs, ToolContext, VfsEntry, VfsGrepHit};
    use mlua::{
        HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Value as LuaValue, Variadic, VmState,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use tokio::sync::{mpsc, oneshot};

    /// Granularity of the interrupt hook (Lua instructions between checks).
    const HOOK_EVERY: u32 = 100_000;
    /// Cap on an HTTP response body handed back to Lua.
    const HTTP_BODY_CAP: usize = 1024 * 1024;

    // ---- Host-call bridge: blocking VM thread -> async runtime ----

    enum Op {
        Read(String),
        Write(String, String),
        Append(String, String),
        Exists(String),
        Stat(String),
        List(String),
        Remove(String, bool),
        Mkdir(String),
        Grep(String, Option<String>),
        Http {
            method: &'static str,
            url: String,
            body: Option<String>,
        },
        Tool {
            name: String,
            args: serde_json::Value,
        },
    }

    enum Reply {
        Str(String),
        Bool(bool),
        Unit,
        Stat(Option<VfsEntry>),
        List(Vec<VfsEntry>),
        Grep(Vec<VfsGrepHit>),
        Http { status: u16, body: String },
        Json(serde_json::Value),
    }

    struct Request {
        op: Op,
        reply: oneshot::Sender<Result<Reply, String>>,
    }

    async fn dispatch(vfs: &LuaVfs, ctx: &ToolContext, op: Op) -> Result<Reply, String> {
        match op {
            Op::Read(p) => vfs.read(&p).await.map(Reply::Str),
            Op::Write(p, c) => vfs.write(&p, &c).await.map(|_| Reply::Unit),
            Op::Append(p, c) => vfs.append(&p, &c).await.map(|_| Reply::Unit),
            Op::Exists(p) => vfs.exists(&p).await.map(Reply::Bool),
            Op::Stat(p) => vfs.stat(&p).await.map(Reply::Stat),
            Op::List(p) => vfs.list(&p).await.map(Reply::List),
            Op::Remove(p, r) => vfs.remove(&p, r).await.map(Reply::Bool),
            Op::Mkdir(p) => vfs.mkdir(&p).await.map(|_| Reply::Unit),
            Op::Grep(pat, scope) => vfs.grep(&pat, scope.as_deref()).await.map(Reply::Grep),
            Op::Http { method, url, body } => do_http(ctx, method, url, body).await,
            Op::Tool { name, args } => do_tool(ctx, &name, args).await.map(Reply::Json),
        }
    }

    /// HTTP via the host egress boundary. Fail-closed: requires both an
    /// `EgressService` and an allow-list that explicitly permits the URL.
    async fn do_http(
        ctx: &ToolContext,
        method: &'static str,
        url: String,
        body: Option<String>,
    ) -> Result<Reply, String> {
        use crate::egress::{EgressRequest, EgressRequestKind};
        let acl = ctx.network_access.as_ref();
        let permitted = acl
            .map(|a| !a.allowed.is_empty() && a.is_url_allowed(&url))
            .unwrap_or(false);
        if !permitted {
            return Err(format!(
                "network egress denied: {url} is not in the allow-list"
            ));
        }
        let egress = ctx
            .egress_service
            .as_ref()
            .ok_or_else(|| "network egress unavailable in this environment".to_string())?;
        let mut req = EgressRequest::new(method, &url, EgressRequestKind::Capability)
            .require_dns_pinning()
            .timeout_ms(15_000);
        if let Some(a) = acl {
            req = req.network_access(Some(a.clone()));
        }
        if let Some(b) = body {
            req = req
                .header("content-type", "application/json")
                .body(b.into_bytes());
        }
        let resp = egress.send(req).await.map_err(|e| e.to_string())?;
        let slice = if resp.body.len() > HTTP_BODY_CAP {
            &resp.body[..HTTP_BODY_CAP]
        } else {
            &resp.body[..]
        };
        Ok(Reply::Http {
            status: resp.status,
            body: String::from_utf8_lossy(slice).into_owned(),
        })
    }

    /// Code mode: re-enter another tool. The child context drops `tool_registry`
    /// so a code-mode tool cannot itself open code mode (no recursion).
    async fn do_tool(
        ctx: &ToolContext,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use crate::tools::ToolExecutionResult as R;
        let reg = ctx
            .tool_registry
            .as_ref()
            .ok_or_else(|| "tools unavailable in this environment".to_string())?;
        let tool = reg
            .get(name)
            .ok_or_else(|| format!("unknown tool: {name}"))?;
        let mut child = ctx.clone();
        child.tool_registry = None;
        child.tool_call_id = Some(format!("lua:{name}"));
        match tool.execute_with_context(args, &child).await {
            R::Success(v) => Ok(v),
            R::SuccessWithImages { result, .. } => Ok(result),
            R::ToolError(e) => Err(e),
            R::InternalError(_) => Err("tool internal error".to_string()),
            R::ConnectionRequired { provider } => {
                Err(format!("tool requires a connection: {provider}"))
            }
        }
    }

    /// Synchronous host call from inside a Lua function (blocking thread).
    fn call(tx: &mpsc::UnboundedSender<Request>, op: Op) -> Result<Reply, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(Request {
            op,
            reply: reply_tx,
        })
        .map_err(|_| "host channel closed".to_string())?;
        reply_rx
            .blocking_recv()
            .map_err(|_| "host reply dropped".to_string())?
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run(
        script: &str,
        vfs: Arc<LuaVfs>,
        ctx: ToolContext,
        allowed_tools: Vec<String>,
        http_enabled: bool,
        limits: &LuaLimits,
        output: mpsc::UnboundedSender<String>,
    ) -> LuaOutcome {
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<Request>();
        let script = script.to_string();
        let limits = limits.clone();

        let join = tokio::task::spawn_blocking(move || {
            run_blocking(script, limits, req_tx, output, allowed_tools, http_enabled)
        });

        while let Some(request) = req_rx.recv().await {
            let result = dispatch(&vfs, &ctx, request.op).await;
            let _ = request.reply.send(result);
        }

        match join.await {
            Ok(outcome) => outcome,
            Err(e) => LuaOutcome::engine_error(format!("lua task panicked: {e}")),
        }
    }

    fn run_blocking(
        script: String,
        limits: LuaLimits,
        req_tx: mpsc::UnboundedSender<Request>,
        output: mpsc::UnboundedSender<String>,
        allowed_tools: Vec<String>,
        http_enabled: bool,
    ) -> LuaOutcome {
        let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::OS | StdLib::UTF8;
        let lua = match Lua::new_with(libs, LuaOptions::default()) {
            Ok(l) => l,
            Err(e) => return LuaOutcome::engine_error(format!("lua init failed: {e}")),
        };

        let _ = lua.set_memory_limit(limits.memory_bytes);

        if let Err(e) = scrub_globals(&lua) {
            return LuaOutcome::engine_error(e);
        }

        let deadline = Instant::now() + limits.timeout;
        let budget = limits.max_instructions;
        let counted = Arc::new(AtomicU64::new(0));
        {
            let counted = counted.clone();
            // mlua 0.11 makes `set_hook` fallible; surface a setup failure as an
            // engine error rather than silently ignoring the result.
            if let Err(e) = lua.set_hook(
                HookTriggers::new().every_nth_instruction(HOOK_EVERY),
                move |_lua, _debug| {
                    if Instant::now() >= deadline {
                        return Err(mlua::Error::runtime("lua: exceeded time limit"));
                    }
                    if counted.fetch_add(HOOK_EVERY as u64, Ordering::Relaxed) >= budget {
                        return Err(mlua::Error::runtime("lua: exceeded instruction budget"));
                    }
                    Ok(VmState::Continue)
                },
            ) {
                return LuaOutcome::engine_error(format!("install hook: {e}"));
            }
        }

        let stdout = Arc::new(Mutex::new(String::new()));
        if let Err(e) = install_print(&lua, stdout.clone(), output, limits.max_output_bytes) {
            return LuaOutcome::engine_error(format!("install print: {e}"));
        }
        if let Err(e) = install_json(&lua) {
            return LuaOutcome::engine_error(format!("install json: {e}"));
        }
        if let Err(e) = install_base64(&lua) {
            return LuaOutcome::engine_error(format!("install base64: {e}"));
        }
        if let Err(e) = install_fs(&lua, req_tx.clone()) {
            return LuaOutcome::engine_error(format!("install fs: {e}"));
        }
        if http_enabled && let Err(e) = install_http(&lua, req_tx.clone()) {
            return LuaOutcome::engine_error(format!("install http: {e}"));
        }
        if !allowed_tools.is_empty()
            && let Err(e) = install_tools(&lua, req_tx, allowed_tools)
        {
            return LuaOutcome::engine_error(format!("install tools: {e}"));
        }

        let result: mlua::Result<LuaValue> = lua.load(&script).set_name("agent_script").eval();
        let captured = stdout.lock().map(|s| s.clone()).unwrap_or_default();

        match result {
            Ok(value) => {
                let return_value = match value {
                    LuaValue::Nil => None,
                    other => lua.from_value::<serde_json::Value>(other).ok(),
                };
                LuaOutcome {
                    stdout: captured,
                    return_value,
                    error: None,
                }
            }
            Err(e) => LuaOutcome {
                stdout: captured,
                return_value: None,
                error: Some(e.to_string()),
            },
        }
    }

    /// Remove dangerous globals exposed by the loaded libraries
    /// (TM-LUA-001/006/007).
    fn scrub_globals(lua: &Lua) -> Result<(), String> {
        let g = lua.globals();
        for name in [
            "io",
            "package",
            "require",
            "dofile",
            "loadfile",
            "load",
            "loadstring",
            "collectgarbage",
        ] {
            g.set(name, LuaValue::Nil).map_err(|e| e.to_string())?;
        }
        if let Ok(os) = g.get::<mlua::Table>("os") {
            for name in [
                "execute",
                "getenv",
                "exit",
                "remove",
                "rename",
                "tmpname",
                "setlocale",
            ] {
                os.set(name, LuaValue::Nil).map_err(|e| e.to_string())?;
            }
        }
        if let Ok(string) = g.get::<mlua::Table>("string") {
            string
                .set("dump", LuaValue::Nil)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn install_print(
        lua: &Lua,
        buf: Arc<Mutex<String>>,
        sink: mpsc::UnboundedSender<String>,
        cap: usize,
    ) -> mlua::Result<()> {
        let print = lua.create_function(move |lua, args: Variadic<LuaValue>| {
            let mut parts = Vec::with_capacity(args.len());
            for a in args.iter() {
                let s = lua
                    .coerce_string(a.clone())?
                    .map(|ls| ls.to_string_lossy())
                    .unwrap_or_else(|| "nil".to_string());
                parts.push(s);
            }
            let mut line = parts.join("\t");
            line.push('\n');
            if let Ok(mut g) = buf.lock()
                && g.len() < cap
            {
                g.push_str(&line);
            }
            let _ = sink.send(line);
            Ok(())
        })?;
        lua.globals().set("print", print)?;
        Ok(())
    }

    fn install_json(lua: &Lua) -> mlua::Result<()> {
        let json = lua.create_table()?;
        json.set(
            "encode",
            lua.create_function(|lua, value: LuaValue| {
                let v: serde_json::Value = lua.from_value(value)?;
                serde_json::to_string(&v).map_err(mlua::Error::external)
            })?,
        )?;
        json.set(
            "decode",
            lua.create_function(|lua, s: String| {
                let v: serde_json::Value =
                    serde_json::from_str(&s).map_err(mlua::Error::external)?;
                lua.to_value(&v)
            })?,
        )?;
        lua.globals().set("json", json)?;
        Ok(())
    }

    fn install_base64(lua: &Lua) -> mlua::Result<()> {
        use base64::Engine as _;
        let table = lua.create_table()?;
        table.set(
            "encode",
            lua.create_function(|lua, s: mlua::LuaString| {
                let out = base64::engine::general_purpose::STANDARD.encode(s.as_bytes());
                lua.create_string(out)
            })?,
        )?;
        table.set(
            "decode",
            lua.create_function(|lua, s: mlua::LuaString| {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(s.as_bytes())
                    .map_err(mlua::Error::external)?;
                lua.create_string(bytes)
            })?,
        )?;
        lua.globals().set("base64", table)?;
        Ok(())
    }

    fn install_http(lua: &Lua, tx: mpsc::UnboundedSender<Request>) -> mlua::Result<()> {
        let http = lua.create_table()?;

        let t = tx.clone();
        http.set(
            "get",
            lua.create_function(move |lua, url: String| {
                let reply = call(
                    &t,
                    Op::Http {
                        method: "GET",
                        url,
                        body: None,
                    },
                )
                .map_err(mlua::Error::runtime)?;
                http_reply_to_table(lua, reply)
            })?,
        )?;

        let t = tx.clone();
        http.set(
            "post",
            lua.create_function(move |lua, (url, body): (String, Option<String>)| {
                let reply = call(
                    &t,
                    Op::Http {
                        method: "POST",
                        url,
                        body,
                    },
                )
                .map_err(mlua::Error::runtime)?;
                http_reply_to_table(lua, reply)
            })?,
        )?;

        lua.globals().set("http", http)?;
        Ok(())
    }

    fn http_reply_to_table(lua: &Lua, reply: Reply) -> mlua::Result<mlua::Table> {
        let Reply::Http { status, body } = reply else {
            return Err(mlua::Error::runtime("http: unexpected reply"));
        };
        let t = lua.create_table()?;
        t.set("status", status)?;
        t.set("body", body)?;
        Ok(t)
    }

    fn install_tools(
        lua: &Lua,
        tx: mpsc::UnboundedSender<Request>,
        names: Vec<String>,
    ) -> mlua::Result<()> {
        let tools = lua.create_table()?;
        for name in names {
            let t = tx.clone();
            let n = name.clone();
            tools.set(
                name,
                lua.create_function(move |lua, args: LuaValue| {
                    let json: serde_json::Value = match args {
                        LuaValue::Nil => serde_json::json!({}),
                        other => lua.from_value(other)?,
                    };
                    let reply = call(
                        &t,
                        Op::Tool {
                            name: n.clone(),
                            args: json,
                        },
                    )
                    .map_err(mlua::Error::runtime)?;
                    let Reply::Json(v) = reply else {
                        return Err(mlua::Error::runtime("tool: unexpected reply"));
                    };
                    lua.to_value(&v)
                })?,
            )?;
        }
        lua.globals().set("tools", tools)?;
        Ok(())
    }

    fn install_fs(lua: &Lua, tx: mpsc::UnboundedSender<Request>) -> mlua::Result<()> {
        let fs = lua.create_table()?;

        let t = tx.clone();
        fs.set(
            "read",
            lua.create_function(move |lua, path: String| {
                match call(&t, Op::Read(path)).map_err(mlua::Error::runtime)? {
                    Reply::Str(s) => lua.create_string(s),
                    _ => Err(mlua::Error::runtime("vfs: unexpected reply")),
                }
            })?,
        )?;

        let t = tx.clone();
        fs.set(
            "write",
            lua.create_function(move |_lua, (path, content): (String, String)| {
                call(&t, Op::Write(path, content)).map_err(mlua::Error::runtime)?;
                Ok(())
            })?,
        )?;

        let t = tx.clone();
        fs.set(
            "append",
            lua.create_function(move |_lua, (path, content): (String, String)| {
                call(&t, Op::Append(path, content)).map_err(mlua::Error::runtime)?;
                Ok(())
            })?,
        )?;

        let t = tx.clone();
        fs.set(
            "exists",
            lua.create_function(move |_lua, path: String| {
                let reply = call(&t, Op::Exists(path)).map_err(mlua::Error::runtime)?;
                Ok(matches!(reply, Reply::Bool(true)))
            })?,
        )?;

        let t = tx.clone();
        fs.set(
            "stat",
            lua.create_function(move |lua, path: String| {
                match call(&t, Op::Stat(path)).map_err(mlua::Error::runtime)? {
                    Reply::Stat(Some(entry)) => Ok(LuaValue::Table(entry_to_table(lua, &entry)?)),
                    Reply::Stat(None) => Ok(LuaValue::Nil),
                    _ => Err(mlua::Error::runtime("vfs: unexpected reply")),
                }
            })?,
        )?;

        let t = tx.clone();
        fs.set(
            "list",
            lua.create_function(move |lua, path: String| {
                let Reply::List(entries) =
                    call(&t, Op::List(path)).map_err(mlua::Error::runtime)?
                else {
                    return Err(mlua::Error::runtime("vfs: unexpected reply"));
                };
                let arr = lua.create_table()?;
                for (i, entry) in entries.iter().enumerate() {
                    arr.set(i + 1, entry_to_table(lua, entry)?)?;
                }
                Ok(arr)
            })?,
        )?;

        let t = tx.clone();
        fs.set(
            "remove",
            lua.create_function(move |_lua, (path, recursive): (String, Option<bool>)| {
                let reply = call(&t, Op::Remove(path, recursive.unwrap_or(false)))
                    .map_err(mlua::Error::runtime)?;
                Ok(matches!(reply, Reply::Bool(true)))
            })?,
        )?;

        let t = tx.clone();
        fs.set(
            "mkdir",
            lua.create_function(move |_lua, path: String| {
                call(&t, Op::Mkdir(path)).map_err(mlua::Error::runtime)?;
                Ok(())
            })?,
        )?;

        let t = tx.clone();
        fs.set(
            "grep",
            lua.create_function(move |lua, (pattern, path): (String, Option<String>)| {
                let Reply::Grep(hits) =
                    call(&t, Op::Grep(pattern, path)).map_err(mlua::Error::runtime)?
                else {
                    return Err(mlua::Error::runtime("vfs: unexpected reply"));
                };
                let arr = lua.create_table()?;
                for (i, h) in hits.iter().enumerate() {
                    let row = lua.create_table()?;
                    row.set("path", h.path.clone())?;
                    row.set("line_number", h.line_number)?;
                    row.set("line", h.line.clone())?;
                    arr.set(i + 1, row)?;
                }
                Ok(arr)
            })?,
        )?;

        lua.globals().set("fs", fs)?;
        Ok(())
    }

    fn entry_to_table(lua: &Lua, entry: &VfsEntry) -> mlua::Result<mlua::Table> {
        let t = lua.create_table()?;
        t.set("name", entry.name.clone())?;
        t.set("is_dir", entry.is_dir)?;
        t.set("size", entry.size)?;
        Ok(t)
    }
}

// ============================================================================
// Tests (engine-agnostic)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn capability_features() {
        let cap = LuaCapability;
        assert_eq!(cap.features(), vec!["file_system"]);
    }

    #[test]
    fn schema_requires_script() {
        let schema = LuaTool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "script"));
        assert!(schema["properties"].get("script").is_some());
    }

    // Path normalization is the shared `session_path::to_session_path` (tested
    // there); LuaVfs no longer carries its own copy and delegates resolution to
    // the store (MountFs in production).

    #[tokio::test]
    async fn execute_without_context_errors() {
        let result = LuaTool.execute(json!({"script": "return 1"})).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn execute_missing_script_errors() {
        let ctx = ToolContext::new(SessionId::new());
        let result = LuaTool.execute_with_context(json!({}), &ctx).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("Missing required parameter"))
        );
    }

    #[tokio::test]
    async fn execute_without_file_store_errors() {
        let ctx = ToolContext::new(SessionId::new());
        let result = LuaTool
            .execute_with_context(json!({"script": "return 1"}), &ctx)
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("not available"))
        );
    }

    /// Minimal file store used by engine tests.
    struct EmptyFileStore;

    #[async_trait]
    impl SessionFileSystem for EmptyFileStore {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> everruns_core::Result<Option<SessionFile>> {
            Ok(None)
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> everruns_core::Result<SessionFile> {
            Ok(SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                is_directory: false,
                is_readonly: false,
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
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
        ) -> everruns_core::Result<Vec<crate::session_file::FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> everruns_core::Result<Option<crate::FileStat>> {
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> everruns_core::Result<Vec<crate::GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> everruns_core::Result<crate::session_file::FileInfo> {
            Ok(crate::session_file::FileInfo {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
    }

    // ========================================================================
    // End-to-end engine tests (run for whichever engine feature is enabled).
    // ========================================================================

    mod engine_tests {
        use super::*;
        use std::collections::HashMap;
        use std::sync::Mutex;

        async fn run(script: &str, store: Arc<dyn SessionFileSystem>) -> Value {
            let mut ctx = ToolContext::new(SessionId::new());
            ctx.file_store = Some(store);
            match LuaTool
                .execute_with_context(json!({ "script": script }), &ctx)
                .await
            {
                ToolExecutionResult::Success(v) => v,
                other => panic!("expected success, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn runs_logic_and_math() {
            let v = run("return 2 + 3 * 4", Arc::new(EmptyFileStore)).await;
            assert_eq!(v["result"], json!(14));
            assert_eq!(v["success"], json!(true));
        }

        #[tokio::test]
        async fn captures_print_output() {
            let v = run("print('hello'); print('world')", Arc::new(EmptyFileStore)).await;
            assert_eq!(v["stdout"], json!("hello\nworld\n"));
        }

        #[tokio::test]
        async fn sandbox_blocks_dangerous_globals() {
            // No filesystem/process/dynamic-code escape hatches (TM-LUA-001/006).
            // Both engines guarantee these globals are nil.
            let script = r#"
                return {
                    io = io == nil,
                    package = package == nil,
                    require = require == nil,
                    load = load == nil,
                    dofile = dofile == nil,
                }
            "#;
            let v = run(script, Arc::new(EmptyFileStore)).await;
            for key in ["io", "package", "require", "load", "dofile"] {
                assert_eq!(v["result"][key], json!(true), "{key} should be nil");
            }
        }

        // The sandbox retains the safe os.* subset.
        #[tokio::test]
        async fn safe_os_subset_available() {
            let v = run("return type(os.time())", Arc::new(EmptyFileStore)).await;
            assert_eq!(v["result"], json!("number"));
        }

        #[tokio::test]
        async fn fs_write_read_roundtrip() {
            let store = Arc::new(MapFileStore::default());
            let v = run(
                r#"
                fs.write("/workspace/a.txt", "hello")
                return fs.read("/workspace/a.txt")
                "#,
                store,
            )
            .await;
            assert_eq!(v["result"], json!("hello"));
        }

        #[tokio::test]
        async fn json_roundtrip_through_vfs() {
            let store = Arc::new(MapFileStore::default());
            let v = run(
                r#"
                fs.write("/workspace/d.json", json.encode({ n = 42, name = "x" }))
                local t = json.decode(fs.read("/workspace/d.json"))
                return t.n
                "#,
                store,
            )
            .await;
            assert_eq!(v["result"], json!(42));
        }

        #[tokio::test]
        async fn tonumber_shim_works() {
            let v = run(r#"return tonumber("41") + 1"#, Arc::new(EmptyFileStore)).await;
            assert_eq!(v["result"], json!(42));
        }

        #[tokio::test]
        async fn base64_roundtrip() {
            let v = run(
                r#"return base64.decode(base64.encode("hello"))"#,
                Arc::new(EmptyFileStore),
            )
            .await;
            assert_eq!(v["result"], json!("hello"));
        }

        // ---- Phase 4: http (egress) + code mode (tools) ----

        #[tokio::test]
        async fn http_and_tools_disabled_by_default() {
            // No egress service, no allow-list, no tool registry → both nil.
            let v = run(
                "return { http = http == nil, tools = tools == nil }",
                Arc::new(EmptyFileStore),
            )
            .await;
            assert_eq!(v["result"]["http"], json!(true));
            assert_eq!(v["result"]["tools"], json!(true));
        }

        #[tokio::test]
        async fn http_get_through_egress_allowlist() {
            let mut ctx = ToolContext::new(SessionId::new());
            ctx.file_store = Some(Arc::new(EmptyFileStore));
            ctx.egress_service = Some(Arc::new(MockEgress));
            ctx.network_access = Some(crate::network_access::NetworkAccessList::allow_only([
                "93.184.216.34",
            ]));
            let v = match LuaTool
                .execute_with_context(
                    json!({ "script": r#"local r = http.get("http://93.184.216.34/x"); return { s = r.status, b = r.body }"# }),
                    &ctx,
                )
                .await
            {
                ToolExecutionResult::Success(v) => v,
                other => panic!("expected success, got {other:?}"),
            };
            assert_eq!(v["result"]["s"], json!(200));
            assert_eq!(v["result"]["b"], json!("pong"));
        }

        #[tokio::test]
        async fn http_denies_allowlisted_loopback_ip_before_egress() {
            let mut ctx = ToolContext::new(SessionId::new());
            ctx.file_store = Some(Arc::new(EmptyFileStore));
            ctx.egress_service = Some(Arc::new(everruns_http::DirectEgressService::new()));
            ctx.network_access = Some(crate::network_access::NetworkAccessList::allow_only([
                "127.0.0.1",
            ]));
            let result = LuaTool
                .execute_with_context(
                    json!({ "script": r#"return http.get("http://127.0.0.1/latest/meta-data").status"# }),
                    &ctx,
                )
                .await;
            assert!(
                matches!(result, ToolExecutionResult::ToolError(ref msg) if msg.contains("private/internal address")),
                "expected egress boundary denial, got {result:?}"
            );
        }

        #[tokio::test]
        async fn http_denied_when_not_in_allowlist() {
            let mut ctx = ToolContext::new(SessionId::new());
            ctx.file_store = Some(Arc::new(EmptyFileStore));
            ctx.egress_service = Some(Arc::new(MockEgress));
            ctx.network_access = Some(crate::network_access::NetworkAccessList::allow_only([
                "allowed.com",
            ]));
            let result = LuaTool
                .execute_with_context(
                    json!({ "script": r#"return http.get("https://evil.com/x").status"# }),
                    &ctx,
                )
                .await;
            assert!(
                matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("egress denied"))
            );
        }

        #[tokio::test]
        async fn code_mode_calls_a_sibling_tool() {
            let mut registry = crate::tools::ToolRegistry::new();
            registry.register(EchoTool);
            let mut ctx = ToolContext::new(SessionId::new());
            ctx.file_store = Some(Arc::new(EmptyFileStore));
            ctx.tool_registry = Some(Arc::new(registry));
            let v = match LuaTool
                .execute_with_context(
                    json!({ "script": r#"return tools.echo({ n = 5 }).n"# }),
                    &ctx,
                )
                .await
            {
                ToolExecutionResult::Success(v) => v,
                other => panic!("expected success, got {other:?}"),
            };
            assert_eq!(v["result"], json!(5));
        }

        #[tokio::test]
        async fn code_mode_excludes_execution_tools() {
            // The `lua` tool itself must never be exposed via code mode.
            let mut registry = crate::tools::ToolRegistry::new();
            registry.register(EchoTool);
            registry.register(LuaTool);
            let mut ctx = ToolContext::new(SessionId::new());
            ctx.file_store = Some(Arc::new(EmptyFileStore));
            ctx.tool_registry = Some(Arc::new(registry));
            let v = run_with_ctx(
                "return { lua = tools.lua == nil, echo = tools.echo ~= nil }",
                &ctx,
            )
            .await;
            assert_eq!(
                v["result"]["lua"],
                json!(true),
                "lua tool must not be exposed"
            );
            assert_eq!(v["result"]["echo"], json!(true));
        }

        async fn run_with_ctx(script: &str, ctx: &ToolContext) -> Value {
            match LuaTool
                .execute_with_context(json!({ "script": script }), ctx)
                .await
            {
                ToolExecutionResult::Success(v) => v,
                other => panic!("expected success, got {other:?}"),
            }
        }

        /// Echoes its JSON argument back as the result.
        struct EchoTool;

        #[async_trait]
        impl Tool for EchoTool {
            fn name(&self) -> &str {
                "echo"
            }
            fn description(&self) -> &str {
                "Echo the argument."
            }
            fn parameters_schema(&self) -> Value {
                json!({ "type": "object" })
            }
            async fn execute(&self, arguments: Value) -> ToolExecutionResult {
                ToolExecutionResult::success(arguments)
            }
        }

        /// Minimal egress service that returns a canned 200 response.
        struct MockEgress;

        #[async_trait]
        impl crate::egress::EgressService for MockEgress {
            async fn send(
                &self,
                request: crate::egress::EgressRequest,
            ) -> crate::egress::EgressResult<crate::egress::EgressResponse> {
                if !request.dns_pinning_required {
                    return Err(crate::egress::EgressError::InvalidRequest(
                        "missing DNS-pinned validation request".to_string(),
                    ));
                }
                Ok(crate::egress::EgressResponse {
                    status: 200,
                    headers: std::collections::BTreeMap::new(),
                    body: b"pong".to_vec(),
                })
            }

            async fn send_stream(
                &self,
                _request: crate::egress::EgressRequest,
            ) -> crate::egress::EgressResult<crate::egress::EgressStreamResponse> {
                Err(crate::egress::EgressError::Transport(
                    "streaming not supported in mock".to_string(),
                ))
            }
        }

        #[tokio::test]
        async fn fs_addresses_paths_outside_workspace() {
            // The store (MountFs) resolves any path into the session-scoped
            // backend — `/workspace` is just the default cwd, the root mount
            // makes everything addressable (still contained by the backend; the
            // session scope is the tenant boundary). A path outside /workspace
            // round-trips rather than being rejected.
            let store = Arc::new(MapFileStore::default());
            let v = run(
                r#"
                fs.write("/tmp/note.txt", "hi")
                return fs.read("/tmp/note.txt")
                "#,
                store,
            )
            .await;
            assert_eq!(v["result"], json!("hi"));
        }

        #[tokio::test]
        async fn instruction_budget_terminates_infinite_loop() {
            // A tight infinite loop must be interrupted by the hook, not hang.
            let result = LuaTool
                .execute_with_context(
                    json!({ "script": "while true do end", "timeout_ms": 1000 }),
                    &{
                        let mut ctx = ToolContext::new(SessionId::new());
                        ctx.file_store = Some(Arc::new(EmptyFileStore));
                        ctx
                    },
                )
                .await;
            assert!(matches!(result, ToolExecutionResult::ToolError(_)));
        }

        /// HashMap-backed store (normalized session paths) for fs round-trips.
        #[derive(Default)]
        struct MapFileStore {
            files: Mutex<HashMap<String, String>>,
        }

        #[async_trait]
        impl SessionFileSystem for MapFileStore {
            fn is_mount_resolver(&self) -> bool {
                false
            }

            async fn read_file(
                &self,
                session_id: SessionId,
                path: &str,
            ) -> everruns_core::Result<Option<SessionFile>> {
                let files = self.files.lock().unwrap();
                Ok(files.get(path).map(|content| SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: path.to_string(),
                    name: path.rsplit('/').next().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly: false,
                    content: Some(content.clone()),
                    encoding: "text".to_string(),
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            }

            async fn write_file(
                &self,
                session_id: SessionId,
                path: &str,
                content: &str,
                encoding: &str,
            ) -> everruns_core::Result<SessionFile> {
                self.files
                    .lock()
                    .unwrap()
                    .insert(path.to_string(), content.to_string());
                Ok(SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: path.to_string(),
                    name: path.rsplit('/').next().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly: false,
                    content: Some(content.to_string()),
                    encoding: encoding.to_string(),
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
            }

            async fn delete_file(
                &self,
                _session_id: SessionId,
                path: &str,
                _recursive: bool,
            ) -> everruns_core::Result<bool> {
                Ok(self.files.lock().unwrap().remove(path).is_some())
            }

            async fn list_directory(
                &self,
                session_id: SessionId,
                path: &str,
            ) -> everruns_core::Result<Vec<crate::session_file::FileInfo>> {
                let prefix = if path == "/" {
                    "/".to_string()
                } else {
                    format!("{path}/")
                };
                let files = self.files.lock().unwrap();
                Ok(files
                    .iter()
                    .filter(|(p, _)| p.starts_with(&prefix))
                    .map(|(p, c)| crate::session_file::FileInfo {
                        id: uuid::Uuid::new_v4(),
                        session_id: session_id.into(),
                        path: p.clone(),
                        name: p.rsplit('/').next().unwrap_or("").to_string(),
                        is_directory: false,
                        is_readonly: false,
                        size_bytes: c.len() as i64,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    })
                    .collect())
            }

            async fn stat_file(
                &self,
                _session_id: SessionId,
                path: &str,
            ) -> everruns_core::Result<Option<crate::FileStat>> {
                let files = self.files.lock().unwrap();
                Ok(files.get(path).map(|c| crate::FileStat {
                    path: path.to_string(),
                    name: path.rsplit('/').next().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly: false,
                    size_bytes: c.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            }

            async fn grep_files(
                &self,
                _session_id: SessionId,
                pattern: &str,
                path_pattern: Option<&str>,
            ) -> everruns_core::Result<Vec<crate::GrepMatch>> {
                let re = regex::Regex::new(pattern)
                    .map_err(|e| crate::error::AgentLoopError::store(e.to_string()))?;
                let files = self.files.lock().unwrap();
                let mut out = Vec::new();
                for (p, c) in files.iter() {
                    if let Some(pp) = path_pattern
                        && !p.starts_with(pp)
                    {
                        continue;
                    }
                    for (i, line) in c.lines().enumerate() {
                        if re.is_match(line) {
                            out.push(crate::GrepMatch {
                                path: p.clone(),
                                line_number: i + 1,
                                line: line.to_string(),
                            });
                        }
                    }
                }
                Ok(out)
            }

            async fn create_directory(
                &self,
                session_id: SessionId,
                path: &str,
            ) -> everruns_core::Result<crate::session_file::FileInfo> {
                Ok(crate::session_file::FileInfo {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: path.to_string(),
                    name: path.rsplit('/').next().unwrap_or("").to_string(),
                    is_directory: true,
                    is_readonly: false,
                    size_bytes: 0,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
            }
        }
    }
}
