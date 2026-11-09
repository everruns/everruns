//! On-demand symbol maps for broad repository orientation.
//!
//! This capability is intentionally read-only and scan-on-demand. Grep/read
//! remain the hot path; repo-map gives the model a compact structural view when
//! lexical search is too narrow.

use crate::failure::{blocking, into_tool_result};
use crate::host::{CodeHost, SourceTree, resolve_scope};
use crate::narration::stable_labeled;
use crate::source::SourceFilter;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use everruns_contracts::narration::ToolNarrationPhase;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityStatus, SystemPromptContext,
};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_narration::{arg_str, truncate};
use everruns_contracts::runtime::tools::Tool;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::tools::ToolExecutionResult;
use serde::Serialize;
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tree_sitter::{Language, Node, Parser};

pub const REPO_MAP_CAPABILITY_ID: &str = "repo_map";

const DEFAULT_UNQUERIED_LIMIT: usize = 50;
const DEFAULT_QUERIED_LIMIT: usize = 200;
const MAX_LIMIT: usize = 1000;
const DEFAULT_MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SIGNATURE_CHARS: usize = 180;
// When a query is supplied we collect every matching symbol before ranking, so
// the candidate pool is bounded independently of `limit` to keep ranking work
// predictable on large workspaces.
const MAX_CANDIDATES: usize = 5000;
// Non-hidden directories that are dependency/build output, not source. Hidden
// entries (any name starting with `.` — `.git`, `.cache`, `.venv`, …) are
// skipped separately, so they don't need listing here.
const SKIP_DIRS: &[&str] = &["target", "node_modules", "dist", "build", "venv"];

#[derive(Clone)]
pub struct RepoMapCapability {
    host: Arc<dyn CodeHost>,
}

impl RepoMapCapability {
    pub fn new(host: Arc<dyn CodeHost>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Capability for RepoMapCapability {
    fn id(&self) -> &str {
        REPO_MAP_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Repo Map"
    }

    fn description(&self) -> &str {
        "Read-only, on-demand multi-language symbol map for broad codebase orientation."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Code Navigation")
    }

    /// Only the truncation rule. Both tool descriptions already position the map
    /// as orientation before targeted grep/read, so that half of the old block
    /// stayed deleted.
    ///
    /// The truncation advice came back because deleting it regressed
    /// `evals/harness_basic`'s `repo-map-bounded` gate on gpt-5.5:
    /// `duplicate_exploration_calls` breached its zero bar in 5/5 trials against
    /// 2/5 before, with one extra tool call per run. `truncation_metadata` puts
    /// the same words in the truncated result, but by then the model has already
    /// decided what to do next — the reactive copy does not substitute for the
    /// anticipatory one. Correctness never broke; only efficiency did.
    ///
    /// Measured on claude-sonnet-4-5 too, where it costs nothing: that model
    /// recovers cleanly either way (`duplicate_exploration_calls` 0 in all
    /// trials, both binaries).
    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        Some(
            "<capability id=\"repo_map\">\n\
             If a `repo_map` or `repo_symbols` result is truncated, do not repeat the same \
             call: add `query` and narrow `path`; do not just raise `limit`.\n\
             </capability>"
                .to_string(),
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(RepoMapTool {
                host: self.host.clone(),
            }),
            Box::new(RepoSymbolsTool {
                host: self.host.clone(),
            }),
        ]
    }
}

struct RepoMapTool {
    host: Arc<dyn CodeHost>,
}

#[async_trait]
impl Tool for RepoMapTool {
    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_contracts::runtime::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let _ = locale;
        let query = scan_narration_query(&tool_call.arguments);
        Some(stable_labeled("Build repo map", query, phase))
    }

    fn name(&self) -> &str {
        "repo_map"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Repo map")
    }

    // Orientation note: without this, the model pages through large files with sed
    // or shells out to grep for a first look. State the entry point explicitly:
    // map first, then read the targeted regions.
    fn description(&self) -> &str {
        "Build a compact, grouped multi-language symbol map for the workspace or a subpath. \
         Prefer a queried map narrowed to a subpath for orientation; an unqueried workspace-root \
         map truncates alphabetically on large workspaces. The limit defaults to 50 without `query` \
         and 200 with it; `max_file_bytes` defaults to 524288. Use this for broad orientation before targeted grep/read. \
         Start here for unfamiliar code instead of paging through large files or \
         shelling out for a first look."
    }

    fn parameters_schema(&self) -> Value {
        compact_scan_schema()
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("repo_map needs the session it runs in")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        match run_scan(self.host.as_ref(), arguments, context, true).await {
            Ok(value) => ToolExecutionResult::success(value),
            Err(err) => into_tool_result(err),
        }
    }
}

struct RepoSymbolsTool {
    host: Arc<dyn CodeHost>,
}

#[async_trait]
impl Tool for RepoSymbolsTool {
    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_contracts::runtime::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let _ = locale;
        let query = scan_narration_query(&tool_call.arguments);
        Some(stable_labeled("Search symbols", query, phase))
    }

    fn name(&self) -> &str {
        "repo_symbols"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Repo symbols")
    }

    fn description(&self) -> &str {
        "Return a flat list of symbols from the workspace or a subpath, optionally \
         filtered by query. Use when you need specific symbol candidates."
    }

    fn parameters_schema(&self) -> Value {
        scan_schema(
            "Optional space-separated terms. Symbols matching any term (in name, path, kind, parent, or signature) are returned, ranked by relevance with the strongest matches first.",
        )
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("repo_symbols needs the session it runs in")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        match run_scan(self.host.as_ref(), arguments, context, false).await {
            Ok(value) => ToolExecutionResult::success(value),
            Err(err) => into_tool_result(err),
        }
    }
}

/// Scan the requested scope and render the report, grouped by file for
/// `repo_map` and flat for `repo_symbols`.
async fn run_scan(
    host: &dyn CodeHost,
    mut arguments: Value,
    context: &ToolContext,
    grouped: bool,
) -> Result<Value> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string);
    let filter = SourceFilter::new(
        LANGUAGES
            .iter()
            .flat_map(|language| language.extensions.iter().copied()),
        LANGUAGES
            .iter()
            .flat_map(|language| language.filenames.iter().copied()),
    );
    let tree: SourceTree = host.materialize(context, path.as_deref(), &filter).await?;
    if let Some(object) = arguments.as_object_mut() {
        match &tree.scope {
            Some(scope) => object.insert("path".to_string(), json!(scope)),
            None => object.remove("path"),
        };
    }
    let root = tree.root.clone();
    let report = blocking(move || scan_from_arguments(&root, &arguments)).await?;
    let mut value = json!({
        "ok": true,
        "workspace_root": tree.display_root,
        "path": report.path,
        "query": report.query,
        "languages": report.languages,
        "count": report.symbols.len(),
        "scanned_files": report.scanned_files,
        "skipped_unsupported_files": report.skipped_unsupported_files,
        "skipped_files": report.skipped_files,
        "skipped_large_files": report.skipped_large_files,
        "parse_error_files": report.parse_error_files,
        "truncated": report.truncated,
        "truncation": truncation_metadata(&report),
    });
    if grouped {
        value["files"] = json!(grouped_symbols(&report.symbols));
    } else {
        value["symbols"] = json!(report.symbols);
    }
    if let Some(note) = &tree.note {
        value["note"] = json!(note);
    }
    Ok(value)
}

