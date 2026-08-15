//! Session-scoped filesystem capability for Everruns agents.
//!
//! This crate adapts Everruns' neutral filesystem and tool contracts into the
//! `session_file_system` capability. Its read, write, edit, list, grep, delete,
//! and stat tools operate only through the filesystem supplied by the host.
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and is the
//! default offline integration selected by the `everruns` Framework facade.
//!
//! # Example
//!
//! ```
//! use everruns_core::Capability;
//! use everruns_integrations_filesystem::FileSystemCapability;
//!
//! assert_eq!(FileSystemCapability.id(), "session_file_system");
//! ```

use crate::error::{FileSystemErrorClass, classify_fs_error};
use crate::session_file::SessionFile;
use crate::tool_output_sanitizer::build_binary_read_file_result;
use crate::tool_types::{ToolDefinition, ToolHints};
use crate::tools::{Tool, ToolExecutionResult};
use crate::truncation_info::{TruncationInfo, TruncationReason};
use async_trait::async_trait;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext, ToolDefinitionHook,
};
use everruns_core::session_files::SessionFileSystem;
use everruns_core::tool_context::{ToolContext, ToolContextService};
use everruns_core::*;
#[cfg(test)]
use everruns_provider::error::AgentLoopError;
#[cfg(test)]
use everruns_provider::typed_id;
use everruns_provider::{ToolResultImage, error, tool_types};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use std::result::Result;
use std::sync::Arc;

/// Detect the MIME type of an image format supported by model providers.
fn image_media_type(content: &str) -> Option<&'static str> {
    use base64::Engine as _;

    // Decode only the prefix needed by supported formats. This avoids trusting an
    // attacker-controlled extension and avoids decoding large image payloads twice.
    let encoded = content.as_bytes();
    let prefix_len = encoded.len().min(16);
    let prefix_len = prefix_len - (prefix_len % 4);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&encoded[..prefix_len])
        .ok()?;

    match bytes.as_slice() {
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some("image/gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        _ => None,
    }
}

/// Workspace prefix used in file paths
const WORKSPACE_PREFIX: &str = "/workspace";
const SESSION_FILE_SYSTEM_TOOL_NAMES: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "list_directory",
    "grep_files",
    "delete_file",
    "stat_file",
];
const MAX_EDIT_DIFF_CHARS: usize = 16_000;
const LIST_DIRECTORY_DEFAULT_LIMIT: usize = 200;
const LIST_DIRECTORY_MAX_LIMIT: usize = 1_000;
const GREP_FILES_DEFAULT_LIMIT: usize = 200;
const GREP_FILES_MAX_LIMIT: usize = 1_000;

fn escape_xml_text(content: &str) -> String {
    content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Model-visible path identity derived from the active primary `SessionFileSystem`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FilePathPresentation {
    root: String,
}

impl FilePathPresentation {
    fn vfs() -> Self {
        Self {
            root: WORKSPACE_PREFIX.to_string(),
        }
    }

    fn from_context(ctx: &SystemPromptContext) -> Self {
        Self::from_file_store(
            ctx.file_store
                .as_ref()
                .map(|store| store.as_ref() as &dyn SessionFileSystem),
        )
    }

    fn from_file_store(store: Option<&dyn SessionFileSystem>) -> Self {
        let root = store
            .map(SessionFileSystem::display_root)
            .unwrap_or_else(|| WORKSPACE_PREFIX.to_string());
        Self { root }
    }

    fn uses_vfs_namespace(&self) -> bool {
        self.root == WORKSPACE_PREFIX
    }

    fn root_guidance(&self) -> String {
        if self.uses_vfs_namespace() {
            format!(
                "Workspace root: `{WORKSPACE_PREFIX}`. All file paths must start with `{WORKSPACE_PREFIX}`. "
            )
        } else {
            let escaped_root = escape_xml_text(&self.root);
            format!(
                "Workspace root: `{escaped_root}`. Paths may be relative to this root or absolute under it. "
            )
        }
    }

    fn system_prompt_preview(&self) -> String {
        if self.uses_vfs_namespace() {
            format!(
                "Workspace root: `{WORKSPACE_PREFIX}`. All file paths must start with `{WORKSPACE_PREFIX}`."
            )
        } else {
            format!(
                "Workspace root: `{}`. Paths may be relative to this root or absolute under it.",
                self.root
            )
        }
    }

    fn path_param_description(&self, example: &str) -> String {
        if self.uses_vfs_namespace() {
            format!(
                "Workspace-relative path (e.g., '{example}'). A leading '/' or '{WORKSPACE_PREFIX}/' prefix is also accepted."
            )
        } else {
            format!(
                "Path relative to `{}` (e.g., '{example}') or an absolute path under `{}`.",
                self.root, self.root
            )
        }
    }

    fn generic_path_param_description(&self) -> String {
        if self.uses_vfs_namespace() {
            "Path to the file or directory. A leading '/' or '/workspace/' prefix is also accepted."
                .to_string()
        } else {
            format!(
                "Path relative to `{}` or an absolute path under `{}`.",
                self.root, self.root
            )
        }
    }

    fn list_directory_path_description(&self) -> String {
        if self.uses_vfs_namespace() {
            format!(
                "Workspace-relative directory path to list (e.g., 'src'). Defaults to the workspace root; a leading '/' or '{WORKSPACE_PREFIX}/' prefix is also accepted."
            )
        } else {
            format!(
                "Directory path relative to `{}` (e.g., 'src'). Defaults to `{}` when omitted.",
                self.root, self.root
            )
        }
    }

    fn parameters_schema_for_tool(&self, tool_name: &str) -> Option<Value> {
        match tool_name {
            "read_file" => Some(read_file_parameters_schema(self)),
            "write_file" => Some(write_file_parameters_schema(self)),
            "edit_file" => Some(edit_file_parameters_schema(self)),
            "list_directory" => Some(list_directory_parameters_schema(self)),
            "grep_files" => Some(grep_files_parameters_schema()),
            "delete_file" => Some(delete_file_parameters_schema(self)),
            "stat_file" => Some(stat_file_parameters_schema(self)),
            _ => None,
        }
    }
}

struct FilePathPresentationHook {
    presentation: FilePathPresentation,
}

impl ToolDefinitionHook for FilePathPresentationHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        tools
            .into_iter()
            .map(|tool| {
                if !SESSION_FILE_SYSTEM_TOOL_NAMES.contains(&tool.name()) {
                    return tool;
                }
                let Some(schema) = self.presentation.parameters_schema_for_tool(tool.name()) else {
                    return tool;
                };
                match tool {
                    ToolDefinition::Builtin(mut builtin) => {
                        builtin.parameters = schema.clone();
                        if let Some(full) = builtin.full_parameters.as_mut() {
                            *full = schema;
                        }
                        ToolDefinition::Builtin(builtin)
                    }
                    ToolDefinition::ClientSide(mut client) => {
                        client.parameters = schema.clone();
                        if let Some(full) = client.full_parameters.as_mut() {
                            *full = schema;
                        }
                        ToolDefinition::ClientSide(client)
                    }
                }
            })
            .collect()
    }
}

fn read_file_parameters_schema(presentation: &FilePathPresentation) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": presentation.path_param_description("docs/readme.txt")
            },
            "offset": {
                "type": "integer",
                "description": "Starting line number (0-indexed). Default: 0",
                "default": 0,
                "minimum": 0
            },
            "limit": {
                "type": "integer",
                "description": "Max lines to return. Default varies by file type: 2000 (source/text), 500 (logs, tail-biased), 100 (CSV/TSV with header). Explicit value always wins.",
                "default": 2000,
                "minimum": 1
            }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

fn write_file_parameters_schema(presentation: &FilePathPresentation) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": presentation.path_param_description("docs/notes.txt")
            },
            "content": {
                "type": "string",
                "description": "Content to write to the file"
            },
            "encoding": {
                "type": "string",
                "enum": ["text", "base64"],
                "default": "text",
                "description": "Content encoding: 'text' for plain text, 'base64' for binary data"
            }
        },
        "required": ["path", "content"],
        "additionalProperties": false
    })
}

fn edit_file_parameters_schema(presentation: &FilePathPresentation) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": presentation.path_param_description("src/main.rs")
            },
            "expected_hash": {
                "type": "string",
                "description": "Current content hash from read_file or write_file (format: 'sha256:...')"
            },
            "edits": {
                "type": "array",
                "description": "One or more replacements to apply, each matched against the original file content. Use a single-element array for one replacement.",
                "items": {
                    "type": "object",
                    "properties": {
                        "old_text": {
                            "type": "string",
                            "description": "Exact text to replace"
                        },
                        "new_text": {
                            "type": "string",
                            "description": "Replacement text"
                        }
                    },
                    "required": ["old_text", "new_text"],
                    "additionalProperties": false
                },
                "minItems": 1
            }
        },
        "required": ["path", "expected_hash", "edits"],
        "additionalProperties": false
    })
}

fn list_directory_parameters_schema(presentation: &FilePathPresentation) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "default": presentation.root,
                "description": presentation.list_directory_path_description()
            },
            "offset": {
                "type": "integer",
                "description": "Starting item offset for large directories. Default: 0",
                "default": 0,
                "minimum": 0
            },
            "limit": {
                "type": "integer",
                "description": "Max directory entries to return. Default: 200, maximum: 1000",
                "default": LIST_DIRECTORY_DEFAULT_LIMIT,
                "minimum": 1,
                "maximum": LIST_DIRECTORY_MAX_LIMIT
            }
        },
        "additionalProperties": false
    })
}

fn grep_files_parameters_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": {
                "type": "string",
                "description": "Regex pattern to search for"
            },
            "path_pattern": {
                "type": "string",
                "description": "Optional glob filtering canonical paths (e.g., '*.txt', 'docs/*', 'src/**/*.rs'). Basename-only globs match at any depth; non-glob values use legacy substring matching"
            },
            "before_context": {
                "type": "integer",
                "description": "Number of lines before each match. Default: 0, maximum: 20. Overlapping ranges are merged",
                "default": 0,
                "minimum": 0,
                "maximum": crate::GREP_MAX_CONTEXT_LINES
            },
            "after_context": {
                "type": "integer",
                "description": "Number of lines after each match. Default: 0, maximum: 20. Overlapping ranges are merged",
                "default": 0,
                "minimum": 0,
                "maximum": crate::GREP_MAX_CONTEXT_LINES
            },
            "offset": {
                "type": "integer",
                "description": "Starting match offset. Default: 0",
                "default": 0,
                "minimum": 0
            },
            "limit": {
                "type": "integer",
                "description": "Max matches to return. Default: 200, maximum: 1000",
                "default": GREP_FILES_DEFAULT_LIMIT,
                "minimum": 1,
                "maximum": GREP_FILES_MAX_LIMIT
            }
        },
        "required": ["pattern"],
        "additionalProperties": false
    })
}

fn delete_file_parameters_schema(presentation: &FilePathPresentation) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": presentation.generic_path_param_description()
            },
            "recursive": {
                "type": "boolean",
                "default": false,
                "description": "If true, delete directories and all contents recursively"
            }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

fn stat_file_parameters_schema(presentation: &FilePathPresentation) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": presentation.generic_path_param_description()
            }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

#[cfg(test)]
fn schema_contains_workspace(value: &Value) -> bool {
    fn walk(value: &Value) -> bool {
        match value {
            Value::String(text) => text.contains(WORKSPACE_PREFIX),
            Value::Array(items) => items.iter().any(walk),
            Value::Object(fields) => fields.values().any(walk),
            _ => false,
        }
    }
    walk(value)
}

#[cfg(test)]
fn filesystem_tool_schemas_with_presentation(
    presentation: &FilePathPresentation,
) -> Vec<(String, Value)> {
    SESSION_FILE_SYSTEM_TOOL_NAMES
        .iter()
        .filter_map(|name| {
            presentation
                .parameters_schema_for_tool(name)
                .map(|schema| ((*name).to_string(), schema))
        })
        .collect()
}

// ============================================================================
// Content-type detection (EVE-249)
// ============================================================================

/// Content type categories for read_file default behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentType {
    /// Source code, markdown, config — standard 2000-line default
    Text,
    /// Log files — tail-biased (last 500 lines)
    Log,
    /// CSV/TSV data — 100-line default with header prepend
    Csv,
    /// Known binary formats — metadata only (no inline content)
    Binary,
    /// Minified files — first 500 chars only
    Minified,
}

/// Read mode for content-type-aware defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadMode {
    /// Read from the beginning (standard)
    FromOffset,
    /// Read from the end (tail-biased for logs)
    FromEnd,
    /// Return metadata only, no content
    MetadataOnly,
}

