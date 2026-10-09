//! The `tools` shell builtin: resolve a command line to a tool, read its
//! input, and run it as an ordinary tool call.
//!
//! THREAT[TM-TOOL-061]: every call goes through the turn's
//! `nested_tool_policy`, exactly as Lua code mode does: the pre-tool chain
//! decides on the call (approval, guardrails, user hooks), the schema check
//! runs, and the post-tool chain sees the result before the script does.
//! Without the policy nothing is dispatched. The child context drops the tool
//! registry, so a called tool cannot re-enter the shell.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use base64::Engine as _;
use bashkit::ExecResult;
use everruns_contracts::runtime::mcp_deferred::reveal_deferred_mcp_server;
use everruns_contracts::runtime::mcp_proxy::{McpServerTools, build_mcp_proxy_tools};
use everruns_contracts::runtime::saved_scripts::{SavedScript, SavedScripts};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::tool_types::{ToolCall, ToolResult, ToolResultImage};
use serde_json::{Value, json};

use super::catalog::{Catalog, Entry, Pending, tool_help};
use super::input::{self, Request};
use super::run::{Outcome, Run, StopReason};

/// How many tool calls one shell execution may make. Each is a real tool call,
/// often a network round trip, and a shell loop multiplies them; the
/// interpreter's own loop limits do not see that cost.
pub(crate) const MAX_CALLS_PER_EXECUTION: usize = 50;

/// How many matches `tools search` prints.
const SEARCH_LIMIT: usize = 10;

/// The `tools` builtin for one shell execution.
pub struct ToolsBuiltin {
    /// Grows when a deferred server loads mid-script; never held across an
    /// await.
    catalog: Mutex<Catalog>,
    pub(super) context: ToolContext,
    calls: AtomicUsize,
    pub(super) run: Arc<Run>,
    /// The agent's saved scripts, when the host bound a store.
    pub(super) saved: Option<SavedScripts>,
    /// Listed once per shell call, refreshed after a save.
    pub(super) saved_list: Mutex<Option<Vec<SavedScript>>>,
    /// How many saved scripts are running inside each other right now.
    pub(super) depth: AtomicUsize,
    /// This builtin, shared with the shells saved scripts run in, so their
    /// calls count against the same run and the same call cap.
    pub(super) me: Weak<ToolsBuiltin>,
}

impl ToolsBuiltin {
    pub(crate) fn new(context: &ToolContext) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            catalog: Mutex::new(Catalog::from_context(context)),
            context: context.clone(),
            calls: AtomicUsize::new(0),
            run: super::run::open(context),
            saved: context.extension::<SavedScripts>().map(|s| (*s).clone()),
            saved_list: Mutex::new(None),
            depth: AtomicUsize::new(0),
            me: me.clone(),
        })
    }

    fn catalog(&self) -> std::sync::MutexGuard<'_, Catalog> {
        self.catalog.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Error codes a script can branch on. They read the same whatever the tool.
pub(super) mod code {
    pub const UNKNOWN_COMMAND: &str = "unknown_command";
    pub const INVALID_INPUT: &str = "invalid_input";
    pub const TOOL_ERROR: &str = "tool_error";
    pub const DENIED: &str = "denied";
    pub const NEEDS_APPROVAL: &str = "needs_approval";
    pub const CONNECTION_REQUIRED: &str = "connection_required";
    pub const CALL_LIMIT: &str = "call_limit";
    pub const STOPPED: &str = "stopped";
    pub const UNAVAILABLE: &str = "unavailable";
}

pub(super) fn error(code: &str, message: impl Into<String>, retryable: bool) -> ExecResult {
    let body = json!({"error": {"code": code, "message": message.into(), "retryable": retryable}});
    ExecResult::err(format!("{body}\n"), 1)
}

/// End the script here: the interpreter exits, and the run refuses any call
/// the exit does not reach.
pub(super) fn stop_script(mut result: ExecResult) -> ExecResult {
    result.control_flow = bashkit::ControlFlow::Exit(result.exit_code);
    result
}

pub(super) fn text(out: String) -> ExecResult {
    if out.ends_with('\n') {
        ExecResult::ok(out)
    } else {
        ExecResult::ok(format!("{out}\n"))
    }
}

/// The `tools` builtin as registered with a shell; the shells saved scripts
/// run in register the same one.
pub(crate) struct SharedTools(pub Arc<ToolsBuiltin>);

#[async_trait]
impl bashkit::Builtin for SharedTools {
    async fn execute(&self, ctx: bashkit::BuiltinContext<'_>) -> bashkit::Result<ExecResult> {
        self.0.execute(ctx).await
    }

