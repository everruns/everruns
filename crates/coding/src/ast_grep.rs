//! Structural search and rewrite backed by ast-grep.
//!
//! `grep_files` remains the right first move for exact text. `ast_grep` is for
//! read-only code shapes; `ast_edit` applies pattern/replacement rewrites with a
//! preview-first `dry_run` flow before writing.

use crate::failure::{blocking, into_tool_result};
use crate::host::{CodeHost, SourceTree, resolve_scope};
use crate::source::SourceFilter;
use anyhow::{Context, Result, bail};
use ast_grep_core::matcher::Pattern;
use ast_grep_core::meta_var::MetaVariable;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{AstGrep, Node};
use ast_grep_language::SupportLang;
use async_trait::async_trait;
use everruns_contracts::narration::ToolNarrationPhase;
use everruns_contracts::runtime::capabilities::{Capability, CapabilityStatus};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::Tool;
use everruns_contracts::tool_types::ToolHints;
use everruns_contracts::tools::ToolExecutionResult;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const AST_GREP_CAPABILITY_ID: &str = "ast_grep";
pub const AST_EDIT_CAPABILITY_ID: &str = "ast_edit";

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;
const DEFAULT_MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MATCH_TEXT_CHARS: usize = 500;
const MAX_CAPTURE_TEXT_CHARS: usize = 240;
const MAX_REPLACEMENT_TEXT_CHARS: usize = 500;
const MAX_PREVIEW_DIFF_CHARS: usize = 8_000;
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    ".next",
    "dist",
    "build",
    ".direnv",
    ".venv",
    "venv",
];

#[derive(Clone)]
pub struct AstGrepCapability {
    host: Arc<dyn CodeHost>,
}

impl AstGrepCapability {
    pub fn new(host: Arc<dyn CodeHost>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Capability for AstGrepCapability {
    fn id(&self) -> &str {
        AST_GREP_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "AST Grep"
    }

    fn description(&self) -> &str {
        "Read-only multi-language structural code search backed by ast-grep patterns."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Code Navigation")
    }

    // No system-prompt contribution: `ast_grep`'s description already states what
    // it searches and which languages it supports. When to reach for it, and
    // reading a match before editing it, are judgement calls the model makes from
    // the surrounding task.

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(AstGrepTool {
            host: self.host.clone(),
        })]
    }
}

#[derive(Clone)]
pub struct AstEditCapability {
    host: Arc<dyn CodeHost>,
}

impl AstEditCapability {
    pub fn new(host: Arc<dyn CodeHost>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Capability for AstEditCapability {
    fn id(&self) -> &str {
        AST_EDIT_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "AST Edit"
    }

    fn description(&self) -> &str {
        "Previewed ast-grep pattern rewrites across common languages (opt-in)."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Code Navigation")
    }

    // No system-prompt contribution: `ast_edit`'s description already carries the
    // preview-then-apply contract (`dry_run` defaults to true; call again with
    // `dry_run=false` once the user accepts).

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(AstEditTool {
            host: self.host.clone(),
        })]
    }
}

struct AstGrepTool {
    host: Arc<dyn CodeHost>,
}

#[async_trait]
impl Tool for AstGrepTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        ctx: everruns_contracts::runtime::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(
            everruns_contracts::runtime::tool_narration::narrate_grep_files(
                &tool_call.arguments,
                phase,
                locale,
                ctx,
            ),
        )
    }

    fn name(&self) -> &str {
        "ast_grep"
    }

    fn display_name(&self) -> Option<&str> {
        Some("AST grep")
    }

    // Boundary note: the model reaches for shell grep/sed out of habit even for
    // structural code tasks, so the description states the split explicitly:
    // structural patterns here, plain text search for literals and docs.
    fn description(&self) -> &str {
        "Search workspace code with ast-grep structural patterns. Prefer this over repeated file \
         reads when locating functions, impl blocks, call shapes, field access, or repeated \
         structural constructs. Choose it over line-oriented text search when matching code structure; for literal strings, comments, or docs, plain text search is more effective. Supports Rust, Python, TypeScript/TSX, JavaScript/JSX, C#, Go, \
         CSS, HTML, and Bash."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Required ast-grep pattern, for example `fn $NAME($$$ARGS) { $$$BODY }`."
                },
                "path": {
                    "type": "string",
                    "description": "Optional workspace-relative file or directory to scan. Defaults to the workspace root."
                },
                "language": {
                    "type": "string",
                    "description": "Optional language filter. Supported values include rust, python, typescript, tsx, javascript, csharp, go, css, html, and bash."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "description": "Maximum number of matches to return. Defaults to 50."
                },
                "max_file_bytes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_FILE_BYTES,
                    "description": "Skip supported source files larger than this many bytes. Defaults to 524288."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("ast_grep needs the session it runs in")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        match run_ast_grep(self.host.as_ref(), arguments, context).await {
            Ok(value) => ToolExecutionResult::success(value),
            Err(err) => into_tool_result(err),
        }
    }
}

