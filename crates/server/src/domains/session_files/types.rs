// Session files domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::common_dto::ListResponse;
use everruns_core::{FileInfo, SessionFile};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request to copy a file
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct CopyFileRequest {
    /// Source path (relative to the workspace filesystem root).
    #[schema(example = "templates/runbook.md")]
    pub src_path: String,
    /// Destination path (relative to the workspace filesystem root).
    #[schema(example = "docs/runbooks/refund-30-days.md")]
    pub dst_path: String,
}

/// Request to create a file
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateFileRequest {
    /// File content (text or base64-encoded). Must match `encoding`.
    #[serde(default)]
    #[schema(example = "# Project notes\n\nDraft outline of the migration plan.\n")]
    pub content: Option<String>,
    /// Content encoding: "text" or "base64". Defaults to text.
    #[serde(default)]
    #[schema(example = "text")]
    pub encoding: Option<String>,
    /// Whether file is read-only
    #[serde(default)]
    #[schema(example = false)]
    pub is_readonly: Option<bool>,
    /// Whether to create a directory instead of a file (ignores `content`/`encoding`).
    #[serde(default)]
    #[schema(example = false)]
    pub is_directory: Option<bool>,
}

/// Query parameters for DELETE requests
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DeleteQuery {
    /// Whether to delete recursively
    #[serde(default)]
    pub recursive: bool,
}

/// Response for delete operation
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = DeleteFileResponse)]
pub struct DeleteResponse {
    pub deleted: bool,
}

/// Query parameters for GET requests
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GetQuery {
    /// For directories: whether to list recursively
    #[serde(default)]
    pub recursive: bool,
}

/// Unified response for GET that can be file or directory listing
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(untagged)]
pub enum GetResponse {
    File(SessionFile),
    Listing(ListResponse<FileInfo>),
}

/// Request to search files
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct GrepRequest {
    /// Regex pattern to search for. Standard PCRE-ish (Rust `regex` crate) syntax.
    #[schema(example = "TODO\\(perf\\)")]
    pub pattern: String,
    /// Optional path glob to filter files (`**/*.rs`, `docs/*.md`).
    #[serde(default)]
    #[schema(example = "**/*.rs")]
    pub path_pattern: Option<String>,
}

/// Request to move/rename a file
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct MoveFileRequest {
    /// Source path (relative to the workspace filesystem root).
    #[schema(example = "drafts/migration-plan.md")]
    pub src_path: String,
    /// Destination path (relative to the workspace filesystem root).
    #[schema(example = "docs/migration-plan.md")]
    pub dst_path: String,
}

/// Request to get file stat
/// Paginated content search with surrounding context.
///
/// Distinct from [`GrepRequest`], which answers with matches grouped by path
/// and no context. This one streams a bounded window of matches plus the lines
/// around them, which is what an agent reading a large repository needs.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct SearchRequest {
    /// Regular expression to match against file contents.
    #[schema(example = "TODO\\(perf\\)")]
    pub pattern: String,

    /// Glob limiting which paths are searched.
    #[serde(default)]
    #[schema(example = "**/*.rs")]
    pub path_pattern: Option<String>,

    /// Lines of context to return before each match.
    #[serde(default)]
    pub before_context: usize,

    /// Lines of context to return after each match.
    #[serde(default)]
    pub after_context: usize,

    /// Number of matches to skip, for paging through a large result set.
    #[serde(default)]
    pub offset: usize,

    /// Maximum matches to return. Omitted means no limit beyond `max_bytes`.
    #[serde(default)]
    pub limit: Option<usize>,

    /// Byte ceiling on the returned payload. Omitted uses the server default.
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct StatRequest {
    /// Path to the file or directory (relative to the workspace filesystem root).
    #[schema(example = "docs/migration-plan.md")]
    pub path: String,
}

/// Request to update a file
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateFileRequest {
    /// Content the file must currently hold for the write to happen.
    ///
    /// When set, the update is a compare-and-swap: the write lands only if the
    /// stored bytes equal these, and a mismatch is reported as a conflict
    /// rather than overwriting a concurrent writer.
    #[serde(default)]
    pub expected_content: Option<String>,

    /// Encoding of `expected_content`. Defaults to `text` when omitted.
    #[serde(default)]
    pub expected_encoding: Option<String>,

    /// New file content
    #[serde(default)]
    #[schema(example = "# Project notes (rev 2)\n\nUpdated migration plan with rollback steps.\n")]
    pub content: Option<String>,
    /// Content encoding: "text" or "base64". Defaults to text.
    #[serde(default)]
    #[schema(example = "text")]
    pub encoding: Option<String>,
    /// Whether file is read-only
    #[serde(default)]
    #[schema(example = false)]
    pub is_readonly: Option<bool>,
}
