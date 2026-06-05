//! Lua Execution Capability (experimental)
//!
//! Provides a sandboxed Lua interpreter over the session virtual filesystem.
//! See `specs/lua-execution.md` for motivation, sandbox model, threat model
//! (TM-LUA-*), and the phased roadmap (the goal is to supersede `virtual_bash`).
//!
//! Design (parallels `virtual_bash`):
//! - `LuaCapability` is `High` risk and admin-gated, exactly like `virtual_bash`.
//! - `LuaVfs` owns the `/workspace` <-> session-store path translation and is the
//!   single seam to the (already session-scoped) `SessionFileSystem`. Tenant
//!   isolation falls out of routing every path through that store.
//! - `LuaLimits` is plain data; `engine::run` enforces it. The `mlua`
//!   implementation lives behind `#[cfg(feature = "lua-mlua")]`, so the default
//!   workspace build never fetches or compiles `mlua`. Swapping to a pure-Rust
//!   VM (`piccolo`) later replaces only the engine module.
//!
//! Trust boundary (TM-LUA-001..008): `risk_level()` returns `High`; assigning
//! this capability requires `OrgRole::Admin` via the same gates as
//! `virtual_bash` (`check_high_risk_caps` / `require_admin_for_high_risk`).

use super::{Capability, CapabilityStatus, RiskLevel};
use crate::exec_tool_result::ExecToolResultPayload;
use crate::session_file::SessionFile;
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SessionFileSystem, ToolContext};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const LUA_CAPABILITY_ID: &str = "lua";

/// Workspace mount point in the virtual filesystem.
const WORKSPACE_PREFIX: &str = "/workspace";

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
- `print(...)` for output; `return value` to return a JSON-serializable result.

Paths are rooted at /workspace. Negative space (do not use — these are nil):
no `io`, no `os.execute`/`os.getenv`, no `require`/`package`, no `load`/`dofile`,
and no network. Use the `fs` table for all file access."#;

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

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        // Scripted code execution + LLM-driven invocation: same trust elevation
        // as virtual_bash. Admin-gated assignment (TM-LUA-001).
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
                    "default": WORKSPACE_PREFIX,
                    "description": "Working directory (informational; paths in `fs.*` are /workspace-rooted)."
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
                return ToolExecutionResult::tool_error("File system not available in this context");
            }
        };

        let vfs = Arc::new(LuaVfs::new(context.session_id, file_store));
        let limits = LuaLimits {
            memory_bytes: MEMORY_LIMIT_BYTES,
            max_instructions: MAX_INSTRUCTIONS,
            timeout: Duration::from_millis(timeout_ms),
            max_output_bytes: MAX_OUTPUT_BYTES,
        };

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
            engine::run(&script, vfs, &limits, tx),
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

// ============================================================================
// Limits & outcome (engine-agnostic)
// ============================================================================

/// Resource limits enforced by the engine (TM-LUA-002/003/008).
///
/// All fields but `timeout` are consumed only by the `mlua` engine, so they
/// read as dead code in the default (feature-off) build.
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "lua-mlua"), allow(dead_code))]
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
// LuaVfs — the only seam to the session filesystem (TM-LUA-004)
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

    /// Translate a Lua VFS path into a session-store path.
    ///
    /// Rules match `SessionFileSystemAdapter`: absolute, forward-slash,
    /// `/workspace` stripped; anything outside `/workspace` is rejected so the
    /// script can never reach another tenant or the host (TM-LUA-004).
    fn to_session_path(path: &str) -> Option<String> {
        let abs = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };

        if abs == WORKSPACE_PREFIX {
            Some("/".to_string())
        } else if let Some(stripped) = abs.strip_prefix(WORKSPACE_PREFIX) {
            // Only `/workspace/...` is valid, not `/workspacefoo`.
            if stripped.starts_with('/') {
                Some(stripped.to_string())
            } else {
                None
            }
        } else {
            None
        }
    }

    fn map_path(path: &str) -> Result<String, String> {
        Self::to_session_path(path).ok_or_else(|| format!("path not in /workspace: {path}"))
    }

    pub async fn read(&self, path: &str) -> Result<String, String> {
        let sp = Self::map_path(path)?;
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
        let sp = Self::map_path(path)?;
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
        let sp = Self::map_path(path)?;
        if matches!(self.store.read_file(self.session_id, &sp).await, Ok(Some(_))) {
            return Ok(true);
        }
        Ok(self
            .store
            .list_directory(self.session_id, &sp)
            .await
            .is_ok())
    }

    pub async fn stat(&self, path: &str) -> Result<Option<VfsEntry>, String> {
        let sp = Self::map_path(path)?;
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
        let sp = Self::map_path(path)?;
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
        let sp = Self::map_path(path)?;
        self.store
            .delete_file(self.session_id, &sp, recursive)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn mkdir(&self, path: &str) -> Result<(), String> {
        let sp = Self::map_path(path)?;
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
                let sp = Self::map_path(p)?;
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
                        path: format!("{WORKSPACE_PREFIX}{}", m.path),
                        line_number: m.line_number,
                        line: m.line,
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }
}

