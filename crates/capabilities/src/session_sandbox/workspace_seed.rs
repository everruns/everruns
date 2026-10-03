use serde_json::json;

use super::{
    SessionSandboxConfig, SessionSandboxInstance, SessionSandboxProvider, SessionSandboxState,
    checkpoint_session_sandbox, now_rfc3339, save_session_sandbox_state,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;

const STATE_KEY: &str = "everruns_workspace_seed";
const PENDING: &str = "pending";
const COMPLETE: &str = "complete";

pub(super) fn mark_pending(instance: &mut SessionSandboxInstance) {
    instance.metadata[STATE_KEY] = json!(PENDING);
}

fn provider_workspace_path(instance: &SessionSandboxInstance, session_path: &str) -> String {
    let relative = session_path
        .strip_prefix("/workspace/")
        .unwrap_or_else(|| session_path.trim_start_matches('/'));
    let root = instance
        .workspace_path
        .as_deref()
        .unwrap_or("/workspace")
        .trim_end_matches('/');
    if relative.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{relative}")
    }
}

/// Seed the durable session workspace into a newly-created physical Environment.
/// Existing pre-Environment sandboxes have no pending marker and are never overwritten.
pub(super) async fn seed_if_pending(
    context: &ToolContext,
    provider: &dyn SessionSandboxProvider,
    config: &SessionSandboxConfig,
    state: &mut SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    if state.instance.metadata[STATE_KEY] != PENDING {
        return Ok(());
    }
    let Some(file_store) = context.file_store.as_ref() else {
        return Ok(());
    };

    let mut directories = vec!["/workspace".to_string()];
    let mut wrote_file = false;
    while let Some(directory) = directories.pop() {
        let entries = file_store
            .list_directory(context.session_id, &directory)
            .await
            .map_err(|error| {
                ToolExecutionResult::tool_error(format!(
                    "Failed to seed Environment workspace: {error}"
                ))
            })?;
        for entry in entries {
            if entry.is_directory {
                directories.push(entry.path);
                continue;
            }
            let Some(file) = file_store
                .read_file(context.session_id, &entry.path)
                .await
                .map_err(|error| {
                    ToolExecutionResult::tool_error(format!(
                        "Failed to read Environment starter file '{}': {error}",
                        entry.path
                    ))
                })?
            else {
                continue;
            };
            let bytes = everruns_core::SessionFile::decode_content(
                file.content.as_deref().unwrap_or_default(),
                &file.encoding,
            )
            .map_err(|error| {
                ToolExecutionResult::tool_error(format!(
                    "Failed to decode Environment starter file '{}': {error}",
                    entry.path
                ))
            })?;
            let destination = provider_workspace_path(&state.instance, &entry.path);
            provider
                .write_file(context, config, &state.instance, &destination, &bytes)
                .await?;
            wrote_file = true;
        }
    }

    if wrote_file {
        checkpoint_session_sandbox(context, provider, config, state).await?;
    }
    state.instance.metadata[STATE_KEY] = json!(COMPLETE);
    state.updated_at = now_rfc3339();
    save_session_sandbox_state(context, state).await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use everruns_core::InitialFile;
    use everruns_core::host::{InMemorySessionFileStore, InMemorySessionStorageStore};

    use super::*;
    use crate::session_sandbox::{
        SessionSandboxExecRequest, SessionSandboxExecResponse, SessionSandboxReadFileResponse,
        SessionSandboxStatus, SessionSandboxStatusResponse, SessionSandboxWriteFileResponse,
    };

    #[derive(Default)]
    struct RecordingProvider(Mutex<Vec<(String, Vec<u8>)>>);

    #[async_trait]
    impl SessionSandboxProvider for RecordingProvider {
        fn id(&self) -> &str {
            "workspace-seed-test"
        }

        async fn create(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
        ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
            unreachable!()
        }

        async fn resume(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
            unreachable!()
        }

        async fn pause(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
            unreachable!()
        }

        async fn delete(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
        ) -> Result<(), ToolExecutionResult> {
            unreachable!()
        }

        async fn exec(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            _request: &SessionSandboxExecRequest,
        ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
            unreachable!()
        }

        async fn read_file(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            _path: &str,
        ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
            unreachable!()
        }

        async fn write_file(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            path: &str,
            content: &[u8],
        ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
            self.0
                .lock()
                .unwrap()
                .push((path.to_string(), content.to_vec()));
            Ok(SessionSandboxWriteFileResponse {
                path: path.to_string(),
                bytes_written: content.len(),
            })
        }

        async fn status(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _state: &SessionSandboxState,
        ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn seeds_initial_files_into_the_provider_workspace() {
        let session_id = everruns_contracts::typed_id::SessionId::new();
        let files = Arc::new(InMemorySessionFileStore::new());
        files
            .seed_initial_file(
                session_id,
                &InitialFile {
                    path: "/workspace/src/main.rs".to_string(),
                    content: "fn main() {}\n".to_string(),
                    encoding: "text".to_string(),
                    is_readonly: false,
                },
            )
            .await
            .unwrap();
        let context = ToolContext::with_stores(
            session_id,
            files,
            Arc::new(InMemorySessionStorageStore::new()),
        );
        let provider = RecordingProvider::default();
        let mut state = SessionSandboxState {
            sandbox: None,
            provider: provider.id().to_string(),
            status: SessionSandboxStatus::Running,
            instance: SessionSandboxInstance {
                external_id: "physical-1".to_string(),
                workspace_path: Some("/home/daytona".to_string()),
                ..Default::default()
            },
            init_completed_at: None,
            last_init_error: None,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        mark_pending(&mut state.instance);

        seed_if_pending(
            &context,
            &provider,
            &SessionSandboxConfig {
                provider: provider.id().to_string(),
                ..Default::default()
            },
            &mut state,
        )
        .await
        .unwrap();

        assert_eq!(state.instance.metadata[STATE_KEY], COMPLETE);
        assert_eq!(
            *provider.0.lock().unwrap(),
            vec![(
                "/home/daytona/src/main.rs".to_string(),
                b"fn main() {}\n".to_vec()
            )]
        );
    }
}