fn scan_narration_query(arguments: &serde_json::Value) -> Option<String> {
    arg_str(arguments, &["query", "path"]).map(|v| truncate(v, 48))
}

// repo_map is eager because models need its real argument names on the first
// turn. Keep the always-on schema to authoritative types and bounds; the tool
// descriptions own when and why to use the map, while result metadata owns
// truncation recovery.
fn compact_scan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "query": { "type": "string" },
            "language": { "type": "string" },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LIMIT
            },
            "max_file_bytes": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_FILE_BYTES
            }
        },
        "additionalProperties": false
    })
}

fn scan_schema(query_description: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Optional workspace-relative file or directory to scan. Defaults to the workspace root."
            },
            "query": {
                "type": "string",
                "description": query_description
            },
            "language": {
                "type": "string",
                "description": "Optional language filter. Supported values include rust, python, typescript, tsx, javascript, csharp, go, zig, css, html, bash, sql, java, c, cpp, php, ruby, kotlin, and scala."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LIMIT,
                "description": "Maximum number of matching symbols to return. Defaults to 50 for an unqueried map and 200 when query is provided."
            },
            "max_file_bytes": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_FILE_BYTES,
                "description": "Skip supported source files larger than this many bytes. Defaults to 524288."
            }
        },
        "additionalProperties": false
    })
}

fn scan_from_arguments(workspace_root: &Path, arguments: &Value) -> Result<SymbolScanReport> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string);
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_string);
    let language = arguments
        .get("language")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(str::to_string);
    let limit = bounded_usize(
        arguments,
        "limit",
        default_limit(query.as_deref()),
        MAX_LIMIT,
    )?;
    let max_file_bytes = bounded_usize(
        arguments,
        "max_file_bytes",
        DEFAULT_MAX_FILE_BYTES,
        MAX_FILE_BYTES,
    )?;

    collect_repo_symbols(
        workspace_root,
        SymbolScanOptions {
            path,
            query,
            language,
            limit,
            max_file_bytes,
        },
    )
}

fn default_limit(query: Option<&str>) -> usize {
    if query.is_some() {
        DEFAULT_QUERIED_LIMIT
    } else {
        DEFAULT_UNQUERIED_LIMIT
    }
}

fn truncation_metadata(report: &SymbolScanReport) -> Option<Value> {
    report.truncated.then(|| {
        json!({
            "reason": "symbol_limit",
            "suggestion": "Do not repeat this call unchanged. Add `query` and narrow `path`; do not just raise `limit`."
        })
    })
}

fn bounded_usize(arguments: &Value, key: &str, default: usize, max: usize) -> Result<usize> {
    let Some(raw) = arguments.get(key) else {
        return Ok(default);
    };
    let Some(value) = raw.as_u64() else {
        bail!("`{key}` must be a positive integer");
    };
    if value == 0 || value as usize > max {
        bail!("`{key}` must be between 1 and {max}");
    }
    Ok(value as usize)
}

#[derive(Clone, Debug)]
struct SymbolScanOptions {
    path: Option<String>,
    query: Option<String>,
    language: Option<String>,
    limit: usize,
    max_file_bytes: usize,
}

#[derive(Clone, Debug)]
struct SymbolScanReport {
    path: String,
    query: Option<String>,
    languages: Vec<String>,
    scanned_files: usize,
    skipped_unsupported_files: usize,
    skipped_files: usize,
    skipped_large_files: usize,
    parse_error_files: usize,
    truncated: bool,
    symbols: Vec<RepoSymbol>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct RepoSymbol {
    path: String,
    language: String,
    kind: String,
    name: String,
    parent: Option<String>,
    visibility: Option<String>,
    signature: String,
    line: usize,
    column: usize,
}

#[derive(Serialize)]
struct SymbolFileGroup<'a> {
    path: &'a str,
    symbols: Vec<&'a RepoSymbol>,
}

fn collect_repo_symbols(
    workspace_root: &Path,
    options: SymbolScanOptions,
) -> Result<SymbolScanReport> {
    let root = workspace_root
        .canonicalize()
        .with_context(|| format!("canonicalize workspace root {}", workspace_root.display()))?;
    let scope = resolve_scope(&root, options.path.as_deref())?;
    let language_filter = options.language.as_deref().map(normalize_language_filter);
    if let Some(filter) = language_filter.as_deref()
        && !LANGUAGES
            .iter()
            .any(|language| language.matches_filter(filter))
    {
        bail!("unsupported language `{filter}`");
    }
    let mut files = Vec::new();
    let mut skipped_unsupported_files = 0;
    collect_supported_files(
        scope.as_path(),
        language_filter.as_deref(),
        &mut files,
        &mut skipped_unsupported_files,
    )?;
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let query_terms = tokenize_query(options.query.as_deref());
    // Without a query every symbol matches, so cap collection at `limit` and
    // preserve scan order. With a query we gather the full candidate pool first
    // and rank it afterwards.
    let budget = if query_terms.is_empty() {
        options.limit
    } else {
        MAX_CANDIDATES
    };
    let mut report = SymbolScanReport {
        path: relative_path(&root, scope.as_path()),
        query: options.query,
        languages: Vec::new(),
        scanned_files: 0,
        skipped_unsupported_files,
        skipped_files: 0,
        skipped_large_files: 0,
        parse_error_files: 0,
        truncated: false,
        symbols: Vec::new(),
    };

    let mut parser = Parser::new();

    for file in files {
        if report.truncated {
            break;
        }
        let metadata = match fs::metadata(&file.path) {
            Ok(metadata) => metadata,
            Err(_) => {
                report.skipped_files += 1;
                continue;
            }
        };
        if metadata.len() as usize > options.max_file_bytes {
            report.skipped_large_files += 1;
            continue;
        }
        let source = match fs::read_to_string(&file.path) {
            Ok(source) => source,
            Err(_) => {
                report.skipped_files += 1;
                continue;
            }
        };
        parser
            .set_language(&file.language.language())
            .with_context(|| format!("load {} tree-sitter grammar", file.language.name))?;
        report.scanned_files += 1;
        let Some(tree) = parser.parse(&source, None) else {
            report.parse_error_files += 1;
            continue;
        };
        if tree.root_node().has_error() {
            report.parse_error_files += 1;
        }
        if !report
            .languages
            .iter()
            .any(|language| language == file.language.name)
        {
            report.languages.push(file.language.name.to_string());
        }
        let path = relative_path(&root, &file.path);
        let mut parents = Vec::new();
        let context = SymbolCollectContext {
            source: &source,
            path: &path,
            language: file.language,
            terms: &query_terms,
            budget,
        };
        collect_symbols_from_node(tree.root_node(), &context, &mut parents, &mut report);
    }

    if !query_terms.is_empty() {
        report.symbols = rank_symbols(std::mem::take(&mut report.symbols), &query_terms);
        if report.symbols.len() > options.limit {
            report.truncated = true;
            report.symbols.truncate(options.limit);
        }
    }

    Ok(report)
}

