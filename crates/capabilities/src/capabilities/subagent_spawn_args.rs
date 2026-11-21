//! Argument parsing for `spawn_agent`: lifetime, seed and sandbox mode.

use everruns_contracts::typed_id::SessionId;
use everruns_core::session::SessionSeedMode;
use everruns_core::{tool_context::ToolContext, tools::ToolExecutionResult};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnLifetime {
    Linked,
    Detached,
}

impl SpawnLifetime {
    pub fn parse(arguments: &Value) -> Result<Self, ToolExecutionResult> {
        match arguments.get("lifetime").and_then(Value::as_str) {
            None | Some("linked") => Ok(Self::Linked),
            Some("detached") => Ok(Self::Detached),
            Some(other) => Err(ToolExecutionResult::tool_error(format!(
                "Invalid lifetime: {other}. Expected 'linked' or 'detached'."
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Detached => "detached",
        }
    }
}

pub(super) fn parse_seed(arguments: &Value) -> Result<SessionSeedMode, ToolExecutionResult> {
    match arguments.get("seed").and_then(Value::as_str) {
        None | Some("fresh") => Ok(SessionSeedMode::Fresh),
        Some("fork") => Ok(SessionSeedMode::Fork),
        Some("workspace") => Ok(SessionSeedMode::Workspace),
        Some(other) => Err(ToolExecutionResult::tool_error(format!(
            "Invalid seed: {other}. Expected 'fresh', 'fork', or 'workspace'."
        ))),
    }
}

/// `sandbox`: `fresh` (default) or `fork`, a copy of the parent's running
/// sandbox made when the child first uses its sandbox.
pub(super) fn parse_fork_sandbox(arguments: &Value) -> Result<bool, ToolExecutionResult> {
    match arguments.get("sandbox").and_then(Value::as_str) {
        None | Some("fresh") => Ok(false),
        Some("fork") => Ok(true),
        Some(other) => Err(ToolExecutionResult::tool_error(format!(
            "Invalid sandbox: {other}. Expected 'fresh' or 'fork'."
        ))),
    }
}

/// Record the sandbox copy for a child spawned with `sandbox: "fork"`, before
/// its first message so its first sandbox use finds it. A host that cannot
/// copy sandboxes leaves the child fresh. Returns the mode the child got.
pub(super) async fn child_sandbox_mode(
    context: &ToolContext,
    fork: bool,
    child_session_id: SessionId,
) -> Result<&'static str, ToolExecutionResult> {
    if !fork {
        return Ok("fresh");
    }
    match crate::sandbox_state::request_session_sandbox_fork(context, child_session_id).await {
        Ok(true) => Ok("fork"),
        Ok(false) => Ok("fresh"),
        Err(error) => Err(error),
    }
}
