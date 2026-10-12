//! [`SandboxWorkspace`] over the session's managed sandbox, so integration
//! tools can run a command that needs a credential without the credential
//! entering the command text, the workspace, or a checkpoint.
//!
//! Decision: the secret travels through the provider's file upload into a
//! fresh owner-only file under `/tmp` (outside the checkpointed workspace),
//! the command reads it through `$EVR_SECRET_FILE`, and a shell `trap`
//! removes it when the command exits. The checkpoint runs only after that
//! exec returned, so it never sees the file. THREAT[TM-GITHUB-007].

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::CapabilityRef;
use everruns_contracts::runtime::sandbox_workspace::{
    SECRET_FILE_ENV, SandboxWorkspace, SandboxWorkspaceExt,
};
use everruns_core::tool_context::{ToolContext, ToolContextExtensions};
use everruns_core::tools::ToolExecutionResult;

use super::environment_tools::{
    canonicalize_workspace_output, resolve_workspace_path, shell_escape,
    translate_workspace_command, workspace_root,
};
use super::session_sandbox::provider_for_config;
use crate::session_sandbox::{
    SESSION_SANDBOX_CAPABILITY_ID, SessionSandboxConfig, SessionSandboxExecRequest,
    SessionSandboxExecResponse, checkpoint_session_sandbox, ensure_session_sandbox_running,
    session_sandbox_config_from_capabilities,
};

const REDACTED: &str = "[redacted]";

/// The session's managed sandbox, bound to its capability config.
pub struct ManagedSandboxWorkspace {
    config: SessionSandboxConfig,
}

impl ManagedSandboxWorkspace {
    pub fn new(config: SessionSandboxConfig) -> Self {
        Self { config }
    }
}

/// Install the sandbox workspace when the session has a managed sandbox.
pub fn install_sandbox_workspace(
    extensions: &mut ToolContextExtensions,
    resolved_capabilities: &[CapabilityRef],
) {
    if !resolved_capabilities
        .iter()
        .any(|capability| capability.capability_id() == SESSION_SANDBOX_CAPABILITY_ID)
    {
        return;
    }
    match session_sandbox_config_from_capabilities(resolved_capabilities) {
        Ok(Some(config)) => extensions.insert(Arc::new(SandboxWorkspaceExt(Arc::new(
            ManagedSandboxWorkspace::new(config),
        )))),
        Ok(None) => {}
        Err(error) => {
            tracing::debug!(%error, "no sandbox workspace: invalid session_sandbox config")
        }
    }
}

/// Wrap `command` so it sees the secret file and always removes it.
fn with_secret_file(secret_path: &str, command: &str) -> String {
    let path = shell_escape(secret_path);
    let trap = shell_escape(&format!("rm -f -- {path}"));
    format!(
        "trap {trap} EXIT HUP INT TERM\nchmod 600 -- {path} || exit 1\n\
         {SECRET_FILE_ENV}={path}; export {SECRET_FILE_ENV}\n(\n{command}\n)"
    )
}

fn redact(text: String, secret: &str) -> String {
    if secret.is_empty() {
        text
    } else {
        text.replace(secret, REDACTED)
    }
}

fn plain(command: String, cwd: Option<String>) -> SessionSandboxExecRequest {
    SessionSandboxExecRequest {
        command,
        cwd,
        timeout_ms: None,
        output_mode: "concise".to_string(),
    }
}

#[async_trait]
impl SandboxWorkspace for ManagedSandboxWorkspace {
    async fn exec_with_secret_file(
        &self,
        context: &ToolContext,
        command: &str,
        cwd: &str,
        secret: &str,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        let config = &self.config;
        let provider = provider_for_config(config)?;
        let mut state = ensure_session_sandbox_running(context, config).await?;
        let root = workspace_root(&state.instance).to_string();
        let cwd = resolve_workspace_path(&state.instance, cwd)
            .map_err(ToolExecutionResult::tool_error)?;
        let secret_path = format!("/tmp/.everruns-secret-{}", uuid::Uuid::new_v4().simple());

        // Create it owner-only before the content arrives, so the upload never
        // lands in a world-readable file.
        let created = provider
            .exec(
                context,
                config,
                &state.instance,
                &plain(
                    format!("umask 077 && : > {}", shell_escape(&secret_path)),
                    None,
                ),
            )
            .await?;
        if created.exit_code != 0 {
            return Err(ToolExecutionResult::tool_error(
                "Could not create a private file in the sandbox",
            ));
        }
        let ran = match provider
            .write_file(
                context,
                config,
                &state.instance,
                &secret_path,
                secret.as_bytes(),
            )
            .await
        {
            Ok(_) => {
                provider
                    .exec(
                        context,
                        config,
                        &state.instance,
                        &plain(
                            with_secret_file(
                                &secret_path,
                                &translate_workspace_command(command, &root),
                            ),
                            Some(cwd),
                        ),
                    )
                    .await
            }
            Err(error) => Err(error),
        };
        // The trap covers a normal exit; a provider error or a killed command
        // may skip it.
        if !matches!(&ran, Ok(response) if response.exit_code == 0) {
            let _ = provider
                .exec(
                    context,
                    config,
                    &state.instance,
                    &plain(format!("rm -f -- {}", shell_escape(&secret_path)), None),
                )
                .await;
        }
        let mut response = ran?;
        checkpoint_session_sandbox(context, provider.as_ref(), config, &mut state).await?;
        response.stdout = redact(
            canonicalize_workspace_output(&response.stdout, &root),
            secret,
        );
        response.stderr = redact(
            canonicalize_workspace_output(&response.stderr, &root),
            secret,
        );
        response.raw_output = response
            .raw_output
            .map(|raw| redact(canonicalize_workspace_output(&raw, &root), secret));
        Ok(response)
    }
}

#[cfg(test)]
#[path = "sandbox_workspace_tests.rs"]
mod tests;
