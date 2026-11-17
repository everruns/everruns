use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;

use super::tests::{MemorySecrets, test_config_with_init};
use super::{
    SandboxWorkspaceSync, SandboxWorkspaceSyncExt, SessionSandboxConfig, SessionSandboxProvider,
    SessionSandboxState, checkpoint_session_sandbox, create_session_sandbox_provider,
    ensure_session_sandbox_running,
};

#[derive(Default)]
struct Recording(Mutex<Vec<String>>);

#[async_trait]
impl SandboxWorkspaceSync for Recording {
    async fn before_create(
        &self,
        _: &ToolContext,
        _: &dyn SessionSandboxProvider,
        _: &SessionSandboxConfig,
    ) -> Result<(), ToolExecutionResult> {
        self.0.lock().unwrap().push("before_create".into());
        Ok(())
    }

    async fn push(
        &self,
        _: &ToolContext,
        _: &dyn SessionSandboxProvider,
        _: &SessionSandboxConfig,
        _: &mut SessionSandboxState,
        fresh: bool,
    ) -> Result<(), ToolExecutionResult> {
        self.0.lock().unwrap().push(format!("push fresh={fresh}"));
        Ok(())
    }

    async fn pull(
        &self,
        _: &ToolContext,
        _: &dyn SessionSandboxProvider,
        _: &SessionSandboxConfig,
        _: &mut SessionSandboxState,
    ) -> Result<(), ToolExecutionResult> {
        self.0.lock().unwrap().push("pull".into());
        Ok(())
    }
}

#[tokio::test]
async fn a_syncing_host_pushes_before_calls_and_pulls_instead_of_checkpoints() {
    let sync = Arc::new(Recording::default());
    let context = ToolContext::with_storage_store(
        everruns_contracts::typed_id::SessionId::new(),
        Arc::new(MemorySecrets::default()),
    )
    .with_extension(Arc::new(SandboxWorkspaceSyncExt(sync.clone())));
    let config = test_config_with_init(vec![]);

    let mut state = ensure_session_sandbox_running(&context, &config)
        .await
        .unwrap();
    ensure_session_sandbox_running(&context, &config)
        .await
        .unwrap();
    let provider = create_session_sandbox_provider(&config.provider).unwrap();
    checkpoint_session_sandbox(&context, provider.as_ref(), &config, &mut state)
        .await
        .unwrap();

    assert_eq!(
        *sync.0.lock().unwrap(),
        [
            "before_create",
            "push fresh=true",
            "push fresh=false",
            "pull"
        ]
    );
    // The provider's own archive is not taken.
    assert!(state.instance.metadata.get("checkpoint_revision").is_none());
}
