use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{EnvironmentLifecycleData, EventContext, EventData, EventRequest};
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
    let data = EnvironmentLifecycleData {
        environment_id: state.sandbox.as_ref().map(|sandbox| sandbox.id.to_string()),
        provider: state.provider.clone(),
        previous_instance_id: previous_instance_id.to_string(),
        current_instance_id: recovered.then(|| state.instance.external_id.clone()),
        generation: state.sandbox.as_ref().map(|sandbox| sandbox.generation),
        process_state_lost: true,
    };
    let event_data = if recovered {
        EventData::EnvironmentRecovered(data)
    } else {
        EventData::EnvironmentInstanceLost(data)
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
        tracing::warn!(%error, "failed to emit managed Environment lifecycle event");
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
    state.status = SessionSandboxStatus::Running;
    state.last_init_error = None;
    state.updated_at = now_rfc3339();
    save_session_sandbox_state(context, state).await?;
    if lost {
        emit_lifecycle_event(context, state, &previous_instance_id, true).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use everruns_contracts::typed_id::{EventId, SessionId};
    use everruns_core::events::{
        ENVIRONMENT_INSTANCE_LOST, ENVIRONMENT_RECOVERED, Event, EventRequest,
    };

    use super::*;
    use crate::session_sandbox::SessionSandboxInstance;

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
        assert_eq!(requests[0].event_type, ENVIRONMENT_INSTANCE_LOST);
        assert_eq!(requests[1].event_type, ENVIRONMENT_RECOVERED);
        let EventData::EnvironmentRecovered(data) = &requests[1].data else {
            panic!("expected environment.recovered payload");
        };
        assert_eq!(data.provider, "daytona");
        assert_eq!(data.previous_instance_id, "physical-old");
        assert_eq!(data.current_instance_id.as_deref(), Some("physical-new"));
        assert!(data.process_state_lost);
    }
}