    fn llm_hint(&self) -> Option<&'static str> {
        Some(
            "tools: call the agent's tools. `tools --help` lists them; input is one JSON object, \
             output is JSON.",
        )
    }
}

impl ToolsBuiltin {
    fn root_help(&self) -> String {
        let mut help = self.catalog().root_help();
        if self.saved.is_some() {
            help.push_str(
                "\nSaved scripts: `tools scripts` lists them; `tools scripts <name> '{...}'` runs one.\n",
            );
        }
        help
    }

    /// Whether `word` is the built-in `scripts` or `search` word rather than a
    /// tool or server that happens to share the name.
    fn is_reserved(&self, word: &str, reserved: &str) -> bool {
        let catalog = self.catalog();
        word == reserved && catalog.top_level(word).is_none() && !catalog.is_source(word)
    }

    async fn execute(&self, ctx: bashkit::BuiltinContext<'_>) -> bashkit::Result<ExecResult> {
        let args = ctx.args;
        let Some(first) = args.first() else {
            return Ok(text(self.root_help()));
        };
        if first == "--help" || first == "-h" || first == "help" {
            return Ok(text(self.root_help()));
        }
        if self.is_reserved(first, "search") {
            return Ok(self.search(&args[1..]).await);
        }
        if self.saved.is_some() && self.is_reserved(first, "scripts") {
            return Ok(self.scripts(&args[1..], &ctx).await);
        }
        let pending = self.catalog().pending_source(first);
        if let Some(pending) = pending
            && let Err(failed) = self.load(&pending).await
        {
            return Ok(failed);
        }

        // `tools <server> ...` when the word names a server, else a top-level tool.
        let is_source = self.catalog().is_source(first);
        let (entry, rest) = if is_source {
            match args.get(1) {
                None => return Ok(text(self.catalog().source_help(first))),
                Some(word) if word == "--help" || word == "-h" => {
                    return Ok(text(self.catalog().source_help(first)));
                }
                Some(word) => match self.catalog().in_source(first, word).cloned() {
                    Some(entry) => (entry, &args[2..]),
                    None => {
                        return Ok(error(
                            code::UNKNOWN_COMMAND,
                            format!(
                                "no tool `{word}` on {first}. Run `tools {first} --help` to list its tools."
                            ),
                            false,
                        ));
                    }
                },
            }
        } else {
            match self.catalog().top_level(first).cloned() {
                Some(entry) => (entry, &args[1..]),
                None => {
                    return Ok(error(
                        code::UNKNOWN_COMMAND,
                        format!(
                            "no tool or server `{first}`. Run `tools --help` to list them, \
                             or `tools search <words>` to find one."
                        ),
                        false,
                    ));
                }
            }
        };

        let schema = super::catalog::strip_human_intent(entry.tool.parameters_schema());
        let stdin = ctx.stdin.and_then(|s| s.text().ok());
        let arguments = match input::parse(rest, stdin, &schema) {
            Ok(Request::Help) => return Ok(text(tool_help(&entry))),
            Ok(Request::Call(arguments)) => arguments,
            Err(message) => {
                return Ok(error(
                    code::INVALID_INPUT,
                    format!(
                        "{message}. Run `{} --help` for its input.",
                        entry.command_line()
                    ),
                    false,
                ));
            }
        };

        if self.run.is_stopped() {
            return Ok(stop_script(error(
                code::STOPPED,
                "the script was stopped at an earlier tools call; nothing after it runs",
                false,
            )));
        }
        if self.calls.fetch_add(1, Ordering::Relaxed) >= MAX_CALLS_PER_EXECUTION {
            self.run.stop(
                StopReason::CallLimit,
                &entry.command_line(),
                &arguments,
                None,
            );
            return Ok(stop_script(error(
                code::CALL_LIMIT,
                format!(
                    "more than {MAX_CALLS_PER_EXECUTION} tool calls in one shell call. Use a tool \
                     that takes or returns a list instead of looping over single calls, or split \
                     the work across shell calls."
                ),
                false,
            )));
        }

        Ok(self.call(&entry, arguments, &ctx).await)
    }
}

