//! Language-server (LSP) integration.
//!
//! Runs real language servers (rust-analyzer, typescript-language-server,
//! pyright, gopls, clangd, or anything configured) and exposes semantic
//! navigation the grep/repo-map/ast-grep stack cannot provide: diagnostics,
//! go-to-definition, references, hover, workspace-wide rename, symbols, and
//! code actions.
//!
//! Servers start lazily, one per project and language, on the first call that
//! needs them, and are reused by later calls. A server that sits unused is
//! dropped after [`IDLE_WORKSPACE`]. Where they run is up to the
//! [`CodeHost`]: beside this process for local code, inside the sandbox for
//! hosted sessions.

mod client;
mod manager;

use crate::failure::into_tool_result;
use crate::host::CodeHost;
use crate::narration::stable_labeled;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use everruns_contracts::narration::ToolNarrationPhase;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityStatus, SystemPromptContext,
};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_narration::{
    ToolNarrationContext, arg_str, basename, truncate,
};
use everruns_contracts::runtime::tools::Tool;
use everruns_contracts::tool_types::{ToolCall, ToolHints};
use everruns_contracts::tools::ToolExecutionResult;
use manager::{
    DEFAULT_DIAGNOSTICS_WAIT_MS, DEFAULT_REQUEST_TIMEOUT_MS, LspManager, MAX_DIAGNOSTICS_WAIT_MS,
    MAX_REQUEST_TIMEOUT_MS, OpenedDocument, PositionEncoding, ServerSpec, apply_workspace_edit,
    default_server_specs, from_lsp_position, line_text, to_lsp_position, uri_to_path,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::{Duration, Instant};

pub const LSP_CAPABILITY_ID: &str = "lsp";

const MAX_LOCATIONS: usize = 500;
/// Servers answer cross-file queries with empty (but successful) results
/// while still analyzing the workspace — pyright returns zero references
/// right after startup and the correct set seconds later. Retry empty
/// results while the server is younger than this window.
const WARMUP_WINDOW: Duration = Duration::from_secs(15);
const WARMUP_RETRY_DELAYS_MS: &[u64] = &[1_000, 2_000, 4_000];
const DEFAULT_REFERENCE_LIMIT: usize = 200;
const MAX_PREVIEW_CHARS: usize = 200;
const MAX_HOVER_CHARS: usize = 8_000;
const MAX_DIAGNOSTICS: usize = 200;
const MAX_SYMBOLS: usize = 500;
/// A workspace whose servers nobody has used for this long gives them up. A
/// local server is killed; one in a sandbox is replaced the next time the
/// workspace asks for it, or goes away with the sandbox.
const IDLE_WORKSPACE: Duration = Duration::from_secs(30 * 60);

// ---------------------------------------------------------------------------
// Config

/// User-facing capability config.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LspSettings {
    /// Per-language overrides keyed by server key (`rust`, `typescript`,
    /// `python`, `go`, `clangd`, or any new key with a `command`).
    #[serde(default)]
    servers: BTreeMap<String, ServerOverride>,
    #[serde(default)]
    request_timeout_ms: Option<u64>,
    #[serde(default)]
    diagnostics_wait_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerOverride {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    extensions: Option<Vec<String>>,
    /// `false` disables a built-in server without replacing it.
    #[serde(default)]
    enabled: Option<bool>,
}

fn parse_settings(config: &Value) -> Result<LspSettings, String> {
    if config.is_null() {
        return Ok(LspSettings::default());
    }
    serde_json::from_value(config.clone()).map_err(|err| format!("invalid lsp config: {err}"))
}

fn resolve_specs(settings: &LspSettings) -> Result<Vec<ServerSpec>, String> {
    let mut specs = default_server_specs();
    for (key, over) in &settings.servers {
        if over.enabled == Some(false) {
            specs.retain(|spec| &spec.key != key);
            continue;
        }
        if let Some(spec) = specs.iter_mut().find(|spec| &spec.key == key) {
            if let Some(command) = &over.command {
                spec.command = command.clone();
            }
            if let Some(args) = &over.args {
                spec.args = args.clone();
            }
            if let Some(extensions) = &over.extensions {
                spec.extensions = normalized_extensions(extensions);
            }
        } else {
            let command = over.command.clone().ok_or_else(|| {
                format!("lsp server `{key}` is not a built-in; it requires a `command`")
            })?;
            let extensions = over.extensions.clone().ok_or_else(|| {
                format!("lsp server `{key}` is not a built-in; it requires `extensions`")
            })?;
            specs.push(ServerSpec {
                key: key.clone(),
                command,
                args: over.args.clone().unwrap_or_default(),
                extensions: normalized_extensions(&extensions),
            });
        }
    }
    Ok(specs)
}

fn normalized_extensions(extensions: &[String]) -> Vec<String> {
    extensions
        .iter()
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect()
}

fn build_manager(
    host: Arc<dyn CodeHost>,
    scope: String,
    config: &Value,
) -> Result<LspManager, String> {
    let settings = parse_settings(config)?;
    let specs = resolve_specs(&settings)?;
    let request_timeout = Duration::from_millis(
        settings
            .request_timeout_ms
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS)
            .clamp(1_000, MAX_REQUEST_TIMEOUT_MS),
    );
    let diagnostics_wait = Duration::from_millis(
        settings
            .diagnostics_wait_ms
            .unwrap_or(DEFAULT_DIAGNOSTICS_WAIT_MS)
            .clamp(100, MAX_DIAGNOSTICS_WAIT_MS),
    );
    Ok(LspManager::new(
        host,
        scope,
        specs,
        request_timeout,
        diagnostics_wait,
    ))
}

// ---------------------------------------------------------------------------
// Capability

struct Workspace {
    config: Value,
    manager: Arc<LspManager>,
    last_used: Instant,
}

/// The running language servers of every workspace this process serves, by
/// the host's scope key. Shared by all tool instances so servers persist
/// across turns, and by all capability instances of a hosted catalog so they
/// persist across sessions' turns too.
#[derive(Default)]
struct Workspaces(std::sync::Mutex<HashMap<String, Workspace>>);

impl Workspaces {
    /// The manager for `scope`. Rebuilt (dropping the old servers) when the
    /// capability config changed.
    fn manager_for(
        &self,
        host: &Arc<dyn CodeHost>,
        scope: String,
        config: &Value,
    ) -> Result<Arc<LspManager>, String> {
        let mut workspaces = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        workspaces.retain(|key, workspace| {
            key == &scope || now.duration_since(workspace.last_used) < IDLE_WORKSPACE
        });
        if let Some(workspace) = workspaces.get_mut(&scope)
            && &workspace.config == config
        {
            workspace.last_used = now;
            return Ok(workspace.manager.clone());
        }
        let manager = Arc::new(build_manager(host.clone(), scope.clone(), config)?);
        workspaces.insert(
            scope,
            Workspace {
                config: config.clone(),
                manager: manager.clone(),
                last_used: now,
            },
        );
        Ok(manager)
    }
}

/// The `lsp` capability over one [`CodeHost`].
pub struct LspCapability {
    host: Arc<dyn CodeHost>,
    workspaces: Arc<Workspaces>,
}

impl LspCapability {
    /// Language servers for the code `host` reaches.
    pub fn new(host: Arc<dyn CodeHost>) -> Self {
        Self {
            host,
            workspaces: Arc::new(Workspaces::default()),
        }
    }