/// The request's `path`, trimmed; `None` for the whole workspace.
fn requested_path(arguments: &Value) -> Option<&str> {
    arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// The files worth fetching for a request: every supported language, or only
/// the one the request names.
fn source_filter(arguments: &Value) -> SourceFilter {
    let wanted = arguments
        .get("language")
        .and_then(Value::as_str)
        .and_then(|language| resolve_language_filter(language).ok());
    let languages = LANGUAGES
        .iter()
        .filter(|language| wanted.is_none_or(|wanted| language.name == wanted));
    SourceFilter::new(
        languages
            .clone()
            .flat_map(|language| language.extensions.iter().copied()),
        languages.flat_map(|language| language.filenames.iter().copied()),
    )
}

/// The request with `path` rewritten to where `tree` holds that scope.
fn scoped(mut arguments: Value, tree: &SourceTree) -> Value {
    if let Some(object) = arguments.as_object_mut() {
        match &tree.scope {
            Some(scope) => object.insert("path".to_string(), json!(scope)),
            None => object.remove("path"),
        };
    }
    arguments
}

fn with_note(mut value: Value, tree: &SourceTree) -> Value {
    if let Some(note) = &tree.note {
        value["note"] = json!(note);
    }
    value
}

async fn run_ast_grep(
    host: &dyn CodeHost,
    arguments: Value,
    context: &ToolContext,
) -> Result<Value> {
    let tree = host
        .materialize(
            context,
            requested_path(&arguments),
            &source_filter(&arguments),
        )
        .await?;
    let root = tree.root.clone();
    let arguments = scoped(arguments, &tree);
    let report = blocking(move || ast_grep_from_arguments(&root, &arguments)).await?;
    Ok(with_note(
        json!({
            "ok": true,
            "workspace_root": tree.display_root,
            "path": report.path,
            "pattern": report.pattern,
            "language": report.language,
            "languages": report.languages,
            "count": report.matches.len(),
            "scanned_files": report.scanned_files,
            "skipped_unsupported_files": report.skipped_unsupported_files,
            "skipped_files": report.skipped_files,
            "skipped_large_files": report.skipped_large_files,
            "parse_error_files": report.parse_error_files,
            "pattern_error_languages": report.pattern_error_languages,
            "truncated": report.truncated,
            "matches": report.matches,
        }),
        &tree,
    ))
}

struct AstEditTool {
    host: Arc<dyn CodeHost>,
}

#[async_trait]
impl Tool for AstEditTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        ctx: everruns_contracts::runtime::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(
            everruns_contracts::runtime::tool_narration::narrate_grep_files(
                &tool_call.arguments,
                phase,
                locale,
                ctx,
            ),
        )
    }

    fn name(&self) -> &str {
        "ast_edit"
    }

    fn display_name(&self) -> Option<&str> {
        Some("AST edit")
    }

    fn description(&self) -> &str {
        "Apply ast-grep pattern/replacement rewrites across workspace source files. Supports \
         Rust, Python, TypeScript/TSX, JavaScript/JSX, C#, Go, CSS, HTML, and Bash. Defaults \
         to `dry_run=true` so the first call previews changes without writing; call again with \
         `dry_run=false` after the user accepts the preview."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Required ast-grep pattern to match, for example `console.log($$$ARGS)`."
                },
                "replacement": {
                    "type": "string",
                    "description": "Replacement template using the same metavariables as `pattern`, for example `logger.info($$$ARGS)`."
                },
                "path": {
                    "type": "string",
                    "description": "Optional workspace-relative file or directory to rewrite. Defaults to the workspace root."
                },
                "language": {
                    "type": "string",
                    "description": "Optional language filter. Supported values include rust, python, typescript, tsx, javascript, csharp, go, css, html, and bash."
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "Preview replacements without writing files. Defaults to true."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "description": "Maximum number of replacements to apply across all scanned files. Defaults to 50."
                },
                "max_file_bytes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_FILE_BYTES,
                    "description": "Skip supported source files larger than this many bytes. Defaults to 524288."
                }
            },
            "required": ["pattern", "replacement"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_readonly(false)
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("ast_edit needs the session it runs in")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        match run_ast_edit(self.host.as_ref(), arguments, context).await {
            Ok(value) => ToolExecutionResult::success(value),
            Err(err) => into_tool_result(err),
        }
    }
}

