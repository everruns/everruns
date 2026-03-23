//! Session File System Capability
//!
//! This capability provides tools for interacting with the session file system.
//! Each session has its own isolated filesystem stored in the database.
//!
//! Tools provided:
//! - `read_file`: Read file content
//! - `write_file`: Create or update a file
//! - `edit_file`: Apply surgical text replacements to an existing file
//! - `list_directory`: List files in a directory
//! - `grep_files`: Search files by regex pattern
//! - `delete_file`: Delete a file or directory
//! - `stat_file`: Get file metadata

use super::{Capability, CapabilityStatus};
use crate::session_file::SessionFile;
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult, ToolResultImage};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use similar::TextDiff;

/// Image MIME types recognized by LLM vision APIs (OpenAI, Anthropic)
const IMAGE_EXTENSIONS: &[(&str, &str)] = &[
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".webp", "image/webp"),
];

/// Get the image MIME type if the path has a known image extension
fn image_media_type(path: &str) -> Option<&'static str> {
    let lower = path.to_lowercase();
    IMAGE_EXTENSIONS
        .iter()
        .find(|(ext, _)| lower.ends_with(ext))
        .map(|(_, mime)| *mime)
}

/// Workspace prefix used in file paths
const WORKSPACE_PREFIX: &str = "/workspace";
const MAX_EDIT_DIFF_CHARS: usize = 16_000;

/// Normalize a file path by stripping the /workspace prefix.
/// This ensures both file_system and virtual_bash capabilities use the same
/// path format in the session file store.
///
/// Examples:
/// - `/workspace/foo.txt` -> `/foo.txt`
/// - `/workspace` -> `/`
/// - `/foo.txt` -> `/foo.txt` (already normalized)
fn normalize_path(path: &str) -> String {
    if path == WORKSPACE_PREFIX {
        "/".to_string()
    } else if let Some(stripped) = path.strip_prefix(WORKSPACE_PREFIX) {
        if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            // path like "/workspacefoo" - not a valid workspace path
            path.to_string()
        }
    } else {
        // Path doesn't start with /workspace - use as-is
        path.to_string()
    }
}

/// Add workspace prefix back to a path for display to the user
fn add_workspace_prefix(path: &str) -> String {
    if path == "/" {
        WORKSPACE_PREFIX.to_string()
    } else if path.starts_with('/') {
        format!("{}{}", WORKSPACE_PREFIX, path)
    } else {
        format!("{}/{}", WORKSPACE_PREFIX, path)
    }
}