    /// Language servers inside each session's sandbox. A hosted catalog may
    /// build this capability more than once, so every instance shares one
    /// table of running servers.
    pub fn hosted() -> Self {
        static WORKSPACES: OnceLock<Arc<Workspaces>> = OnceLock::new();
        Self {
            host: Arc::new(crate::host::SandboxCodeHost),
            workspaces: WORKSPACES.get_or_init(Arc::default).clone(),
        }
    }
}

#[async_trait]
impl Capability for LspCapability {
    fn id(&self) -> &str {
        LSP_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Language Server (LSP)"
    }

    fn description(&self) -> &str {
        "Semantic code intelligence from real language servers: diagnostics, \
         go-to-definition, references, hover, workspace-wide rename, symbols, \
         and code actions."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Code Navigation")
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        // Directive on purpose: with a softer "prefer them" phrasing, models
        // solved symbol-level tasks around these tools with grep/read
        // (adoption measured near zero in evals/lsp_integration).
        Some(
            "<capability id=\"lsp\">\n\
             Language-server tools are enabled. For symbol-level questions, call \
             the `lsp_*` tool FIRST, before any grep/read exploration — the server \
             resolves semantics that text search gets wrong (same-named symbols, \
             aliases, re-exports):\n\
             - where is X defined / what is X → `lsp_definition`, `lsp_hover`\n\
             - who uses X / count call sites → `lsp_references`\n\
             - rename X everywhere → `lsp_rename` (fixes every reference, \
             including imports, aliases, and re-exports; never rename a symbol \
             with text edits when this tool is available)\n\
             - errors/warnings in a file → `lsp_diagnostics` (no build needed; \
             also run it on files you just edited)\n\
             - outline or fuzzy symbol search → `lsp_symbols`; server quick \
             fixes → `lsp_code_actions`\n\
             The first call per language and project starts its server, so it \
             may be slow while the project indexes; results improve on retry. \
             If a server is not installed where the code lives, the tool says \
             which one: install it and call again. Fall back to \
             grep only for plain-text matters (strings, comments, config).\n\
             </capability>"
                .to_string(),
        )
    }

    fn system_prompt_preview(&self) -> Option<String> {
        Some(
            "<capability id=\"lsp\">\n\
             Use `lsp_*` tools for semantic navigation: definitions, references, \
             hover, diagnostics, rename, symbols, code actions.\n\
             </capability>"
                .to_string(),
        )
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "servers": {
                    "type": "object",
                    "description": "Per-language server overrides keyed by server key (rust, typescript, python, go, clangd, or a new key). New keys require `command` and `extensions`.",
                    "additionalProperties": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "command": { "type": "string", "description": "Language-server executable." },
                            "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments (e.g. [\"--stdio\"])." },
                            "extensions": { "type": "array", "items": { "type": "string" }, "description": "File extensions routed to this server." },
                            "enabled": { "type": "boolean", "description": "Set false to disable a built-in server." }
                        }
                    }
                },
                "request_timeout_ms": {
                    "type": "integer", "minimum": 1000, "maximum": MAX_REQUEST_TIMEOUT_MS,
                    "description": "Per-request timeout. Defaults to 60000."
                },
                "diagnostics_wait_ms": {
                    "type": "integer", "minimum": 100, "maximum": MAX_DIAGNOSTICS_WAIT_MS,
                    "description": "How long lsp_diagnostics waits for a fresh publish. Defaults to 15000."
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        let settings = parse_settings(config)?;
        resolve_specs(&settings)?;
        Ok(())
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        // Invalid config should not make the tools vanish: fall back to the
        // defaults and say so in the log. `validate_config` rejects it earlier
        // wherever a host validates.
        let config = match self.validate_config(config) {
            Ok(()) => config.clone(),
            Err(err) => {
                tracing::warn!(target: "everruns::lsp", "invalid lsp config, using defaults: {err}");
                Value::Null
            }
        };
        [
            LspAction::Diagnostics,
            LspAction::Definition,
            LspAction::References,
            LspAction::Hover,
            LspAction::Rename,
            LspAction::Symbols,
            LspAction::CodeActions,
        ]
        .into_iter()
        .map(|action| {
            Box::new(LspTool {
                host: self.host.clone(),
                workspaces: self.workspaces.clone(),
                config: config.clone(),
                action,
            }) as Box<dyn Tool>
        })
        .collect()
    }
}

// ---------------------------------------------------------------------------
// Tools

#[derive(Clone, Copy, Debug)]
enum LspAction {
    Diagnostics,
    Definition,
    References,
    Hover,
    Rename,
    Symbols,
    CodeActions,
}

struct LspTool {
    host: Arc<dyn CodeHost>,
    workspaces: Arc<Workspaces>,
    config: Value,
    action: LspAction,
}

fn position_properties() -> Value {
    json!({
        "path": { "type": "string", "description": "Workspace-relative source file." },
        "line": { "type": "integer", "minimum": 1, "description": "1-based line number." },
        "column": { "type": "integer", "minimum": 1, "description": "1-based character column." }
    })
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        match self.action {
            LspAction::Diagnostics => "lsp_diagnostics",
            LspAction::Definition => "lsp_definition",
            LspAction::References => "lsp_references",
            LspAction::Hover => "lsp_hover",
            LspAction::Rename => "lsp_rename",
            LspAction::Symbols => "lsp_symbols",
            LspAction::CodeActions => "lsp_code_actions",
        }
    }

    fn display_name(&self) -> Option<&str> {
        Some(match self.action {
            LspAction::Diagnostics => "LSP diagnostics",
            LspAction::Definition => "LSP definition",
            LspAction::References => "LSP references",
            LspAction::Hover => "LSP hover",
            LspAction::Rename => "LSP rename",
            LspAction::Symbols => "LSP symbols",
            LspAction::CodeActions => "LSP code actions",
        })
    }

    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(stable_labeled(
            self.display_name().unwrap_or(self.name()),
            lsp_narration_detail(self.action, &tool_call.arguments),
            phase,
        ))
    }

    fn description(&self) -> &str {
        match self.action {
            LspAction::Diagnostics => {
                "Language-server diagnostics (errors/warnings) for one file. The first call \
                 per language starts its server and may be slow while the project indexes."
            }
            LspAction::Definition => {
                "Jump to where the symbol under (path, line, column) is defined. `kind` \
                 selects definition (default), type_definition, implementation, or declaration."
            }
            LspAction::References => {
                "Find every reference to the symbol under (path, line, column) across the \
                 workspace — semantic, not textual."
            }
            LspAction::Hover => {
                "Type signature and documentation for the symbol under (path, line, column)."
            }
            LspAction::Rename => {
                "Rename the symbol under (path, line, column) across the whole workspace, \
                 fixing every reference (imports, re-exports, aliases). Set dry_run=true to \
                 preview the edits without applying them."
            }
            LspAction::Symbols => {
                "List symbols. With `path`: the file's outline. With `query`: fuzzy \
                 workspace-wide symbol search (requires a `hint_path` to pick the language \
                 server unless one is already running)."
            }
            LspAction::CodeActions => {
                "List language-server code actions (quick fixes, refactorings) at a position, \
                 and optionally apply one by exact title via `apply`."
            }
        }
    }

    fn parameters_schema(&self) -> Value {
        let mut properties = position_properties();
        let required: Vec<&str> = match self.action {
            LspAction::Diagnostics => {
                properties["wait_ms"] = json!({
                    "type": "integer", "minimum": 100, "maximum": MAX_DIAGNOSTICS_WAIT_MS,
                    "description": "Max time to wait for fresh diagnostics."
                });
                vec!["path"]
            }
            LspAction::Definition => {
                properties["kind"] = json!({
                    "type": "string",
                    "enum": ["definition", "type_definition", "implementation", "declaration"],
                    "description": "Which target to resolve. Defaults to definition."
                });
                vec!["path", "line", "column"]
            }
            LspAction::References => {
                properties["include_declaration"] = json!({
                    "type": "boolean",
                    "description": "Include the declaration itself. Defaults to true."
                });
                properties["limit"] = json!({
                    "type": "integer", "minimum": 1, "maximum": MAX_LOCATIONS,
                    "description": "Maximum references to return. Defaults to 200."
                });
                vec!["path", "line", "column"]
            }
            LspAction::Hover => vec!["path", "line", "column"],
            LspAction::Rename => {
                properties["new_name"] = json!({
                    "type": "string", "description": "The new symbol name."
                });
                properties["dry_run"] = json!({
                    "type": "boolean",
                    "description": "Preview the edits without writing files. Defaults to false."
                });
                vec!["path", "line", "column", "new_name"]
            }
            LspAction::Symbols => {
                properties = json!({
                    "path": { "type": "string", "description": "File to outline (document symbols)." },
                    "query": { "type": "string", "description": "Workspace symbol query (fuzzy)." },
                    "hint_path": { "type": "string", "description": "Any file of the target language, used to pick the server for `query` searches." }
                });
                vec![]
            }
            LspAction::CodeActions => {
                properties["end_line"] = json!({
                    "type": "integer", "minimum": 1,
                    "description": "1-based end line of the range. Defaults to `line`."
                });
                properties["end_column"] = json!({
                    "type": "integer", "minimum": 1,
                    "description": "1-based end column. Defaults to `column`."
                });
                properties["apply"] = json!({
                    "type": "string",
                    "description": "Exact title of the action to apply. Omit to just list actions."
                });
                vec!["path", "line", "column"]
            }
        };
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        // Rename and code actions can write files; the rest only read.
        ToolHints::default().with_readonly(!matches!(
            self.action,
            LspAction::Rename | LspAction::CodeActions
        ))
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(format!("{} needs the session it runs in", self.name()))
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let scope = self.host.scope_key(context);
        let manager = match self.workspaces.manager_for(&self.host, scope, &self.config) {
            Ok(manager) => manager,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };
        let manager = manager.as_ref();
        let result = manager
            .drive(context, async {
                match self.action {
                    LspAction::Diagnostics => diagnostics(manager, context, &arguments).await,
                    LspAction::Definition => definition(manager, context, &arguments).await,
                    LspAction::References => references(manager, context, &arguments).await,
                    LspAction::Hover => hover(manager, context, &arguments).await,
                    LspAction::Rename => rename(manager, context, &arguments).await,
                    LspAction::Symbols => symbols(manager, context, &arguments).await,
                    LspAction::CodeActions => code_actions(manager, context, &arguments).await,
                }
            })
            .await;
        match result {
            Ok(value) => ToolExecutionResult::success(value),
            Err(err) => into_tool_result(err),
        }
    }
}

