//! A host whose own workspace is the managed sandbox's source of truth.
//!
//! By default the sandbox becomes the source of truth once seeded, and a
//! provider checkpoint archives it after each mutating call. A host that
//! installs [`SandboxWorkspaceSyncExt`] keeps the session's workspace
//! authoritative instead (data plane v2, `fs.md` "sync mode"): before a
//! sandbox call the sandbox catches up with the workspace, after a mutating
//! call the workspace takes what the sandbox changed. A lost sandbox then
//! restores from the workspace, and provider checkpoints are not taken.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;

use super::{
    SessionSandboxConfig, SessionSandboxProvider, SessionSandboxState, now_rfc3339,
    save_session_sandbox_state, workspace_seed,
};

#[async_trait]
pub trait SandboxWorkspaceSync: Send + Sync {
    /// Before a new sandbox is created for the session: a chance to remove
    /// one an earlier attempt created but never recorded.
    async fn before_create(
        &self,
        _context: &ToolContext,
        _provider: &dyn SessionSandboxProvider,
        _config: &SessionSandboxConfig,
    ) -> Result<(), ToolExecutionResult> {
        Ok(())
    }

    /// Bring the sandbox's workspace to the session's. `fresh`: the sandbox
    /// was just created or replaced, and holds nothing of the session yet.
    async fn push(
        &self,
        context: &ToolContext,
        provider: &dyn SessionSandboxProvider,
        config: &SessionSandboxConfig,
        state: &mut SessionSandboxState,
        fresh: bool,
    ) -> Result<(), ToolExecutionResult>;

    /// Take what a sandbox call changed into the session's workspace.
    async fn pull(
        &self,
        context: &ToolContext,
        provider: &dyn SessionSandboxProvider,
        config: &SessionSandboxConfig,
        state: &mut SessionSandboxState,
    ) -> Result<(), ToolExecutionResult>;
}

/// Installs a [`SandboxWorkspaceSync`] on a tool context.
#[derive(Clone)]
pub struct SandboxWorkspaceSyncExt(pub Arc<dyn SandboxWorkspaceSync>);

pub(super) fn installed(context: &ToolContext) -> Option<Arc<dyn SandboxWorkspaceSync>> {
    context
        .extensions
        .get::<SandboxWorkspaceSyncExt>()
        .map(|ext| ext.0.clone())
}

/// The workspace into the sandbox: synced when the host syncs, else the
/// one-time seed of a new sandbox.
pub(super) async fn seed_or_push(
    context: &ToolContext,
    provider: &dyn SessionSandboxProvider,
    config: &SessionSandboxConfig,
    state: &mut SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    let Some(sync) = installed(context) else {
        return workspace_seed::seed_if_pending(context, provider, config, state).await;
    };
    let fresh = workspace_seed::is_pending(&state.instance);
    sync.push(context, provider, config, state, fresh).await?;
    if fresh {
        state.instance.metadata[workspace_seed::STATE_KEY] = json!(workspace_seed::COMPLETE);
        state.updated_at = now_rfc3339();
        save_session_sandbox_state(context, state).await?;
    }
    Ok(())
}