async fn run_ast_edit(
    host: &dyn CodeHost,
    arguments: Value,
    context: &ToolContext,
) -> Result<Value> {
    let tree = host
        .materialize(
            context,
            requested_path(&arguments),
            &source_filter(&arguments),
        )
        .await?;
    let root = tree.root.clone();
    let arguments = scoped(arguments, &tree);
    let report = blocking(move || ast_edit_from_arguments(&root, &arguments)).await?;
    if report.applied {
        // The rewrite happened in `tree`; a host that works on a copy sends
        // the changed files back now.
        let changed: Vec<String> = report.files.iter().map(|file| file.path.clone()).collect();
        host.commit(context, &tree, &changed).await?;
    }
    Ok(with_note(
        json!({
            "ok": true,
            "workspace_root": tree.display_root,
            "path": report.path,
            "pattern": report.pattern,
            "replacement": report.replacement,
            "language": report.language,
            "dry_run": report.dry_run,
            "applied": report.applied,
            "count": report.count,
            "files_changed": report.files_changed,
            "scanned_files": report.scanned_files,
            "skipped_unsupported_files": report.skipped_unsupported_files,
            "skipped_files": report.skipped_files,
            "skipped_large_files": report.skipped_large_files,
            "parse_error_files": report.parse_error_files,
            "pattern_error_languages": report.pattern_error_languages,
            "truncated": report.truncated,
            "diff": report.diff,
            "diff_truncated": report.diff_truncated,
            "files": report.files,
        }),
        &tree,
    ))
}

fn ast_edit_from_arguments(workspace_root: &Path, arguments: &Value) -> Result<AstEditReport> {
    let pattern = arguments
        .get("pattern")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .context("`pattern` is required")?
        .to_string();
    let replacement = arguments
        .get("replacement")
        .and_then(Value::as_str)
        .context("`replacement` is required")?
        .to_string();
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned);
    let language = arguments
        .get("language")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(ToOwned::to_owned);
    let dry_run = arguments
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let limit = bounded_usize(arguments.get("limit"), DEFAULT_LIMIT, MAX_LIMIT, "limit")?;
    let max_file_bytes = bounded_usize(
        arguments.get("max_file_bytes"),
        DEFAULT_MAX_FILE_BYTES,
        MAX_FILE_BYTES,
        "max_file_bytes",
    )?;

    ast_edit(
        workspace_root,
        AstEditOptions {
            pattern,
            replacement,
            path,
            language,
            dry_run,
            limit,
            max_file_bytes,
        },
    )
}

fn ast_grep_from_arguments(workspace_root: &Path, arguments: &Value) -> Result<AstGrepReport> {
    let pattern = arguments
        .get("pattern")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .context("`pattern` is required")?
        .to_string();
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned);
    let language = arguments
        .get("language")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(ToOwned::to_owned);
    let limit = bounded_usize(arguments.get("limit"), DEFAULT_LIMIT, MAX_LIMIT, "limit")?;
    let max_file_bytes = bounded_usize(
        arguments.get("max_file_bytes"),
        DEFAULT_MAX_FILE_BYTES,
        MAX_FILE_BYTES,
        "max_file_bytes",
    )?;

    ast_grep(
        workspace_root,
        AstGrepOptions {
            pattern,
            path,
            language,
            limit,
            max_file_bytes,
        },
    )
}

#[derive(Clone, Debug)]
struct AstEditOptions {
    pattern: String,
    replacement: String,
    path: Option<String>,
    language: Option<String>,
    dry_run: bool,
    limit: usize,
    max_file_bytes: usize,
}

#[derive(Clone, Debug)]
struct AstGrepOptions {
    pattern: String,
    path: Option<String>,
    language: Option<String>,
    limit: usize,
    max_file_bytes: usize,
}

#[derive(Clone, Debug)]
struct SourceFile {
    path: PathBuf,
    language: &'static LanguageSpec,
}

#[derive(Clone, Copy, Debug)]
struct LanguageSpec {
    name: &'static str,
    ast_lang: SupportLang,
    filters: &'static [&'static str],
    extensions: &'static [&'static str],
    filenames: &'static [&'static str],
}

