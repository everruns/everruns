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