// ---------------------------------------------------------------------------
// Argument helpers

fn lsp_narration_detail(action: LspAction, arguments: &Value) -> Option<String> {
    match action {
        LspAction::Symbols => {
            if let Some(query) = arg_str(arguments, &["query"]) {
                return Some(truncate(query, 48));
            }
            arg_str(arguments, &["path", "hint_path"]).map(|path| basename(path).to_string())
        }
        LspAction::Rename => {
            let path = arg_str(arguments, &["path"]).map(|path| basename(path).to_string())?;
            match arg_str(arguments, &["new_name"]) {
                Some(name) => Some(format!("{path} → {}", truncate(name, 32))),
                None => Some(path),
            }
        }
        _ => {
            let path = arg_str(arguments, &["path"]).map(|path| basename(path).to_string())?;
            match arguments.get("line").and_then(Value::as_u64) {
                Some(line) => Some(format!("{path}:{line}")),
                None => Some(path),
            }
        }
    }
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("`{key}` is required"))
}

fn required_position(arguments: &Value) -> Result<(usize, usize)> {
    let line = arguments
        .get("line")
        .and_then(Value::as_u64)
        .context("`line` is required (1-based)")?;
    let column = arguments
        .get("column")
        .and_then(Value::as_u64)
        .context("`column` is required (1-based)")?;
    if line == 0 || column == 0 {
        bail!("`line` and `column` are 1-based and must be >= 1");
    }
    Ok((line as usize, column as usize))
}

/// Open the document and resolve the request position in one step.
async fn open_at(
    manager: &LspManager,
    context: &ToolContext,
    arguments: &Value,
) -> Result<(OpenedDocument, Value)> {
    let path = required_str(arguments, "path")?;
    let (line, column) = required_position(arguments)?;
    let doc = manager.open(context, path).await?;
    let position = to_lsp_position(&doc.text, line, column, doc.server.encoding)?;
    Ok((doc, position))
}

/// Send a cross-file request, retrying while the server warms up and keeps
/// answering with `looks_empty` results. Returns the last response either way
/// — an empty result after the window is a real answer, not a warmup artifact.
async fn request_retrying_warmup(
    server: &manager::LanguageServer,
    method: &str,
    params: Value,
    looks_empty: impl Fn(&Value) -> bool,
) -> Result<Value> {
    let mut result = server.client.request(method, params.clone()).await?;
    for delay_ms in WARMUP_RETRY_DELAYS_MS {
        if !looks_empty(&result) || server.started_at.elapsed() > WARMUP_WINDOW {
            break;
        }
        tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
        result = server.client.request(method, params.clone()).await?;
    }
    Ok(result)
}

fn no_locations(result: &Value) -> bool {
    normalize_locations(result).is_empty()
}

/// A null, empty, or single-file rename edit during warmup may reflect
/// incomplete workspace analysis (cross-file references not resolved yet);
/// two or more touched files proves cross-file resolution happened. The
/// worst case for a genuinely single-file symbol is a few extra seconds,
/// bounded by the warmup window.
fn rename_looks_incomplete(edit: &Value) -> bool {
    if edit.is_null() {
        return true;
    }
    let mut files = std::collections::BTreeSet::new();
    if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
        files.extend(changes.keys().cloned());
    }
    if let Some(document_changes) = edit.get("documentChanges").and_then(Value::as_array) {
        for change in document_changes {
            if let Some(uri) = change
                .pointer("/textDocument/uri")
                .or_else(|| change.get("uri"))
                .and_then(Value::as_str)
            {
                files.insert(uri.to_string());
            }
        }
    }
    files.len() < 2
}

// ---------------------------------------------------------------------------
// Location rendering

async fn workspace_root(manager: &LspManager, context: &ToolContext) -> Result<PathBuf> {
    Ok(PathBuf::from(manager.host.root(context).await?))
}

/// File contents by URI, for turning server positions into tool-facing
/// coordinates and previews. Read in one batch: with remote code every file
/// is a round trip otherwise. A file that cannot be read is simply absent,
/// and renders without a preview.
type Texts = HashMap<String, String>;

async fn load_texts(
    manager: &LspManager,
    context: &ToolContext,
    uris: impl IntoIterator<Item = &str>,
) -> Texts {
    let uris: BTreeSet<&str> = uris.into_iter().collect();
    let files: Vec<(String, String)> = uris
        .into_iter()
        .filter_map(|uri| {
            let path = uri_to_path(uri).ok()?;
            Some((uri.to_string(), path.to_str()?.to_string()))
        })
        .collect();
    let paths: Vec<String> = files.iter().map(|(_, path)| path.clone()).collect();
    let Ok(texts) = manager.host.read_texts(context, &paths).await else {
        return Texts::new();
    };
    files
        .into_iter()
        .zip(texts)
        .filter_map(|((uri, _), text)| text.map(|text| (uri, text)))
        .collect()
}