#[derive(Clone)]
struct SourceFile {
    path: PathBuf,
    language: &'static LanguageSpec,
}

struct LanguageSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    filenames: &'static [&'static str],
    grammar: fn() -> Language,
    symbol_kinds: &'static [KindMap],
    parent_kinds: &'static [&'static str],
}

impl LanguageSpec {
    fn language(&self) -> Language {
        (self.grammar)()
    }

    fn matches_filter(&self, filter: &str) -> bool {
        self.name == filter || self.aliases.contains(&filter)
    }

    fn symbol_kind(&self, node_kind: &str) -> Option<&'static str> {
        self.symbol_kinds
            .iter()
            .find(|kind| kind.node == node_kind)
            .map(|kind| kind.kind)
    }

    fn is_parent_kind(&self, node_kind: &str) -> bool {
        self.parent_kinds.contains(&node_kind)
    }
}

struct KindMap {
    node: &'static str,
    kind: &'static str,
}

const fn kind(node: &'static str, kind: &'static str) -> KindMap {
    KindMap { node, kind }
}

fn rust_language() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}

fn python_language() -> Language {
    tree_sitter_python::LANGUAGE.into()
}

fn javascript_language() -> Language {
    tree_sitter_javascript::LANGUAGE.into()
}

fn typescript_language() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}

fn tsx_language() -> Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}

fn csharp_language() -> Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}

fn go_language() -> Language {
    tree_sitter_go::LANGUAGE.into()
}

fn zig_language() -> Language {
    tree_sitter_zig::LANGUAGE.into()
}

fn css_language() -> Language {
    tree_sitter_css::LANGUAGE.into()
}

fn html_language() -> Language {
    tree_sitter_html::LANGUAGE.into()
}

fn bash_language() -> Language {
    tree_sitter_bash::LANGUAGE.into()
}

fn sql_language() -> Language {
    tree_sitter_sequel::LANGUAGE.into()
}

fn java_language() -> Language {
    tree_sitter_java::LANGUAGE.into()
}

fn c_language() -> Language {
    tree_sitter_c::LANGUAGE.into()
}

fn cpp_language() -> Language {
    tree_sitter_cpp::LANGUAGE.into()
}

fn php_language() -> Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}

fn ruby_language() -> Language {
    tree_sitter_ruby::LANGUAGE.into()
}

fn kotlin_language() -> Language {
    tree_sitter_kotlin_ng::LANGUAGE.into()
}

fn scala_language() -> Language {
    tree_sitter_scala::LANGUAGE.into()
}

const RUST_KINDS: &[KindMap] = &[
    kind("const_item", "const"),
    kind("enum_item", "enum"),
    kind("function_item", "function"),
    kind("function_signature_item", "function_signature"),
    kind("impl_item", "impl"),
    kind("macro_definition", "macro"),
    kind("mod_item", "module"),
    kind("static_item", "static"),
    kind("struct_item", "struct"),
    kind("trait_item", "trait"),
    kind("type_item", "type"),
];

const PYTHON_KINDS: &[KindMap] = &[
    kind("class_definition", "class"),
    kind("function_definition", "function"),
];

const JS_KINDS: &[KindMap] = &[
    kind("class_declaration", "class"),
    kind("function_declaration", "function"),
    kind("generator_function_declaration", "function"),
    kind("method_definition", "method"),
];

const TS_KINDS: &[KindMap] = &[
    kind("abstract_class_declaration", "class"),
    kind("class_declaration", "class"),
    kind("enum_declaration", "enum"),
    kind("function_declaration", "function"),
    kind("generator_function_declaration", "function"),
    kind("interface_declaration", "interface"),
    kind("internal_module", "module"),
    kind("method_definition", "method"),
    kind("type_alias_declaration", "type"),
];

const CSHARP_KINDS: &[KindMap] = &[
    kind("class_declaration", "class"),
    kind("constructor_declaration", "constructor"),
    kind("delegate_declaration", "delegate"),
    kind("enum_declaration", "enum"),
    kind("interface_declaration", "interface"),
    kind("method_declaration", "method"),
    kind("namespace_declaration", "namespace"),
    kind("property_declaration", "property"),
    kind("record_declaration", "record"),
    kind("struct_declaration", "struct"),
];

const GO_KINDS: &[KindMap] = &[
    kind("function_declaration", "function"),
    kind("method_declaration", "method"),
    kind("type_declaration", "type"),
];

const ZIG_KINDS: &[KindMap] = &[
    kind("enum_declaration", "enum"),
    kind("function_declaration", "function"),
    kind("struct_declaration", "struct"),
    kind("variable_declaration", "variable"),
];

const CSS_KINDS: &[KindMap] = &[
    kind("import_statement", "import"),
    kind("keyframes_statement", "keyframes"),
    kind("rule_set", "selector"),
];

const HTML_KINDS: &[KindMap] = &[kind("element", "element"), kind("script_element", "script")];

const BASH_KINDS: &[KindMap] = &[kind("function_definition", "function")];

const SQL_KINDS: &[KindMap] = &[
    kind("create_database", "database"),
    kind("create_extension", "extension"),
    kind("create_function", "function"),
    kind("create_index", "index"),
    kind("create_materialized_view", "view"),
    kind("create_schema", "schema"),
    kind("create_sequence", "sequence"),
    kind("create_table", "table"),
    kind("create_trigger", "trigger"),
    kind("create_type", "type"),
    kind("create_view", "view"),
    kind("function_declaration", "function"),
];

const JAVA_KINDS: &[KindMap] = &[
    kind("annotation_type_declaration", "annotation"),
    kind("class_declaration", "class"),
    kind("constructor_declaration", "constructor"),
    kind("enum_declaration", "enum"),
    kind("interface_declaration", "interface"),
    kind("method_declaration", "method"),
    kind("record_declaration", "record"),
];

const C_KINDS: &[KindMap] = &[
    kind("enum_specifier", "enum"),
    kind("function_definition", "function"),
    kind("struct_specifier", "struct"),
    kind("union_specifier", "union"),
];

const CPP_KINDS: &[KindMap] = &[
    kind("class_specifier", "class"),
    kind("enum_specifier", "enum"),
    kind("function_definition", "function"),
    kind("namespace_definition", "namespace"),
    kind("struct_specifier", "struct"),
    kind("union_specifier", "union"),
];

const PHP_KINDS: &[KindMap] = &[
    kind("class_declaration", "class"),
    kind("enum_declaration", "enum"),
    kind("function_definition", "function"),
    kind("interface_declaration", "interface"),
    kind("method_declaration", "method"),
    kind("namespace_definition", "namespace"),
    kind("trait_declaration", "trait"),
];