impl ToolsBuiltin {
    async fn search(&self, words: &[String]) -> ExecResult {
        let query = words.join(" ");
        if query.trim().is_empty() {
            return error(code::INVALID_INPUT, "usage: tools search <words>", false);
        }
        // A deferred server the words point at is loaded first, so its tools
        // rank with everything else. One that fails to load is left out.
        let pending = self.catalog().pending_matching(&query);
        for server in pending {
            let _ = self.load(&server).await;
        }
        let catalog = self.catalog();
        let matches = catalog.search(&query, SEARCH_LIMIT);
        if matches.is_empty() {
            return text(format!(
                "No tools match `{query}`. Run `tools --help` to list everything.\n"
            ));
        }
        let mut out = String::new();
        for entry in matches {
            out.push_str(&format!(
                "{}  {}\n",
                entry.command_line(),
                super::catalog::first_sentence(entry.tool.description())
            ));
        }
        out.push_str("\nRun `<line> --help` for a tool's input.\n");
        text(out)
    }

    /// Load a deferred MCP server's tools into this shell call, through the
    /// turn's MCP invoker, and record the reveal so the turn lists the server
    /// from the next step on. A host that cannot list mid-call still gets the
    /// reveal, and the script is told to try again on the next step.
    async fn load(&self, pending: &Pending) -> Result<(), ExecResult> {
        let server = &pending.source;
        let Some(invoker) = self.context.mcp_invoker.clone() else {
            return Err(error(
                code::UNAVAILABLE,
                format!("MCP server {server} cannot be loaded in this session"),
                false,
            ));
        };
        let listing = invoker
            .list_server_tools(&pending.prefix, self.context.session_id.uuid())
            .await;
        let definitions = match listing {
            Ok(Some(McpServerTools::Listed(definitions))) => definitions,
            Ok(Some(McpServerTools::ConnectionRequired(result))) => {
                return Err(error(
                    code::CONNECTION_REQUIRED,
                    result
                        .error
                        .unwrap_or_else(|| format!("MCP server {server} needs a connection first")),
                    false,
                ));
            }
            Ok(None) => {
                self.reveal(pending).await;
                return Err(error(
                    code::UNAVAILABLE,
                    format!(
                        "MCP server {server} is loading; its tools are available from your \
                         next step"
                    ),
                    true,
                ));
            }
            Err(failure) => {
                return Err(error(
                    code::TOOL_ERROR,
                    format!("could not load MCP server {server}: {failure}"),
                    true,
                ));
            }
        };
        self.reveal(pending).await;
        let tools = build_mcp_proxy_tools(&definitions, invoker)
            .into_iter()
            .map(Arc::from)
            .collect();
        tracing::info!(
            target: "bashkit.tools",
            session_id = %self.context.session_id,
            server = %pending.prefix,
            "deferred MCP server loaded from shell"
        );
        self.catalog().add_loaded(&pending.prefix, tools);
        Ok(())
    }

    /// Best effort: without the record the server is only loaded again by the
    /// next shell call that names it.
    async fn reveal(&self, pending: &Pending) {
        let Some(storage) = self.context.storage_store.as_ref() else {
            return;
        };
        if let Err(failure) =
            reveal_deferred_mcp_server(storage.as_ref(), self.context.session_id, &pending.prefix)
                .await
        {
            tracing::warn!(error = %failure, server = %pending.prefix, "failed to record MCP reveal");
        }
    }

    async fn call(
        &self,
        entry: &Entry,
        arguments: Value,
        ctx: &bashkit::BuiltinContext<'_>,
    ) -> ExecResult {
        let Some(policy) = self.context.nested_tool_policy.clone() else {
            return error(
                code::UNAVAILABLE,
                "tools can only call tools from an agent turn",
                false,
            );
        };
        let name = entry.tool_name.as_str();
        let tool_def = entry.tool.to_definition();
        let ordinal = self.calls.load(Ordering::Relaxed);
        let call_id = format!(
            "{}:tools:{ordinal}:{name}",
            self.context.tool_call_id.as_deref().unwrap_or("bash")
        );
        let command = entry.command_line();
        let requested = ToolCall {
            id: call_id.clone(),
            name: name.to_string(),
            arguments: arguments.clone(),
        };
        let authorized = match policy.authorize(requested, &tool_def, &self.context).await {
            Ok(authorized) => authorized,
            Err(outcome) => return self.refused(&command, &arguments, outcome),
        };
        // The decision covers this tool; a hook that retargets the call would
        // run something no gate decided on as that tool.
        if authorized.name != name {
            return error(
                code::DENIED,
                format!(
                    "a pre-tool hook changed the target from {name} to {}; refusing to run it",
                    authorized.name
                ),
                false,
            );
        }
        if let Some(problem) = schema_problem(entry, &authorized.arguments) {
            return error(
                code::INVALID_INPUT,
                format!(
                    "{problem}. Run `{} --help` for its input.",
                    entry.command_line()
                ),
                false,
            );
        }

        let mut child = self.context.clone();
        child.tool_registry = None;
        child.tool_call_id = Some(call_id.clone());
        let mut result = entry
            .tool
            .execute_with_context(authorized.execution_arguments(), &child)
            .await
            .into_tool_result(&call_id, name);
        policy
            .after_exec(&authorized, &tool_def, &mut result, &self.context)
            .await;
        let outcome = match &result.error {
            Some(message) => Outcome::Failed(message),
            None => Outcome::Ok(result.result.as_ref()),
        };
        let read_only = entry.tool.hints().readonly == Some(true);
        self.run
            .record(&command, &authorized.arguments, read_only, outcome);
        tracing::info!(
            target: "bashkit.tools",
            session_id = %self.context.session_id,
            tool = %name,
            success = result.error.is_none(),
            "tools call from shell"
        );
        render(entry, result, ctx).await
    }
}