/// Normalize `Location | Location[] | LocationLink[]` to a flat list.
fn normalize_locations(result: &Value) -> Vec<(String, Value)> {
    let items: Vec<&Value> = match result {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![result],
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            // LocationLink uses targetUri/targetSelectionRange.
            if let Some(uri) = item.get("targetUri").and_then(Value::as_str) {
                let range = item
                    .get("targetSelectionRange")
                    .or_else(|| item.get("targetRange"))?;
                return Some((uri.to_string(), range.clone()));
            }
            let uri = item.get("uri").and_then(Value::as_str)?;
            let range = item.get("range")?;
            Some((uri.to_string(), range.clone()))
        })
        .collect()
}

/// Render one location with tool-facing 1-based coordinates and a preview of
/// the target line. Files outside the workspace (stdlib, dependencies) keep
/// their absolute path and are flagged `external`.
fn render_location(
    root: &Path,
    uri: &str,
    range: &Value,
    encoding: PositionEncoding,
    texts: &Texts,
) -> Value {
    let Ok(file) = uri_to_path(uri) else {
        return json!({ "uri": uri });
    };
    let external = !file.starts_with(root);
    let path = if external {
        file.display().to_string()
    } else {
        file.strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let text = texts.get(uri).map(String::as_str).unwrap_or_default();
    let (line, column) = range
        .get("start")
        .map(|start| from_lsp_position(text, start, encoding))
        .unwrap_or((1, 1));
    let (end_line, end_column) = range
        .get("end")
        .map(|end| from_lsp_position(text, end, encoding))
        .unwrap_or((line, column));
    let preview = truncate_chars(
        line_text(text, line.saturating_sub(1)).trim(),
        MAX_PREVIEW_CHARS,
    );
    let mut rendered = json!({
        "path": path,
        "line": line,
        "column": column,
        "end_line": end_line,
        "end_column": end_column,
        "preview": preview,
    });
    if external {
        rendered["external"] = json!(true);
    }
    rendered
}

async fn render_location_list(
    manager: &LspManager,
    context: &ToolContext,
    result: &Value,
    encoding: PositionEncoding,
    limit: usize,
) -> Result<(Vec<Value>, bool)> {
    let root = workspace_root(manager, context).await?;
    let mut locations = normalize_locations(result);
    let truncated = locations.len() > limit;
    locations.truncate(limit);
    let texts = load_texts(
        manager,
        context,
        locations.iter().map(|(uri, _)| uri.as_str()),
    )
    .await;
    let rendered = locations
        .iter()
        .map(|(uri, range)| render_location(&root, uri, range, encoding, &texts))
        .collect();
    Ok((rendered, truncated))
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max.saturating_sub(3)).collect();
    truncated.push_str("...");
    truncated
}

// ---------------------------------------------------------------------------
// Tool implementations

async fn diagnostics(
    manager: &LspManager,
    context: &ToolContext,
    arguments: &Value,
) -> Result<Value> {
    let path = required_str(arguments, "path")?;
    let wait = arguments
        .get("wait_ms")
        .and_then(Value::as_u64)
        .map(|ms| Duration::from_millis(ms.clamp(100, MAX_DIAGNOSTICS_WAIT_MS)))
        .unwrap_or(manager.diagnostics_wait);
    let doc = manager.open(context, path).await?;
    let supports_pull = doc
        .server
        .capabilities
        .get("diagnosticProvider")
        .is_some_and(|provider| !provider.is_null());

    let mut items: Vec<Value> = Vec::new();
    let mut methods: Vec<&str> = Vec::new();
    if supports_pull {
        let report = doc
            .server
            .client
            .request(
                "textDocument/diagnostic",
                json!({ "textDocument": { "uri": doc.uri } }),
            )
            .await?;
        if let Some(pulled) = report.get("items").and_then(Value::as_array) {
            items.extend(pulled.iter().cloned());
        }
        methods.push("pull");
    }

    // Push diagnostics complement pull ones (e.g. rust-analyzer surfaces
    // cargo-check results only via publish). If our sync just changed the
    // document, give the server time to publish for the new content.
    let published = doc.server.client.published_diagnostics(&doc.uri);
    let need_fresh = doc.outcome != client::SyncOutcome::Unchanged || published.is_none();
    let (published, fresh) = if need_fresh && !supports_pull {
        let after = published.map_or(0, |entry| entry.generation);
        doc.server
            .client
            .wait_for_diagnostics(&doc.uri, after, wait)
            .await
    } else {
        (published, true)
    };
    if let Some(entry) = &published {
        methods.push("publish");
        for diagnostic in &entry.diagnostics {
            // Dedup pull/publish overlap on (range, message).
            let duplicate = items.iter().any(|existing| {
                existing.get("range") == diagnostic.get("range")
                    && existing.get("message") == diagnostic.get("message")
            });
            if !duplicate {
                items.push(diagnostic.clone());
            }
        }
    }

    let total = items.len();
    let rendered: Vec<Value> = items
        .iter()
        .take(MAX_DIAGNOSTICS)
        .map(|diagnostic| render_diagnostic(diagnostic, &doc.text, doc.server.encoding))
        .collect();
    Ok(json!({
        "ok": true,
        "path": path,
        "count": total,
        "truncated": total > MAX_DIAGNOSTICS,
        "methods": methods,
        // False when no publish arrived in time: results may lag the last edit.
        "fresh": fresh,
        "diagnostics": rendered,
    }))
}

fn render_diagnostic(diagnostic: &Value, text: &str, encoding: PositionEncoding) -> Value {
    let severity = match diagnostic.get("severity").and_then(Value::as_u64) {
        Some(1) => "error",
        Some(2) => "warning",
        Some(3) => "information",
        Some(4) => "hint",
        _ => "unspecified",
    };
    let (line, column) = diagnostic
        .pointer("/range/start")
        .map(|start| from_lsp_position(text, start, encoding))
        .unwrap_or((1, 1));
    let (end_line, end_column) = diagnostic
        .pointer("/range/end")
        .map(|end| from_lsp_position(text, end, encoding))
        .unwrap_or((line, column));
    let mut rendered = json!({
        "severity": severity,
        "line": line,
        "column": column,
        "end_line": end_line,
        "end_column": end_column,
        "message": diagnostic.get("message").and_then(Value::as_str).unwrap_or(""),
    });
    if let Some(code) = diagnostic.get("code")
        && !code.is_null()
    {
        rendered["code"] = match code {
            Value::String(code) => json!(code),
            other => json!(other.to_string()),
        };
    }
    if let Some(source) = diagnostic.get("source").and_then(Value::as_str) {
        rendered["source"] = json!(source);
    }
    rendered
}

async fn definition(
    manager: &LspManager,
    context: &ToolContext,
    arguments: &Value,
) -> Result<Value> {
    let kind = arguments
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("definition");
    let method = match kind {
        "definition" => "textDocument/definition",
        "type_definition" => "textDocument/typeDefinition",
        "implementation" => "textDocument/implementation",
        "declaration" => "textDocument/declaration",
        other => bail!("unsupported definition kind: {other}"),
    };
    let (doc, position) = open_at(manager, context, arguments).await?;
    let result = request_retrying_warmup(
        &doc.server,
        method,
        json!({ "textDocument": { "uri": doc.uri }, "position": position }),
        no_locations,
    )
    .await?;
    let (locations, truncated) = render_location_list(
        manager,
        context,
        &result,
        doc.server.encoding,
        MAX_LOCATIONS,
    )
    .await?;
    Ok(json!({
        "ok": true,
        "kind": kind,
        "count": locations.len(),
        "truncated": truncated,
        "locations": locations,
    }))
}

