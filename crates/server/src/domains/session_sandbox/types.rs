// Session sandbox domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request body for the `manage_session_sandbox` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ManageSessionSandboxRequest {
    /// Action to take on the sandbox (`reset`, `delete`, etc.).
    pub action: SessionSandboxAction,
}

/// Response body for the `manage_session_sandbox` operation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ManageSessionSandboxResponse {
    /// Action that was taken.
    pub action: SessionSandboxAction,
    /// Whether a sandbox instance still exists after the action.
    pub exists: bool,
    /// Whether the action deleted the sandbox (`true` after a successful `delete`).
    pub deleted: bool,
    /// Sandbox provider (`daytona`, `e2b`, `docker`, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Current sandbox lifecycle status after the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_status: Option<SessionSandboxStatusValue>,
    /// Provider-side sandbox identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Human-readable sandbox label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Absolute path of the sandbox workspace root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
}

/// Operator action to take against a session's managed sandbox. `Pause`
/// suspends the instance, `Resume` restarts it, `Delete` releases the
/// lease.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionSandboxAction {
    Pause,
    Resume,
    Delete,
}

/// Wire-facing status of a session sandbox. Mirrors
/// `everruns_capabilities::session_sandbox::SessionSandboxStatus` for the public API.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionSandboxStatusValue {
    Running,
    Paused,
    Lost,
}

impl From<everruns_capabilities::session_sandbox::SessionSandboxStatus>
    for SessionSandboxStatusValue
{
    fn from(status: everruns_capabilities::session_sandbox::SessionSandboxStatus) -> Self {
        match status {
            everruns_capabilities::session_sandbox::SessionSandboxStatus::Running => Self::Running,
            everruns_capabilities::session_sandbox::SessionSandboxStatus::Paused => Self::Paused,
            everruns_capabilities::session_sandbox::SessionSandboxStatus::Lost => Self::Lost,
        }
    }
}

/// What a session changed in the repositories checked out in its sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionRepositoriesResponse {
    /// `running` when the checkouts were read. Any other value (`paused`,
    /// `not_started`, `none`, ...) means the sandbox was left alone and
    /// `repositories` is empty.
    #[schema(example = "running")]
    pub sandbox_state: String,
    /// The branch the agent works on in every writable repository.
    #[schema(example = "everruns/bdaf0ce2")]
    pub work_branch: Option<String>,
    pub repositories: Vec<SessionRepository>,
}

/// One checkout under `/workspace`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionRepository {
    /// `owner/name`, when the checkout's remote is a GitHub repository.
    #[schema(example = "acme/app")]
    pub repo: Option<String>,
    #[schema(example = "/workspace/app")]
    pub path: String,
    /// `read`, `write`, or `none` when the session's settings do not list it.
    #[schema(example = "write")]
    pub access: String,
    #[schema(example = "everruns/bdaf0ce2")]
    pub branch: String,
    pub on_work_branch: bool,
    #[schema(example = "main")]
    pub base_branch: String,
    pub uncommitted_files: u64,
    /// Absent when the base branch is not known locally.
    pub commits_ahead_of_base: Option<u64>,
    /// Absent when the branch was never pushed.
    pub commits_not_pushed: Option<u64>,
    /// Files that differ from the base branch, committed or not.
    #[serde(default)]
    pub files: Vec<SessionRepositoryFile>,
    /// Unified diff against the base branch, cut at a fixed size.
    #[serde(default)]
    pub diff: String,
    #[serde(default)]
    pub diff_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionRepositoryFile {
    #[schema(example = "src/config.rs")]
    pub path: String,
    /// Absent for binary and new untracked files.
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub committed: bool,
}