const RUBY_KINDS: &[KindMap] = &[
    kind("class", "class"),
    kind("method", "method"),
    kind("module", "module"),
    kind("singleton_method", "method"),
];

const KOTLIN_KINDS: &[KindMap] = &[
    kind("class_declaration", "class"),
    kind("function_declaration", "function"),
    kind("object_declaration", "object"),
    kind("property_declaration", "property"),
];

const SCALA_KINDS: &[KindMap] = &[
    kind("class_definition", "class"),
    kind("function_definition", "function"),
    kind("object_definition", "object"),
    kind("trait_definition", "trait"),
    kind("type_definition", "type"),
    kind("val_definition", "val"),
];

static LANGUAGES: &[LanguageSpec] = &[
    LanguageSpec {
        name: "rust",
        aliases: &["rs"],
        extensions: &["rs"],
        filenames: &[],
        grammar: rust_language,
        symbol_kinds: RUST_KINDS,
        parent_kinds: &["impl_item", "mod_item", "trait_item"],
    },
    LanguageSpec {
        name: "python",
        aliases: &["py"],
        extensions: &["py", "pyi"],
        filenames: &[],
        grammar: python_language,
        symbol_kinds: PYTHON_KINDS,
        parent_kinds: &["class_definition"],
    },
    LanguageSpec {
        name: "typescript",
        aliases: &["ts"],
        extensions: &["ts", "mts", "cts"],
        filenames: &[],
        grammar: typescript_language,
        symbol_kinds: TS_KINDS,
        parent_kinds: &[
            "abstract_class_declaration",
            "class_declaration",
            "interface_declaration",
            "internal_module",
        ],
    },
    LanguageSpec {
        name: "tsx",
        aliases: &[],
        extensions: &["tsx"],
        filenames: &[],
        grammar: tsx_language,
        symbol_kinds: TS_KINDS,
        parent_kinds: &[
            "abstract_class_declaration",
            "class_declaration",
            "interface_declaration",
            "internal_module",
        ],
    },
    LanguageSpec {
        name: "javascript",
        aliases: &["js", "jsx"],
        extensions: &["js", "jsx", "mjs", "cjs"],
        filenames: &[],
        grammar: javascript_language,
        symbol_kinds: JS_KINDS,
        parent_kinds: &["class_declaration"],
    },
    LanguageSpec {
        name: "csharp",
        aliases: &["c#", "cs"],
        extensions: &["cs"],
        filenames: &[],
        grammar: csharp_language,
        symbol_kinds: CSHARP_KINDS,
        parent_kinds: &[
            "class_declaration",
            "interface_declaration",
            "namespace_declaration",
            "record_declaration",
            "struct_declaration",
        ],
    },
    LanguageSpec {
        name: "go",
        aliases: &["golang"],
        extensions: &["go"],
        filenames: &[],
        grammar: go_language,
        symbol_kinds: GO_KINDS,
        parent_kinds: &[],
    },
    LanguageSpec {
        name: "zig",
        aliases: &["zed"],
        extensions: &["zig", "zon"],
        filenames: &[],
        grammar: zig_language,
        symbol_kinds: ZIG_KINDS,
        parent_kinds: &["struct_declaration", "enum_declaration"],
    },
    LanguageSpec {
        name: "css",
        aliases: &[],
        extensions: &["css"],
        filenames: &[],
        grammar: css_language,
        symbol_kinds: CSS_KINDS,
        parent_kinds: &[],
    },
    LanguageSpec {
        name: "html",
        aliases: &["htm"],
        extensions: &["html", "htm"],
        filenames: &[],
        grammar: html_language,
        symbol_kinds: HTML_KINDS,
        parent_kinds: &["element"],
    },
    LanguageSpec {
        name: "bash",
        aliases: &["sh", "shell"],
        extensions: &["bash", "sh", "zsh"],
        filenames: &[".bashrc", ".bash_profile", ".zshrc", "bashrc", "zshrc"],
        grammar: bash_language,
        symbol_kinds: BASH_KINDS,
        parent_kinds: &[],
    },
    LanguageSpec {
        name: "sql",
        aliases: &[],
        extensions: &["sql"],
        filenames: &[],
        grammar: sql_language,
        symbol_kinds: SQL_KINDS,
        parent_kinds: &[],
    },
    LanguageSpec {
        name: "java",
        aliases: &[],
        extensions: &["java"],
        filenames: &[],
        grammar: java_language,
        symbol_kinds: JAVA_KINDS,
        parent_kinds: &[
            "class_declaration",
            "enum_declaration",
            "interface_declaration",
            "record_declaration",
        ],
    },
    LanguageSpec {
        name: "c",
        aliases: &[],
        extensions: &["c", "h"],
        filenames: &[],
        grammar: c_language,
        symbol_kinds: C_KINDS,
        parent_kinds: &[],
    },
    LanguageSpec {
        name: "cpp",
        aliases: &["c++", "cc", "cxx", "hpp"],
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        filenames: &[],
        grammar: cpp_language,
        symbol_kinds: CPP_KINDS,
        parent_kinds: &[
            "class_specifier",
            "namespace_definition",
            "struct_specifier",
        ],
    },
    LanguageSpec {
        name: "php",
        aliases: &[],
        extensions: &["php"],
        filenames: &[],
        grammar: php_language,
        symbol_kinds: PHP_KINDS,
        parent_kinds: &[
            "class_declaration",
            "enum_declaration",
            "interface_declaration",
            "trait_declaration",
        ],
    },
    LanguageSpec {
        name: "ruby",
        aliases: &["rb"],
        extensions: &["rb"],
        filenames: &["Rakefile", "Gemfile"],
        grammar: ruby_language,
        symbol_kinds: RUBY_KINDS,
        parent_kinds: &["class", "module"],
    },
    LanguageSpec {
        name: "kotlin",
        aliases: &["kt"],
        extensions: &["kt", "kts"],
        filenames: &[],
        grammar: kotlin_language,
        symbol_kinds: KOTLIN_KINDS,
        parent_kinds: &["class_declaration", "object_declaration"],
    },
    LanguageSpec {
        name: "scala",
        aliases: &["sc"],
        extensions: &["scala", "sc"],
        filenames: &[],
        grammar: scala_language,
        symbol_kinds: SCALA_KINDS,
        parent_kinds: &["class_definition", "object_definition", "trait_definition"],
    },
];

fn normalize_language_filter(language: &str) -> String {
    language.trim().to_ascii_lowercase()
}