async fn references(
    manager: &LspManager,
    context: &ToolContext,
    arguments: &Value,
) -> Result<Value> {
    let include_declaration = arguments
        .get("include_declaration")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .map(|limit| (limit as usize).clamp(1, MAX_LOCATIONS))
        .unwrap_or(DEFAULT_REFERENCE_LIMIT);
    let (doc, position) = open_at(manager, context, arguments).await?;
    let result = request_retrying_warmup(
        &doc.server,
        "textDocument/references",
        json!({
            "textDocument": { "uri": doc.uri },
            "position": position,
            "context": { "includeDeclaration": include_declaration },
        }),
        no_locations,
    )
    .await?;
    let (locations, truncated) =
        render_location_list(manager, context, &result, doc.server.encoding, limit).await?;
    Ok(json!({
        "ok": true,
        "count": locations.len(),
        "truncated": truncated,
        "locations": locations,
    }))
}

async fn hover(manager: &LspManager, context: &ToolContext, arguments: &Value) -> Result<Value> {
    let (doc, position) = open_at(manager, context, arguments).await?;
    let result = doc
        .server
        .client
        .request(
            "textDocument/hover",
            json!({ "textDocument": { "uri": doc.uri }, "position": position }),
        )
        .await?;
    if result.is_null() {
        return Ok(
            json!({ "ok": true, "contents": Value::Null, "note": "no hover information at this position" }),
        );
    }
    let contents = hover_contents(result.get("contents").unwrap_or(&Value::Null));
    Ok(json!({
        "ok": true,
        "contents": truncate_chars(&contents, MAX_HOVER_CHARS),
    }))
}

/// Flatten `MarkedString | MarkedString[] | MarkupContent` to plain text.
fn hover_contents(contents: &Value) -> String {
    match contents {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(hover_contents)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(map) => map
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

async fn rename(manager: &LspManager, context: &ToolContext, arguments: &Value) -> Result<Value> {
    let new_name = required_str(arguments, "new_name")?;
    let dry_run = arguments
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (doc, position) = open_at(manager, context, arguments).await?;
    // Renames computed mid-analysis can silently miss cross-file references
    // (the exact failure this tool exists to prevent), so retry while the
    // server warms up and the edit stays empty or single-file.
    let edit = request_retrying_warmup(
        &doc.server,
        "textDocument/rename",
        json!({
            "textDocument": { "uri": doc.uri },
            "position": position,
            "newName": new_name,
        }),
        rename_looks_incomplete,
    )
    .await?;
    if edit.is_null() {
        bail!("the server returned no rename edit (symbol may not be renameable here)");
    }
    if dry_run {
        let preview = render_edit_preview(manager, context, &edit, doc.server.encoding).await?;
        return Ok(json!({
            "ok": true,
            "applied": false,
            "dry_run": true,
            "new_name": new_name,
            "files": preview,
        }));
    }
    let summaries =
        apply_workspace_edit(manager, context, &edit, doc.server.encoding, &doc.server).await?;
    let total_edits: usize = summaries.iter().map(|summary| summary.edits).sum();
    Ok(json!({
        "ok": true,
        "applied": true,
        "new_name": new_name,
        "files_changed": summaries,
        "total_edits": total_edits,
    }))
}

/// Per-file edit preview for `dry_run` renames and listed code actions.
async fn render_edit_preview(
    manager: &LspManager,
    context: &ToolContext,
    edit: &Value,
    encoding: PositionEncoding,
) -> Result<Vec<Value>> {
    let root = workspace_root(manager, context).await?;
    // (uri, edits) per touched document, or the raw resource operation.
    let mut entries: Vec<Result<(&str, &[Value]), Value>> = Vec::new();
    if let Some(document_changes) = edit.get("documentChanges").and_then(Value::as_array) {
        for change in document_changes {
            if let Some(kind) = change.get("kind").and_then(Value::as_str) {
                entries.push(Err(json!({ "operation": kind, "raw": change })));
            } else if let (Some(uri), Some(edits)) = (
                change.pointer("/textDocument/uri").and_then(Value::as_str),
                change.get("edits").and_then(Value::as_array),
            ) {
                entries.push(Ok((uri, edits.as_slice())));
            }
        }
    } else if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
        let mut uris: Vec<&String> = changes.keys().collect();
        uris.sort();
        for uri in uris {
            if let Some(edits) = changes[uri].as_array() {
                entries.push(Ok((uri.as_str(), edits.as_slice())));
            }
        }
    }
    let texts = load_texts(
        manager,
        context,
        entries
            .iter()
            .filter_map(|entry| entry.as_ref().ok().map(|(uri, _)| *uri)),
    )
    .await;
    Ok(entries
        .into_iter()
        .map(|entry| match entry {
            Ok((uri, edits)) => preview_document_edits(&root, uri, edits, encoding, &texts),
            Err(operation) => operation,
        })
        .collect())
}

fn preview_document_edits(
    root: &Path,
    uri: &str,
    edits: &[Value],
    encoding: PositionEncoding,
    texts: &Texts,
) -> Value {
    let Ok(file) = uri_to_path(uri) else {
        return json!({ "uri": uri, "edits": edits.len() });
    };
    let text = texts.get(uri).map(String::as_str).unwrap_or_default();
    let path = file
        .strip_prefix(root)
        .unwrap_or(&file)
        .to_string_lossy()
        .replace('\\', "/");
    let rendered: Vec<Value> = edits
        .iter()
        .map(|edit| {
            let (line, column) = edit
                .pointer("/range/start")
                .map(|start| from_lsp_position(text, start, encoding))
                .unwrap_or((1, 1));
            let (end_line, end_column) = edit
                .pointer("/range/end")
                .map(|end| from_lsp_position(text, end, encoding))
                .unwrap_or((line, column));
            json!({
                "line": line,
                "column": column,
                "end_line": end_line,
                "end_column": end_column,
                "new_text": truncate_chars(
                    edit.get("newText").and_then(Value::as_str).unwrap_or(""),
                    MAX_PREVIEW_CHARS,
                ),
            })
        })
        .collect();
    json!({ "path": path, "edits": rendered })
}

async fn symbols(manager: &LspManager, context: &ToolContext, arguments: &Value) -> Result<Value> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty());
    match (path, query) {
        (Some(path), _) => {
            let doc = manager.open(context, path).await?;
            let result = doc
                .server
                .client
                .request(
                    "textDocument/documentSymbol",
                    json!({ "textDocument": { "uri": doc.uri } }),
                )
                .await?;
            let root = workspace_root(manager, context).await?;
            let items = result.as_array().map(Vec::as_slice).unwrap_or(&[]);
            // Only the flat `SymbolInformation` shape names other files.
            let texts = load_texts(manager, context, symbol_uris(items)).await;
            let mut symbols = Vec::new();
            flatten_document_symbols(
                items,
                None,
                &doc.text,
                doc.server.encoding,
                &root,
                &texts,
                &mut symbols,
            );
            let total = symbols.len();
            symbols.truncate(MAX_SYMBOLS);
            Ok(json!({
                "ok": true,
                "path": path,
                "count": total,
                "truncated": total > MAX_SYMBOLS,
                "symbols": symbols,
            }))
        }
        (None, Some(query)) => {
            // Workspace search needs a server; pick it via hint_path, or fall
            // back to any server that is already running.
            let hint = arguments
                .get("hint_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|hint| !hint.is_empty());
            let server = match hint {
                Some(hint) => manager.open(context, hint).await?.server,
                None => manager.any_running_server().await.context(
                    "workspace symbol search needs `hint_path` (any file of the target \
                     language) when no language server is running yet",
                )?,
            };
            let result = request_retrying_warmup(
                &server,
                "workspace/symbol",
                json!({ "query": query }),
                |result| result.as_array().is_none_or(Vec::is_empty),
            )
            .await?;
            let root = workspace_root(manager, context).await?;
            let items = result.as_array().cloned().unwrap_or_default();
            let total = items.len();
            let shown = &items[..total.min(MAX_SYMBOLS)];
            let texts = load_texts(manager, context, symbol_uris(shown)).await;
            let symbols: Vec<Value> = shown
                .iter()
                .map(|symbol| render_symbol_information(symbol, server.encoding, &root, &texts))
                .collect();
            Ok(json!({
                "ok": true,
                "query": query,
                "count": total,
                "truncated": total > MAX_SYMBOLS,
                "symbols": symbols,
            }))
        }
        (None, None) => bail!("provide `path` (file outline) or `query` (workspace search)"),
    }
}

