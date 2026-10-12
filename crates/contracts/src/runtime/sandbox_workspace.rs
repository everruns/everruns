//! The session's primary sandbox, as other capabilities may use it.
//!
//! Integration tools (for example `github_checkout_pull_request`) need to run
//! a command in the managed sandbox without depending on the crate that owns
//! it, so the sandbox capability installs this as a tool-context extension and
//! the tool looks it up. No extension means the session has no sandbox.

use std::sync::Arc;

use async_trait::async_trait;

use crate::runtime::tool_context::ToolContext;
use crate::session_sandbox::SessionSandboxExecResponse;
use crate::tools::ToolExecutionResult;

/// Name of the variable that holds the secret file's path inside the command.
pub const SECRET_FILE_ENV: &str = "EVR_SECRET_FILE";

/// Runs commands in the session's primary sandbox.
#[async_trait]
pub trait SandboxWorkspace: Send + Sync {
    /// The workspace root as the model sees it.
    fn workspace_root(&self) -> &str {
        "/workspace"
    }

    /// Run `command` in `cwd` (a `/workspace` path) with a secret readable at
    /// `$EVR_SECRET_FILE`.
    ///
    /// The secret goes to a fresh owner-only file outside the workspace, never
    /// into the command text, and the file is removed when the command exits,
    /// before the workspace is checkpointed. The variable is set for this one
    /// command only.
    async fn exec_with_secret_file(
        &self,
        context: &ToolContext,
        command: &str,
        cwd: &str,
        secret: &str,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult>;
}

/// Tool-context extension carrying the session's [`SandboxWorkspace`].
#[derive(Clone)]
pub struct SandboxWorkspaceExt(pub Arc<dyn SandboxWorkspace>);