fn collect_supported_files(
    path: &Path,
    language_filter: Option<&str>,
    files: &mut Vec<SourceFile>,
    skipped_unsupported_files: &mut usize,
) -> Result<()> {
    if path.is_file() {
        match language_for_path(path, language_filter) {
            Some(language) => files.push(SourceFile {
                path: path.to_path_buf(),
                language,
            }),
            None => *skipped_unsupported_files += 1,
        }
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }

    let mut entries = fs::read_dir(path)
        .with_context(|| format!("read directory {}", path.display()))?
        .filter_map(std::result::Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        // Skip hidden entries (files and dirs): VCS, caches, virtualenvs, tool
        // config — noise that crowds out real source and truncates the map. The
        // swebench eval run showed `.cache/work/<vendored astropy>` dominating
        // the map of this repo. An explicit scope INTO a hidden dir is still
        // honored: only the recursive walk filters them.
        if name.starts_with('.') {
            continue;
        }
        if file_type.is_dir() {
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            collect_supported_files(&path, language_filter, files, skipped_unsupported_files)?;
        } else if file_type.is_file() {
            match language_for_path(&path, language_filter) {
                Some(language) => files.push(SourceFile { path, language }),
                None => *skipped_unsupported_files += 1,
            }
        }
    }
    Ok(())
}

fn language_for_path(path: &Path, language_filter: Option<&str>) -> Option<&'static LanguageSpec> {
    let file_name = path.file_name().and_then(|name| name.to_str())?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    LANGUAGES.iter().find(|language| {
        language_filter.is_none_or(|filter| language.matches_filter(filter))
            && (language.filenames.contains(&file_name)
                || extension
                    .as_deref()
                    .is_some_and(|extension| language.extensions.contains(&extension)))
    })
}

struct SymbolCollectContext<'a> {
    source: &'a str,
    path: &'a str,
    language: &'static LanguageSpec,
    terms: &'a [String],
    budget: usize,
}

fn collect_symbols_from_node(
    node: Node<'_>,
    context: &SymbolCollectContext<'_>,
    parents: &mut Vec<String>,
    report: &mut SymbolScanReport,
) {
    if report.truncated {
        return;
    }

    let symbol = symbol_for_node(
        node,
        context.source,
        context.path,
        context.language,
        parents.last().cloned(),
    );
    let parent_label = symbol
        .as_ref()
        .and_then(|symbol| parent_context_label(symbol, context.language, node.kind()));
    if let Some(symbol) = symbol
        && symbol_matches(&symbol, context.terms)
    {
        if report.symbols.len() >= context.budget {
            report.truncated = true;
            return;
        }
        report.symbols.push(symbol);
    }

    if let Some(label) = parent_label.as_ref() {
        parents.push(label.clone());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols_from_node(child, context, parents, report);
        if report.truncated {
            break;
        }
    }
    if parent_label.is_some() {
        parents.pop();
    }
}

fn symbol_for_node(
    node: Node<'_>,
    source: &str,
    path: &str,
    language: &LanguageSpec,
    parent: Option<String>,
) -> Option<RepoSymbol> {
    let kind = language.symbol_kind(node.kind())?;
    let name = symbol_name(node, source, language, kind)?;
    let position = node.start_position();
    Some(RepoSymbol {
        path: path.to_string(),
        language: language.name.to_string(),
        kind: kind.to_string(),
        name,
        parent,
        visibility: visibility(node, source),
        signature: signature(node, source),
        line: position.row + 1,
        column: position.column + 1,
    })
}

fn symbol_name(
    node: Node<'_>,
    source: &str,
    language: &LanguageSpec,
    kind: &str,
) -> Option<String> {
    if kind == "impl" {
        return Some(impl_name(node, source));
    }
    if matches!(kind, "selector" | "import") {
        return Some(signature_name(node, source));
    }
    if language.name == "html" {
        return descendant_text(node, source, &["tag_name"]).map(str::to_string);
    }
    if language.name == "sql" {
        return descendant_text(
            node,
            source,
            &[
                "identifier",
                "object_reference",
                "function_reference",
                "table_reference",
            ],
        )
        .map(str::to_string)
        .or_else(|| Some(signature_name(node, source)));
    }
    node.child_by_field_name("name")
        .and_then(|name| node_text(name, source))
        .map(str::to_string)
        .or_else(|| {
            descendant_text(
                node,
                source,
                &[
                    "identifier",
                    "property_identifier",
                    "type_identifier",
                    "field_identifier",
                    "simple_identifier",
                    "constant",
                    "word",
                ],
            )
            .map(str::to_string)
        })
}

fn impl_name(node: Node<'_>, source: &str) -> String {
    let sig = signature(node, source);
    let without_brace = sig.split('{').next().unwrap_or(&sig).trim();
    if without_brace.is_empty() {
        "impl".to_string()
    } else {
        without_brace.to_string()
    }
}

fn visibility(node: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "visibility_modifier")
        .and_then(|child| node_text(child, source))
        .map(str::to_string)
}

fn signature(node: Node<'_>, source: &str) -> String {
    let raw = node_text(node, source).unwrap_or_default();
    let line = raw.lines().next().unwrap_or_default().trim();
    truncate_chars(line, MAX_SIGNATURE_CHARS)
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.byte_range())
}

fn descendant_text<'a>(node: Node<'_>, source: &'a str, kinds: &[&str]) -> Option<&'a str> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if kinds.contains(&child.kind()) {
            return node_text(child, source);
        }
        if let Some(text) = descendant_text(child, source, kinds) {
            return Some(text);
        }
    }
    None
}

fn signature_name(node: Node<'_>, source: &str) -> String {
    signature(node, source)
        .split('{')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn parent_context_label(
    symbol: &RepoSymbol,
    language: &LanguageSpec,
    node_kind: &str,
) -> Option<String> {
    if language.is_parent_kind(node_kind) {
        if symbol.kind == "impl" {
            Some(symbol.name.clone())
        } else {
            Some(format!("{} {}", symbol.kind, symbol.name))
        }
    } else {
        None
    }
}

/// Split a query into lowercased, de-duplicated terms (preserving first-seen
/// order). Duplicates would otherwise double-count in document frequency,
/// scoring, and the multi-term boost.
fn tokenize_query(query: Option<&str>) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for raw in query.unwrap_or_default().split_whitespace() {
        let term = raw.to_lowercase();
        if !term.is_empty() && !terms.contains(&term) {
            terms.push(term);
        }
    }
    terms
}

/// A symbol's searchable fields, lowercased once so term matching avoids
/// re-allocating on every term/symbol comparison.
struct LoweredFields {
    name: String,
    kind: String,
    parent: Option<String>,
    signature: String,
    path: String,
}

impl LoweredFields {
    fn from_symbol(symbol: &RepoSymbol) -> Self {
        Self {
            name: symbol.name.to_lowercase(),
            kind: symbol.kind.to_lowercase(),
            parent: symbol.parent.as_ref().map(|parent| parent.to_lowercase()),
            signature: symbol.signature.to_lowercase(),
            path: symbol.path.to_lowercase(),
        }
    }
}

/// A symbol is a candidate when it matches at least one query term in any
/// searchable field. With no terms (no query) every symbol matches.
fn symbol_matches(symbol: &RepoSymbol, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let fields = LoweredFields::from_symbol(symbol);
    terms
        .iter()
        .any(|term| term_field_weight(&fields, term) > 0.0)
}