// ============================================================================
// Engine seam
// ============================================================================

#[cfg(not(feature = "lua-mlua"))]
mod engine {
    use super::{LuaLimits, LuaOutcome, LuaVfs};
    use std::sync::Arc;

    /// Stub used when the `lua` cargo feature is disabled (default). Keeps the
    /// workspace buildable without pulling in `mlua`.
    pub(super) async fn run(
        _script: &str,
        _vfs: Arc<LuaVfs>,
        _limits: &LuaLimits,
        _output: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> LuaOutcome {
        LuaOutcome::engine_error(
            "Lua engine is not compiled in this build (enable the `lua` cargo feature).",
        )
    }
}

// NOTE: The `mlua` engine below targets mlua 0.10 (Lua 5.4, vendored). It is
// compiled only with `--features lua` and is wired end to end in Phase 2
// (`specs/lua-execution.md`); exact API details may need to be pinned to the
// resolved mlua version. The sandbox controls (stdlib whitelist, global
// scrubbing, memory limit, instruction/deadline hook, no network) are the
// contract that must hold regardless of API shape.
#[cfg(feature = "lua-mlua")]
mod engine {
    use super::{LuaLimits, LuaOutcome, LuaVfs};
    use mlua::{
        HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Value as LuaValue, Variadic, VmState,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Granularity of the interrupt hook.
    const HOOK_EVERY: u32 = 100_000;

    pub(super) async fn run(
        script: &str,
        vfs: Arc<LuaVfs>,
        limits: &LuaLimits,
        output: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> LuaOutcome {
        // Minimal safe stdlib: string/table/math/utf8 plus a scrubbed os. No
        // io, package, debug, ffi (TM-LUA-001/006/007).
        let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::OS | StdLib::UTF8;
        let lua = match Lua::new_with(libs, LuaOptions::default()) {
            Ok(l) => l,
            Err(e) => return LuaOutcome::engine_error(format!("lua init failed: {e}")),
        };

        // Hard memory cap (TM-LUA-003).
        let _ = lua.set_memory_limit(limits.memory_bytes);

        if let Err(e) = scrub_globals(&lua) {
            return LuaOutcome::engine_error(e);
        }

        // Deadline + instruction budget interrupt (TM-LUA-002).
        let deadline = Instant::now() + limits.timeout;
        let budget = limits.max_instructions;
        let counted = Arc::new(AtomicU64::new(0));
        {
            let counted = counted.clone();
            let _ = lua.set_hook(
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
            );
        }

        let stdout = Arc::new(Mutex::new(String::new()));
        if let Err(e) = install_print(&lua, stdout.clone(), output, limits.max_output_bytes) {
            return LuaOutcome::engine_error(format!("install print: {e}"));
        }
        if let Err(e) = install_fs(&lua, vfs) {
            return LuaOutcome::engine_error(format!("install fs: {e}"));
        }
        if let Err(e) = install_json(&lua) {
            return LuaOutcome::engine_error(format!("install json: {e}"));
        }

        let result: mlua::Result<LuaValue> =
            lua.load(script).set_name("agent_script").eval_async().await;
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
    /// (TM-LUA-001/006). Anything not on the allow-list is set to nil.
    fn scrub_globals(lua: &Lua) -> Result<(), String> {
        let g = lua.globals();
        for name in [
            "io", "package", "require", "dofile", "loadfile", "load", "loadstring", "collectgarbage",
        ] {
            g.set(name, LuaValue::Nil).map_err(|e| e.to_string())?;
        }
        // Keep only the safe os subset.
        if let Ok(os) = g.get::<mlua::Table>("os") {
            for name in [
                "execute", "getenv", "exit", "remove", "rename", "tmpname", "setlocale",
            ] {
                os.set(name, LuaValue::Nil).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    fn install_print(
        lua: &Lua,
        buf: Arc<Mutex<String>>,
        sink: tokio::sync::mpsc::UnboundedSender<String>,
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
            if let Ok(mut g) = buf.lock() {
                if g.len() < cap {
                    g.push_str(&line);
                }
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

    fn install_fs(lua: &Lua, vfs: Arc<LuaVfs>) -> mlua::Result<()> {
        let fs = lua.create_table()?;

        let v = vfs.clone();
        fs.set(
            "read",
            lua.create_async_function(move |_, path: String| {
                let v = v.clone();
                async move { v.read(&path).await.map_err(mlua::Error::runtime) }
            })?,
        )?;

        let v = vfs.clone();
        fs.set(
            "write",
            lua.create_async_function(move |_, (path, content): (String, String)| {
                let v = v.clone();
                async move { v.write(&path, &content).await.map_err(mlua::Error::runtime) }
            })?,
        )?;

        let v = vfs.clone();
        fs.set(
            "append",
            lua.create_async_function(move |_, (path, content): (String, String)| {
                let v = v.clone();
                async move { v.append(&path, &content).await.map_err(mlua::Error::runtime) }
            })?,
        )?;

        let v = vfs.clone();
        fs.set(
            "exists",
            lua.create_async_function(move |_, path: String| {
                let v = v.clone();
                async move { v.exists(&path).await.map_err(mlua::Error::runtime) }
            })?,
        )?;

        let v = vfs.clone();
        fs.set(
            "stat",
            lua.create_async_function(move |lua, path: String| {
                let v = v.clone();
                async move {
                    match v.stat(&path).await.map_err(mlua::Error::runtime)? {
                        Some(e) => {
                            let t = lua.create_table()?;
                            t.set("name", e.name)?;
                            t.set("is_dir", e.is_dir)?;
                            t.set("size", e.size)?;
                            Ok(LuaValue::Table(t))
                        }
                        None => Ok(LuaValue::Nil),
                    }
                }
            })?,
        )?;

        let v = vfs.clone();
        fs.set(
            "list",
            lua.create_async_function(move |lua, path: String| {
                let v = v.clone();
                async move {
                    let entries = v.list(&path).await.map_err(mlua::Error::runtime)?;
                    let arr = lua.create_table()?;
                    for (i, e) in entries.into_iter().enumerate() {
                        let t = lua.create_table()?;
                        t.set("name", e.name)?;
                        t.set("is_dir", e.is_dir)?;
                        t.set("size", e.size)?;
                        arr.set(i + 1, t)?;
                    }
                    Ok(arr)
                }
            })?,
        )?;

        let v = vfs.clone();
        fs.set(
            "remove",
            lua.create_async_function(move |_, (path, recursive): (String, Option<bool>)| {
                let v = v.clone();
                async move {
                    v.remove(&path, recursive.unwrap_or(false))
                        .await
                        .map_err(mlua::Error::runtime)
                }
            })?,
        )?;

        let v = vfs.clone();
        fs.set(
            "mkdir",
            lua.create_async_function(move |_, path: String| {
                let v = v.clone();
                async move { v.mkdir(&path).await.map_err(mlua::Error::runtime) }
            })?,
        )?;

        let v = vfs.clone();
        fs.set(
            "grep",
            lua.create_async_function(move |lua, (pattern, path): (String, Option<String>)| {
                let v = v.clone();
                async move {
                    let hits = v
                        .grep(&pattern, path.as_deref())
                        .await
                        .map_err(mlua::Error::runtime)?;
                    let arr = lua.create_table()?;
                    for (i, h) in hits.into_iter().enumerate() {
                        let t = lua.create_table()?;
                        t.set("path", h.path)?;
                        t.set("line_number", h.line_number)?;
                        t.set("line", h.line)?;
                        arr.set(i + 1, t)?;
                    }
                    Ok(arr)
                }
            })?,
        )?;

        lua.globals().set("fs", fs)?;
        Ok(())
    }
}

// ============================================================================
// Tests (engine-agnostic)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_metadata() {
        let cap = LuaCapability;
        assert_eq!(cap.id(), "lua");
        assert_eq!(cap.name(), "Lua");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.risk_level(), RiskLevel::High);
        assert_eq!(cap.category(), Some("Execution"));
        assert_eq!(cap.dependencies(), vec!["session_file_system"]);
        assert_eq!(cap.features(), vec!["file_system"]);
    }

    #[test]
    fn capability_has_one_tool() {
        let tools = LuaCapability.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "lua");
        assert!(tools[0].requires_context());
    }

    #[test]
    fn schema_requires_script() {
        let schema = LuaTool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "script"));
        assert!(schema["properties"].get("script").is_some());
    }

    // Path translation mirrors SessionFileSystemAdapter (TM-LUA-004).
    #[test]
    fn path_workspace_root_maps_to_root() {
        assert_eq!(LuaVfs::to_session_path("/workspace"), Some("/".to_string()));
    }

    #[test]
    fn path_workspace_file() {
        assert_eq!(
            LuaVfs::to_session_path("/workspace/a/b.txt"),
            Some("/a/b.txt".to_string())
        );
    }

    #[test]
    fn path_relative_is_normalized() {
        assert_eq!(
            LuaVfs::to_session_path("workspace/x.txt"),
            Some("/x.txt".to_string())
        );
    }

    #[test]
    fn path_outside_workspace_rejected() {
        assert_eq!(LuaVfs::to_session_path("/etc/passwd"), None);
        assert_eq!(LuaVfs::to_session_path("/home/agent/x"), None);
        // /workspacefoo is not under /workspace
        assert_eq!(LuaVfs::to_session_path("/workspacefoo"), None);
    }

    #[tokio::test]
    async fn execute_without_context_errors() {
        let result = LuaTool.execute(json!({"script": "return 1"})).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context")));
    }

    #[tokio::test]
    async fn execute_missing_script_errors() {
        let ctx = ToolContext::new(SessionId::new());
        let result = LuaTool.execute_with_context(json!({}), &ctx).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("Missing required parameter")));
    }