fn file_content_hash(content: &str, encoding: &str) -> crate::error::Result<String> {
    let bytes = SessionFile::decode_content(content, encoding)
        .map_err(|error| anyhow::anyhow!("failed to decode file content for hashing: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
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
        &normalize_line_endings(before),
        &normalize_line_endings(after),
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

fn parse_text_edits(arguments: &Value) -> std::result::Result<Vec<TextEdit>, String> {
    let has_single = arguments.get("old_text").is_some() || arguments.get("new_text").is_some();
    let has_batch = arguments.get("edits").is_some();

    if has_single && has_batch {
        return Err("Provide either old_text/new_text or edits, not both".to_string());
    }

    if has_single {
        let old_text = arguments
            .get("old_text")
            .and_then(Value::as_str)
            .ok_or_else(|| "Missing required parameter: old_text".to_string())?;
        let new_text = arguments
            .get("new_text")
            .and_then(Value::as_str)
            .ok_or_else(|| "Missing required parameter: new_text".to_string())?;
        if old_text.is_empty() {
            return Err("old_text cannot be empty".to_string());
        }
        return Ok(vec![TextEdit {
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
        }]);
    }

    let edits = arguments
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| "Provide old_text/new_text or a non-empty edits array".to_string())?;

    if edits.is_empty() {
        return Err("edits must contain at least one replacement".to_string());
    }

    edits
        .iter()
        .enumerate()
        .map(|(index, edit)| {
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
            Ok(TextEdit {
                old_text: old_text.to_string(),
                new_text: new_text.to_string(),
            })
        })
        .collect()
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

/// Session File System capability - provides file operations for session storage
pub struct FileSystemCapability;

impl Capability for FileSystemCapability {
    fn id(&self) -> &str {
        "session_file_system"
    }

    fn name(&self) -> &str {
        "File System"
    }

    fn description(&self) -> &str {
        r#"Tools to access and manipulate files in the session workspace - read, write, list, grep, and more.

> [!NOTE]
> Each session has its own isolated workspace at `/workspace`. Files persist for the session duration.

> [!TIP]
> Use `list_directory` to explore the workspace structure before reading or writing files."#
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

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"Session workspace root: `/workspace`. All file paths must start with `/workspace`. Directories are created automatically when writing files."#,
        )
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
    fn name(&self) -> &str {
        "read_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read File")
    }

    fn description(&self) -> &str {
        "Read the content of a file. Returns text content directly. For image files (PNG, JPEG, GIF, WebP), the image is returned as a native image so you can see it visually."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file (e.g., '/workspace/docs/readme.txt')"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
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
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

        match file_store
            .read_file(context.session_id, &normalized_path)
            .await
        {
            Ok(Some(file)) => {
                if file.is_directory {
                    return ToolExecutionResult::tool_error(format!(
                        "Path '{}' is a directory, not a file. Use list_directory instead.",
                        display_path
                    ));
                }

                // Check if this is an image file that should be returned as native image content
                if let Some(media_type) = image_media_type(&normalized_path) {
                    // For base64-encoded files, return as image
                    if file.encoding == "base64"
                        && let Some(ref content) = file.content
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
                    // Text-encoded image paths still get returned as text (unusual case)
                }

                let content_hash = match session_file_content_hash(&file) {
                    Ok(hash) => hash,
                    Err(e) => return ToolExecutionResult::internal_error(e),
                };
                ToolExecutionResult::success(json!({
                    "path": display_path,
                    "content": file.content,
                    "encoding": file.encoding,
                    "size_bytes": file.size_bytes,
                    "content_hash": content_hash
                }))
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
}

// ============================================================================
// WriteFileTool
// ============================================================================

/// Tool to write/create a file
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Write File")
    }

    fn description(&self) -> &str {
        "Create a new file or update an existing file's content. Parent directories are created automatically."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path for the file (e.g., '/workspace/docs/notes.txt')"
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

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_idempotent(true)
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
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

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
            Err(e) => {
                // Check if it's a user-facing error (like readonly file)
                let msg = e.to_string();
                if msg.contains("readonly") || msg.contains("is a directory") {
                    ToolExecutionResult::tool_error(msg)
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// EditFileTool
// ============================================================================

/// Tool to apply exact text replacements to an existing text file
pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Edit File")
    }

    fn description(&self) -> &str {
        "Apply one or more exact text replacements to an existing text file. Requires the current content hash from read_file or write_file."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the existing text file (e.g., '/workspace/src/main.rs')"
                },
                "expected_hash": {
                    "type": "string",
                    "description": "Current content hash from read_file or write_file (format: 'sha256:...')"
                },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to replace. Use for single-edit shorthand."
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text. Use for single-edit shorthand."
                },
                "edits": {
                    "type": "array",
                    "description": "Batch multiple replacements in a single file. Each edit matches against the original file content.",
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
            "required": ["path", "expected_hash"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
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

        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

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
        if expected_hash != current_hash {
            return ToolExecutionResult::tool_error(format!(
                "File '{}' changed since the last read. Expected {}, found {}. Read the file again before editing.",
                display_path, expected_hash, current_hash
            ));
        }

        let current_content = existing.content.unwrap_or_default();
        let (updated_content, applied_edits) = match apply_text_edits(&current_content, &edits) {
            Ok(result) => result,
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
                    "first_changed_line": first_changed_line,
                    "diff": diff,
                    "diff_truncated": diff_truncated
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("readonly") || msg.contains("is a directory") {
                    ToolExecutionResult::tool_error(msg)
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// ListDirectoryTool
// ============================================================================

/// Tool to list directory contents
pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
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
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "default": "/workspace",
                    "description": "Directory path to list (default: '/workspace')"
                }
            },
            "additionalProperties": false
        })
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
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("/workspace");

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

        match file_store
            .list_directory(context.session_id, &normalized_path)
            .await
        {
            Ok(files) => {
                let entries: Vec<Value> = files
                    .iter()
                    .map(|f| {
                        json!({
                            "name": f.name,
                            "path": add_workspace_prefix(&f.path),
                            "is_directory": f.is_directory,
                            "size_bytes": f.size_bytes,
                            "is_readonly": f.is_readonly
                        })
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "path": display_path,
                    "entries": entries,
                    "count": entries.len()
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("not a directory") {
                    ToolExecutionResult::tool_error(msg)
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// GrepFilesTool
// ============================================================================

/// Tool to search files by pattern
pub struct GrepFilesTool;

#[async_trait]
impl Tool for GrepFilesTool {
    fn name(&self) -> &str {
        "grep_files"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Grep Files")
    }

    fn description(&self) -> &str {
        "Search file contents using a regex pattern. Returns matching lines with file paths and line numbers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path_pattern": {
                    "type": "string",
                    "description": "Optional path pattern to filter files (e.g., '*.txt', '/workspace/docs/*')"
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
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

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        match file_store
            .grep_files(context.session_id, pattern, path_pattern)
            .await
        {
            Ok(matches) => {
                let results: Vec<Value> = matches
                    .iter()
                    .map(|m| {
                        json!({
                            "path": add_workspace_prefix(&m.path),
                            "line_number": m.line_number,
                            "line": m.line
                        })
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "pattern": pattern,
                    "matches": results,
                    "match_count": results.len()
                }))
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
}

// ============================================================================
// DeleteFileTool
// ============================================================================

/// Tool to delete a file or directory
pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
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
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file or directory to delete"
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_destructive(true)
            .with_idempotent(true)
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
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

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
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not empty") || msg.contains("recursive") {
                    ToolExecutionResult::tool_error(msg)
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// StatFileTool
// ============================================================================

/// Tool to get file metadata
pub struct StatFileTool;

#[async_trait]
impl Tool for StatFileTool {
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
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file or directory"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
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
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

        match file_store
            .stat_file(context.session_id, &normalized_path)
            .await
        {
            Ok(Some(stat)) => ToolExecutionResult::success(json!({
                "path": add_workspace_prefix(&stat.path),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::traits::SessionFileStore;
    use crate::typed_id::SessionId;
    use chrono::Utc;
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
    }

    impl MockFileStore {
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
    impl SessionFileStore for MockFileStore {
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
            _path: &str,
        ) -> Result<Vec<FileInfo>> {
            Ok(vec![])
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
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            Ok(vec![])
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
        ToolContext::with_file_store(SessionId::new(), file_store)
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

    #[test]
    fn test_normalize_path_workspace_root() {
        assert_eq!(normalize_path("/workspace"), "/");
    }

    #[test]
    fn test_normalize_path_workspace_file() {
        assert_eq!(normalize_path("/workspace/test.txt"), "/test.txt");
    }

    #[test]
    fn test_normalize_path_workspace_nested() {
        assert_eq!(
            normalize_path("/workspace/foo/bar/test.txt"),
            "/foo/bar/test.txt"
        );
    }

    #[test]
    fn test_normalize_path_already_normalized() {
        assert_eq!(normalize_path("/test.txt"), "/test.txt");
    }

    #[test]
    fn test_normalize_path_invalid_workspace_prefix() {
        assert_eq!(normalize_path("/workspacefoo"), "/workspacefoo");
    }

    #[test]
    fn test_add_workspace_prefix_root() {
        assert_eq!(add_workspace_prefix("/"), "/workspace");
    }

    #[test]
    fn test_add_workspace_prefix_file() {
        assert_eq!(add_workspace_prefix("/test.txt"), "/workspace/test.txt");
    }

    #[test]
    fn test_add_workspace_prefix_nested() {
        assert_eq!(
            add_workspace_prefix("/foo/bar.txt"),
            "/workspace/foo/bar.txt"
        );
    }

    #[test]
    fn test_add_workspace_prefix_no_leading_slash() {
        assert_eq!(add_workspace_prefix("test.txt"), "/workspace/test.txt");
    }

    #[test]
    fn test_parse_text_edits_rejects_mixed_modes() {
        let result = parse_text_edits(&json!({
            "old_text": "a",
            "new_text": "b",
            "edits": [{"old_text": "c", "new_text": "d"}]
        }));

        assert_eq!(
            result.unwrap_err(),
            "Provide either old_text/new_text or edits, not both"
        );
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

    #[test]
    fn test_capability_metadata() {
        let cap = FileSystemCapability;
        assert_eq!(cap.id(), "session_file_system");
        assert_eq!(cap.name(), "File System");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("hard-drive"));
        assert_eq!(cap.category(), Some("File Operations"));
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = FileSystemCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 7);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"read_file"));
        assert!(tool_names.contains(&"write_file"));
        assert!(tool_names.contains(&"edit_file"));
        assert!(tool_names.contains(&"list_directory"));
        assert!(tool_names.contains(&"grep_files"));
        assert!(tool_names.contains(&"delete_file"));
        assert!(tool_names.contains(&"stat_file"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = FileSystemCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("/workspace"));
    }

    #[test]
    fn test_tools_require_context() {
        assert!(ReadFileTool.requires_context());
        assert!(WriteFileTool.requires_context());
        assert!(EditFileTool.requires_context());
        assert!(ListDirectoryTool.requires_context());
        assert!(GrepFilesTool.requires_context());
        assert!(DeleteFileTool.requires_context());
        assert!(StatFileTool.requires_context());
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
        assert_eq!(value["encoding"], "text");
        assert_eq!(value["content"], "hello world");
        assert_eq!(
            value["content_hash"].as_str().unwrap(),
            file_content_hash("hello world", "text").unwrap()
        );
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
        assert_eq!(value["first_changed_line"], 1);
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
    async fn test_edit_file_rejects_hash_mismatch() {
        let store = Arc::new(MockFileStore::default());
        store.add_text_file("/stale.txt", "hello");
        let context = make_context(store);

        let result = EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/stale.txt",
                    "expected_hash": "sha256:stale",
                    "old_text": "hello",
                    "new_text": "goodbye"
                }),
                &context,
            )
            .await;

        assert!(expect_tool_error(result).contains("changed since the last read"));
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

    #[test]
    fn test_image_media_type_png() {
        assert_eq!(
            image_media_type("/workspace/screenshot.png"),
            Some("image/png")
        );
    }

    #[test]
    fn test_image_media_type_jpeg() {
        assert_eq!(image_media_type("/workspace/photo.jpg"), Some("image/jpeg"));
        assert_eq!(
            image_media_type("/workspace/photo.jpeg"),
            Some("image/jpeg")
        );
    }

    #[test]
    fn test_image_media_type_gif() {
        assert_eq!(image_media_type("/data/anim.gif"), Some("image/gif"));
    }

    #[test]
    fn test_image_media_type_webp() {
        assert_eq!(image_media_type("/images/art.webp"), Some("image/webp"));
    }

    #[test]
    fn test_image_media_type_case_insensitive() {
        assert_eq!(image_media_type("/workspace/PHOTO.PNG"), Some("image/png"));
        assert_eq!(image_media_type("/workspace/image.JPG"), Some("image/jpeg"));
    }

    #[test]
    fn test_image_media_type_not_image() {
        assert_eq!(image_media_type("/workspace/readme.txt"), None);
        assert_eq!(image_media_type("/workspace/data.json"), None);
        assert_eq!(image_media_type("/workspace/script.py"), None);
    }
}