/// Field-weighted match strength of a single term against a symbol. Higher
/// weights mean a more specific match: an exact name beats a name substring,
/// which beats an incidental hit in the signature or path. Returns 0 when the
/// term is absent. `fields` are expected to already be lowercased; `term` too.
fn term_field_weight(fields: &LoweredFields, term: &str) -> f64 {
    if fields.name == term {
        return 12.0;
    }
    let mut weight = 0.0_f64;
    if fields.name.contains(term) {
        weight = weight.max(6.0);
    }
    if fields.kind == term {
        weight = weight.max(4.0);
    }
    if fields
        .parent
        .as_ref()
        .is_some_and(|parent| parent.contains(term))
    {
        weight = weight.max(3.0);
    }
    if fields.signature.contains(term) {
        weight = weight.max(2.0);
    }
    if fields.path.contains(term) {
        weight = weight.max(1.0);
    }
    weight
}

/// Rank candidate symbols by TF-IDF-ish relevance to the query terms. Rarer
/// terms (matching fewer candidates) contribute more via an inverse-document-
/// frequency weight, and symbols covering more distinct terms are boosted.
/// Ties break deterministically on path, then position.
fn rank_symbols(symbols: Vec<RepoSymbol>, terms: &[String]) -> Vec<RepoSymbol> {
    let lowered: Vec<LoweredFields> = symbols.iter().map(LoweredFields::from_symbol).collect();
    let total = symbols.len() as f64;
    let idf: Vec<f64> = terms
        .iter()
        .map(|term| {
            let frequency = lowered
                .iter()
                .filter(|fields| term_field_weight(fields, term) > 0.0)
                .count();
            (1.0 + total / (1.0 + frequency as f64)).ln()
        })
        .collect();

    let mut scored: Vec<(f64, RepoSymbol)> = symbols
        .into_iter()
        .enumerate()
        .map(|(index, symbol)| (symbol_score(&lowered[index], terms, &idf), symbol))
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.1.path.cmp(&b.1.path))
            .then_with(|| a.1.line.cmp(&b.1.line))
            .then_with(|| a.1.column.cmp(&b.1.column))
    });
    scored.into_iter().map(|(_, symbol)| symbol).collect()
}

fn symbol_score(fields: &LoweredFields, terms: &[String], idf: &[f64]) -> f64 {
    let mut score = 0.0_f64;
    let mut matched = 0u32;
    for (index, term) in terms.iter().enumerate() {
        let weight = term_field_weight(fields, term);
        if weight > 0.0 {
            score += weight * idf[index];
            matched += 1;
        }
    }
    // Reward symbols that cover more of a multi-term query.
    if terms.len() > 1 {
        score *= 1.0 + (matched.saturating_sub(1) as f64) * 0.5;
    }
    score
}

fn grouped_symbols(symbols: &[RepoSymbol]) -> Vec<SymbolFileGroup<'_>> {
    let mut groups = Vec::new();
    let mut index = 0;
    while index < symbols.len() {
        let path = symbols[index].path.as_str();
        let mut group = SymbolFileGroup {
            path,
            symbols: Vec::new(),
        };
        while index < symbols.len() && symbols[index].path == path {
            group.symbols.push(&symbols[index]);
            index += 1;
        }
        groups.push(group);
    }
    groups
}

fn relative_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let rendered = relative.to_string_lossy().replace('\\', "/");
    if rendered.is_empty() {
        ".".to_string()
    } else {
        rendered
    }
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let suffix = "...";
    let mut truncated: String = value
        .chars()
        .take(max.saturating_sub(suffix.len()))
        .collect();
    truncated.push_str(suffix);
    truncated
}

#[cfg(all(test, feature = "local"))]
mod tests {
    use super::*;
    use crate::host::LocalCodeHost;
    use everruns_contracts::typed_id::SessionId;
    use std::sync::Arc;

    #[test]
    fn eager_repo_map_schema_stays_compact_and_bounded() {
        let schema = compact_scan_schema();
        let properties = schema["properties"].as_object().expect("properties");
        let mut names = properties.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            ["language", "limit", "max_file_bytes", "path", "query"]
        );
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["properties"]["limit"]["maximum"], json!(MAX_LIMIT));
        assert_eq!(
            schema["properties"]["max_file_bytes"]["maximum"],
            json!(MAX_FILE_BYTES)
        );
        let bytes = serde_json::to_vec(&schema).expect("serialize schema").len();
        assert_eq!(bytes, 261, "eager repo_map schema cost changed");
    }

    #[test]
    fn deferred_repo_symbols_keeps_its_descriptive_reveal_schema() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = host(dir.path());
        let map_schema = RepoMapTool {
            host: workspace.clone(),
        }
        .parameters_schema();
        let symbols_schema = RepoSymbolsTool { host: workspace }.parameters_schema();

        assert!(
            symbols_schema["properties"]["query"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("ranked by relevance"))
        );
        assert!(
            serde_json::to_vec(&symbols_schema)
                .expect("serialize symbols schema")
                .len()
                > serde_json::to_vec(&map_schema)
                    .expect("serialize map schema")
                    .len()
        );
    }

    #[test]
    fn eager_repo_map_description_explains_optional_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool = RepoMapTool {
            host: host(dir.path()),
        };
        let description = tool.description();

        assert!(description.contains("Prefer a queried map narrowed"));
        assert!(description.contains("truncates alphabetically"));
        assert!(description.contains("defaults to 50"));
    }

    fn host(path: &Path) -> Arc<dyn CodeHost> {
        Arc::new(LocalCodeHost::new(path))
    }

    async fn run_tool(tool: &dyn Tool, arguments: Value) -> ToolExecutionResult {
        tool.execute_with_context(arguments, &ToolContext::new(SessionId::new()))
            .await
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, content).expect("write file");
    }

    #[test]
    fn collects_rust_symbols_with_parent_context() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            &dir.path().join("src/lib.rs"),
            r#"
pub struct Widget {
    id: String,
}

impl Widget {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    fn id(&self) -> &str {
        &self.id
    }
}

