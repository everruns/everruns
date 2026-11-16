use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{EventContext, EventData, EventRequest, SandboxLifecycleData};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;

use super::{
    SessionSandboxConfig, SessionSandboxProvider, SessionSandboxState, SessionSandboxStatus,
    now_rfc3339, save_session_sandbox_state,
};

type SessionSandboxLock = tokio::sync::Mutex<()>;

static SESSION_SANDBOX_LOCKS: LazyLock<Mutex<HashMap<String, Weak<SessionSandboxLock>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn lifecycle_lock(
    session_id: everruns_contracts::typed_id::SessionId,
) -> Arc<SessionSandboxLock> {
    let mut locks = SESSION_SANDBOX_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    let key = session_id.to_string();
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(SessionSandboxLock::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

async fn emit_lifecycle_event(
    context: &ToolContext,
    state: &SessionSandboxState,
    previous_instance_id: &str,
    recovered: bool,
) {
    let Some(emitter) = context.event_emitter.as_ref() else {
        return;
    };
    let data = SandboxLifecycleData {
        sandbox_id: state.sandbox.as_ref().map(|sandbox| sandbox.id.to_string()),
        provider: state.provider.clone(),
        previous_instance_id: previous_instance_id.to_string(),
        current_instance_id: recovered.then(|| state.instance.external_id.clone()),
        generation: state.sandbox.as_ref().map(|sandbox| sandbox.generation),
        process_state_lost: true,
    };
    let event_data = if recovered {
        EventData::SandboxRecovered(data)
    } else {
        EventData::SandboxInstanceLost(data)
    };
    let request = EventRequest::new(
        context.session_id,
        context
            .event_context
            .clone()
            .unwrap_or_else(EventContext::empty),
        event_data,
    );
    if let Err(error) = emitter.emit(request).await {
        tracing::warn!(%error, "failed to emit managed Sandbox lifecycle event");
    }
}

pub(super) async fn resume_if_needed(
    context: &ToolContext,
    provider: &dyn SessionSandboxProvider,
    config: &SessionSandboxConfig,
    state: &mut SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    let observed = match state.status {
        SessionSandboxStatus::Paused | SessionSandboxStatus::Lost => state.status,
        SessionSandboxStatus::Running => {
            provider
                .status(context, config, state)
                .await?
                .session_status
        }
    };
    if observed == SessionSandboxStatus::Running {
        return Ok(());
    }

    let previous_instance_id = state.instance.external_id.clone();
    let lost = observed == SessionSandboxStatus::Lost;
    if lost {
        emit_lifecycle_event(context, state, &previous_instance_id, false).await;
    }
    state.instance = provider.resume(context, config, &state.instance).await?;
    // A provider may answer a resume with a different physical sandbox: the
    // old one was deleted while paused, which a stored `Paused` status cannot
    // know. Unless the provider restored the workspace into it, the new one is
    // empty, so it gets the starter files and the init commands again.
    let replaced = state.instance.external_id != previous_instance_id;
    if replaced && state.instance.metadata["recovered"] != true {
        state.init_completed_at = None;
        super::workspace_seed::mark_pending(&mut state.instance);
    }
    state.status = SessionSandboxStatus::Running;
    state.last_init_error = None;
    state.updated_at = now_rfc3339();
    save_session_sandbox_state(context, state).await?;
    if replaced && !lost {
        emit_lifecycle_event(context, state, &previous_instance_id, false).await;
    }
    if lost || replaced {
        emit_lifecycle_event(context, state, &previous_instance_id, true).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use everruns_contracts::typed_id::{EventId, SessionId};
    use everruns_core::events::{Event, EventRequest, SANDBOX_INSTANCE_LOST, SANDBOX_RECOVERED};

    use super::*;
    use crate::session_sandbox::tests::{
        MemorySecrets, reset_test_provider_state, test_config_with_init, test_instance,
        test_provider_state,
    };
    use crate::session_sandbox::{SessionSandboxInstance, ensure_session_sandbox_running};
    use serde_json::json;

    #[derive(Default)]
    struct RecordingEmitter(Mutex<Vec<EventRequest>>);

    #[async_trait]
    impl EventEmitter for RecordingEmitter {
        async fn emit(
            &self,
            request: EventRequest,
        ) -> Result<Event, everruns_contracts::error::AgentLoopError> {
            self.0.lock().unwrap().push(request.clone());
            Ok(request.into_event(EventId::new(), 1))
        }
    }

    #[tokio::test]
    async fn loss_and_recovery_events_identify_both_incarnations() {
        let emitter = Arc::new(RecordingEmitter::default());
        let mut context = ToolContext::new(SessionId::new());
        context.event_emitter = Some(emitter.clone());
        let mut state = SessionSandboxState {
            sandbox: None,
            provider: "daytona".to_string(),
            status: SessionSandboxStatus::Lost,
            instance: SessionSandboxInstance {
                external_id: "physical-old".to_string(),
                ..Default::default()
            },
            init_completed_at: None,
            last_init_error: None,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };

        emit_lifecycle_event(&context, &state, "physical-old", false).await;
        state.instance.external_id = "physical-new".to_string();
        emit_lifecycle_event(&context, &state, "physical-old", true).await;

        let requests = emitter.0.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].event_type, SANDBOX_INSTANCE_LOST);
        assert_eq!(requests[1].event_type, SANDBOX_RECOVERED);
        let EventData::SandboxRecovered(data) = &requests[1].data else {
            panic!("expected sandbox.recovered payload");
        };
        assert_eq!(data.provider, "daytona");
        assert_eq!(data.previous_instance_id, "physical-old");
        assert_eq!(data.current_instance_id.as_deref(), Some("physical-new"));
        assert!(data.process_state_lost);
    }

    #[tokio::test]
    async fn ensure_running_replaces_provider_instance_when_remote_status_is_lost() {
        let external_id = "sb_lost";
        reset_test_provider_state(external_id, SessionSandboxStatus::Lost);

        let storage = Arc::new(MemorySecrets::default());
        let context = ToolContext::with_storage_store(
            everruns_contracts::typed_id::SessionId::new(),
            storage,
        );
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

        let resolved = ensure_session_sandbox_running(&context, &test_config_with_init(vec![]))
            .await
            .unwrap();

        let provider_state = test_provider_state(external_id);
        assert_eq!(provider_state.resume_calls, 1);
        assert_eq!(resolved.status, SessionSandboxStatus::Running);
        assert_eq!(resolved.instance.metadata, json!({ "resumed": true }));
    }

    #[tokio::test]
    async fn ensure_running_initializes_a_sandbox_that_replaced_a_deleted_one() {
        let external_id = "sb_gone_paused";
        reset_test_provider_state(external_id, SessionSandboxStatus::Paused);
        reset_test_provider_state("sb_new_paused", SessionSandboxStatus::Running);

        let storage = Arc::new(MemorySecrets::default());
        let context = ToolContext::with_storage_store(
            everruns_contracts::typed_id::SessionId::new(),
            storage,
        );
        let mut state = SessionSandboxState {
            sandbox: None,
            provider: "core-test-session-sandbox".to_string(),
            // Stored as paused: nothing asks the provider whether it survived.
            status: SessionSandboxStatus::Paused,
            instance: test_instance(external_id),
            init_completed_at: Some(now_rfc3339()),
            last_init_error: None,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        save_session_sandbox_state(&context, &mut state)
            .await
            .unwrap();

        let config = test_config_with_init(vec!["echo ready"]);
        let resolved = ensure_session_sandbox_running(&context, &config)
            .await
            .unwrap();

        assert_eq!(resolved.instance.external_id, "sb_new_paused");
        assert_eq!(resolved.status, SessionSandboxStatus::Running);
        assert_eq!(
            test_provider_state("sb_new_paused").exec_commands,
            vec!["echo ready"],
            "the empty replacement is initialized again"
        );
        assert!(resolved.init_completed_at.is_some());

        // The same sandbox, resumed in place, keeps its init.
        ensure_session_sandbox_running(&context, &config)
            .await
            .unwrap();
        assert_eq!(test_provider_state("sb_new_paused").exec_commands.len(), 1);
    }
}