/// Detect content type from file extension.
fn content_type_from_extension(path: &str) -> ContentType {
    let lower = path.to_lowercase();

    // Check minified first (.min.js, .min.css) before generic .js/.css
    if lower.ends_with(".min.js") || lower.ends_with(".min.css") {
        return ContentType::Minified;
    }

    // Log files
    if lower.ends_with(".log") || lower.ends_with(".out") {
        return ContentType::Log;
    }

    // CSV/TSV data files
    if lower.ends_with(".csv") || lower.ends_with(".tsv") {
        return ContentType::Csv;
    }

    // Binary formats (images already handled separately via image_media_type)
    const BINARY_EXTENSIONS: &[&str] = &[
        ".wasm", ".zip", ".tar", ".gz", ".bz2", ".xz", ".zst", ".7z", ".rar", ".exe", ".dll",
        ".so", ".dylib", ".bin", ".dat", ".o", ".a", ".pyc", ".class", ".woff", ".woff2", ".ttf",
        ".otf", ".eot", ".ico", ".bmp", ".tiff", ".tif", ".psd", ".mp3", ".mp4", ".avi", ".mov",
        ".flv", ".wmv", ".pdf",
    ];
    if BINARY_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        return ContentType::Binary;
    }

    ContentType::Text
}

/// Resolve effective limit and read mode based on content type.
/// Returns (limit, read_mode). Explicit user values always win.
fn effective_read_defaults(
    path: &str,
    explicit_offset: bool,
    explicit_limit: bool,
) -> (usize, ReadMode) {
    if explicit_limit && explicit_offset {
        // User provided both — don't override anything
        return (0, ReadMode::FromOffset); // limit is already set by caller
    }
    match content_type_from_extension(path) {
        ContentType::Log if !explicit_offset => (500, ReadMode::FromEnd),
        ContentType::Log => (500, ReadMode::FromOffset),
        ContentType::Csv => (100, ReadMode::FromOffset),
        ContentType::Binary => (0, ReadMode::MetadataOnly),
        ContentType::Minified => (20, ReadMode::FromOffset), // ~20 lines, capped by byte limit
        ContentType::Text => (
            crate::tool_output_sanitizer::READ_FILE_DEFAULT_LIMIT,
            ReadMode::FromOffset,
        ),
    }
}

fn fs_display_path(file_store: &dyn SessionFileSystem, path: &str) -> String {
    file_store.display_path(path)
}

fn fs_input_display_path(file_store: &dyn SessionFileSystem, path: &str) -> String {
    if file_store.is_mount_resolver() {
        file_store.resolve_path(path)
    } else {
        file_store.display_path(&file_store.resolve_path(path))
    }
}

fn file_content_hash(content: &str, encoding: &str) -> crate::error::Result<String> {
    let bytes = SessionFile::decode_content(content, encoding)
        .map_err(|error| anyhow::anyhow!("failed to decode file content for hashing: {error}"))?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}

fn session_file_content_hash(file: &SessionFile) -> crate::error::Result<String> {
    file_content_hash(file.content.as_deref().unwrap_or_default(), &file.encoding)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Cr,
    Crlf,
}

fn strip_utf8_bom(content: &str) -> (bool, &str) {
    if let Some(stripped) = content.strip_prefix('\u{feff}') {
        (true, stripped)
    } else {
        (false, content)
    }
}

fn detect_line_ending(content: &str) -> LineEnding {
    if content.contains("\r\n") {
        LineEnding::Crlf
    } else if content.contains('\r') {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    }
}

fn align_to_file_line_endings(content: &str, line_ending: LineEnding) -> String {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    match line_ending {
        LineEnding::Lf => normalized,
        LineEnding::Cr => normalized.replace('\n', "\r"),
        LineEnding::Crlf => normalized.replace('\n', "\r\n"),
    }
}

fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

fn truncate_snippet(content: &str, max_chars: usize) -> String {
    let clean = content.replace('\n', "\\n").replace('\r', "\\r");
    if clean.chars().count() <= max_chars {
        clean
    } else {
        let truncated: String = clean.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

fn first_changed_line(before: &str, after: &str) -> Option<usize> {
    if before == after {
        return None;
    }

    let before = normalize_line_endings(before);
    let after = normalize_line_endings(after);
    let before_lines: Vec<&str> = before.split('\n').collect();
    let after_lines: Vec<&str> = after.split('\n').collect();

    for index in 0..before_lines.len().max(after_lines.len()) {
        if before_lines.get(index) != after_lines.get(index) {
            return Some(index + 1);
        }
    }

    Some(1)
}

fn render_unified_diff(path: &str, before: &str, after: &str) -> String {
    TextDiff::from_lines(
        normalize_line_endings(before).as_str(),
        normalize_line_endings(after).as_str(),
    )
    .unified_diff()
    .context_radius(2)
    .header(&format!("{path} (before)"), &format!("{path} (after)"))
    .to_string()
}

fn truncate_diff(diff: String) -> (String, bool) {
    if diff.chars().count() <= MAX_EDIT_DIFF_CHARS {
        return (diff, false);
    }

    let truncated: String = diff.chars().take(MAX_EDIT_DIFF_CHARS).collect();
    (
        format!("{truncated}\n... diff truncated after {MAX_EDIT_DIFF_CHARS} characters ..."),
        true,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextEdit {
    old_text: String,
    new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedEdit {
    start: usize,
    end: usize,
    replacement: String,
}

/// Coerce a legacy top-level `old_text`/`new_text` pair into a single edit.
///
/// The advertised `edit_file` schema is `edits[]`-only (EVE-620), but stored
/// calls and stubborn structured-tool-call models may still emit the scalar
/// fields. Rather than rejecting (the EVE-616 corrective error is now a last
/// resort), we fold them into `edits[]` — the same `prepareArguments` approach
/// pi uses. Empty-string placeholders (which some models emit alongside a real
/// `edits[]`) are treated as absent, not as an error.
fn coerce_top_level_edit(arguments: &Value) -> std::result::Result<Option<TextEdit>, String> {
    let old_text_arg = arguments.get("old_text");
    let new_text_arg = arguments.get("new_text");
    if old_text_arg.is_none() && new_text_arg.is_none() {
        return Ok(None);
    }

    let old_text = old_text_arg
        .and_then(Value::as_str)
        .ok_or_else(|| "Legacy top-level old_text must be a string".to_string())?;
    let new_text = new_text_arg
        .and_then(Value::as_str)
        .ok_or_else(|| "Legacy top-level new_text must be a string".to_string())?;

    // Empty legacy placeholders carry no replacement target.
    if old_text.is_empty() {
        if !new_text.is_empty() {
            return Err("Legacy top-level old_text cannot be empty".to_string());
        }
        return Ok(None);
    }

    Ok(Some(TextEdit {
        old_text: old_text.to_string(),
        new_text: new_text.to_string(),
    }))
}

fn parse_text_edits(arguments: &Value) -> std::result::Result<Vec<TextEdit>, String> {
    let mut edits: Vec<TextEdit> = Vec::new();

    // Backward-compat: fold a legacy top-level old_text/new_text pair into
    // edits[] instead of rejecting (EVE-620).
    if let Some(top_level) = coerce_top_level_edit(arguments)? {
        edits.push(top_level);
    }

    if let Some(array) = arguments.get("edits").and_then(Value::as_array) {
        for (index, edit) in array.iter().enumerate() {
            let old_text = edit
                .get("old_text")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("Edit {} is missing old_text", index + 1))?;
            let new_text = edit
                .get("new_text")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("Edit {} is missing new_text", index + 1))?;
            if old_text.is_empty() {
                return Err(format!("Edit {} has an empty old_text", index + 1));
            }
            let edit = TextEdit {
                old_text: old_text.to_string(),
                new_text: new_text.to_string(),
            };
            // Dedup the coerced top-level edit when a model duplicates it into
            // edits[] (the gpt-5.5 mixed-mode pattern): two identical edits would
            // otherwise match the same span and trip the overlap check.
            if !edits.contains(&edit) {
                edits.push(edit);
            }
        }
    }

    if edits.is_empty() {
        return Err(
            "edit_file requires a non-empty edits[] array; each entry needs old_text and new_text"
                .to_string(),
        );
    }

    Ok(edits)
}

fn plan_text_edits(
    content: &str,
    edits: &[TextEdit],
) -> std::result::Result<Vec<PlannedEdit>, String> {
    let (_, body) = strip_utf8_bom(content);
    let line_ending = detect_line_ending(body);
    let mut planned = Vec::with_capacity(edits.len());

    for edit in edits {
        let old_text = align_to_file_line_endings(
            edit.old_text
                .strip_prefix('\u{feff}')
                .unwrap_or(&edit.old_text),
            line_ending,
        );
        let new_text = align_to_file_line_endings(
            edit.new_text
                .strip_prefix('\u{feff}')
                .unwrap_or(&edit.new_text),
            line_ending,
        );

        let mut matches = body.match_indices(&old_text);
        let Some((start, _)) = matches.next() else {
            return Err(format!(
                "Could not find an exact match for old_text: '{}'",
                truncate_snippet(&old_text, 80)
            ));
        };
        if matches.next().is_some() {
            return Err(format!(
                "old_text is ambiguous and matched multiple locations: '{}'",
                truncate_snippet(&old_text, 80)
            ));
        }

        planned.push(PlannedEdit {
            start,
            end: start + old_text.len(),
            replacement: new_text,
        });
    }

    planned.sort_by_key(|edit| edit.start);
    for pair in planned.windows(2) {
        if pair[1].start < pair[0].end {
            return Err("Edits overlap in the target file".to_string());
        }
    }

    Ok(planned)
}

fn apply_text_edits(
    content: &str,
    edits: &[TextEdit],
) -> std::result::Result<(String, usize), String> {
    let (had_bom, body) = strip_utf8_bom(content);
    let planned = plan_text_edits(content, edits)?;

    let mut edited = String::with_capacity(content.len());
    let mut cursor = 0;
    for edit in &planned {
        edited.push_str(&body[cursor..edit.start]);
        edited.push_str(&edit.replacement);
        cursor = edit.end;
    }
    edited.push_str(&body[cursor..]);

    if had_bom {
        edited.insert(0, '\u{feff}');
    }

    Ok((edited, planned.len()))
}

pub const SESSION_FILE_SYSTEM_CAPABILITY_ID: &str = "session_file_system";

/// Activate the session filesystem with its default configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileSystem;

impl everruns_capability::IntoCapability for FileSystem {
    fn into_capability(self) -> everruns_capability::CapabilitySpec {
        everruns_capability::CapabilityRef::new(SESSION_FILE_SYSTEM_CAPABILITY_ID).into()
    }
}

/// Session File System capability - provides file operations for session storage
pub struct FileSystemCapability;

#[async_trait]
impl Capability for FileSystemCapability {
    fn id(&self) -> &str {
        SESSION_FILE_SYSTEM_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "File System"
    }

    fn description(&self) -> &str {
        r#"Tools to access and manipulate files in the session workspace - read, write, list, grep, and more.

> [!NOTE]
> Each session has its own isolated workspace. Files persist for the session duration.

> [!TIP]
> Use `list_directory` to explore the workspace structure before reading or writing files."#
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Файлова система",
            r#"Інструменти для доступу до файлів у робочому просторі сесії та роботи з ними — читання, запис, перегляд, пошук grep тощо.

> [!NOTE]
> Кожна сесія має власний ізольований робочий простір. Файли зберігаються протягом усієї сесії.

> [!TIP]
> Використовуйте `list_directory`, щоб дослідити структуру робочого простору перед читанням або записом файлів."#,
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("hard-drive")
    }

    fn category(&self) -> Option<&str> {
        Some("File Operations")
    }

    async fn system_prompt_contribution(&self, ctx: &SystemPromptContext) -> Option<String> {
        use crate::tool_output_sanitizer::READ_ECONOMY_HINT;
        let presentation = FilePathPresentation::from_context(ctx);
        Some(format!(
            "<capability id=\"{}\">\n{}Directories are created on write. Read files before claiming what they contain — never speculate about code you have not opened.{}\n</capability>",
            self.id(),
            presentation.root_guidance(),
            READ_ECONOMY_HINT
        ))
    }

    fn system_prompt_preview(&self) -> Option<String> {
        Some(FilePathPresentation::vfs().system_prompt_preview())
    }

    fn tool_definition_hooks_with_context(
        &self,
        ctx: &SystemPromptContext,
        _config: &serde_json::Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(FilePathPresentationHook {
            presentation: FilePathPresentation::from_context(ctx),
        })]
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ReadFileTool),
            Box::new(WriteFileTool),
            Box::new(EditFileTool),
            Box::new(ListDirectoryTool),
            Box::new(GrepFilesTool),
            Box::new(DeleteFileTool),
            Box::new(StatFileTool),
        ]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

// ============================================================================
// ReadFileTool
// ============================================================================

/// Tool to read file content
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_read_file(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "read_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read File")
    }

    fn description(&self) -> &str {
        "Read a file from the session workspace. Returns text content directly. For image files (PNG, JPEG, GIF, WebP), the image is returned as a native image so you can see it visually. This is NOT for reading files in cloud sandboxes — use the sandbox-specific read tool instead."
    }

    fn parameters_schema(&self) -> Value {
        read_file_parameters_schema(&FilePathPresentation::vfs())
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "read_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        use crate::tool_output_sanitizer::{
            READ_FILE_DEFAULT_LIMIT, apply_read_file_hard_cap, format_lines,
        };

        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };

        let explicit_offset = arguments.get("offset").and_then(|v| v.as_u64()).is_some();
        let explicit_limit = arguments.get("limit").and_then(|v| v.as_u64()).is_some();

        let mut offset = arguments
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let mut limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(READ_FILE_DEFAULT_LIMIT as u64) as usize;

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        // The store (MountFs in production) is the sole resolver: hand it the
        // raw path and it routes `/workspace`, the root mount, and relatives.
        let normalized_path = path.to_string();
        let display_path = fs_input_display_path(file_store.as_ref(), &normalized_path);

        match file_store
            .read_file(context.session_id, &normalized_path)
            .await
        {
            Ok(Some(file)) => {
                let resolved_path = file.path.as_str();
                let display_path = fs_display_path(file_store.as_ref(), resolved_path);

                if file.is_directory {
                    return ToolExecutionResult::tool_error(format!(
                        "Path '{}' is a directory, not a file. Use list_directory instead.",
                        display_path
                    ));
                }

                // Return supported image formats as native image content. Detect the format from
                // bytes rather than the path so extensionless and mislabeled files work safely.
                if file.encoding == "base64"
                    && let Some(ref content) = file.content
                    && let Some(media_type) = image_media_type(content)
                {
                    let content_hash = match file_content_hash(content, &file.encoding) {
                        Ok(hash) => hash,
                        Err(e) => return ToolExecutionResult::internal_error(e),
                    };
                    return ToolExecutionResult::success_with_images(
                        json!({
                            "path": display_path,
                            "media_type": media_type,
                            "size_bytes": file.size_bytes,
                            "content_hash": content_hash
                        }),
                        vec![ToolResultImage {
                            base64: content.clone(),
                            media_type: media_type.to_string(),
                        }],
                    );
                }

                let content_hash = match session_file_content_hash(&file) {
                    Ok(hash) => hash,
                    Err(e) => return ToolExecutionResult::internal_error(e),
                };

                // Non-image binary files: return metadata only. Base64 payloads
                // are token-expensive and usually not useful to the model.
                if file.encoding == "base64" {
                    let mut result = build_binary_read_file_result(
                        &display_path,
                        file.size_bytes as usize,
                        "base64",
                    );
                    result["content_hash"] = json!(content_hash);
                    return ToolExecutionResult::success(result);
                }

                let raw_content = file.content.as_deref().unwrap_or("");

                // Apply content-type-aware defaults (EVE-249)
                let (ct_limit, read_mode) =
                    effective_read_defaults(resolved_path, explicit_offset, explicit_limit);
                let content_type = content_type_from_extension(resolved_path);

                // Metadata-only for known binary extensions
                if read_mode == ReadMode::MetadataOnly {
                    let mut result = build_binary_read_file_result(
                        &display_path,
                        file.size_bytes as usize,
                        "binary",
                    );
                    result["content_hash"] = json!(content_hash);
                    return ToolExecutionResult::success(result);
                }

                // Apply content-type defaults when user didn't specify
                if !explicit_limit {
                    limit = ct_limit;
                }

                // Tail-biased reading for log files
                if read_mode == ReadMode::FromEnd && !explicit_offset {
                    let total = raw_content.lines().count();
                    offset = total.saturating_sub(limit);
                }

                let (formatted, total_lines, truncated) = format_lines(raw_content, offset, limit);

                // CSV: prepend header row when reading from an offset past line 0
                let formatted = if content_type == ContentType::Csv && offset > 0 {
                    if let Some(header) = raw_content.lines().next() {
                        format!("1|{header}\n{formatted}")
                    } else {
                        formatted
                    }
                } else {
                    formatted
                };

                let shown_count = total_lines.saturating_sub(offset).min(limit);
                let (start_line, end_line) = if shown_count == 0 {
                    (0, 0)
                } else {
                    (offset + 1, offset + shown_count)
                };

                // Generate structural outline for unread portions (EVE-248)
                let mut formatted = if truncated && start_line > 0 {
                    let outline_items =
                        crate::outline::generate_outline(raw_content, resolved_path);
                    if let Some(outline_text) = crate::outline::format_outline(
                        &outline_items,
                        start_line,
                        end_line,
                        total_lines,
                    ) {
                        format!("{formatted}{outline_text}")
                    } else {
                        formatted
                    }
                } else {
                    formatted
                };
                // Reapply hard cap after any post-format decorations (e.g. outlines).
                let hard_capped = apply_read_file_hard_cap(&mut formatted);
                let truncated = truncated || hard_capped;

                let mut result = json!({
                    "path": display_path,
                    "content": formatted,
                    "total_lines": total_lines,
                    "lines_shown": {
                        "start": start_line,
                        "end": end_line
                    },
                    "truncated": truncated,
                    "size_bytes": file.size_bytes,
                    "content_hash": content_hash
                });

                // Add content_type and read_mode metadata (EVE-249)
                if content_type != ContentType::Text {
                    let ct_label = match content_type {
                        ContentType::Log => "log",
                        ContentType::Csv => "csv",
                        ContentType::Minified => "minified",
                        _ => "text",
                    };
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert("content_type".to_string(), json!(ct_label));
                        if read_mode == ReadMode::FromEnd {
                            obj.insert("read_mode".to_string(), json!("tail"));
                        }
                    }
                }

                // Unified reading-tool truncation envelope (EVE-339).
                //
                // Distinguishing which cap fired:
                // - When `end_line < total_lines` the line window was clipped
                //   by `limit`, so this is a line cap and line-based resume is
                //   safe.
                // - When `truncated == true` but `end_line == total_lines` the
                //   line window covered every line and the cut must have come
                //   from the byte cap inside `format_lines`. Byte truncation
                //   can cut mid-line, so `next_offset = end_line` is not a
                //   reliable resume point — emit `without_resume` and let the
                //   caller narrow `limit` or shift `offset`.
                let truncation = if truncated {
                    if end_line < total_lines {
                        TruncationInfo::with_resume(
                            formatted.len(),
                            Some(file.size_bytes as usize),
                            end_line as u64,
                            format!(
                                "call read_file with offset={} to resume from line {}",
                                end_line,
                                end_line + 1,
                            ),
                            TruncationReason::LineCap,
                        )
                    } else {
                        TruncationInfo::without_resume(
                            formatted.len(),
                            Some(file.size_bytes as usize),
                            TruncationReason::SizeCap,
                        )
                    }
                } else {
                    TruncationInfo::not_truncated(formatted.len())
                };
                truncation.attach(&mut result);

                ToolExecutionResult::success(result)
            }
            Ok(None) => {
                ToolExecutionResult::tool_error(format!("File not found: {}", display_path))
            }
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