    #[tokio::test]
    async fn execute_without_file_store_errors() {
        let ctx = ToolContext::new(SessionId::new());
        let result = LuaTool
            .execute_with_context(json!({"script": "return 1"}), &ctx)
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("not available")));
    }

    // When the engine is not compiled in (default build), the tool surfaces a
    // clear error rather than silently succeeding.
    #[cfg(not(feature = "lua-mlua"))]
    #[tokio::test]
    async fn engine_disabled_reports_not_compiled() {
        let mut ctx = ToolContext::new(SessionId::new());
        ctx.file_store = Some(Arc::new(EmptyFileStore));
        let result = LuaTool
            .execute_with_context(json!({"script": "return 1"}), &ctx)
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("not compiled")));
    }

    /// Minimal file store: enough to populate `ToolContext::file_store` so the
    /// tool reaches the engine seam. The engine-off path never touches it.
    struct EmptyFileStore;

    #[async_trait]
    impl SessionFileSystem for EmptyFileStore {
        async fn read_file(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> crate::Result<Option<SessionFile>> {
            Ok(None)
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> crate::Result<SessionFile> {
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
        ) -> crate::Result<bool> {
            Ok(false)
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> crate::Result<Vec<crate::session_file::FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> crate::Result<Option<crate::FileStat>> {
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> crate::Result<Vec<crate::GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> crate::Result<crate::session_file::FileInfo> {
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
    // End-to-end engine tests (only when the `lua` feature is compiled in).
    // ========================================================================

    #[cfg(feature = "lua-mlua")]
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
            // io/os.execute/load/require/dofile must be unavailable (TM-LUA-001/006).
            let script = r#"
                return {
                    io = io == nil,
                    exec = os.execute == nil,
                    getenv = os.getenv == nil,
                    load = load == nil,
                    require = require == nil,
                    dofile = dofile == nil,
                }
            "#;
            let v = run(script, Arc::new(EmptyFileStore)).await;
            for key in ["io", "exec", "getenv", "load", "require", "dofile"] {
                assert_eq!(v["result"][key], json!(true), "{key} should be nil");
            }
        }

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
        async fn fs_rejects_paths_outside_workspace() {
            // Reaching outside /workspace raises a Lua error -> tool_error.
            let mut ctx = ToolContext::new(SessionId::new());
            ctx.file_store = Some(Arc::new(EmptyFileStore));
            let result = LuaTool
                .execute_with_context(
                    json!({ "script": "return fs.read('/etc/passwd')" }),
                    &ctx,
                )
                .await;
            assert!(matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("workspace")));
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
            async fn read_file(
                &self,
                session_id: SessionId,
                path: &str,
            ) -> crate::Result<Option<SessionFile>> {
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
            ) -> crate::Result<SessionFile> {
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
            ) -> crate::Result<bool> {
                Ok(self.files.lock().unwrap().remove(path).is_some())
            }

            async fn list_directory(
                &self,
                session_id: SessionId,
                path: &str,
            ) -> crate::Result<Vec<crate::session_file::FileInfo>> {
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
            ) -> crate::Result<Option<crate::FileStat>> {
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
            ) -> crate::Result<Vec<crate::GrepMatch>> {
                let re = regex::Regex::new(pattern)
                    .map_err(|e| crate::error::AgentLoopError::store(e.to_string()))?;
                let files = self.files.lock().unwrap();
                let mut out = Vec::new();
                for (p, c) in files.iter() {
                    if let Some(pp) = path_pattern {
                        if !p.starts_with(pp) {
                            continue;
                        }
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
            ) -> crate::Result<crate::session_file::FileInfo> {
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