const SYMBOL_KINDS: &[&str] = &[
    "File",
    "Module",
    "Namespace",
    "Package",
    "Class",
    "Method",
    "Property",
    "Field",
    "Constructor",
    "Enum",
    "Interface",
    "Function",
    "Variable",
    "Constant",
    "String",
    "Number",
    "Boolean",
    "Array",
    "Object",
    "Key",
    "Null",
    "EnumMember",
    "Struct",
    "Event",
    "Operator",
    "TypeParameter",
];

/// URIs named by flat `SymbolInformation` items.
fn symbol_uris(items: &[Value]) -> impl Iterator<Item = &str> {
    items
        .iter()
        .filter_map(|item| item.pointer("/location/uri").and_then(Value::as_str))
}

fn symbol_kind_name(kind: Option<u64>) -> &'static str {
    kind.and_then(|kind| SYMBOL_KINDS.get((kind as usize).saturating_sub(1)))
        .copied()
        .unwrap_or("Unknown")
}

/// Flatten hierarchical `DocumentSymbol[]` (or flat `SymbolInformation[]`)
/// into rows with a `container` breadcrumb.
fn flatten_document_symbols(
    items: &[Value],
    container: Option<&str>,
    text: &str,
    encoding: PositionEncoding,
    root: &Path,
    texts: &Texts,
    out: &mut Vec<Value>,
) {
    for item in items {
        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
        let kind = symbol_kind_name(item.get("kind").and_then(Value::as_u64));
        if let Some(range) = item.get("selectionRange").or_else(|| item.get("range")) {
            // Hierarchical DocumentSymbol.
            let (line, column) = range
                .get("start")
                .map(|start| from_lsp_position(text, start, encoding))
                .unwrap_or((1, 1));
            let mut rendered =
                json!({ "name": name, "kind": kind, "line": line, "column": column });
            if let Some(container) = container {
                rendered["container"] = json!(container);
            }
            out.push(rendered);
            if let Some(children) = item.get("children").and_then(Value::as_array) {
                let breadcrumb = match container {
                    Some(parent) => format!("{parent}::{name}"),
                    None => name.to_string(),
                };
                flatten_document_symbols(
                    children,
                    Some(&breadcrumb),
                    text,
                    encoding,
                    root,
                    texts,
                    out,
                );
            }
        } else {
            // Flat SymbolInformation (has a location instead).
            out.push(render_symbol_information(item, encoding, root, texts));
        }
    }
}

fn render_symbol_information(
    symbol: &Value,
    encoding: PositionEncoding,
    root: &Path,
    texts: &Texts,
) -> Value {
    let name = symbol.get("name").and_then(Value::as_str).unwrap_or("");
    let kind = symbol_kind_name(symbol.get("kind").and_then(Value::as_u64));
    let mut rendered = json!({ "name": name, "kind": kind });
    if let Some(container) = symbol.get("containerName").and_then(Value::as_str)
        && !container.is_empty()
    {
        rendered["container"] = json!(container);
    }
    if let (Some(uri), Some(range)) = (
        symbol.pointer("/location/uri").and_then(Value::as_str),
        symbol.pointer("/location/range"),
    ) {
        let location = render_location(root, uri, range, encoding, texts);
        for key in ["path", "line", "column", "external"] {
            if let Some(value) = location.get(key) {
                rendered[key] = value.clone();
            }
        }
    }
    rendered
}

async fn code_actions(
    manager: &LspManager,
    context: &ToolContext,
    arguments: &Value,
) -> Result<Value> {
    let (line, column) = required_position(arguments)?;
    let end_line = arguments
        .get("end_line")
        .and_then(Value::as_u64)
        .map(|line| line as usize)
        .unwrap_or(line);
    let end_column = arguments
        .get("end_column")
        .and_then(Value::as_u64)
        .map(|column| column as usize)
        .unwrap_or(column);
    let apply_title = arguments
        .get("apply")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty());

    let path = required_str(arguments, "path")?;
    let doc = manager.open(context, path).await?;
    let start = to_lsp_position(&doc.text, line, column, doc.server.encoding)?;
    let end = to_lsp_position(&doc.text, end_line, end_column, doc.server.encoding)?;
    // Servers expect the current diagnostics overlapping the range as context.
    let range = json!({ "start": start, "end": end });
    let overlapping: Vec<Value> = doc
        .server
        .client
        .published_diagnostics(&doc.uri)
        .map(|entry| entry.diagnostics)
        .unwrap_or_default()
        .into_iter()
        .filter(|diagnostic| ranges_overlap(diagnostic.get("range"), &range))
        .collect();
    let result = doc
        .server
        .client
        .request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": doc.uri },
                "range": range,
                "context": { "diagnostics": overlapping },
            }),
        )
        .await?;
    let actions = result.as_array().cloned().unwrap_or_default();

    let listed: Vec<Value> = actions
        .iter()
        .map(|action| {
            json!({
                "title": action.get("title").and_then(Value::as_str).unwrap_or(""),
                "kind": action.get("kind").and_then(Value::as_str).unwrap_or(""),
                "preferred": action.get("isPreferred").and_then(Value::as_bool).unwrap_or(false),
                "has_edit": action.get("edit").is_some_and(|edit| !edit.is_null()),
                "has_command": action.get("command").is_some_and(|command| !command.is_null()),
            })
        })
        .collect();

    let Some(title) = apply_title else {
        return Ok(json!({ "ok": true, "count": listed.len(), "actions": listed }));
    };

    let mut action = actions
        .iter()
        .find(|action| action.get("title").and_then(Value::as_str) == Some(title))
        .cloned()
        .with_context(|| format!("no code action titled `{title}` at this position"))?;
    if action.get("edit").is_none_or(Value::is_null) {
        // Lazily-computed edits are fetched via codeAction/resolve.
        action = doc
            .server
            .client
            .request("codeAction/resolve", action)
            .await?;
    }
    let edit = action.get("edit").filter(|edit| !edit.is_null()).context(
        "this code action carries a server command instead of a workspace edit; \
         it cannot be applied from here",
    )?;
    let summaries =
        apply_workspace_edit(manager, context, edit, doc.server.encoding, &doc.server).await?;
    let total_edits: usize = summaries.iter().map(|summary| summary.edits).sum();
    Ok(json!({
        "ok": true,
        "applied": title,
        "files_changed": summaries,
        "total_edits": total_edits,
    }))
}