trait Named {
    fn name(&self) -> &str;
}
"#,
        );

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: None,
                query: None,
                language: None,
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert_eq!(report.scanned_files, 1);
        assert!(report.symbols.iter().any(|symbol| symbol.language == "rust"
            && symbol.kind == "struct"
            && symbol.name == "Widget"));
        let new_fn = report
            .symbols
            .iter()
            .find(|symbol| symbol.kind == "function" && symbol.name == "new")
            .expect("new function");
        assert_eq!(new_fn.parent.as_deref(), Some("impl Widget"));
        assert_eq!(new_fn.visibility.as_deref(), Some("pub"));
        assert!(
            report
                .symbols
                .iter()
                .any(|symbol| symbol.kind == "function_signature" && symbol.name == "name")
        );
    }

    #[test]
    fn ranks_exact_name_match_above_incidental_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Files are scanned in path order, so `a_helpers.rs` comes before
        // `b_widget.rs`. Without relevance ranking the incidental signature
        // match would be returned first.
        write(
            &dir.path().join("a_helpers.rs"),
            "pub fn render_widget_list() {}\n",
        );
        write(&dir.path().join("b_widget.rs"), "pub fn widget() {}\n");

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: None,
                query: Some("widget".to_string()),
                language: None,
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert_eq!(report.symbols.len(), 2);
        assert_eq!(
            report.symbols[0].name, "widget",
            "exact name match should rank first, got {:?}",
            report.symbols
        );
    }

    #[test]
    fn ranks_symbols_matching_more_query_terms_higher() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("a.rs"), "pub fn render() {}\n");
        write(&dir.path().join("b.rs"), "pub fn render_widget() {}\n");

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: None,
                query: Some("render widget".to_string()),
                language: None,
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert_eq!(
            report.symbols[0].name, "render_widget",
            "symbol matching both terms should rank first, got {:?}",
            report.symbols
        );
        assert!(
            report.symbols.iter().any(|symbol| symbol.name == "render"),
            "single-term match should still be returned"
        );
    }

    #[test]
    fn tokenizes_query_lowercased_without_duplicates() {
        assert_eq!(
            tokenize_query(Some("Render RENDER  widget")),
            vec!["render".to_string(), "widget".to_string()]
        );
        assert!(tokenize_query(None).is_empty());
        assert!(tokenize_query(Some("   ")).is_empty());
    }

    #[test]
    fn filters_by_query_and_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("src/lib.rs"), "pub fn keep_me() {}\n");
        write(&dir.path().join("tests/demo.rs"), "fn skip_me() {}\n");

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: Some("src".to_string()),
                query: Some("keep".to_string()),
                language: None,
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert_eq!(report.symbols.len(), 1);
        assert_eq!(report.symbols[0].path, "src/lib.rs");
        assert_eq!(report.symbols[0].name, "keep_me");
    }

    #[test]
    fn collects_symbols_across_supported_languages() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            &dir.path().join("app.py"),
            "class Service:\n    def run(self):\n        pass\n",
        );
        write(
            &dir.path().join("widget.ts"),
            "export interface Widget {}\nexport function render() {}\n",
        );
        write(&dir.path().join("style.css"), ".button { color: red; }\n");
        write(
            &dir.path().join("index.html"),
            "<main><section></section></main>\n",
        );
        write(&dir.path().join("script.sh"), "deploy() { echo ok; }\n");
        write(
            &dir.path().join("schema.sql"),
            "CREATE TABLE users (id int);\n",
        );

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: None,
                query: None,
                language: None,
                limit: 100,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        for language in ["python", "typescript", "css", "html", "bash", "sql"] {
            assert!(
                report.languages.iter().any(|seen| seen == language),
                "expected {language} in {:?}",
                report.languages
            );
        }
        assert!(
            report
                .symbols
                .iter()
                .any(|symbol| symbol.language == "python"
                    && symbol.kind == "class"
                    && symbol.name == "Service")
        );
        assert!(
            report
                .symbols
                .iter()
                .any(|symbol| symbol.language == "typescript"
                    && symbol.kind == "function"
                    && symbol.name == "render")
        );
        assert!(
            report
                .symbols
                .iter()
                .any(|symbol| symbol.language == "css" && symbol.name.contains(".button"))
        );
        assert!(report.symbols.iter().any(|symbol| symbol.language == "bash"
            && symbol.kind == "function"
            && symbol.name == "deploy"));
    }

    #[test]
    fn collects_symbols_from_newly_added_languages() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            &dir.path().join("Service.java"),
            "class Service {\n  void run() {}\n}\n",
        );
        write(
            &dir.path().join("math.c"),
            "int add(int a, int b) {\n  return a + b;\n}\n",
        );
        write(
            &dir.path().join("widget.cpp"),
            "class Widget {\npublic:\n  void render() {}\n};\n",
        );
        write(
            &dir.path().join("app.php"),
            "<?php\nclass App {\n  function boot() {}\n}\n",
        );
        write(
            &dir.path().join("service.rb"),
            "class Service\n  def run\n  end\nend\n",
        );
        write(
            &dir.path().join("Main.kt"),
            "class Greeter {\n  fun greet() {}\n}\n",
        );
        write(
            &dir.path().join("Main.scala"),
            "object Main {\n  def run(): Unit = {}\n}\n",
        );

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: None,
                query: None,
                language: None,
                limit: 200,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        let has = |language: &str, kind: &str, name: &str| {
            report.symbols.iter().any(|symbol| {
                symbol.language == language && symbol.kind == kind && symbol.name == name
            })
        };

        assert!(
            has("java", "class", "Service"),
            "java class: {:?}",
            report.symbols
        );
        assert!(
            has("java", "method", "run"),
            "java method: {:?}",
            report.symbols
        );
        assert!(
            has("c", "function", "add"),
            "c function: {:?}",
            report.symbols
        );
        assert!(
            has("cpp", "class", "Widget"),
            "cpp class: {:?}",
            report.symbols
        );
        assert!(
            has("php", "class", "App"),
            "php class: {:?}",
            report.symbols
        );
        assert!(
            has("ruby", "class", "Service"),
            "ruby class: {:?}",
            report.symbols
        );
        assert!(
            has("ruby", "method", "run"),
            "ruby method: {:?}",
            report.symbols
        );
        assert!(
            has("kotlin", "class", "Greeter"),
            "kotlin class: {:?}",
            report.symbols
        );
        assert!(
            has("kotlin", "function", "greet"),
            "kotlin fn: {:?}",
            report.symbols
        );
        assert!(
            has("scala", "object", "Main"),
            "scala object: {:?}",
            report.symbols
        );
    }

    #[test]
    fn filters_by_language() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("src/lib.rs"), "pub fn rust_fn() {}\n");
        write(&dir.path().join("app.py"), "def py_fn():\n    pass\n");

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: None,
                query: None,
                language: Some("python".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert_eq!(report.languages, vec!["python"]);
        assert!(report.symbols.iter().any(|symbol| symbol.name == "py_fn"));
        assert!(!report.symbols.iter().any(|symbol| symbol.name == "rust_fn"));
    }

    #[tokio::test]
    async fn repo_symbols_tool_executes_multilanguage_scan() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("app.py"), "def py_fn():\n    pass\n");

        let capability = RepoMapCapability::new(host(dir.path()));
        let tools = capability.tools();
        let tool = tools
            .iter()
            .find(|tool| tool.name() == "repo_symbols")
            .expect("repo_symbols tool");
        let result = run_tool(
            tool.as_ref(),
            json!({
                "language": "python",
                "query": "py_fn",
                "limit": 10
            }),
        )
        .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };

        assert_eq!(value["languages"], json!(["python"]));
        assert_eq!(value["count"], json!(1));
        assert_eq!(value["symbols"][0]["language"], json!("python"));
        assert_eq!(value["symbols"][0]["name"], json!("py_fn"));
    }

    #[tokio::test]
    async fn unqueried_repo_map_uses_compact_default_and_explains_truncation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = (0..80)
            .map(|index| format!("pub fn symbol_{index}() {{}}\n"))
            .collect::<String>();
        write(&dir.path().join("src/lib.rs"), &source);

        let capability = RepoMapCapability::new(host(dir.path()));
        let tools = capability.tools();
        let tool = tools
            .iter()
            .find(|tool| tool.name() == "repo_map")
            .expect("repo_map tool");
        let result = run_tool(tool.as_ref(), json!({})).await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };

        assert_eq!(value["count"], json!(50));
        assert_eq!(value["truncated"], json!(true));
        assert_eq!(value["truncation"]["reason"], json!("symbol_limit"));
        assert!(
            value["truncation"]["suggestion"]
                .as_str()
                .is_some_and(|suggestion| suggestion.contains("Do not repeat")
                    && suggestion.contains("query"))
        );
    }

    #[test]
    fn queried_scans_keep_the_larger_default_limit() {
        assert_eq!(default_limit(Some("needle")), 200);
        assert_eq!(default_limit(None), 50);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_directories_during_recursive_scan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        write(&dir.path().join("src/lib.rs"), "pub fn inside_fn() {}\n");
        write(
            &outside.path().join("outside.rs"),
            "pub fn outside_fn() {}\n",
        );
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked_outside"))
            .expect("symlink outside dir");

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: None,
                query: None,
                language: Some("rust".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert!(
            report
                .symbols
                .iter()
                .any(|symbol| symbol.name == "inside_fn")
        );
        assert!(
            !report
                .symbols
                .iter()
                .any(|symbol| symbol.name == "outside_fn")
        );
    }

    #[test]
    fn rejects_paths_outside_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: Some("../outside".to_string()),
                query: None,
                language: None,
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect_err("outside path should fail");

        assert!(err.to_string().contains("inside the workspace"));
    }

    #[test]
    fn accepts_host_workspace_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("src/lib.rs"), "pub fn inside_fn() {}\n");

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: Some(dir.path().display().to_string()),
                query: None,
                language: Some("rust".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("host root should scan");

        assert!(report.symbols.iter().any(|s| s.name == "inside_fn"));
    }

    #[tokio::test]
    async fn repo_map_tool_accepts_repository_relative_path_scope() {
        // Repository-relative paths resolve from the active repository root.
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("pkg/inside.rs"), "pub fn pkg_fn() {}\n");
        write(
            &dir.path().join("other/elsewhere.rs"),
            "pub fn other_fn() {}\n",
        );

        let capability = RepoMapCapability::new(host(dir.path()));
        let tools = capability.tools();
        let tool = tools
            .iter()
            .find(|tool| tool.name() == "repo_map")
            .expect("repo_map tool");

        // Repository root scans the whole workspace.
        let ToolExecutionResult::Success(root) =
            run_tool(tool.as_ref(), json!({ "path": ".", "language": "rust" })).await
        else {
            panic!("expected success scanning repository root");
        };
        assert_eq!(root["count"], json!(2));

        // Repository-relative subpath scopes to just that directory.
        let ToolExecutionResult::Success(sub) =
            run_tool(tool.as_ref(), json!({ "path": "pkg", "language": "rust" })).await
        else {
            panic!("expected success scanning pkg");
        };
        assert_eq!(sub["count"], json!(1));
        assert_eq!(sub["files"][0]["symbols"][0]["name"], json!("pkg_fn"));
    }

    #[tokio::test]
    async fn repo_map_scans_nested_repository_in_linked_worktree_layout() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A linked Git worktree identifies its repository with a .git file,
        // not a .git directory. The scanner must treat that as metadata while
        // still accepting a repository-relative nested path.
        write(
            &dir.path().join(".git"),
            "gitdir: /tmp/repo/.git/worktrees/fixture\n",
        );
        write(
            &dir.path().join("REPOS/nested/src/lib.rs"),
            "pub fn nested_owner() {}\n",
        );
        write(
            &dir.path().join("outside.rs"),
            "pub fn outside_scope() {}\n",
        );

        let capability = RepoMapCapability::new(host(dir.path()));
        let tools = capability.tools();
        let tool = tools
            .iter()
            .find(|tool| tool.name() == "repo_map")
            .expect("repo_map tool");

        let path = "REPOS/nested";
        let ToolExecutionResult::Success(result) =
            run_tool(tool.as_ref(), json!({ "path": path, "language": "rust" })).await
        else {
            panic!("expected success scanning {path}");
        };
        assert_eq!(result["count"], json!(1), "scope {path}");
        assert_eq!(
            result["files"][0]["symbols"][0]["name"],
            json!("nested_owner"),
            "scope {path}"
        );
    }

    #[test]
    fn accepts_host_absolute_subpath() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("pkg/inside.rs"), "pub fn pkg_fn() {}\n");
        write(
            &dir.path().join("other/elsewhere.rs"),
            "pub fn other_fn() {}\n",
        );

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: Some(dir.path().join("pkg").display().to_string()),
                query: None,
                language: Some("rust".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("host subpath should scan");

        assert!(report.symbols.iter().any(|s| s.name == "pkg_fn"));
        assert!(
            !report.symbols.iter().any(|s| s.name == "other_fn"),
            "scope should be limited to the requested host subpath"
        );
    }

    // Hidden files and directories (`.cache`, `.git`, `.env`, …) are noise for a
    // symbol map and crowded out real source in the swebench eval run, where a
    // vendored astropy checkout under `.cache/work/` dominated the map. The
    // recursive walk skips every `.`-prefixed entry.
    #[test]
    fn recursive_scan_skips_hidden_files_and_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("src/real.rs"), "pub fn real_fn() {}\n");
        write(
            &dir.path().join(".cache/work/vendored.rs"),
            "pub fn vendored_fn() {}\n",
        );
        write(
            &dir.path().join(".git/hooks/hook.rs"),
            "pub fn hook_fn() {}\n",
        );
        write(
            &dir.path().join(".hidden.rs"),
            "pub fn hidden_file_fn() {}\n",
        );

        let report = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: None,
                query: None,
                language: Some("rust".to_string()),
                limit: 50,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        let names: Vec<&str> = report.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"real_fn"), "real source should be mapped");
        for hidden in ["vendored_fn", "hook_fn", "hidden_file_fn"] {
            assert!(
                !names.contains(&hidden),
                "hidden entry {hidden} should be skipped"
            );
        }
    }

    #[test]
    fn absolute_host_path_does_not_bypass_containment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        let err = collect_repo_symbols(
            dir.path(),
            SymbolScanOptions {
                path: Some(outside.path().display().to_string()),
                query: None,
                language: None,
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect_err("an absolute path outside the workspace should fail");

        assert!(err.to_string().contains("inside the workspace"));
    }
}
