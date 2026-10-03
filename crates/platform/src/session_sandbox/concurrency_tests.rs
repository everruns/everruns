use std::sync::Arc;

use everruns_core::tool_context::ToolContext;

use super::tests::{
    MemorySecrets, reset_test_provider_state, test_config_with_init, test_instance,
    test_provider_state,
};
use super::{
    SessionSandboxState, SessionSandboxStatus, ensure_session_sandbox_running, now_rfc3339,
    save_session_sandbox_state,
};

#[tokio::test]
async fn concurrent_tools_share_one_lifecycle_recovery() {
    let external_id = "sb_parallel_loss";
    reset_test_provider_state(external_id, SessionSandboxStatus::Lost);

    let storage = Arc::new(MemorySecrets::default());
    let context = Arc::new(ToolContext::with_storage_store(
        everruns_provider::typed_id::SessionId::new(),
        storage,
    ));
    let mut state = SessionSandboxState {
        sandbox: None,
        provider: "core-test-session-sandbox".to_string(),
        status: SessionSandboxStatus::Running,
        instance: test_instance(external_id),
        init_completed_at: Some(now_rfc3339()),
        last_init_error: None,
        created_at: now_rfc3339(),
        updated_at: now_rfc3339(),
    };
    save_session_sandbox_state(&context, &mut state)
        .await
        .unwrap();
    let config = test_config_with_init(vec![]);

    let (first, second) = tokio::join!(
        ensure_session_sandbox_running(&context, &config),
        ensure_session_sandbox_running(&context, &config),
    );

    assert!(first.is_ok(), "{first:?}");
    assert!(second.is_ok(), "{second:?}");
    assert_eq!(test_provider_state(external_id).resume_calls, 1);
}