fn ranges_overlap(diagnostic_range: Option<&Value>, range: &Value) -> bool {
    let Some(diagnostic_range) = diagnostic_range else {
        return false;
    };
    let point = |value: &Value, key: &str| -> (u64, u64) {
        (
            value
                .pointer(&format!("/{key}/line"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            value
                .pointer(&format!("/{key}/character"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
        )
    };
    // Overlap iff neither range ends before the other starts.
    point(diagnostic_range, "end") >= point(range, "start")
        && point(range, "end") >= point(diagnostic_range, "start")
}

#[cfg(all(test, feature = "local"))]
mod tests {
    use super::*;
    use crate::host::LocalCodeHost;
    use everruns_contracts::typed_id::SessionId;
    use manager::path_to_uri;
    use std::fs;
    use std::path::PathBuf;
    use tokio::io::duplex;

    fn host(path: &Path) -> Arc<dyn CodeHost> {
        Arc::new(LocalCodeHost::new(path))
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, content).expect("write file");
    }

    fn range(start_line: u64, start_char: u64, end_line: u64, end_char: u64) -> Value {
        json!({
            "start": { "line": start_line, "character": start_char },
            "end": { "line": end_line, "character": end_char },
        })
    }

    /// In-process fake language server covering every request the tools send.
    /// Speaks utf-8 positions so ASCII test fixtures assert 1:1. The first
    /// `references` request returns an empty result to exercise the warmup
    /// retry path — tools must transparently ask again and get the real set.
    async fn fake_server(transport: tokio::io::DuplexStream, root: PathBuf) {
        let (read_half, mut write_half) = tokio::io::split(transport);
        let mut reader = tokio::io::BufReader::new(read_half);
        let uri = |name: &str| path_to_uri(&root.join(name));
        let mut references_requests = 0u32;
        let diagnostic = json!({
            "range": range(0, 3, 0, 6),
            "severity": 1,
            "message": "mock error",
            "source": "fake",
            "code": "E999",
        });
        while let Ok(Some(message)) = client::read_message(&mut reader).await {
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            let method = message.get("method").and_then(Value::as_str).unwrap_or("");
            let result = match method {
                "initialize" => json!({
                    "capabilities": {
                        "positionEncoding": "utf-8",
                        "diagnosticProvider": {},
                        "renameProvider": true,
                    }
                }),
                "textDocument/didOpen" => {
                    let doc_uri = message["params"]["textDocument"]["uri"].clone();
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/publishDiagnostics",
                        "params": { "uri": doc_uri, "diagnostics": [diagnostic.clone()] },
                    });
                    client::write_message(&mut write_half, &notification)
                        .await
                        .expect("publish");
                    continue;
                }
                "initialized" | "textDocument/didChange" => continue,
                "textDocument/diagnostic" => {
                    json!({ "kind": "full", "items": [diagnostic.clone()] })
                }
                "textDocument/definition" => json!([{
                    "uri": uri("target.rs"),
                    "range": range(0, 3, 0, 12),
                }]),
                "textDocument/references" => {
                    references_requests += 1;
                    if references_requests == 1 {
                        // Warmup artifact: empty-but-successful first answer.
                        json!([])
                    } else {
                        json!([
                            { "uri": uri("main.rs"), "range": range(0, 3, 0, 6) },
                            { "uri": uri("main.rs"), "range": range(1, 14, 1, 17) },
                        ])
                    }
                }
                "textDocument/hover" => json!({
                    "contents": { "kind": "markdown", "value": "```rust\nfn old()\n```" }
                }),
                "textDocument/rename" => json!({
                    "changes": {
                        uri("main.rs"): [
                            { "range": range(0, 3, 0, 6), "newText": "renamed" },
                            { "range": range(1, 14, 1, 17), "newText": "renamed" },
                        ],
                        uri("other.rs"): [
                            { "range": range(0, 20, 0, 23), "newText": "renamed" },
                        ],
                    }
                }),
                "textDocument/documentSymbol" => json!([{
                    "name": "S",
                    "kind": 23,
                    "range": range(0, 0, 2, 1),
                    "selectionRange": range(0, 7, 0, 8),
                    "children": [{
                        "name": "m",
                        "kind": 6,
                        "range": range(1, 4, 1, 20),
                        "selectionRange": range(1, 7, 1, 8),
                    }],
                }]),
                "workspace/symbol" => json!([{
                    "name": "target_fn",
                    "kind": 12,
                    "location": { "uri": uri("target.rs"), "range": range(0, 3, 0, 12) },
                    "containerName": "target",
                }]),
                "textDocument/codeAction" => json!([
                    {
                        "title": "Insert comment",
                        "kind": "quickfix",
                        "isPreferred": true,
                        "edit": {
                            "changes": {
                                uri("main.rs"): [
                                    { "range": range(0, 0, 0, 0), "newText": "// fixed\n" }
                                ]
                            }
                        }
                    },
                    { "title": "Run command", "kind": "refactor", "command": { "command": "x" } }
                ]),
                other => {
                    if !id.is_null() {
                        let response = json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": format!("fake: {other}") },
                        });
                        client::write_message(&mut write_half, &response)
                            .await
                            .expect("error response");
                    }
                    continue;
                }
            };
            let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
            client::write_message(&mut write_half, &response)
                .await
                .expect("response");
        }
    }

    /// The tools of one workspace, wired to the in-process fake server.
    struct TestTools {
        host: Arc<dyn CodeHost>,
        workspaces: Arc<Workspaces>,
    }

    fn test_manager(root: &Path) -> TestTools {
        let canonical = root.canonicalize().expect("canonicalize test root");
        let host = host(&canonical);
        let scope = host.scope_key(&ToolContext::new(SessionId::new()));
        let manager = LspManager::new(
            host.clone(),
            scope.clone(),
            default_server_specs(),
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .with_transport_factory(Box::new(move |_spec| {
            let (client_transport, server_transport) = duplex(1 << 20);
            tokio::spawn(fake_server(server_transport, canonical.clone()));
            let (read_half, write_half) = tokio::io::split(client_transport);
            Ok((Box::new(read_half) as _, Box::new(write_half) as _))
        }));
        let workspaces = Arc::new(Workspaces::default());
        workspaces.0.lock().expect("workspaces lock").insert(
            scope,
            Workspace {
                config: Value::Null,
                manager: Arc::new(manager),
                last_used: Instant::now(),
            },
        );
        TestTools { host, workspaces }
    }

    fn tool(tools: &TestTools, action: LspAction) -> LspTool {
        LspTool {
            host: tools.host.clone(),
            workspaces: tools.workspaces.clone(),
            config: Value::Null,
            action,
        }
    }

    fn seed_workspace(root: &Path) {
        write(
            &root.join("main.rs"),
            "fn old() {}\nfn caller() { old(); }\n",
        );
        write(&root.join("other.rs"), "fn other() { crate::old(); }\n");
        write(&root.join("target.rs"), "fn target_fn() {}\n");
    }

    async fn run_tool(tools: &TestTools, action: LspAction, arguments: Value) -> Value {
        let result = tool(tools, action)
            .execute_with_context(arguments, &ToolContext::new(SessionId::new()))
            .await;
        match result {
            ToolExecutionResult::Success(value) => value,
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn definition_returns_workspace_relative_location_with_preview() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let value = run_tool(
            &manager,
            LspAction::Definition,
            json!({ "path": "main.rs", "line": 2, "column": 15 }),
        )
        .await;
        assert_eq!(value["count"], 1);
        let location = &value["locations"][0];
        assert_eq!(location["path"], "target.rs");
        assert_eq!(location["line"], 1);
        assert_eq!(location["column"], 4);
        assert_eq!(location["end_column"], 13);
        assert_eq!(location["preview"], "fn target_fn() {}");
        assert!(location.get("external").is_none());
    }

    #[tokio::test]
    async fn references_lists_all_sites() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let value = run_tool(
            &manager,
            LspAction::References,
            json!({ "path": "main.rs", "line": 1, "column": 4 }),
        )
        .await;
        assert_eq!(value["count"], 2);
        assert_eq!(value["locations"][0]["path"], "main.rs");
        assert_eq!(value["locations"][1]["line"], 2);
        assert_eq!(value["truncated"], false);
    }

    #[tokio::test]
    async fn hover_flattens_markup_contents() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let value = run_tool(
            &manager,
            LspAction::Hover,
            json!({ "path": "main.rs", "line": 1, "column": 4 }),
        )
        .await;
        assert_eq!(value["contents"], "```rust\nfn old()\n```");
    }

    #[tokio::test]
    async fn diagnostics_merges_pull_and_publish_without_duplicates() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let value = run_tool(
            &manager,
            LspAction::Diagnostics,
            json!({ "path": "main.rs" }),
        )
        .await;
        assert_eq!(
            value["count"], 1,
            "pull+publish overlap must dedup: {value}"
        );
        let diagnostic = &value["diagnostics"][0];
        assert_eq!(diagnostic["severity"], "error");
        assert_eq!(diagnostic["message"], "mock error");
        assert_eq!(diagnostic["line"], 1);
        assert_eq!(diagnostic["column"], 4);
        assert_eq!(diagnostic["code"], "E999");
        assert_eq!(diagnostic["source"], "fake");
    }

    #[tokio::test]
    async fn rename_applies_workspace_edit_across_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let value = run_tool(
            &manager,
            LspAction::Rename,
            json!({ "path": "main.rs", "line": 1, "column": 4, "new_name": "renamed" }),
        )
        .await;
        assert_eq!(value["applied"], true);
        assert_eq!(value["total_edits"], 3);
        let root = dir.path().canonicalize().expect("canonicalize");
        assert_eq!(
            fs::read_to_string(root.join("main.rs")).expect("read"),
            "fn renamed() {}\nfn caller() { renamed(); }\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("other.rs")).expect("read"),
            "fn other() { crate::renamed(); }\n"
        );
    }

    #[tokio::test]
    async fn rename_dry_run_previews_without_writing() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let value = run_tool(
            &manager,
            LspAction::Rename,
            json!({
                "path": "main.rs", "line": 1, "column": 4,
                "new_name": "renamed", "dry_run": true
            }),
        )
        .await;
        assert_eq!(value["applied"], false);
        assert_eq!(value["files"].as_array().map(Vec::len), Some(2));
        let root = dir.path().canonicalize().expect("canonicalize");
        assert_eq!(
            fs::read_to_string(root.join("main.rs")).expect("read"),
            "fn old() {}\nfn caller() { old(); }\n",
            "dry_run must not touch the file"
        );
    }

    #[tokio::test]
    async fn symbols_flattens_document_outline_with_containers() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let value = run_tool(&manager, LspAction::Symbols, json!({ "path": "main.rs" })).await;
        assert_eq!(value["count"], 2);
        assert_eq!(value["symbols"][0]["name"], "S");
        assert_eq!(value["symbols"][0]["kind"], "Struct");
        assert_eq!(value["symbols"][1]["name"], "m");
        assert_eq!(value["symbols"][1]["kind"], "Method");
        assert_eq!(value["symbols"][1]["container"], "S");
    }

    #[tokio::test]
    async fn workspace_symbol_search_uses_hint_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let value = run_tool(
            &manager,
            LspAction::Symbols,
            json!({ "query": "target", "hint_path": "main.rs" }),
        )
        .await;
        assert_eq!(value["count"], 1);
        assert_eq!(value["symbols"][0]["name"], "target_fn");
        assert_eq!(value["symbols"][0]["kind"], "Function");
        assert_eq!(value["symbols"][0]["path"], "target.rs");
        assert_eq!(value["symbols"][0]["container"], "target");
    }

    #[tokio::test]
    async fn code_actions_lists_and_applies_by_title() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_workspace(dir.path());
        let manager = test_manager(dir.path());

        let listed = run_tool(
            &manager,
            LspAction::CodeActions,
            json!({ "path": "main.rs", "line": 1, "column": 4 }),
        )
        .await;
        assert_eq!(listed["count"], 2);
        assert_eq!(listed["actions"][0]["title"], "Insert comment");
        assert_eq!(listed["actions"][0]["has_edit"], true);
        assert_eq!(listed["actions"][1]["has_command"], true);

        let applied = run_tool(
            &manager,
            LspAction::CodeActions,
            json!({ "path": "main.rs", "line": 1, "column": 4, "apply": "Insert comment" }),
        )
        .await;
        assert_eq!(applied["applied"], "Insert comment");
        let root = dir.path().canonicalize().expect("canonicalize");
        assert!(
            fs::read_to_string(root.join("main.rs"))
                .expect("read")
                .starts_with("// fixed\n"),
            "the quickfix edit should be applied"
        );
    }

    #[tokio::test]
    async fn unsupported_extension_yields_helpful_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("notes.zig"), "const x = 1;\n");
        let manager = test_manager(dir.path());

        let result = tool(&manager, LspAction::Hover)
            .execute_with_context(
                json!({ "path": "notes.zig", "line": 1, "column": 1 }),
                &ToolContext::new(SessionId::new()),
            )
            .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected tool error, got {result:?}");
        };
        assert!(
            message.contains("no language server configured"),
            "{message}"
        );
    }

    #[test]
    fn resolve_specs_merges_overrides() {
        let settings = parse_settings(&json!({
            "servers": {
                "rust": { "command": "my-ra", "args": ["--custom"] },
                "python": { "enabled": false },
                "zig": { "command": "zls", "extensions": [".zig"] },
            }
        }))
        .expect("parse");
        let specs = resolve_specs(&settings).expect("resolve");
        let rust = specs.iter().find(|spec| spec.key == "rust").expect("rust");
        assert_eq!(rust.command, "my-ra");
        assert_eq!(rust.args, vec!["--custom"]);
        assert!(!specs.iter().any(|spec| spec.key == "python"));
        let zig = specs.iter().find(|spec| spec.key == "zig").expect("zig");
        assert_eq!(zig.extensions, vec!["zig"]);
    }

    #[test]
    fn resolve_specs_rejects_new_server_without_command() {
        let settings =
            parse_settings(&json!({ "servers": { "zig": { "args": ["x"] } } })).expect("parse");
        let err = resolve_specs(&settings).expect_err("missing command");
        assert!(err.contains("requires a `command`"), "{err}");
    }

    #[test]
    fn capability_validates_config_and_exposes_seven_tools() {
        let dir = tempfile::tempdir().expect("tempdir");
        let capability = LspCapability::new(host(dir.path()));

        assert!(
            capability
                .validate_config(&json!({ "bogus": true }))
                .is_err()
        );
        assert!(
            capability
                .validate_config(&json!({ "request_timeout_ms": 30000 }))
                .is_ok()
        );

        let tools = capability.tools();
        let mut names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "lsp_code_actions",
                "lsp_definition",
                "lsp_diagnostics",
                "lsp_hover",
                "lsp_references",
                "lsp_rename",
                "lsp_symbols",
            ]
        );
        for tool in &tools {
            assert_eq!(tool.parameters_schema()["type"], "object");
        }
    }
}
