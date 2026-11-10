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
///
/// Every command is a POSIX `sh` script: an implementation runs it under `sh`
/// even when the provider's default shell is another one.
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
    ///
    /// Concurrent sandbox code can read the file as the same identity. Callers
    /// must admit only credentials whose verified scope permits that exposure;
    /// temporary-file cleanup does not provide credential isolation.
    async fn exec_with_secret_file(
        &self,
        context: &ToolContext,
        command: &str,
        cwd: &str,
        secret: &str,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult>;

    /// Where `/workspace` lives on the provider's own filesystem.
    ///
    /// The methods below address the sandbox by provider paths, not by the
    /// model's `/workspace` alias, because their callers (code navigation)
    /// exchange absolute paths with programs running inside the sandbox.
    async fn provider_root(&self, _context: &ToolContext) -> Result<String, ToolExecutionResult> {
        Err(unsupported("provider_root"))
    }

    /// Run `command` as written (no `/workspace` translation) and return its
    /// whole output. Nothing is checkpointed: a caller that changed files calls
    /// [`Self::checkpoint`] once it is done.
    async fn exec_raw(
        &self,
        _context: &ToolContext,
        _command: &str,
        _cwd: Option<&str>,
        _timeout_ms: Option<u64>,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        Err(unsupported("exec_raw"))
    }

    /// Read one file by provider path. Providers cap the size of one read.
    async fn read_file_raw(
        &self,
        _context: &ToolContext,
        _path: &str,
    ) -> Result<Vec<u8>, ToolExecutionResult> {
        Err(unsupported("read_file_raw"))
    }

    /// Write one file by provider path, without checkpointing.
    async fn write_file_raw(
        &self,
        _context: &ToolContext,
        _path: &str,
        _content: &[u8],
    ) -> Result<(), ToolExecutionResult> {
        Err(unsupported("write_file_raw"))
    }

    /// Persist the workspace after a batch of raw writes.
    async fn checkpoint(&self, _context: &ToolContext) -> Result<(), ToolExecutionResult> {
        Err(unsupported("checkpoint"))
    }
}

fn unsupported(operation: &str) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(format!(
        "this sandbox workspace does not support `{operation}`"
    ))
}

/// Tool-context extension carrying the session's [`SandboxWorkspace`].
#[derive(Clone)]
pub struct SandboxWorkspaceExt(pub Arc<dyn SandboxWorkspace>);