// ============================================================================
// WriteFileTool
// ============================================================================

/// Tool to write/create a file
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_write_file(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "write_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Write File")
    }

    fn description(&self) -> &str {
        "Create or update a file in the session workspace. Parent directories are created automatically. This is NOT for writing files in cloud sandboxes — use sandbox-specific write tools (e.g. daytona_write_file, e2b_write_file) instead."
    }

    fn parameters_schema(&self) -> Value {
        write_file_parameters_schema(&FilePathPresentation::vfs())
    }

    fn hints(&self) -> ToolHints {
        // Mutates the shared session workspace: serialize against other
        // workspace writes (and bash) within a batch to avoid races.
        ToolHints::default()
            .with_idempotent(true)
            .with_concurrency_class("session_workspace")
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "write_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };

        let content = match arguments.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolExecutionResult::tool_error("Missing required parameter: content"),
        };

        let encoding = arguments
            .get("encoding")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        // The store (MountFs in production) is the sole resolver: hand it the
        // raw path and it routes `/workspace`, the root mount, and relatives.
        let normalized_path = path.to_string();
        let display_path = fs_input_display_path(file_store.as_ref(), &normalized_path);

        match file_store
            .write_file(context.session_id, &normalized_path, content, encoding)
            .await
        {
            Ok(file) => {
                let content_hash = match session_file_content_hash(&file) {
                    Ok(hash) => hash,
                    Err(e) => return ToolExecutionResult::internal_error(e),
                };
                ToolExecutionResult::success(json!({
                    "path": display_path,
                    "size_bytes": file.size_bytes,
                    "created": true,
                    "content_hash": content_hash
                }))
            }
            Err(e) => write_failure_result(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

/// Map a write/edit failure to a tool error (agent-correctable) or an internal
/// error. Read-only targets and directory-vs-file mismatches are the agent's to
/// fix; everything else is internal. EVE-645: routed through the typed
/// [`classify_fs_error`] seam instead of inline `msg.contains(...)`.
fn write_failure_result<E>(e: E) -> ToolExecutionResult
where
    E: std::error::Error + Send + Sync + 'static,
{
    match classify_fs_error(&e) {
        FileSystemErrorClass::ReadOnly | FileSystemErrorClass::IsADirectory => {
            ToolExecutionResult::tool_error(e.to_string())
        }
        _ => ToolExecutionResult::internal_error(e),
    }
}

// ============================================================================
// EditFileTool
// ============================================================================

/// Tool to apply exact text replacements to an existing text file
pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_edit_file(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "edit_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Edit File")
    }

    fn description(&self) -> &str {
        "Apply one or more exact text replacements to an existing text file. Requires the current content hash from read_file or write_file. Provide every replacement as an entry in edits[] (use a single-element array for one replacement)."
    }

    fn parameters_schema(&self) -> Value {
        edit_file_parameters_schema(&FilePathPresentation::vfs())
    }

    fn hints(&self) -> ToolHints {
        // Mutates the shared session workspace: serialize against other
        // workspace writes (and bash) within a batch to avoid races.
        ToolHints::default().with_concurrency_class("session_workspace")
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "edit_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(path) => path,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };
        let expected_hash = match arguments.get("expected_hash").and_then(|v| v.as_str()) {
            Some(hash) => hash,
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: expected_hash",
                );
            }
        };
        let edits = match parse_text_edits(&arguments) {
            Ok(edits) => edits,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // The store (MountFs in production) is the sole resolver: hand it the
        // raw path and it routes `/workspace`, the root mount, and relatives.
        let normalized_path = path.to_string();
        let display_path = fs_input_display_path(file_store.as_ref(), &normalized_path);

        let existing = match file_store
            .read_file(context.session_id, &normalized_path)
            .await
        {
            Ok(Some(file)) => file,
            Ok(None) => {
                return ToolExecutionResult::tool_error(format!(
                    "File not found: {}",
                    display_path
                ));
            }
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        if existing.is_directory {
            return ToolExecutionResult::tool_error(format!(
                "Path '{}' is a directory, not a file. Use list_directory instead.",
                display_path
            ));
        }

        if existing.encoding != "text" {
            return ToolExecutionResult::tool_error(format!(
                "File '{}' is not a text file. edit_file only supports text files; use write_file for binary/base64 content.",
                display_path
            ));
        }

        let current_hash = match session_file_content_hash(&existing) {
            Ok(hash) => hash,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };
        let rebased = expected_hash != current_hash;

        let current_content = existing.content.unwrap_or_default();
        // Plan every exact, unique hunk against one current snapshot, then commit
        // the whole result with compare-and-swap. This safely rebases across an
        // unrelated stale change while missing, ambiguous, overlapping, or racing
        // hunks leave the file untouched. Fuzzy matching was rejected because it
        // can silently select the wrong occurrence; the returned content hash
        // invalidates caller-side workspace and validation caches after success.
        let (updated_content, applied_edits) = match apply_text_edits(&current_content, &edits) {
            Ok(result) => result,
            Err(error) if rebased => {
                return ToolExecutionResult::tool_error(format!(
                    "File '{}' changed since the last read (expected {}, found {}) and the edits conflict with its current content: {}. Read the file again before editing.",
                    display_path, expected_hash, current_hash, error
                ));
            }
            Err(error) => return ToolExecutionResult::tool_error(error),
        };

        let first_changed_line = first_changed_line(&current_content, &updated_content);
        let (diff, diff_truncated) = truncate_diff(render_unified_diff(
            &display_path,
            &current_content,
            &updated_content,
        ));

        match file_store
            .write_file_if_content_matches(
                context.session_id,
                &normalized_path,
                &current_content,
                "text",
                &updated_content,
                "text",
            )
            .await
        {
            Ok(updated_file) => {
                let Some(updated_file) = updated_file else {
                    let latest = match file_store
                        .read_file(context.session_id, &normalized_path)
                        .await
                    {
                        Ok(file) => file,
                        Err(e) => return ToolExecutionResult::internal_error(e),
                    };

                    return match latest {
                        Some(file) if file.is_directory => {
                            ToolExecutionResult::tool_error(format!(
                                "Path '{}' is a directory, not a file. Use list_directory instead.",
                                display_path
                            ))
                        }
                        Some(file) if file.is_readonly => ToolExecutionResult::tool_error(format!(
                            "Cannot modify readonly file: {}",
                            display_path
                        )),
                        Some(file) if file.encoding != "text" => {
                            ToolExecutionResult::tool_error(format!(
                                "File '{}' is not a text file. edit_file only supports text files; use write_file for binary/base64 content.",
                                display_path
                            ))
                        }
                        Some(file) => {
                            let latest_hash = match session_file_content_hash(&file) {
                                Ok(hash) => hash,
                                Err(e) => return ToolExecutionResult::internal_error(e),
                            };
                            ToolExecutionResult::tool_error(format!(
                                "File '{}' changed since the last read. Expected {}, found {}. Read the file again before editing.",
                                display_path, expected_hash, latest_hash
                            ))
                        }
                        None => ToolExecutionResult::tool_error(format!(
                            "File not found: {}",
                            display_path
                        )),
                    };
                };

                let new_hash = match session_file_content_hash(&updated_file) {
                    Ok(hash) => hash,
                    Err(e) => return ToolExecutionResult::internal_error(e),
                };
                ToolExecutionResult::success(json!({
                    "path": display_path,
                    "size_bytes": updated_file.size_bytes,
                    "content_hash": new_hash,
                    "previous_content_hash": current_hash,
                    "applied_edits": applied_edits,
                    "rebased": rebased,
                    "first_changed_line": first_changed_line,
                    "diff": diff,
                    "diff_truncated": diff_truncated
                }))
            }
            Err(e) => write_failure_result(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

// ============================================================================
// ListDirectoryTool
// ============================================================================

/// Tool to list directory contents
pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_list_directory(
            &tool_call.arguments,
            phase,
            locale,
            ctx,
        ))
    }

    fn name(&self) -> &str {
        "list_directory"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Directory")
    }

    fn description(&self) -> &str {
        "List files and directories at a given path. Returns file metadata including size and type."
    }

    fn parameters_schema(&self) -> Value {
        list_directory_parameters_schema(&FilePathPresentation::vfs())
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "list_directory requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let offset = arguments
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let limit = match arguments.get("limit").and_then(|v| v.as_u64()) {
            Some(0) => return ToolExecutionResult::tool_error("limit must be greater than 0"),
            Some(value) => (value as usize).min(LIST_DIRECTORY_MAX_LIMIT),
            None => LIST_DIRECTORY_DEFAULT_LIMIT,
        };

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| "/".to_string());

        // Normalize path to strip /workspace prefix for storage
        // The store (MountFs in production) is the sole resolver: hand it the
        // raw path and it routes `/workspace`, the root mount, and relatives.
        let normalized_path = path.to_string();
        let display_path = fs_input_display_path(file_store.as_ref(), &normalized_path);

        match file_store
            .list_directory(context.session_id, &normalized_path)
            .await
        {
            Ok(files) => {
                let total_count = files.len();
                let entries: Vec<Value> = files
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .map(|f| {
                        json!({
                            "name": f.name,
                            "path": fs_display_path(file_store.as_ref(), &f.path),
                            "is_directory": f.is_directory,
                            "size_bytes": f.size_bytes,
                            "is_readonly": f.is_readonly
                        })
                    })
                    .collect();

                let mut result = json!({
                    "path": display_path,
                    "entries": entries,
                    "count": entries.len(),
                    "total_count": total_count,
                    "offset": offset,
                    "limit": limit
                });
                let bytes_returned = serde_json::to_string(&entries)
                    .expect("list_directory entries always serialize")
                    .len();
                let next_offset = offset.saturating_add(entries.len());
                let truncation = if next_offset < total_count {
                    TruncationInfo::with_resume(
                        bytes_returned,
                        None,
                        next_offset as u64,
                        format!(
                            "call list_directory with offset={} to resume from item {}",
                            next_offset,
                            next_offset + 1
                        ),
                        TruncationReason::ItemCap,
                    )
                } else {
                    TruncationInfo::not_truncated(bytes_returned)
                };
                truncation.attach(&mut result);
                ToolExecutionResult::success(result)
            }
            Err(e) => match classify_fs_error(&e) {
                // A missing or non-directory listing target is the agent's to
                // fix; everything else is internal. EVE-645: typed seam.
                FileSystemErrorClass::NotFound | FileSystemErrorClass::NotADirectory => {
                    ToolExecutionResult::tool_error(e.to_string())
                }
                _ => ToolExecutionResult::internal_error(e),
            },
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

// ============================================================================
// GrepFilesTool
// ============================================================================

/// Tool to search files by pattern
pub struct GrepFilesTool;

#[async_trait]
impl Tool for GrepFilesTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_grep_files(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "grep_files"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Grep Files")
    }

    fn description(&self) -> &str {
        "Search file contents using a Rust regex. Optionally returns bounded before/after context as merged blocks with numbered lines and explicit match markers. Offset and limit paginate matches, not context lines; output is capped at 64 KiB with resume metadata."
    }

    fn parameters_schema(&self) -> Value {
        grep_files_parameters_schema()
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "grep_files requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let pattern = match arguments.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: pattern"),
        };

        let path_pattern = arguments.get("path_pattern").and_then(|v| v.as_str());
        let parse_context = |name: &str| -> Result<usize, String> {
            let Some(value) = arguments.get(name) else {
                return Ok(0);
            };
            let Some(value) = value.as_i64() else {
                return Err(format!("{name} must be a non-negative integer"));
            };
            if value < 0 {
                return Err(format!("{name} must be a non-negative integer"));
            }
            let value = value as usize;
            if value > crate::GREP_MAX_CONTEXT_LINES {
                return Err(format!(
                    "{name} must not exceed {}",
                    crate::GREP_MAX_CONTEXT_LINES
                ));
            }
            Ok(value)
        };
        let before_context = match parse_context("before_context") {
            Ok(value) => value,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let after_context = match parse_context("after_context") {
            Ok(value) => value,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let offset = arguments
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let limit = match arguments.get("limit").and_then(|v| v.as_u64()) {
            Some(0) => return ToolExecutionResult::tool_error("limit must be greater than 0"),
            Some(value) => (value as usize).min(GREP_FILES_MAX_LIMIT),
            None => GREP_FILES_DEFAULT_LIMIT,
        };

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        match file_store
            .grep_files_with_options(
                context.session_id,
                pattern,
                &crate::GrepOptions {
                    path_pattern: path_pattern.map(ToString::to_string),
                    before_context,
                    after_context,
                    offset,
                    limit,
                    max_bytes: crate::GREP_MAX_RETURN_BYTES,
                },
            )
            .await
        {
            Ok(search) => {
                let results: Vec<Value> = search
                    .matches
                    .iter()
                    .map(|m| {
                        json!({
                            "path": fs_display_path(file_store.as_ref(), &m.path),
                            "line_number": m.line_number,
                            "line": m.line
                        })
                    })
                    .collect();
                let blocks: Vec<Value> = search
                    .blocks
                    .iter()
                    .map(|block| {
                        json!({
                            "path": fs_display_path(file_store.as_ref(), &block.path),
                            "start_line": block.start_line,
                            "end_line": block.end_line,
                            "match_line_numbers": block.match_line_numbers,
                            "lines": block.lines
                        })
                    })
                    .collect();

                let mut result = json!({
                    "pattern": pattern,
                    "match_count": search.returned_matches,
                    "total_matches": search.total_matches,
                    "offset": offset,
                    "limit": limit
                });
                if before_context == 0 && after_context == 0 {
                    result["matches"] = Value::Array(results);
                } else {
                    result["blocks"] = Value::Array(blocks);
                }
                let truncation = if let Some(next_offset) = search.next_offset {
                    TruncationInfo::with_resume(
                        search.bytes_returned,
                        Some(search.bytes_total),
                        next_offset as u64,
                        format!(
                            "call grep_files with offset={} to resume from match {}",
                            next_offset,
                            next_offset + 1
                        ),
                        if search.byte_truncated {
                            TruncationReason::SizeCap
                        } else {
                            TruncationReason::LineCap
                        },
                    )
                } else if search.byte_truncated {
                    TruncationInfo::without_resume(
                        search.bytes_returned,
                        Some(search.bytes_total),
                        TruncationReason::SizeCap,
                    )
                } else {
                    TruncationInfo::not_truncated(search.bytes_returned)
                };
                truncation.attach(&mut result);
                ToolExecutionResult::success(result)
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("regex") || msg.contains("pattern") {
                    ToolExecutionResult::tool_error(format!("Invalid regex pattern: {}", msg))
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

// ============================================================================
// DeleteFileTool
// ============================================================================

/// Tool to delete a file or directory
pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_delete_file(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "delete_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Delete File")
    }

    fn description(&self) -> &str {
        "Delete a file or directory. Use recursive=true to delete non-empty directories."
    }

    fn parameters_schema(&self) -> Value {
        delete_file_parameters_schema(&FilePathPresentation::vfs())
    }

    fn hints(&self) -> ToolHints {
        // Mutates the shared session workspace: serialize against other
        // workspace writes (and bash) within a batch to avoid races.
        ToolHints::default()
            .with_destructive(true)
            .with_idempotent(true)
            .with_concurrency_class("session_workspace")
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "delete_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };

        let recursive = arguments
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        // The store (MountFs in production) is the sole resolver: hand it the
        // raw path and it routes `/workspace`, the root mount, and relatives.
        let normalized_path = path.to_string();
        let display_path = fs_input_display_path(file_store.as_ref(), &normalized_path);

        match file_store
            .delete_file(context.session_id, &normalized_path, recursive)
            .await
        {
            Ok(deleted) => {
                if deleted {
                    ToolExecutionResult::success(json!({
                        "path": display_path,
                        "deleted": true
                    }))
                } else {
                    ToolExecutionResult::tool_error(format!("File not found: {}", display_path))
                }
            }
            Err(e) => match classify_fs_error(&e) {
                // A non-empty directory deleted without `recursive` is the
                // agent's to fix; everything else is internal. EVE-645: typed
                // seam. (The legacy `recursive` substring maps to NotEmpty so a
                // "without recursive flag" / "recursive delete failed" message
                // keeps surfacing as a tool error.)
                FileSystemErrorClass::NotEmpty => ToolExecutionResult::tool_error(e.to_string()),
                _ => ToolExecutionResult::internal_error(e),
            },
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

// ============================================================================
// StatFileTool
// ============================================================================

/// Tool to get file metadata
pub struct StatFileTool;

#[async_trait]
impl Tool for StatFileTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_stat_file(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "stat_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("File Info")
    }

    fn description(&self) -> &str {
        "Get metadata about a file or directory (exists, size, type, dates)."
    }

    fn parameters_schema(&self) -> Value {
        stat_file_parameters_schema(&FilePathPresentation::vfs())
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "stat_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        // The store (MountFs in production) is the sole resolver: hand it the
        // raw path and it routes `/workspace`, the root mount, and relatives.
        let normalized_path = path.to_string();
        let display_path = fs_input_display_path(file_store.as_ref(), &normalized_path);

        match file_store
            .stat_file(context.session_id, &normalized_path)
            .await
        {
            Ok(Some(stat)) => ToolExecutionResult::success(json!({
                "path": fs_display_path(file_store.as_ref(), &stat.path),
                "name": stat.name,
                "exists": true,
                "is_directory": stat.is_directory,
                "is_readonly": stat.is_readonly,
                "size_bytes": stat.size_bytes,
                "created_at": stat.created_at.to_rfc3339(),
                "updated_at": stat.updated_at.to_rfc3339()
            })),
            Ok(None) => ToolExecutionResult::success(json!({
                "path": display_path,
                "exists": false
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
    use crate::tool_types::ToolCall;

    #[tokio::test]
    async fn system_prompt_stays_within_budget() {
        let contribution = FileSystemCapability
            .system_prompt_contribution(&SystemPromptContext::without_file_store(SessionId::new()))
            .await
            .expect("filesystem contributes a prompt");
        assert!(
            contribution.len() <= 1000,
            "filesystem prompt grew to {} bytes",
            contribution.len()
        );
    }

    #[test]
    fn capability_narrates_its_own_tools_only() {
        let cap = FileSystemCapability;
        let read = ToolCall {
            id: "c1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({ "path": "/workspace/AGENTS.md" }),
        };
        assert_eq!(
            cap.narrate(
                None,
                &read,
                ToolNarrationPhase::Completed,
                None,
                ToolNarrationContext::default()
            ),
            Some("Read AGENTS.md".to_string())
        );
        // A tool this capability does not own returns None for its owner to handle.
        let bash = ToolCall {
            id: "c2".to_string(),
            name: "bash".to_string(),
            arguments: serde_json::json!({ "command": "ls" }),
        };
        assert_eq!(
            cap.narrate(
                None,
                &bash,
                ToolNarrationPhase::Started,
                None,
                ToolNarrationContext::default()
            ),
            None
        );
    }
    use crate::error::Result;
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::typed_id::SessionId;
    use chrono::Utc;
    use everruns_core::session_files::SessionFileSystem;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    #[derive(Debug, Clone)]
    struct StoredFile {
        content: Option<String>,
        encoding: String,
        is_directory: bool,
        is_readonly: bool,
        created_at: chrono::DateTime<Utc>,
        updated_at: chrono::DateTime<Utc>,
    }

    impl StoredFile {
        fn text(content: &str) -> Self {
            let now = Utc::now();
            Self {
                content: Some(content.to_string()),
                encoding: "text".to_string(),
                is_directory: false,
                is_readonly: false,
                created_at: now,
                updated_at: now,
            }
        }

        fn base64(content: &str) -> Self {
            let now = Utc::now();
            Self {
                content: Some(content.to_string()),
                encoding: "base64".to_string(),
                is_directory: false,
                is_readonly: false,
                created_at: now,
                updated_at: now,
            }
        }

        fn directory() -> Self {
            let now = Utc::now();
            Self {
                content: None,
                encoding: "text".to_string(),
                is_directory: true,
                is_readonly: false,
                created_at: now,
                updated_at: now,
            }
        }

        fn readonly_text(content: &str) -> Self {
            let mut entry = Self::text(content);
            entry.is_readonly = true;
            entry
        }
    }

    #[derive(Default)]
    struct MockFileStore {
        files: Mutex<HashMap<String, StoredFile>>,
        conditional_write_injections: Mutex<HashMap<String, StoredFile>>,
        display_root: Option<String>,
    }

    impl MockFileStore {
        fn with_display_root(root: &str) -> Self {
            Self {
                display_root: Some(root.to_string()),
                ..Self::default()
            }
        }

        fn insert(&self, path: &str, file: StoredFile) {
            self.files.lock().unwrap().insert(path.to_string(), file);
        }

        fn add_text_file(&self, path: &str, content: &str) {
            self.insert(path, StoredFile::text(content));
        }

        fn add_base64_file(&self, path: &str, content: &str) {
            self.insert(path, StoredFile::base64(content));
        }

        fn add_directory(&self, path: &str) {
            self.insert(path, StoredFile::directory());
        }

        fn add_readonly_text_file(&self, path: &str, content: &str) {
            self.insert(path, StoredFile::readonly_text(content));
        }

        fn content(&self, path: &str) -> Option<String> {
            self.files
                .lock()
                .unwrap()
                .get(path)
                .and_then(|file| file.content.clone())
        }

        fn inject_conditional_write_change(&self, path: &str, file: StoredFile) {
            self.conditional_write_injections
                .lock()
                .unwrap()
                .insert(path.to_string(), file);
        }

        fn entry_to_session_file(path: &str, entry: &StoredFile) -> SessionFile {
            let size_bytes = entry
                .content
                .as_deref()
                .map(|content| {
                    SessionFile::decode_content(content, &entry.encoding)
                        .map(|bytes| bytes.len() as i64)
                        .unwrap_or(content.len() as i64)
                })
                .unwrap_or(0);

            SessionFile {
                id: Uuid::new_v4(),
                session_id: Uuid::nil(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                content: entry.content.clone(),
                encoding: entry.encoding.clone(),
                is_directory: entry.is_directory,
                is_readonly: entry.is_readonly,
                size_bytes,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
            }
        }
    }

    #[async_trait]
    impl SessionFileSystem for MockFileStore {
        fn display_root(&self) -> String {
            self.display_root
                .clone()
                .unwrap_or_else(|| WORKSPACE_PREFIX.to_string())
        }

        fn display_path(&self, path: &str) -> String {
            match &self.display_root {
                Some(root) if path == "/" => root.clone(),
                Some(root) => format!(
                    "{}/{}",
                    root.trim_end_matches('/'),
                    path.trim_start_matches('/')
                ),
                None if path == "/" => WORKSPACE_PREFIX.to_string(),
                None if path.starts_with('/') => format!("{WORKSPACE_PREFIX}{path}"),
                None => format!("{WORKSPACE_PREFIX}/{path}"),
            }
        }

        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            _session_id: SessionId,
            path: &str,
        ) -> Result<Option<SessionFile>> {
            let files = self.files.lock().unwrap();
            Ok(files
                .get(path)
                .map(|entry| Self::entry_to_session_file(path, entry)))
        }

        async fn write_file(
            &self,
            _session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> Result<SessionFile> {
            let mut files = self.files.lock().unwrap();
            if let Some(existing) = files.get(path) {
                if existing.is_directory {
                    return Err(anyhow::anyhow!("Path '{}' is a directory", path).into());
                }
                if existing.is_readonly {
                    return Err(anyhow::anyhow!("File '{}' is readonly", path).into());
                }
            }

            let created_at = files
                .get(path)
                .map(|entry| entry.created_at)
                .unwrap_or_else(Utc::now);
            let entry = StoredFile {
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                is_directory: false,
                is_readonly: false,
                created_at,
                updated_at: Utc::now(),
            };
            files.insert(path.to_string(), entry.clone());
            Ok(Self::entry_to_session_file(path, &entry))
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            path: &str,
            _recursive: bool,
        ) -> Result<bool> {
            Ok(self.files.lock().unwrap().remove(path).is_some())
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            path: &str,
        ) -> Result<Vec<FileInfo>> {
            let prefix = if path == "/" {
                "/".to_string()
            } else {
                format!("{}/", path.trim_end_matches('/'))
            };
            let files = self.files.lock().unwrap();
            let mut entries: Vec<FileInfo> = files
                .iter()
                .filter_map(|(entry_path, entry)| {
                    if path != "/" && entry_path == path {
                        return None;
                    }
                    let rest = entry_path.strip_prefix(&prefix)?;
                    if rest.is_empty() || rest.contains('/') {
                        return None;
                    }
                    Some(FileInfo {
                        id: Uuid::new_v4(),
                        session_id: Uuid::nil(),
                        name: rest.to_string(),
                        path: entry_path.clone(),
                        is_directory: entry.is_directory,
                        is_readonly: entry.is_readonly,
                        size_bytes: entry
                            .content
                            .as_ref()
                            .map(|content| content.len() as i64)
                            .unwrap_or(0),
                        created_at: entry.created_at,
                        updated_at: entry.updated_at,
                    })
                })
                .collect();
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(entries)
        }

        async fn stat_file(&self, _session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
            let files = self.files.lock().unwrap();
            Ok(files.get(path).map(|entry| FileStat {
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                is_directory: entry.is_directory,
                is_readonly: entry.is_readonly,
                size_bytes: entry
                    .content
                    .as_ref()
                    .map(|content| content.len() as i64)
                    .unwrap_or(0),
                created_at: entry.created_at,
                updated_at: entry.updated_at,
            }))
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            pattern: &str,
            _path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            let files = self.files.lock().unwrap();
            let mut matches = Vec::new();
            for (path, entry) in files.iter() {
                if entry.is_directory || entry.encoding != "text" {
                    continue;
                }
                let Some(content) = entry.content.as_deref() else {
                    continue;
                };
                for (idx, line) in content.lines().enumerate() {
                    if line.contains(pattern) {
                        matches.push(GrepMatch {
                            path: path.clone(),
                            line_number: idx + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
            matches.sort_by(|a, b| {
                a.path
                    .cmp(&b.path)
                    .then_with(|| a.line_number.cmp(&b.line_number))
            });
            Ok(matches)
        }

        async fn grep_files_with_options(
            &self,
            _session_id: SessionId,
            pattern: &str,
            options: &crate::GrepOptions,
        ) -> Result<crate::GrepSearchResult> {
            let regex = regex::Regex::new(pattern).map_err(|error| {
                crate::AgentLoopError::tool(format!("Invalid regex pattern: {error}"))
            })?;
            let path_matcher = options
                .path_pattern
                .as_deref()
                .map(crate::session_path::GrepPathPattern::new)
                .transpose()?;
            let files = self
                .files
                .lock()
                .unwrap()
                .iter()
                .filter(|(path, entry)| {
                    !entry.is_directory
                        && entry.encoding == "text"
                        && path_matcher
                            .as_ref()
                            .is_none_or(|matcher| matcher.is_match(path))
                })
                .filter_map(|(path, entry)| {
                    entry
                        .content
                        .as_ref()
                        .map(|content| (path.clone(), content.clone()))
                })
                .collect();
            Ok(crate::session_file::build_grep_search_result(
                files, &regex, options,
            ))
        }

        async fn create_directory(&self, _session_id: SessionId, path: &str) -> Result<FileInfo> {
            self.add_directory(path);
            Ok(FileInfo {
                id: Uuid::new_v4(),
                session_id: Uuid::nil(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }

        async fn write_file_if_content_matches(
            &self,
            _session_id: SessionId,
            path: &str,
            expected_content: &str,
            expected_encoding: &str,
            content: &str,
            encoding: &str,
        ) -> Result<Option<SessionFile>> {
            let mut files = self.files.lock().unwrap();
            if let Some(injected) = self
                .conditional_write_injections
                .lock()
                .unwrap()
                .remove(path)
            {
                files.insert(path.to_string(), injected);
            }

            let Some(existing) = files.get(path).cloned() else {
                return Ok(None);
            };

            if existing.is_directory
                || existing.is_readonly
                || existing.encoding != expected_encoding
                || existing.content.unwrap_or_default() != expected_content
            {
                return Ok(None);
            }

            let entry = StoredFile {
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                is_directory: false,
                is_readonly: false,
                created_at: existing.created_at,
                updated_at: Utc::now(),
            };
            files.insert(path.to_string(), entry.clone());
            Ok(Some(Self::entry_to_session_file(path, &entry)))
        }
    }

    fn make_context(file_store: Arc<MockFileStore>) -> ToolContext {
        // Wrap in MountFs exactly as production does, so the file tools resolve
        // `/workspace` and the root mount through the same path they use live.
        ToolContext::with_file_store(SessionId::new(), crate::mount_fs::MountFs::wrap(file_store))
    }

    fn expect_success(result: ToolExecutionResult) -> Value {
        match result {
            ToolExecutionResult::Success(value) => value,
            ToolExecutionResult::SuccessWithImages { result, .. } => result,
            other => panic!("Expected success, got {other:?}"),
        }
    }

    fn expect_tool_error(result: ToolExecutionResult) -> String {
        match result {
            ToolExecutionResult::ToolError(message) => message,
            other => panic!("Expected tool error, got {other:?}"),
        }
    }

    async fn read_hash(context: &ToolContext, path: &str) -> String {
        let result = ReadFileTool
            .execute_with_context(json!({ "path": path }), context)
            .await;
        expect_success(result)["content_hash"]
            .as_str()
            .unwrap()
            .to_string()
    }

    // Path normalization now lives in `session_path` (and the resolver
    // `MountFs`), exercised there; the file tools just hand raw paths to the
    // store.

    #[test]
    fn test_display_path_root_defaults_to_workspace_namespace() {
        let store = MockFileStore::default();
        assert_eq!(fs_display_path(&store, "/"), "/workspace");
    }

    #[test]
    fn test_display_path_file_defaults_to_workspace_namespace() {
        let store = MockFileStore::default();
        assert_eq!(fs_display_path(&store, "/test.txt"), "/workspace/test.txt");
    }

    #[test]
    fn test_display_path_nested_defaults_to_workspace_namespace() {
        let store = MockFileStore::default();
        assert_eq!(
            fs_display_path(&store, "/foo/bar.txt"),
            "/workspace/foo/bar.txt"
        );
    }

    #[test]
    fn test_display_path_no_leading_slash_defaults_to_workspace_namespace() {
        let store = MockFileStore::default();
        assert_eq!(fs_display_path(&store, "test.txt"), "/workspace/test.txt");
    }

    #[tokio::test]
    async fn read_file_uses_mountfs_workspace_display_path() {
        // File tools run behind MountFs in production; mounted real-disk stores
        // must not leak host-absolute roots to model-visible output.
        let store = Arc::new(MockFileStore::with_display_root("/host/repo"));
        store.add_text_file("/notes.txt", "hello");
        let context = make_context(store);

        let result = ReadFileTool
            .execute_with_context(json!({ "path": "/workspace/notes.txt" }), &context)
            .await;
        let value = expect_success(result);

        assert_eq!(value["path"], "/workspace/notes.txt");
    }

    #[test]
    fn test_parse_text_edits_coerces_mixed_modes() {
        // EVE-620: the schema is edits[]-only, but a mixed-mode call (top-level
        // old_text/new_text AND a non-empty edits[]) must be coerced rather than
        // rejected — the top-level pair is folded in as a leading edit.
        let edits = parse_text_edits(&json!({
            "old_text": "a",
            "new_text": "b",
            "edits": [{"old_text": "c", "new_text": "d"}]
        }))
        .expect("mixed-mode call should be coerced, not rejected");

        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].old_text, "a");
        assert_eq!(edits[0].new_text, "b");
        assert_eq!(edits[1].old_text, "c");
        assert_eq!(edits[1].new_text, "d");
    }

    #[test]
    fn test_parse_text_edits_dedupes_duplicated_top_level_edit() {
        // EVE-620: gpt-5.5 duplicates the same edit into the scalar fields and
        // edits[]. Folding both verbatim would create two identical edits that
        // match the same span and trip the overlap check — dedup keeps it to one.
        let edits = parse_text_edits(&json!({
            "old_text": "a",
            "new_text": "b",
            "edits": [{"old_text": "a", "new_text": "b"}]
        }))
        .expect("duplicated mixed-mode call should be coerced to a single edit");

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].old_text, "a");
        assert_eq!(edits[0].new_text, "b");
    }

    #[test]
    fn test_parse_text_edits_coerces_top_level_only() {
        // Backward-compat: a legacy call with only top-level scalars (no edits[])
        // is folded into a single edit instead of rejected.
        let edits = parse_text_edits(&json!({
            "old_text": "hello",
            "new_text": "world"
        }))
        .expect("legacy top-level scalars should be coerced into edits[]");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].old_text, "hello");
        assert_eq!(edits[0].new_text, "world");
    }

    #[test]
    fn test_parse_text_edits_allows_legacy_top_level_deletion() {
        // Backward-compat: an explicit empty-string replacement is still a valid
        // deletion edit when both legacy scalar fields are well-formed strings.
        let edits = parse_text_edits(&json!({
            "old_text": "remove me",
            "new_text": ""
        }))
        .expect("explicit legacy deletion edit should be accepted");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].old_text, "remove me");
        assert_eq!(edits[0].new_text, "");
    }

    #[test]
    fn test_parse_text_edits_rejects_malformed_legacy_top_level_new_text() {
        let missing_err = parse_text_edits(&json!({
            "old_text": "remove me"
        }))
        .unwrap_err();
        assert!(
            missing_err.contains("new_text"),
            "error should mention new_text: {missing_err}"
        );

        let non_string_err = parse_text_edits(&json!({
            "old_text": "remove me",
            "new_text": null
        }))
        .unwrap_err();
        assert!(
            non_string_err.contains("new_text"),
            "error should mention new_text: {non_string_err}"
        );
    }

    #[test]
    fn test_parse_text_edits_requires_edits() {
        // No edits[] and no usable top-level pair is a hard error.
        let err = parse_text_edits(&json!({})).unwrap_err();
        assert!(err.contains("edits"), "error: {err}");
    }

    #[test]
    fn test_parse_text_edits_accepts_batch() {
        let edits = parse_text_edits(&json!({
            "edits": [
                {"old_text": "a", "new_text": "1"},
                {"old_text": "b", "new_text": "2"}
            ]
        }))
        .expect("batch mode should be accepted");
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[1].old_text, "b");
        assert_eq!(edits[1].new_text, "2");
    }

    #[test]
    fn test_parse_text_edits_allows_empty_single_placeholders_with_batch() {
        // EVE-498 compat: some models emit empty top-level placeholders alongside a
        // real edits[]. That is treated as batch mode, not an ambiguous mixed call.
        let edits = parse_text_edits(&json!({
            "old_text": "",
            "new_text": "",
            "edits": [{"old_text": "c", "new_text": "d"}]
        }))
        .expect("empty placeholders + batch should be accepted as batch");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].old_text, "c");
        assert_eq!(edits[0].new_text, "d");
    }

    #[test]
    fn test_apply_text_edits_rejects_overlaps() {
        let result = apply_text_edits(
            "abcdef",
            &[
                TextEdit {
                    old_text: "abcd".to_string(),
                    new_text: "wxyz".to_string(),
                },
                TextEdit {
                    old_text: "cdef".to_string(),
                    new_text: "1234".to_string(),
                },
            ],
        );

        assert_eq!(result.unwrap_err(), "Edits overlap in the target file");
    }

    // Metadata and tool-list constants are covered registry-wide by
    // `builtin_capabilities_satisfy_registry_invariants` in `capabilities::tests`;
    // the per-capability constant mirrors were removed.

    #[tokio::test]
    async fn test_capability_has_system_prompt() {
        let cap = FileSystemCapability;
        let ctx = SystemPromptContext::without_file_store(SessionId::new());
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(prompt.contains("/workspace"));
        assert!(prompt.contains("File reading economy"));
        assert!(prompt.contains("offset"));
        assert!(prompt.contains("total_lines"));
    }

    #[tokio::test]
    async fn system_prompt_uses_mounted_workspace_display_root() {
        let cap = FileSystemCapability;
        let store = Arc::new(MockFileStore::with_display_root("/host/repo"));
        let mounted = crate::mount_fs::MountFs::wrap(store);
        let ctx = SystemPromptContext {
            session_id: SessionId::new(),
            locale: None,
            file_store: Some(mounted),
            model: None,
        };

        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();

        assert!(prompt.contains("Workspace root: `/workspace`"));
        assert!(!prompt.contains("/host/repo"));
    }

    #[tokio::test]
    async fn system_prompt_backend_native_store_shows_host_root() {
        // #258 end-to-end: a local embedder whose MountFs opted into
        // backend-native display, wrapped by the same `scoped_prompt_file_store`
        // helper the reason/executor paths use, must surface real host paths in
        // the model-facing system prompt — not the `/workspace` alias.
        let cap = FileSystemCapability;
        let backend = Arc::new(MockFileStore::with_display_root("/host/repo"));
        let embedder_store: Arc<dyn SessionFileSystem> =
            Arc::new(crate::mount_fs::MountFs::new(backend).with_backend_display());
        let prompt_store = crate::mount_fs::scoped_prompt_file_store(
            embedder_store,
            crate::typed_id::WorkspaceId::from_seed(7),
        );
        let ctx = SystemPromptContext {
            session_id: SessionId::new(),
            locale: None,
            file_store: Some(prompt_store),
            model: None,
        };

        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();

        assert!(
            prompt.contains("Workspace root: `/host/repo`"),
            "system prompt should present the host root: {prompt}"
        );
        assert!(!prompt.contains("Workspace root: `/workspace`"));
    }

    #[tokio::test]
    async fn system_prompt_escapes_store_display_root_xml_text() {
        let cap = FileSystemCapability;
        let store = Arc::new(MockFileStore::with_display_root(
            "/tmp/repo</capability><capability id=\"attacker\">",
        ));
        let ctx = SystemPromptContext {
            session_id: SessionId::new(),
            locale: None,
            file_store: Some(store),
            model: None,
        };

        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();

        assert!(prompt.contains(
            "Workspace root: `/tmp/repo&lt;/capability&gt;&lt;capability id=\"attacker\"&gt;`"
        ));
        assert!(!prompt.contains("</capability><capability id=\"attacker\">"));
    }

    #[test]
    fn test_tool_schemas_have_no_top_level_composition_keywords() {
        // OpenAI Responses API rejects schemas with oneOf/anyOf/allOf/enum/not at top level
        let cap = FileSystemCapability;
        let forbidden = ["oneOf", "anyOf", "allOf", "enum", "not"];
        for tool in cap.tools() {
            let schema = tool.parameters_schema();
            for kw in &forbidden {
                assert!(
                    schema.get(*kw).is_none(),
                    "Tool '{}' schema has forbidden top-level keyword '{}'",
                    tool.name(),
                    kw
                );
            }
        }
    }

    #[test]
    fn test_edit_file_schema_is_edits_only() {
        // EVE-620: the advertised schema must not offer top-level old_text/new_text
        // and must require edits[]. The single ambiguity-free shape is what keeps
        // structured-tool-call models from populating both fields on the first call.
        let schema = EditFileTool.parameters_schema();
        let props = schema["properties"].as_object().expect("properties object");
        assert!(
            !props.contains_key("old_text"),
            "schema must not advertise top-level old_text"
        );
        assert!(
            !props.contains_key("new_text"),
            "schema must not advertise top-level new_text"
        );
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("required array")
            .iter()
            .map(|v| v.as_str().expect("required entries are strings"))
            .collect();
        assert!(required.contains(&"edits"), "edits[] must be required");
        assert!(required.contains(&"path"));
        assert!(required.contains(&"expected_hash"));
    }

    #[tokio::test]
    async fn test_read_file_without_context() {
        let result = ReadFileTool.execute(json!({"path": "/test.txt"})).await;
        assert!(expect_tool_error(result).contains("requires context"));
    }

    #[tokio::test]
    async fn test_write_file_without_context() {
        let result = WriteFileTool
            .execute(json!({"path": "/test.txt", "content": "hello"}))
            .await;
        assert!(expect_tool_error(result).contains("requires context"));
    }

    #[tokio::test]
    async fn test_edit_file_without_context() {
        let result = EditFileTool
            .execute(json!({
                "path": "/test.txt",
                "expected_hash": "sha256:deadbeef",
                "old_text": "hello",
                "new_text": "goodbye"
            }))
            .await;
        assert!(expect_tool_error(result).contains("requires context"));
    }

    #[tokio::test]
    async fn test_read_file_missing_path() {
        let context = ToolContext::new(SessionId::new());
        let result = ReadFileTool.execute_with_context(json!({}), &context).await;
        assert!(expect_tool_error(result).contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn test_read_file_no_file_store() {
        let context = ToolContext::new(SessionId::new());
        let result = ReadFileTool
            .execute_with_context(json!({"path": "/test.txt"}), &context)
            .await;
        assert!(expect_tool_error(result).contains("not available"));
    }

    #[tokio::test]
    async fn test_read_file_returns_content_hash() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/notes.txt", "hello world");
        let context = make_context(store);

        let result = ReadFileTool
            .execute_with_context(json!({"path": "/workspace/notes.txt"}), &context)
            .await;
        let value = expect_success(result);

        assert_eq!(value["path"], "/workspace/notes.txt");
        assert_eq!(value["content"], "1|hello world");
        assert_eq!(value["total_lines"], 1);
        assert_eq!(value["truncated"], false);
        assert_eq!(
            value["content_hash"].as_str().unwrap(),
            file_content_hash("hello world", "text").unwrap()
        );
    }

    #[tokio::test]
    async fn test_read_file_offset_limit() {
        let store = Arc::new(MockFileStore::default());
        let content = (1..=100)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        store.add_text_file("/big.txt", &content);
        let context = make_context(store);

        // Read lines 10-14 (0-indexed offset=9, limit=5)
        let result = ReadFileTool
            .execute_with_context(
                json!({"path": "/workspace/big.txt", "offset": 9, "limit": 5}),
                &context,
            )
            .await;
        let value = expect_success(result);

        assert_eq!(value["total_lines"], 100);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["lines_shown"]["start"], 10);
        assert_eq!(value["lines_shown"]["end"], 14);
        let content_str = value["content"].as_str().unwrap();
        assert!(content_str.starts_with("10|line 10"));
        assert!(content_str.ends_with("14|line 14"));
    }

    #[tokio::test]
    async fn test_read_file_default_limit_truncates() {
        let store = Arc::new(MockFileStore::default());
        let content = (1..=2500)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        store.add_text_file("/huge.txt", &content);
        let context = make_context(store);

        let result = ReadFileTool
            .execute_with_context(json!({"path": "/workspace/huge.txt"}), &context)
            .await;
        let value = expect_success(result);

        assert_eq!(value["total_lines"], 2500);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["lines_shown"]["start"], 1);
        assert_eq!(value["lines_shown"]["end"], 2000);
    }

    // ============================================================================
    // EVE-339 — Reading-tool truncation envelope conformance
    // ============================================================================

    #[tokio::test]
    async fn test_read_file_truncation_envelope_when_not_truncated() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/notes.txt", "hello world");
        let context = make_context(store);

        let result = ReadFileTool
            .execute_with_context(json!({"path": "/workspace/notes.txt"}), &context)
            .await;
        let value = expect_success(result);

        crate::truncation_info::assert_conforms("read_file", &value);
        assert_eq!(value["truncation"]["truncated"], false);
    }

    #[tokio::test]
    async fn test_read_file_truncation_envelope_with_resume() {
        let store = Arc::new(MockFileStore::default());
        let content = (1..=2500)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        store.add_text_file("/huge.txt", &content);
        let context = make_context(store);

        let result = ReadFileTool
            .execute_with_context(json!({"path": "/workspace/huge.txt"}), &context)
            .await;
        let value = expect_success(result);

        crate::truncation_info::assert_conforms("read_file", &value);
        assert_eq!(value["truncation"]["truncated"], true);
        assert_eq!(value["truncation"]["reason"], "line_cap");
        assert_eq!(value["truncation"]["next_offset"], 2000);
        assert!(
            value["truncation"]["resume_hint"]
                .as_str()
                .unwrap()
                .contains("offset=2000")
        );
    }

    #[tokio::test]
    async fn test_read_file_resume_roundtrip_reaches_end() {
        let store = Arc::new(MockFileStore::default());
        let content = (1..=2500)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        store.add_text_file("/huge.txt", &content);
        let context = make_context(store);

        // First page
        let first = expect_success(
            ReadFileTool
                .execute_with_context(json!({"path": "/workspace/huge.txt"}), &context)
                .await,
        );
        let next_offset = first["truncation"]["next_offset"].as_u64().unwrap();

        // Resume from next_offset
        let second = expect_success(
            ReadFileTool
                .execute_with_context(
                    json!({"path": "/workspace/huge.txt", "offset": next_offset, "limit": 1000}),
                    &context,
                )
                .await,
        );

        // After resuming we cover the remaining 500 lines and the envelope
        // reports `truncated: false` on the final chunk.
        assert_eq!(second["truncation"]["truncated"], false);
        let shown = &second["lines_shown"];
        assert_eq!(shown["start"], 2001);
        assert_eq!(shown["end"], 2500);
    }

    #[tokio::test]
    async fn test_list_directory_emits_truncation_envelope() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/a.txt", "a");
        store.add_text_file("/b.txt", "b");
        let context = make_context(store);

        let result = ListDirectoryTool
            .execute_with_context(json!({"path": "/workspace"}), &context)
            .await;
        let value = expect_success(result);

        crate::truncation_info::assert_conforms("list_directory", &value);
        assert_eq!(value["truncation"]["truncated"], false);
    }

    #[tokio::test]
    async fn test_list_directory_applies_item_window() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/a.txt", "a");
        store.add_text_file("/b.txt", "b");
        store.add_text_file("/c.txt", "c");
        let context = make_context(store);

        let result = ListDirectoryTool
            .execute_with_context(json!({"path": "/workspace", "limit": 2}), &context)
            .await;
        let value = expect_success(result);

        crate::truncation_info::assert_conforms("list_directory", &value);
        assert_eq!(value["count"], 2);
        assert_eq!(value["total_count"], 3);
        assert_eq!(value["truncation"]["truncated"], true);
        assert_eq!(value["truncation"]["reason"], "item_cap");
        assert_eq!(value["truncation"]["next_offset"], 2);
    }

    #[tokio::test]
    async fn test_grep_files_emits_truncation_envelope() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/notes.txt", "hello world");
        let context = make_context(store);

        let result = GrepFilesTool
            .execute_with_context(json!({"pattern": "hello"}), &context)
            .await;
        let value = expect_success(result);

        crate::truncation_info::assert_conforms("grep_files", &value);
        assert_eq!(value["truncation"]["truncated"], false);
    }

    #[tokio::test]
    async fn test_grep_files_applies_match_window() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/notes.txt", "hello one\nhello two\nhello three");
        let context = make_context(store);

        let result = GrepFilesTool
            .execute_with_context(json!({"pattern": "hello", "limit": 2}), &context)
            .await;
        let value = expect_success(result);

        crate::truncation_info::assert_conforms("grep_files", &value);
        assert_eq!(value["match_count"], 2);
        assert_eq!(value["total_matches"], 3);
        assert_eq!(value["truncation"]["truncated"], true);
        assert_eq!(value["truncation"]["reason"], "line_cap");
        assert_eq!(value["truncation"]["next_offset"], 2);
    }

    #[tokio::test]
    async fn test_grep_files_returns_merged_numbered_context() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/notes.txt", "before\nmatch one\nbetween\nmatch two\nafter");
        let context = make_context(store);

        let value = expect_success(
            GrepFilesTool
                .execute_with_context(
                    json!({"pattern": "match", "before_context": 1, "after_context": 1}),
                    &context,
                )
                .await,
        );

        assert!(value.get("matches").is_none());
        assert_eq!(value["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(value["blocks"][0]["start_line"], 1);
        assert_eq!(value["blocks"][0]["end_line"], 5);
        assert_eq!(value["blocks"][0]["match_line_numbers"], json!([2, 4]));
        assert_eq!(value["blocks"][0]["lines"].as_array().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn test_grep_files_rejects_invalid_context_values() {
        let context = make_context(Arc::new(MockFileStore::default()));
        for arguments in [
            json!({"pattern": "x", "before_context": -1}),
            json!({"pattern": "x", "after_context": 21}),
            json!({"pattern": "x", "before_context": 1.5}),
        ] {
            let result = GrepFilesTool
                .execute_with_context(arguments, &context)
                .await;
            assert!(matches!(result, ToolExecutionResult::ToolError(_)));
        }
    }

    #[tokio::test]
    async fn test_grep_files_enforces_total_byte_budget() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/large.txt", &format!("match {}", "x".repeat(70_000)));
        let context = make_context(store);
        let value = expect_success(
            GrepFilesTool
                .execute_with_context(json!({"pattern": "match"}), &context)
                .await,
        );

        assert!(
            value["matches"][0]["line"].as_str().unwrap().len() <= crate::GREP_MAX_RETURN_BYTES
        );
        assert_eq!(value["truncation"]["truncated"], true);
        assert_eq!(value["truncation"]["reason"], "size_cap");
        assert!(value["truncation"]["bytes_total"].as_u64().unwrap() > 64 * 1024);
    }

    #[tokio::test]
    async fn test_write_file_returns_content_hash() {
        let store = Arc::new(MockFileStore::default());
        let context = make_context(store.clone());

        let result = WriteFileTool
            .execute_with_context(
                json!({"path": "/workspace/new.txt", "content": "hello world"}),
                &context,
            )
            .await;
        let value = expect_success(result);

        assert_eq!(value["path"], "/workspace/new.txt");
        assert_eq!(value["size_bytes"], 11);
        assert_eq!(
            value["content_hash"].as_str().unwrap(),
            file_content_hash("hello world", "text").unwrap()
        );
        assert_eq!(store.content("/new.txt").unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_edit_file_single_replace_success() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/notes.txt", "alpha\nbeta\ngamma\n");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/notes.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/notes.txt",
                    "expected_hash": expected_hash,
                    "old_text": "beta",
                    "new_text": "delta"
                }),
                &context,
            )
            .await;
        let value = expect_success(result);

        assert_eq!(
            store.content("/notes.txt").unwrap(),
            "alpha\ndelta\ngamma\n"
        );
        assert_eq!(value["applied_edits"], 1);
        assert_eq!(value["first_changed_line"], 2);
        assert!(value["diff"].as_str().unwrap().contains("-beta"));
        assert!(value["diff"].as_str().unwrap().contains("+delta"));
        assert_ne!(
            value["content_hash"].as_str().unwrap(),
            value["previous_content_hash"].as_str().unwrap()
        );
    }

    #[tokio::test]
    async fn test_edit_file_batch_replace_success() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/batch.txt", "one\ntwo\nthree\n");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/batch.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/batch.txt",
                    "expected_hash": expected_hash,
                    "edits": [
                        {"old_text": "one", "new_text": "ONE"},
                        {"old_text": "three", "new_text": "THREE"}
                    ]
                }),
                &context,
            )
            .await;
        let value = expect_success(result);

        assert_eq!(store.content("/batch.txt").unwrap(), "ONE\ntwo\nTHREE\n");
        assert_eq!(value["applied_edits"], 2);
        assert_eq!(value["rebased"], false);
        assert_eq!(value["first_changed_line"], 1);
    }

    #[tokio::test]
    async fn test_edit_file_batch_replace_ignores_empty_single_placeholders() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/batch-placeholders.txt", "one\ntwo\nthree\n");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/batch-placeholders.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/batch-placeholders.txt",
                    "expected_hash": expected_hash,
                    "edits": [
                        {"old_text": "one", "new_text": "ONE"},
                        {"old_text": "three", "new_text": "THREE"}
                    ],
                    "old_text": "",
                    "new_text": ""
                }),
                &context,
            )
            .await;
        let value = expect_success(result);

        assert_eq!(
            store.content("/batch-placeholders.txt").unwrap(),
            "ONE\ntwo\nTHREE\n"
        );
        assert_eq!(value["applied_edits"], 2);
    }

    #[tokio::test]
    async fn test_edit_file_allows_delete_replacement() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/delete.txt", "keep\nremove me\nkeep\n");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/delete.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/delete.txt",
                    "expected_hash": expected_hash,
                    "old_text": "remove me\n",
                    "new_text": ""
                }),
                &context,
            )
            .await;

        expect_success(result);
        assert_eq!(store.content("/delete.txt").unwrap(), "keep\nkeep\n");
    }

    #[tokio::test]
    async fn test_edit_file_preserves_bom_and_crlf() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/windows.txt", "\u{feff}alpha\r\nbeta\r\n");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/windows.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/windows.txt",
                    "expected_hash": expected_hash,
                    "old_text": "beta\n",
                    "new_text": "gamma\n"
                }),
                &context,
            )
            .await;

        expect_success(result);
        assert_eq!(
            store.content("/windows.txt").unwrap(),
            "\u{feff}alpha\r\ngamma\r\n"
        );
    }

    #[tokio::test]
    async fn test_edit_file_preserves_cr_line_endings() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/classic-mac.txt", "alpha\rbeta\r");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/classic-mac.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/classic-mac.txt",
                    "expected_hash": expected_hash,
                    "old_text": "beta\n",
                    "new_text": "gamma\n"
                }),
                &context,
            )
            .await;

        expect_success(result);
        assert_eq!(store.content("/classic-mac.txt").unwrap(), "alpha\rgamma\r");
    }

    #[tokio::test]
    async fn test_edit_file_rebases_exact_edits_over_unrelated_stale_change() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/stale.txt", "title\nold value\nfooter\n");
        let context = make_context(store.clone());
        let stale_hash = read_hash(&context, "/workspace/stale.txt").await;
        store.add_text_file("/stale.txt", "new title\nold value\nfooter\n");

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/stale.txt",
                    "expected_hash": stale_hash,
                    "edits": [
                        {"old_text": "old value", "new_text": "new value"},
                        {"old_text": "footer", "new_text": "new footer"}
                    ]
                }),
                &context,
            )
            .await;

        let value = expect_success(result);
        assert_eq!(
            store.content("/stale.txt").unwrap(),
            "new title\nnew value\nnew footer\n"
        );
        assert_eq!(value["applied_edits"], 2);
        assert_eq!(value["rebased"], true);
        assert_ne!(value["previous_content_hash"], stale_hash);
    }

    #[tokio::test]
    async fn test_edit_file_rejects_stale_target_conflict_without_changes() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/stale-conflict.txt", "title\nold value\n");
        let context = make_context(store.clone());
        let stale_hash = read_hash(&context, "/workspace/stale-conflict.txt").await;
        store.add_text_file("/stale-conflict.txt", "title\nother writer value\n");

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/stale-conflict.txt",
                    "expected_hash": stale_hash,
                    "edits": [
                        {"old_text": "title", "new_text": "new title"},
                        {"old_text": "old value", "new_text": "new value"}
                    ]
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("Could not find an exact match"));
        assert_eq!(
            store.content("/stale-conflict.txt").unwrap(),
            "title\nother writer value\n"
        );
    }

    #[tokio::test]
    async fn test_edit_file_rejects_stale_ambiguity_without_changes() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/stale-ambiguous.txt", "header\nunique target\n");
        let context = make_context(store.clone());
        let stale_hash = read_hash(&context, "/workspace/stale-ambiguous.txt").await;
        store.add_text_file(
            "/stale-ambiguous.txt",
            "header\nunique target\nunique target\n",
        );

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/stale-ambiguous.txt",
                    "expected_hash": stale_hash,
                    "edits": [{"old_text": "unique target", "new_text": "replacement"}]
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("matched multiple locations"));
        assert_eq!(
            store.content("/stale-ambiguous.txt").unwrap(),
            "header\nunique target\nunique target\n"
        );
    }

    #[tokio::test]
    async fn test_edit_file_rejects_binary_file() {
        let store = Arc::new(MockFileStore::default());
        store.add_base64_file("/image.png", "aGVsbG8=");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/image.png").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/image.png",
                    "expected_hash": expected_hash,
                    "old_text": "hello",
                    "new_text": "goodbye"
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("only supports text files"));
    }

    #[tokio::test]
    async fn test_read_file_detects_image_from_content_without_extension() {
        use base64::Engine as _;

        let store = Arc::new(MockFileStore::default());
        let png = b"\x89PNG\r\n\x1a\nimage data";
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);
        store.add_base64_file("/diagram", &encoded);
        let context = make_context(store);

        let result = ReadFileTool
            .execute_with_context(json!({"path": "/workspace/diagram"}), &context)
            .await;

        match result {
            ToolExecutionResult::SuccessWithImages { result, images } => {
                assert_eq!(result["media_type"], "image/png");
                assert_eq!(images.len(), 1);
                assert_eq!(images[0].media_type, "image/png");
                assert_eq!(images[0].base64, encoded);
            }
            other => panic!("Expected image success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_read_file_non_image_binary_omits_base64_content() {
        let store = Arc::new(MockFileStore::default());
        store.add_base64_file("/archive.png", "UEsDBAoAAAAAAA==");
        let context = make_context(store);

        let result = ReadFileTool
            .execute_with_context(json!({"path": "/workspace/archive.png"}), &context)
            .await;
        let value = expect_success(result);

        assert_eq!(value["content_type"], "binary");
        assert_eq!(value["encoding"], "base64");
        assert_eq!(value["truncation"]["truncated"], false);
        assert_eq!(value["truncation"]["bytes_returned"], 0);
        assert!(value.get("content").is_none());
        assert!(value.get("content_hash").is_some());
    }

    #[tokio::test]
    async fn test_edit_file_rejects_directory() {
        let store = Arc::new(MockFileStore::default());
        store.add_directory("/docs");
        let context = make_context(store);

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/docs",
                    "expected_hash": "sha256:anything",
                    "old_text": "hello",
                    "new_text": "goodbye"
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("is a directory"));
    }

    #[tokio::test]
    async fn test_edit_file_rejects_missing_match() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/missing.txt", "hello");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/missing.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/missing.txt",
                    "expected_hash": expected_hash,
                    "old_text": "absent",
                    "new_text": "present"
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("Could not find an exact match"));
    }

    #[tokio::test]
    async fn test_edit_file_rejects_ambiguous_match() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/ambiguous.txt", "hello\nhello\n");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/ambiguous.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/ambiguous.txt",
                    "expected_hash": expected_hash,
                    "old_text": "hello",
                    "new_text": "goodbye"
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("matched multiple locations"));
    }

    #[tokio::test]
    async fn test_edit_file_rejects_overlapping_batch_edits() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/overlap.txt", "abcdef");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/overlap.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/overlap.txt",
                    "expected_hash": expected_hash,
                    "edits": [
                        {"old_text": "abcd", "new_text": "WXYZ"},
                        {"old_text": "cdef", "new_text": "1234"}
                    ]
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("Edits overlap"));
    }

    #[tokio::test]
    async fn test_edit_file_rejects_missing_expected_hash() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/hashless.txt", "hello");
        let context = make_context(store);

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/hashless.txt",
                    "old_text": "hello",
                    "new_text": "goodbye"
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("Missing required parameter: expected_hash"));
    }

    #[tokio::test]
    async fn test_edit_file_rejects_readonly_target() {
        let store = Arc::new(MockFileStore::default());
        store.add_readonly_text_file("/readonly.txt", "hello");
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/readonly.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/readonly.txt",
                    "expected_hash": expected_hash,
                    "old_text": "hello",
                    "new_text": "goodbye"
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("readonly"));
    }

    #[tokio::test]
    async fn test_edit_file_detects_concurrent_change_during_write() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/race.txt", "hello");
        store.inject_conditional_write_change("/race.txt", StoredFile::text("hola"));
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/race.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/race.txt",
                    "expected_hash": expected_hash,
                    "old_text": "hello",
                    "new_text": "goodbye"
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("changed since the last read"));
        assert_eq!(store.content("/race.txt").unwrap(), "hola");
    }

    #[tokio::test]
    async fn test_edit_file_truncates_large_diffs() {
        let store = Arc::new(MockFileStore::default());
        let original = format!("{}\n", "a".repeat(MAX_EDIT_DIFF_CHARS + 2000));
        let replacement = format!("{}\n", "b".repeat(MAX_EDIT_DIFF_CHARS + 2000));
        store.add_text_file("/large.txt", &original);
        let context = make_context(store.clone());
        let expected_hash = read_hash(&context, "/workspace/large.txt").await;

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/large.txt",
                    "expected_hash": expected_hash,
                    "old_text": original,
                    "new_text": replacement
                }),
                &context,
            )
            .await;
        let value = expect_success(result);

        assert_eq!(value["diff_truncated"], true);
        assert!(
            value["diff"]
                .as_str()
                .unwrap()
                .contains("diff truncated after")
        );
    }

    fn encoded_prefix(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn test_image_media_type_supported_formats() {
        assert_eq!(
            image_media_type(&encoded_prefix(b"\x89PNG\r\n\x1a\nrest")),
            Some("image/png")
        );
        assert_eq!(
            image_media_type(&encoded_prefix(b"\xff\xd8\xff\xe0rest")),
            Some("image/jpeg")
        );
        assert_eq!(
            image_media_type(&encoded_prefix(b"GIF87arest")),
            Some("image/gif")
        );
        assert_eq!(
            image_media_type(&encoded_prefix(b"GIF89arest")),
            Some("image/gif")
        );
        assert_eq!(
            image_media_type(&encoded_prefix(b"RIFF\x04\x00\x00\x00WEBPrest")),
            Some("image/webp")
        );
    }

    #[test]
    fn test_image_media_type_rejects_non_images_and_invalid_base64() {
        assert_eq!(image_media_type(&encoded_prefix(b"not an image")), None);
        assert_eq!(image_media_type("not base64!"), None);
        assert_eq!(image_media_type(""), None);
    }

    // EVE-249: Content-type detection tests
    #[test]
    fn test_content_type_log_files() {
        assert_eq!(content_type_from_extension("/app.log"), ContentType::Log);
        assert_eq!(content_type_from_extension("/build.out"), ContentType::Log);
        assert_eq!(content_type_from_extension("/debug.LOG"), ContentType::Log);
    }

    #[test]
    fn test_content_type_csv_files() {
        assert_eq!(content_type_from_extension("/data.csv"), ContentType::Csv);
        assert_eq!(content_type_from_extension("/export.tsv"), ContentType::Csv);
        assert_eq!(content_type_from_extension("/data.CSV"), ContentType::Csv);
    }

    #[test]
    fn test_content_type_binary_files() {
        assert_eq!(
            content_type_from_extension("/app.wasm"),
            ContentType::Binary
        );
        assert_eq!(content_type_from_extension("/lib.so"), ContentType::Binary);
        assert_eq!(
            content_type_from_extension("/archive.zip"),
            ContentType::Binary
        );
        assert_eq!(
            content_type_from_extension("/font.woff2"),
            ContentType::Binary
        );
    }

    #[test]
    fn test_content_type_minified_files() {
        assert_eq!(
            content_type_from_extension("/bundle.min.js"),
            ContentType::Minified
        );
        assert_eq!(
            content_type_from_extension("/styles.min.css"),
            ContentType::Minified
        );
    }

    #[test]
    fn test_content_type_text_files() {
        assert_eq!(content_type_from_extension("/main.rs"), ContentType::Text);
        assert_eq!(content_type_from_extension("/index.ts"), ContentType::Text);
        assert_eq!(content_type_from_extension("/README.md"), ContentType::Text);
        assert_eq!(
            content_type_from_extension("/config.json"),
            ContentType::Text
        );
    }

    #[test]
    fn test_content_type_minified_before_generic_js() {
        // .min.js should be Minified, not Text
        assert_eq!(
            content_type_from_extension("/bundle.min.js"),
            ContentType::Minified
        );
        // Plain .js should be Text
        assert_eq!(content_type_from_extension("/app.js"), ContentType::Text);
    }

    #[test]
    fn test_effective_read_defaults_explicit_wins() {
        // When user provides both offset and limit, don't override
        let (_, mode) = effective_read_defaults("/app.log", true, true);
        assert_eq!(mode, ReadMode::FromOffset);
    }

    #[test]
    fn test_effective_read_defaults_log_tail() {
        let (limit, mode) = effective_read_defaults("/app.log", false, false);
        assert_eq!(limit, 500);
        assert_eq!(mode, ReadMode::FromEnd);
    }

    #[test]
    fn test_effective_read_defaults_csv() {
        let (limit, mode) = effective_read_defaults("/data.csv", false, false);
        assert_eq!(limit, 100);
        assert_eq!(mode, ReadMode::FromOffset);
    }

    #[test]
    fn test_effective_read_defaults_binary() {
        let (_, mode) = effective_read_defaults("/app.wasm", false, false);
        assert_eq!(mode, ReadMode::MetadataOnly);
    }

    #[test]
    fn localized_name_differs_from_default() {
        let cap = FileSystemCapability;
        assert_ne!(cap.localized_name(Some("uk")), cap.name());
    }

    #[test]
    fn host_backed_tool_schemas_contain_no_workspace_guidance() {
        let presentation = FilePathPresentation::from_file_store(Some(
            &MockFileStore::with_display_root("/repo") as &dyn SessionFileSystem,
        ));
        for (tool_name, schema) in filesystem_tool_schemas_with_presentation(&presentation) {
            assert!(
                !schema_contains_workspace(&schema),
                "tool '{tool_name}' schema must not advertise /workspace for host-backed roots"
            );
            if tool_name == "list_directory" {
                assert_eq!(schema["properties"]["path"]["default"], "/repo");
            }
        }
    }

    #[test]
    fn vfs_tool_schemas_advertise_workspace_identity() {
        let presentation = FilePathPresentation::vfs();
        let schemas = filesystem_tool_schemas_with_presentation(&presentation);
        let path_tools = ["read_file", "write_file", "edit_file", "list_directory"];
        for tool_name in path_tools {
            let schema = schemas
                .iter()
                .find(|(name, _)| name == tool_name)
                .map(|(_, schema)| schema)
                .expect("schema present");
            assert!(
                schema_contains_workspace(schema),
                "tool '{tool_name}' schema should mention /workspace for VFS sessions"
            );
        }
        assert_eq!(
            schemas
                .iter()
                .find(|(name, _)| name == "list_directory")
                .unwrap()
                .1["properties"]["path"]["default"],
            "/workspace"
        );
    }

    #[tokio::test]
    async fn assembled_prompt_uses_host_root_without_workspace_guidance() {
        use crate::capabilities::{CapabilityRegistry, collect_capabilities_with_configs};
        use everruns_capability::CapabilityRef as AgentCapabilityConfig;

        let store = Arc::new(MockFileStore::with_display_root("/repo"));
        let ctx = SystemPromptContext {
            session_id: SessionId::new(),
            locale: None,
            file_store: Some(store),
            model: None,
        };
        let mut registry = CapabilityRegistry::new();
        registry.register(FileSystemCapability);
        let collected = collect_capabilities_with_configs(
            &[AgentCapabilityConfig::new(
                SESSION_FILE_SYSTEM_CAPABILITY_ID,
            )],
            &registry,
            &ctx,
        )
        .await;

        let prompt = collected.system_prompt_prefix().expect("system prompt");
        assert!(prompt.contains("Workspace root: `/repo`"));
        assert!(!prompt.contains("/workspace"));
    }

    #[tokio::test]
    async fn list_directory_without_path_uses_workspace_display_root() {
        // File tools run behind MountFs; the mounted primary presents the stable
        // host-agnostic /workspace root rather than the backend's host path.
        let store = Arc::new(MockFileStore::with_display_root("/repo"));
        store.add_text_file("/notes.txt", "hello");
        let context = make_context(store);

        let result = ListDirectoryTool
            .execute_with_context(json!({}), &context)
            .await;
        let value = expect_success(result);

        assert_eq!(value["path"], "/workspace");
    }
}
