//! Sandbox-backed `read_many_files` and `list_directory`, and the session VFS
//! paths the sandbox read tools still reach.
//!
//! A managed sandbox owns every model-facing file tool (see
//! `everruns_core::capabilities::SESSION_FILE_TOOL_NAMES`), so these keep the
//! VFS tools' names and parameter shapes: prompts and skills that name them
//! keep working, now against the sandbox checkout.

use async_trait::async_trait;
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_output_sanitizer::{
    READ_FILE_BATCH_MAX_OUTPUT_BYTES, READ_FILE_BATCH_MAX_PATH_BYTES, READ_FILE_BATCH_MAX_PATHS,
    READ_FILE_DEFAULT_LIMIT,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::truncation_info::{TruncationInfo, TruncationReason};
use serde_json::{Map, Value, json};

use super::environment_tools::{
    EnvironmentReadFileTool, exec, exec_result, read_result, shell_escape,
};
use super::session_sandbox::parse_config;
use crate::session_sandbox::{SessionSandboxReadFileResponse, session_sandbox_tool_hints};

const LIST_DIRECTORY_DEFAULT_LIMIT: usize = 200;
const LIST_DIRECTORY_MAX_LIMIT: usize = 1_000;
const LIST_TIMEOUT_MS: u64 = 30_000;
const LIST_TOTAL_MARKER: &str = "__everruns_total__";

/// Paths the session VFS writes after the sandbox was seeded: skills activated
/// later (`/.agents/skills`) and persisted tool output (`/outputs`). A pointer
/// the runtime hands the model must still open, so a sandbox miss on one of
/// these falls back to the VFS. The sandbox stays primary: a copy the model
/// wrote there wins.
fn is_session_vfs_owned(path: &str) -> bool {
    let relative = path
        .strip_prefix("/workspace")
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
        .unwrap_or(path);
    ["/.agents", "/outputs"].iter().any(|prefix| {
        relative
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// Read a VFS-owned file the sandbox does not have. `None` leaves the sandbox
/// error standing.
pub(super) async fn read_session_vfs_fallback(
    context: &ToolContext,
    path: &str,
    offset: usize,
    limit: usize,
) -> Option<ToolExecutionResult> {
    if !is_session_vfs_owned(path) {
        return None;
    }
    let store = context.file_store.as_ref()?;
    let file = store.read_file(context.session_id, path).await.ok()??;
    if file.is_directory {
        return None;
    }
    Some(read_result(
        SessionSandboxReadFileResponse {
            path: path.to_string(),
            content: file.content.unwrap_or_default(),
            encoding: file.encoding,
        },
        path,
        offset,
        limit,
    ))
}

/// The model-facing `/workspace/...` spelling of a path argument. `exec`
/// translates it to the provider's root.
fn workspace_path(input: &str) -> Result<String, String> {
    let relative = input
        .strip_prefix("/workspace/")
        .or_else(|| (input == "/workspace").then_some(""))
        .unwrap_or_else(|| input.trim_start_matches('/'))
        .trim_end_matches('/');
    if relative.split('/').any(|part| part == "..") {
        return Err("Path must stay inside /workspace".to_string());
    }
    Ok(if relative.is_empty() || relative == "." {
        "/workspace".to_string()
    } else {
        format!("/workspace/{relative}")
    })
}

#[derive(Clone)]
pub struct EnvironmentReadManyFilesTool(Value);

impl EnvironmentReadManyFilesTool {
    pub fn new(config: Value) -> Self {
        Self(config)
    }
}

fn batch_output_limit_error(index: usize, attempted: bool) -> Value {
    let error = if attempted {
        "Not returned because the batch output limit was reached; read this file individually"
    } else {
        "Not read because the batch output limit was reached; read this file individually"
    };
    json!({ "index": index, "error": error })
}

fn parse_batch_paths(arguments: &Value) -> Result<Vec<String>, ToolExecutionResult> {
    let Some(paths) = arguments.get("paths").and_then(Value::as_array) else {
        return Err(ToolExecutionResult::tool_error(
            "Missing required parameter: paths",
        ));
    };
    if !(2..=READ_FILE_BATCH_MAX_PATHS).contains(&paths.len()) {
        return Err(ToolExecutionResult::tool_error(format!(
            "Parameter `paths` must contain between 2 and {READ_FILE_BATCH_MAX_PATHS} paths"
        )));
    }
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| match path.as_str() {
            Some(path) if !path.is_empty() && path.len() <= READ_FILE_BATCH_MAX_PATH_BYTES => {
                Ok(path.to_string())
            }
            Some(_) => Err(ToolExecutionResult::tool_error(format!(
                "Parameter `paths[{index}]` must contain between 1 and {READ_FILE_BATCH_MAX_PATH_BYTES} bytes"
            ))),
            None => Err(ToolExecutionResult::tool_error(format!(
                "Parameter `paths[{index}]` must be a string"
            ))),
        })
        .collect()
}

#[async_trait]
impl Tool for EnvironmentReadManyFilesTool {
    fn narrate(
        &self,
        _tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_read_many_files(
            phase, locale,
        ))
    }

    fn name(&self) -> &str {
        "read_many_files"
    }

    fn description(&self) -> &str {
        "Read 2-10 independent files from /workspace in the active Environment in one ordered \
         call. Batch only paths known before the call; if one file reveals another path, read \
         it in a later call. Each result keeps its own success or error."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "description": "Ordered paths for 2-10 independent files known before this call. Results use the same order.",
                    "items": {"type": "string", "maxLength": READ_FILE_BATCH_MAX_PATH_BYTES},
                    "minItems": 2,
                    "maxItems": READ_FILE_BATCH_MAX_PATHS
                },
                "offset": {"type": "integer", "minimum": 0, "default": 0},
                "limit": {"type": "integer", "minimum": 1, "default": READ_FILE_DEFAULT_LIMIT}
            },
            "required": ["paths"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        session_sandbox_tool_hints()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("read_many_files requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if let Err(error) = parse_config(&self.0) {
            return error;
        }
        let paths = match parse_batch_paths(&arguments) {
            Ok(paths) => paths,
            Err(error) => return error,
        };
        let mut window = Map::new();
        for key in ["offset", "limit"] {
            if let Some(value) = arguments.get(key) {
                window.insert(key.to_string(), value.clone());
            }
        }

        let read_file = EnvironmentReadFileTool::new(self.0.clone());
        let mut results: Vec<Value> = Vec::with_capacity(paths.len());
        let mut output_limit_reached = false;
        for (index, path) in paths.iter().enumerate() {
            if output_limit_reached {
                results.push(batch_output_limit_error(index, false));
                continue;
            }
            let mut single = window.clone();
            single.insert("path".to_string(), json!(path));
            let item = match read_file
                .execute_with_context(Value::Object(single), context)
                .await
            {
                ToolExecutionResult::Success(value) => value,
                ToolExecutionResult::ToolError(error) => json!({"path": path, "error": error}),
                // Lifecycle, policy and internal failures end the batch.
                stop => return stop,
            };

            let mut candidate = results.clone();
            candidate.push(item.clone());
            candidate.extend(
                ((index + 1)..paths.len()).map(|rest| batch_output_limit_error(rest, false)),
            );
            let envelope = json!({"results": candidate, "count": paths.len()});
            if serde_json::to_vec(&envelope)
                .is_ok_and(|bytes| bytes.len() <= READ_FILE_BATCH_MAX_OUTPUT_BYTES)
            {
                results.push(item);
            } else {
                output_limit_reached = true;
                results.push(batch_output_limit_error(index, true));
            }
        }

        ToolExecutionResult::success(json!({"results": results, "count": paths.len()}))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct EnvironmentListDirectoryTool(Value);

impl EnvironmentListDirectoryTool {
    pub fn new(config: Value) -> Self {
        Self(config)
    }
}

/// Entries of one directory, sorted by name, rows `(start, end]` as
/// `type<TAB>size<TAB>name`, then a total-count row. Exits 2 when the
/// directory does not exist.
fn list_command(quoted_dir: &str, start: usize, end: usize) -> String {
    format!(
        "cd {quoted_dir} 2>/dev/null || exit 2; \
         find . -mindepth 1 -maxdepth 1 -printf '%y\\t%s\\t%P\\n' \
         | LC_ALL=C sort -t \"$(printf '\\t')\" -k3 \
         | awk -v start={start} -v end={end} 'NR > start && NR <= end {{ print }} \
         END {{ printf \"{LIST_TOTAL_MARKER}\\t0\\t%d\\n\", NR }}'"
    )
}

/// One page of a listing plus the directory's total entry count.
struct Listing {
    entries: Vec<Value>,
    total: usize,
}

/// Parse `type<TAB>size<TAB>name` lines and the trailing total marker.
fn parse_listing(stdout: &str, directory: &str) -> Listing {
    let mut entries = Vec::new();
    let mut total = None;
    for line in stdout.lines() {
        let mut fields = line.splitn(3, '\t');
        let (Some(kind), Some(size), Some(name)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if kind == LIST_TOTAL_MARKER {
            total = name.parse().ok();
            continue;
        }
        let is_directory = kind == "d";
        entries.push(json!({
            "name": name,
            "path": format!("{}/{name}", directory.trim_end_matches('/')),
            "is_directory": is_directory,
            "size_bytes": if is_directory { 0 } else { size.parse::<u64>().unwrap_or(0) },
            "is_readonly": false,
        }));
    }
    let total = total.unwrap_or(entries.len());
    Listing { entries, total }
}

fn listing_result(
    path: &str,
    listing: Listing,
    offset: usize,
    limit: usize,
) -> ToolExecutionResult {
    let count = listing.entries.len();
    let bytes_returned = serde_json::to_string(&listing.entries).map_or(0, |json| json.len());
    let mut result = json!({
        "path": path,
        "entries": listing.entries,
        "count": count,
        "total_count": listing.total,
        "offset": offset,
        "limit": limit,
    });
    let next_offset = offset.saturating_add(count);
    let truncation = if next_offset < listing.total {
        TruncationInfo::with_resume(
            bytes_returned,
            None,
            next_offset as u64,
            format!(
                "call list_directory with offset={next_offset} to resume from item {}",
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

/// List a VFS-owned directory the sandbox does not have.
async fn list_session_vfs_fallback(
    context: &ToolContext,
    path: &str,
    offset: usize,
    limit: usize,
) -> Option<ToolExecutionResult> {
    if !is_session_vfs_owned(path) {
        return None;
    }
    let store = context.file_store.as_ref()?;
    let mut files = store
        .list_directory(context.session_id, path)
        .await
        .ok()
        .filter(|files| !files.is_empty())?;
    files.sort_by(|a, b| a.name.cmp(&b.name));
    let total = files.len();
    let entries = files
        .iter()
        .skip(offset)
        .take(limit)
        .map(|file| {
            json!({
                "name": file.name,
                "path": format!("{}/{}", path.trim_end_matches('/'), file.name),
                "is_directory": file.is_directory,
                "size_bytes": file.size_bytes,
                "is_readonly": file.is_readonly,
            })
        })
        .collect();
    Some(listing_result(
        path,
        Listing { entries, total },
        offset,
        limit,
    ))
}

#[async_trait]
impl Tool for EnvironmentListDirectoryTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_list_directory(
            &tool_call.arguments,
            phase,
            locale,
            ctx,
        ))
    }

    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List the files and directories directly inside a /workspace directory in the active \
         Environment, sorted by name, with size and type."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "default": "/workspace",
                    "description": "Directory to list (e.g. '/workspace/src'). Defaults to /workspace."
                },
                "offset": {"type": "integer", "minimum": 0, "default": 0},
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": LIST_DIRECTORY_MAX_LIMIT,
                    "default": LIST_DIRECTORY_DEFAULT_LIMIT
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        session_sandbox_tool_hints()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_directory requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.0) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let offset = match arguments.get("offset") {
            None => 0,
            Some(value) => match value.as_u64() {
                Some(offset) => offset as usize,
                None => {
                    return ToolExecutionResult::tool_error(
                        "offset must be a non-negative integer",
                    );
                }
            },
        };
        let limit = match arguments.get("limit") {
            None => LIST_DIRECTORY_DEFAULT_LIMIT,
            Some(value) => match value.as_u64() {
                Some(limit) if limit > 0 => (limit as usize).min(LIST_DIRECTORY_MAX_LIMIT),
                _ => return ToolExecutionResult::tool_error("limit must be a positive integer"),
            },
        };
        let requested = arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("/workspace");
        let path = match workspace_path(requested) {
            Ok(path) => path,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };

        let command = list_command(&shell_escape(&path), offset, offset.saturating_add(limit));
        match exec(
            &config,
            context,
            command,
            None,
            Some(LIST_TIMEOUT_MS),
            // The command pages its own output; the lines are parsed, so a byte
            // cut would drop or split entries.
            "full".to_string(),
            false,
        )
        .await
        {
            Ok(response) if response.exit_code == 0 => {
                listing_result(&path, parse_listing(&response.stdout, &path), offset, limit)
            }
            Ok(response) => match list_session_vfs_fallback(context, &path, offset, limit).await {
                Some(result) => result,
                None if response.exit_code == 2 => {
                    ToolExecutionResult::tool_error(format!("Directory not found: {path}"))
                }
                None => exec_result(response, None),
            },
            Err(error) => error,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[path = "environment_file_tools_tests.rs"]
mod tests;