const LANGUAGES: &[LanguageSpec] = &[
    LanguageSpec {
        name: "rust",
        ast_lang: SupportLang::Rust,
        filters: &["rust", "rs"],
        extensions: &["rs"],
        filenames: &[],
    },
    LanguageSpec {
        name: "python",
        ast_lang: SupportLang::Python,
        filters: &["python", "py"],
        extensions: &["py", "pyi", "py3", "bzl"],
        filenames: &[],
    },
    LanguageSpec {
        name: "typescript",
        ast_lang: SupportLang::TypeScript,
        filters: &["typescript", "ts"],
        extensions: &["ts", "mts", "cts"],
        filenames: &[],
    },
    LanguageSpec {
        name: "tsx",
        ast_lang: SupportLang::Tsx,
        filters: &["tsx"],
        extensions: &["tsx"],
        filenames: &[],
    },
    LanguageSpec {
        name: "javascript",
        ast_lang: SupportLang::JavaScript,
        filters: &["javascript", "js", "jsx"],
        extensions: &["js", "jsx", "mjs", "cjs"],
        filenames: &[],
    },
    LanguageSpec {
        name: "csharp",
        ast_lang: SupportLang::CSharp,
        filters: &["csharp", "cs", "c#"],
        extensions: &["cs"],
        filenames: &[],
    },
    LanguageSpec {
        name: "go",
        ast_lang: SupportLang::Go,
        filters: &["go", "golang"],
        extensions: &["go"],
        filenames: &[],
    },
    LanguageSpec {
        name: "css",
        ast_lang: SupportLang::Css,
        filters: &["css", "scss"],
        extensions: &["css", "scss"],
        filenames: &[],
    },
    LanguageSpec {
        name: "html",
        ast_lang: SupportLang::Html,
        filters: &["html"],
        extensions: &["html", "htm", "xhtml"],
        filenames: &[],
    },
    LanguageSpec {
        name: "bash",
        ast_lang: SupportLang::Bash,
        filters: &["bash", "sh", "zsh", "shell"],
        extensions: &[
            "bash", "bats", "cgi", "command", "env", "fcgi", "ksh", "sh", "zsh",
        ],
        filenames: &[".bashrc", ".bash_profile", ".zshrc", ".profile"],
    },
];

impl LanguageSpec {
    fn matches_filter(&self, filter: &str) -> bool {
        self.filters
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(filter))
    }
}

#[derive(Debug, Serialize)]
struct AstGrepReport {
    path: String,
    pattern: String,
    language: Option<String>,
    languages: Vec<String>,
    scanned_files: usize,
    skipped_unsupported_files: usize,
    skipped_files: usize,
    skipped_large_files: usize,
    parse_error_files: usize,
    pattern_error_languages: Vec<PatternLanguageError>,
    truncated: bool,
    matches: Vec<AstGrepMatch>,
}

#[derive(Debug, Serialize)]
struct PatternLanguageError {
    language: String,
    error: String,
}

#[derive(Debug, Serialize)]
struct AstGrepMatch {
    path: String,
    language: String,
    kind: String,
    line: usize,
    column: usize,
    end_line: usize,
    end_column: usize,
    byte_start: usize,
    byte_end: usize,
    text: String,
    captures: Vec<AstGrepCapture>,
}