impl ToolsBuiltin {
    /// The pre-tool chain did not let the call run. A call that needs a
    /// person's approval stops the script, and the `bash` result asks.
    fn refused(&self, command: &str, input: &Value, outcome: ToolResult) -> ExecResult {
        let approval_required = outcome
            .result
            .as_ref()
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            == Some(everruns_contracts::TOOL_APPROVAL_REQUIRED_CODE);
        if approval_required {
            self.run
                .stop(StopReason::NeedsApproval, command, input, outcome.result);
            return stop_script(error(
                code::NEEDS_APPROVAL,
                "this call needs a person's approval. The script stopped here and the person \
                 is being asked. After they answer, write a new script for what is left; never \
                 run this one again.",
                false,
            ));
        }
        error(
            code::DENIED,
            outcome
                .error
                .unwrap_or_else(|| "blocked by tool policy".to_string()),
            false,
        )
    }
}

/// The tool's own schema check, reported before anything runs.
fn schema_problem(entry: &Entry, arguments: &Value) -> Option<String> {
    let call = ToolCall {
        id: String::new(),
        name: entry.tool_name.clone(),
        arguments: arguments.clone(),
    };
    match everruns_contracts::runtime::tools::validate_tool_arguments(entry.tool.as_ref(), &call) {
        Ok(None) => None,
        Ok(Some(problem)) => Some(problem),
        Err(_) => Some("the tool's input schema could not be checked".to_string()),
    }
}

/// JSON on stdout for success; the error envelope on stderr for failure. A
/// string result is the tool's own text and is printed as is, so `jq` sees
/// the document the tool returned rather than a quoted string.
async fn render(
    entry: &Entry,
    result: ToolResult,
    ctx: &bashkit::BuiltinContext<'_>,
) -> ExecResult {
    if let Some(required) = &result.connection_required {
        return error(
            code::CONNECTION_REQUIRED,
            format!(
                "{} needs a connection to {} first. Tell the person to connect it.",
                entry.command_line(),
                required.provider
            ),
            false,
        );
    }
    if let Some(message) = result.error {
        return error(code::TOOL_ERROR, message, false);
    }
    let mut out = match result.result.unwrap_or(Value::Null) {
        Value::String(text) => text,
        other => other.to_string(),
    };
    if !out.ends_with('\n') {
        out.push('\n');
    }
    let mut exec = ExecResult::ok(out);
    if let Some(images) = result.images.filter(|images| !images.is_empty()) {
        let note = save_images(entry, &images, ctx).await;
        exec.stderr = note.into();
    }
    exec
}

/// Images are written next to the script's working directory and named on
/// stderr, so stdout stays the tool's JSON.
async fn save_images(
    entry: &Entry,
    images: &[ToolResultImage],
    ctx: &bashkit::BuiltinContext<'_>,
) -> String {
    let dir = ctx.cwd.join("tool-images");
    if ctx.fs.mkdir(&dir, true).await.is_err() {
        return format!(
            "{} returned {} image(s) that could not be saved\n",
            entry.command_line(),
            images.len()
        );
    }
    let stem = entry
        .tool_name
        .replace(|c: char| !c.is_ascii_alphanumeric(), "-");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut saved = Vec::new();
    for (index, image) in images.iter().enumerate() {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(image.base64.as_bytes())
        else {
            continue;
        };
        let extension = image.media_type.rsplit('/').next().unwrap_or("bin");
        let path = dir.join(format!("{stem}-{stamp}-{index}.{extension}"));
        if ctx.fs.write_file(&path, &bytes).await.is_ok() {
            saved.push(path.display().to_string());
        }
    }
    if saved.is_empty() {
        format!(
            "{} returned {} image(s) that could not be saved\n",
            entry.command_line(),
            images.len()
        )
    } else {
        format!("saved {} image(s): {}\n", saved.len(), saved.join(" "))
    }
}