#[derive(Debug, Serialize)]
struct AstGrepCapture {
    name: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct AstEditReport {
    path: String,
    pattern: String,
    replacement: String,
    language: Option<String>,
    dry_run: bool,
    applied: bool,
    count: usize,
    files_changed: usize,
    scanned_files: usize,
    skipped_unsupported_files: usize,
    skipped_files: usize,
    skipped_large_files: usize,
    parse_error_files: usize,
    pattern_error_languages: Vec<PatternLanguageError>,
    truncated: bool,
    diff: String,
    diff_truncated: bool,
    files: Vec<AstEditFileReport>,
}

#[derive(Debug, Serialize)]
struct AstEditFileReport {
    path: String,
    language: String,
    replacements: usize,
    replacements_detail: Vec<AstEditReplacement>,
    #[serde(skip_serializing)]
    diff: String,
}

#[derive(Debug, Serialize)]
struct AstEditReplacement {
    line: usize,
    column: usize,
    old_text: String,
    new_text: String,
}

fn ast_grep(workspace_root: &Path, options: AstGrepOptions) -> Result<AstGrepReport> {
    let root = workspace_root
        .canonicalize()
        .context("canonicalize workspace root")?;
    let scope = resolve_scope(&root, options.path.as_deref())?;
    let language_filter = match options.language.as_deref() {
        Some(language) => Some(resolve_language_filter(language)?),
        None => None,
    };

    let mut files = Vec::new();
    let mut skipped_unsupported_files = 0;
    collect_supported_files(
        scope.as_path(),
        language_filter,
        &mut files,
        &mut skipped_unsupported_files,
    )?;

    let mut matches = Vec::new();
    let mut scanned_files = 0;
    let mut skipped_files = 0;
    let mut skipped_large_files = 0;
    let mut parse_error_files = 0;
    let mut pattern_errors = HashMap::<&'static str, String>::new();
    let mut compiled_patterns = HashMap::<&'static str, Pattern>::new();

    for file in files {
        if matches.len() >= options.limit {
            break;
        }
        let metadata = match fs::metadata(&file.path) {
            Ok(metadata) => metadata,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        if !metadata.is_file() {
            skipped_files += 1;
            continue;
        }
        if metadata.len() as usize > options.max_file_bytes {
            skipped_large_files += 1;
            continue;
        }
        let pattern = match compiled_patterns.get(file.language.name) {
            Some(pattern) => pattern,
            None if pattern_errors.contains_key(file.language.name) => continue,
            None => match Pattern::try_new(&options.pattern, file.language.ast_lang) {
                Ok(pattern) => &*compiled_patterns
                    .entry(file.language.name)
                    .or_insert(pattern),
                Err(err) => {
                    pattern_errors.insert(file.language.name, err.to_string());
                    continue;
                }
            },
        };
        let source = match fs::read_to_string(&file.path) {
            Ok(source) => source,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        scanned_files += 1;
        let doc = match StrDoc::try_new(&source, file.language.ast_lang) {
            Ok(doc) => doc,
            Err(_) => {
                parse_error_files += 1;
                continue;
            }
        };
        let root_ast = AstGrep::doc(doc);
        let remaining = options.limit - matches.len();
        for node_match in root_ast.root().find_all(pattern).take(remaining) {
            matches.push(format_match(&root, &file, &node_match));
        }
    }

    let mut languages = matches
        .iter()
        .map(|mat| mat.language.clone())
        .collect::<Vec<_>>();
    languages.sort();
    languages.dedup();

    let mut pattern_error_languages = pattern_errors
        .into_iter()
        .map(|(language, error)| PatternLanguageError {
            language: language.to_string(),
            error,
        })
        .collect::<Vec<_>>();
    pattern_error_languages.sort_by(|left, right| left.language.cmp(&right.language));

    Ok(AstGrepReport {
        path: options.path.unwrap_or_else(|| ".".to_string()),
        pattern: options.pattern,
        language: options.language,
        languages,
        scanned_files,
        skipped_unsupported_files,
        skipped_files,
        skipped_large_files,
        parse_error_files,
        pattern_error_languages,
        truncated: matches.len() >= options.limit,
        matches,
    })
}

fn ast_edit(workspace_root: &Path, options: AstEditOptions) -> Result<AstEditReport> {
    let root = workspace_root
        .canonicalize()
        .context("canonicalize workspace root")?;
    let scope = resolve_scope(&root, options.path.as_deref())?;
    let language_filter = match options.language.as_deref() {
        Some(language) => Some(resolve_language_filter(language)?),
        None => None,
    };

    let mut files = Vec::new();
    let mut skipped_unsupported_files = 0;
    collect_supported_files(
        scope.as_path(),
        language_filter,
        &mut files,
        &mut skipped_unsupported_files,
    )?;

    let mut file_reports = Vec::new();
    let mut total_replacements = 0;
    let mut scanned_files = 0;
    let mut skipped_files = 0;
    let mut skipped_large_files = 0;
    let mut parse_error_files = 0;
    let mut pattern_errors = HashMap::<&'static str, String>::new();
    let mut compiled_patterns = HashMap::<&'static str, Pattern>::new();

    for file in files {
        if total_replacements >= options.limit {
            break;
        }
        let metadata = match fs::metadata(&file.path) {
            Ok(metadata) => metadata,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        if !metadata.is_file() {
            skipped_files += 1;
            continue;
        }
        if metadata.len() as usize > options.max_file_bytes {
            skipped_large_files += 1;
            continue;
        }
        let pattern = match compiled_patterns.get(file.language.name) {
            Some(pattern) => pattern,
            None if pattern_errors.contains_key(file.language.name) => continue,
            None => match Pattern::try_new(&options.pattern, file.language.ast_lang) {
                Ok(pattern) => &*compiled_patterns
                    .entry(file.language.name)
                    .or_insert(pattern),
                Err(err) => {
                    pattern_errors.insert(file.language.name, err.to_string());
                    continue;
                }
            },
        };
        let source = match fs::read_to_string(&file.path) {
            Ok(source) => source,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        scanned_files += 1;
        let remaining = options.limit - total_replacements;
        let rewrite = rewrite_source(
            &source,
            file.language.ast_lang,
            pattern,
            &options.replacement,
            remaining,
        );
        let rewrite = match rewrite {
            Ok(rewrite) => rewrite,
            Err(_) => {
                parse_error_files += 1;
                continue;
            }
        };
        if rewrite.replacements.is_empty() {
            continue;
        }
        total_replacements += rewrite.replacements.len();
        if !options.dry_run {
            fs::write(&file.path, &rewrite.updated)
                .with_context(|| format!("write rewritten file {}", file.path.display()))?;
        }
        let relative = relative_path(&root, &file.path);
        let file_diff = render_file_diff(&relative, &source, &rewrite.updated);
        file_reports.push(AstEditFileReport {
            path: relative,
            language: file.language.name.to_string(),
            replacements: rewrite.replacements.len(),
            replacements_detail: rewrite.replacements,
            diff: file_diff,
        });
    }

    let mut pattern_error_languages = pattern_errors
        .into_iter()
        .map(|(language, error)| PatternLanguageError {
            language: language.to_string(),
            error,
        })
        .collect::<Vec<_>>();
    pattern_error_languages.sort_by(|left, right| left.language.cmp(&right.language));

    let (diff, diff_truncated) = truncate_diff(
        file_reports
            .iter()
            .map(|file| file.diff.as_str())
            .filter(|chunk| !chunk.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    );

    Ok(AstEditReport {
        path: options.path.unwrap_or_else(|| ".".to_string()),
        pattern: options.pattern,
        replacement: options.replacement,
        language: options.language,
        dry_run: options.dry_run,
        applied: !options.dry_run && !file_reports.is_empty(),
        count: total_replacements,
        files_changed: file_reports.len(),
        scanned_files,
        skipped_unsupported_files,
        skipped_files,
        skipped_large_files,
        parse_error_files,
        pattern_error_languages,
        truncated: total_replacements >= options.limit,
        diff,
        diff_truncated,
        files: file_reports,
    })
}

struct RewriteResult {
    updated: String,
    replacements: Vec<AstEditReplacement>,
}

fn rewrite_source(
    source: &str,
    lang: SupportLang,
    pattern: &Pattern,
    replacement: &str,
    limit: usize,
) -> Result<RewriteResult> {
    let doc = StrDoc::try_new(source, lang).map_err(|err| anyhow::anyhow!(err))?;
    let mut root = AstGrep::doc(doc);
    let mut replacements = Vec::new();
    while replacements.len() < limit {
        let node = root.root();
        let Some(node_match) = node.find(pattern) else {
            break;
        };
        let matched_node = node_match.get_node();
        let start = matched_node.start_pos();
        let old_text = matched_node.text().to_string();
        let edit = node_match.replace_by(replacement);
        let new_text = String::from_utf8_lossy(&edit.inserted_text).into_owned();
        replacements.push(AstEditReplacement {
            line: start.line() + 1,
            column: start.column(matched_node) + 1,
            old_text: truncate_chars(&old_text, MAX_REPLACEMENT_TEXT_CHARS),
            new_text: truncate_chars(&new_text, MAX_REPLACEMENT_TEXT_CHARS),
        });
        root.edit(edit).map_err(|err| anyhow::anyhow!(err))?;
    }
    Ok(RewriteResult {
        updated: root.get_text().to_string(),
        replacements,
    })
}

fn render_file_diff(path: &str, before: &str, after: &str) -> String {
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let max = before_lines.len().max(after_lines.len());
    for index in 0..max {
        let old_line = before_lines.get(index).copied().unwrap_or("");
        let new_line = after_lines.get(index).copied().unwrap_or("");
        if old_line == new_line {
            continue;
        }
        if !old_line.is_empty() {
            out.push('-');
            out.push_str(old_line);
            out.push('\n');
        }
        if !new_line.is_empty() {
            out.push('+');
            out.push_str(new_line);
            out.push('\n');
        }
    }
    out
}

fn truncate_diff(diff: String) -> (String, bool) {
    if diff.chars().count() <= MAX_PREVIEW_DIFF_CHARS {
        return (diff, false);
    }
    let truncated: String = diff.chars().take(MAX_PREVIEW_DIFF_CHARS).collect();
    (
        format!("{truncated}\n... diff truncated after {MAX_PREVIEW_DIFF_CHARS} characters ..."),
        true,
    )
}

fn format_match(
    workspace_root: &Path,
    file: &SourceFile,
    node_match: &ast_grep_core::matcher::NodeMatch<'_, StrDoc<SupportLang>>,
) -> AstGrepMatch {
    let node = node_match.get_node();
    let start = node.start_pos();
    let end = node.end_pos();
    let range = node.range();
    AstGrepMatch {
        path: relative_path(workspace_root, &file.path),
        language: file.language.name.to_string(),
        kind: node.kind().to_string(),
        line: start.line() + 1,
        column: start.column(node) + 1,
        end_line: end.line() + 1,
        end_column: end.column(node) + 1,
        byte_start: range.start,
        byte_end: range.end,
        text: truncate_chars(&node.text(), MAX_MATCH_TEXT_CHARS),
        captures: format_captures(node_match),
    }
}

fn format_captures(
    node_match: &ast_grep_core::matcher::NodeMatch<'_, StrDoc<SupportLang>>,
) -> Vec<AstGrepCapture> {
    let env = node_match.get_env();
    let mut captures = Vec::new();
    for variable in env.get_matched_variables() {
        match variable {
            MetaVariable::Capture(name, _) => {
                if let Some(node) = env.get_match(&name) {
                    captures.push(AstGrepCapture {
                        name,
                        text: truncate_chars(&node.text(), MAX_CAPTURE_TEXT_CHARS),
                    });
                }
            }
            MetaVariable::MultiCapture(name) => {
                let text = env
                    .get_multiple_matches(&name)
                    .iter()
                    .map(Node::text)
                    .collect::<Vec<_>>()
                    .join(" ");
                captures.push(AstGrepCapture {
                    name,
                    text: truncate_chars(&text, MAX_CAPTURE_TEXT_CHARS),
                });
            }
            MetaVariable::Dropped(_) | MetaVariable::Multiple => {}
        }
    }
    captures.sort_by(|left, right| left.name.cmp(&right.name));
    captures
}

fn resolve_language_filter(filter: &str) -> Result<&'static str> {
    LANGUAGES
        .iter()
        .find(|language| language.matches_filter(filter))
        .map(|language| language.name)
        .with_context(|| format!("unsupported ast-grep language filter: {filter}"))
}

fn bounded_usize(value: Option<&Value>, default: usize, max: usize, name: &str) -> Result<usize> {
    let Some(value) = value else {
        return Ok(default);
    };
    let Some(number) = value.as_u64() else {
        bail!("`{name}` must be a positive integer");
    };
    if number == 0 || number > max as u64 {
        bail!("`{name}` must be between 1 and {max}");
    }
    Ok(number as usize)
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
        if file_type.is_dir() {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| SKIP_DIRS.contains(&name))
            {
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
        language_filter.is_none_or(|filter| language.name == filter)
            && (language.filenames.contains(&file_name)
                || extension
                    .as_deref()
                    .is_some_and(|extension| language.extensions.contains(&extension)))
    })
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
    fn finds_rust_function_pattern() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            &dir.path().join("src/lib.rs"),
            "fn keep() {}\nfn skip(arg: usize) { let _ = arg; }\n",
        );

        let report = ast_grep(
            dir.path(),
            AstGrepOptions {
                pattern: "fn $NAME() {}".to_string(),
                path: None,
                language: Some("rust".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert_eq!(report.matches.len(), 1);
        let mat = &report.matches[0];
        assert_eq!(mat.path, "src/lib.rs");
        assert_eq!(mat.language, "rust");
        assert_eq!(mat.kind, "function_item");
        assert!(
            mat.captures
                .iter()
                .any(|capture| capture.name == "NAME" && capture.text == "keep")
        );
    }

    #[test]
    fn filters_by_path_and_language() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("app.py"), "print(value)\n");
        write(&dir.path().join("web/app.ts"), "console.log(value);\n");

        let report = ast_grep(
            dir.path(),
            AstGrepOptions {
                pattern: "console.log($A)".to_string(),
                path: Some("web".to_string()),
                language: Some("typescript".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert_eq!(report.languages, vec!["typescript"]);
        assert_eq!(report.matches.len(), 1);
        assert_eq!(report.matches[0].path, "web/app.ts");
        assert!(
            report.matches[0]
                .captures
                .iter()
                .any(|capture| capture.name == "A" && capture.text == "value")
        );
    }

    #[tokio::test]
    async fn ast_grep_tool_executes_structural_search() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("src/lib.rs"), "fn target() {}\n");

        let capability = AstGrepCapability::new(host(dir.path()));
        let tools = capability.tools();
        let tool = tools
            .iter()
            .find(|tool| tool.name() == "ast_grep")
            .expect("ast_grep tool");
        let result = run_tool(
            tool.as_ref(),
            json!({
                "pattern": "fn $NAME() {}",
                "language": "rust",
                "limit": 10
            }),
        )
        .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };

        assert_eq!(value["count"], json!(1));
        assert_eq!(value["matches"][0]["path"], json!("src/lib.rs"));
        assert_eq!(value["matches"][0]["captures"][0]["name"], json!("NAME"));
        assert_eq!(value["matches"][0]["captures"][0]["text"], json!("target"));
    }

    #[tokio::test]
    async fn ast_grep_tool_accepts_repository_relative_path_scope() {
        // Like repo_map, ast_grep resolves repository-relative paths through
        // resolve_host_scope.
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("src/lib.rs"), "fn target() {}\n");
        write(&dir.path().join("other/lib.rs"), "fn other() {}\n");

        let capability = AstGrepCapability::new(host(dir.path()));
        let tools = capability.tools();
        let tool = tools
            .iter()
            .find(|tool| tool.name() == "ast_grep")
            .expect("ast_grep tool");
        let result = run_tool(
            tool.as_ref(),
            json!({
                "pattern": "fn $NAME() {}",
                "language": "rust",
                "path": "src",
                "limit": 10
            }),
        )
        .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };

        assert_eq!(value["count"], json!(1));
        assert_eq!(value["matches"][0]["path"], json!("src/lib.rs"));
    }

    #[test]
    fn accepts_host_absolute_subpath() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("src/lib.rs"), "fn target() {}\n");

        let report = ast_grep(
            dir.path(),
            AstGrepOptions {
                pattern: "fn $NAME() {}".to_string(),
                path: Some(dir.path().join("src").display().to_string()),
                language: Some("rust".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("host subpath should scan");

        assert_eq!(report.matches.len(), 1);
        assert_eq!(report.matches[0].path, "src/lib.rs");
    }

    #[test]
    fn rejects_paths_outside_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = ast_grep(
            dir.path(),
            AstGrepOptions {
                pattern: "fn $NAME() {}".to_string(),
                path: Some("../outside".to_string()),
                language: Some("rust".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect_err("outside path should fail");

        assert!(err.to_string().contains("inside the workspace"));
    }

    #[test]
    fn rejects_oversized_numeric_limits_before_casting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = ast_grep_from_arguments(
            dir.path(),
            &json!({
                "pattern": "fn $NAME() {}",
                "limit": u64::MAX,
            }),
        )
        .expect_err("oversized limit should fail");

        assert!(err.to_string().contains("between 1 and"));
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_directories_during_recursive_scan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        write(&dir.path().join("src/lib.rs"), "fn inside_fn() {}\n");
        write(&outside.path().join("outside.rs"), "fn outside_fn() {}\n");
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked_outside"))
            .expect("symlink outside dir");

        let report = ast_grep(
            dir.path(),
            AstGrepOptions {
                pattern: "fn $NAME() {}".to_string(),
                path: None,
                language: Some("rust".to_string()),
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("scan");

        assert!(
            report
                .matches
                .iter()
                .any(|mat| mat.text == "fn inside_fn() {}")
        );
        assert!(
            !report
                .matches
                .iter()
                .any(|mat| mat.text == "fn outside_fn() {}")
        );
    }

    #[test]
    fn ast_edit_dry_run_previews_without_writing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.ts");
        write(&path, "console.log(value);\n");

        let report = ast_edit(
            dir.path(),
            AstEditOptions {
                pattern: "console.log($$$ARGS)".to_string(),
                replacement: "logger.info($$$ARGS)".to_string(),
                path: None,
                language: Some("typescript".to_string()),
                dry_run: true,
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("preview rewrite");

        assert_eq!(report.count, 1);
        assert!(report.dry_run);
        assert!(!report.applied);
        assert!(report.diff.contains("console.log"));
        assert!(report.diff.contains("logger.info"));
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "console.log(value);\n"
        );
    }

    #[test]
    fn ast_edit_applies_rewrite_when_dry_run_false() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.ts");
        write(&path, "console.log(value);\n");

        let report = ast_edit(
            dir.path(),
            AstEditOptions {
                pattern: "console.log($$$ARGS)".to_string(),
                replacement: "logger.info($$$ARGS)".to_string(),
                path: None,
                language: Some("typescript".to_string()),
                dry_run: false,
                limit: 20,
                max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            },
        )
        .expect("apply rewrite");

        assert_eq!(report.count, 1);
        assert!(!report.dry_run);
        assert!(report.applied);
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "logger.info(value);\n"
        );
    }

    #[tokio::test]
    async fn ast_edit_tool_defaults_to_preview_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("app.ts"), "console.log(value);\n");

        let capability = AstEditCapability::new(host(dir.path()));
        let tool = capability
            .tools()
            .into_iter()
            .find(|tool| tool.name() == "ast_edit")
            .expect("ast_edit tool");
        let result = run_tool(
            tool.as_ref(),
            json!({
                "pattern": "console.log($$$ARGS)",
                "replacement": "logger.info($$$ARGS)",
                "language": "typescript"
            }),
        )
        .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };

        assert_eq!(value["dry_run"], json!(true));
        assert_eq!(value["applied"], json!(false));
        assert_eq!(value["count"], json!(1));
        assert!(value["diff"].as_str().unwrap_or("").contains("logger.info"));
    }

    #[test]
    fn capability_exposes_ast_grep_tool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let capability = AstGrepCapability::new(host(dir.path()));
        let names: Vec<String> = capability
            .tools()
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();
        assert_eq!(names, vec!["ast_grep"]);
    }

    #[test]
    fn capability_exposes_ast_edit_tool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let capability = AstEditCapability::new(host(dir.path()));
        let names: Vec<String> = capability
            .tools()
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();
        assert_eq!(names, vec!["ast_edit"]);
    }
}
