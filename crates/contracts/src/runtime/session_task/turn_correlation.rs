// Turn correlation for session-task lifecycle events.
//
// A task outlives the turn that created it, and `task.*` events are emitted
// from the registry long after that turn's tool call returned. Without the
// origin recorded on the task itself, every later event carries a default
// `EventContext` and concurrent turns in one session become indistinguishable
// downstream.
//
// Split out of `session_task.rs`, which sits one line under the source-file
// size threshold.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskUpdate, TaskMessage,
};
use crate::runtime::error::Result;
use crate::runtime::events::EventContext;
use crate::runtime::typed_id::SessionId;

/// Spec key holding the origin context. Server-owned, and stripped from the
/// public task projection by `redacted_public_task_spec`.
pub(super) const ORIGIN_EVENT_CONTEXT_KEY: &str = "__everruns_origin_event_context";

/// Persist turn correlation inside the task's opaque spec so later updates can
/// emit lifecycle events for the turn that created the task. The key is
/// server-owned and overwritten when a task is created from a tool context.
pub fn set_task_origin_event_context(spec: &mut Value, context: &EventContext) {
    let Value::Object(spec) = spec else {
        return;
    };
    if let Ok(value) = serde_json::to_value(context) {
        spec.insert(ORIGIN_EVENT_CONTEXT_KEY.to_string(), value);
    }
}

/// Read back the turn a task was created in, when one was recorded.
pub fn task_origin_event_context(task: &SessionTask) -> Option<EventContext> {
    serde_json::from_value(task.spec.get(ORIGIN_EVENT_CONTEXT_KEY)?.clone()).ok()
}

/// The counterpart that keeps the server-owned key above out of the public
/// task projection, alongside the push-config secrets it already hid. Both are
/// the same question — what of a task's opaque spec a caller may read back — so
/// they stay next to the writer rather than drifting apart from it.
pub(super) fn redacted_public_task_spec(spec: &Value) -> Value {
    let mut public = spec.clone();
    if let Some(object) = public.as_object_mut() {
        object.remove(ORIGIN_EVENT_CONTEXT_KEY);
    }
    let Some(configs) = public.get_mut("push_configs").and_then(Value::as_array_mut) else {
        return public;
    };
    for config in configs {
        let Some(config) = config.as_object_mut() else {
            continue;
        };
        if config.remove("secret").is_some() {
            config.insert("has_secret".to_string(), Value::Bool(true));
        }
    }
    public
}

/// Turn-scoped view of a registry. Only creation is decorated; the durable
/// task snapshot carries the origin for all later updates and worker reattach.
pub struct TurnCorrelatedSessionTaskRegistry {
    inner: Arc<dyn SessionTaskRegistry>,
    context: EventContext,
}

impl TurnCorrelatedSessionTaskRegistry {
    /// Decorate `inner` so tasks it creates record `context` as their origin.
    pub fn wrap(
        inner: Arc<dyn SessionTaskRegistry>,
        context: EventContext,
    ) -> Arc<dyn SessionTaskRegistry> {
        Arc::new(Self { inner, context })
    }
}

#[async_trait]
impl SessionTaskRegistry for TurnCorrelatedSessionTaskRegistry {
    async fn create(&self, mut input: CreateSessionTask) -> Result<SessionTask> {
        set_task_origin_event_context(&mut input.spec, &self.context);
        self.inner.create(input).await
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        self.inner.update(session_id, task_id, update).await
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        self.inner.get(session_id, task_id).await
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        self.inner.list(session_id, filter).await
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        self.inner.request_cancel(session_id, task_id).await
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        self.inner
            .record_message(session_id, task_id, message)
            .await
    }

    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        self.inner
            .list_messages(session_id, task_id, limit, after_id)
            .await
    }
}

// The binding lives here rather than next to `ToolContext`'s other setters so
// the three pieces that make turn correlation work — the spec key, the registry
// wrapper, and the one place that applies it — stay readable together.
impl crate::runtime::tool_context::ToolContext {
    /// Bind this context to the turn it is executing in.
    ///
    /// Sets the event context, and rewraps any session-task registry already on
    /// the context so tasks created during the turn record it as their origin.
    /// A task outlives the turn that created it, and its later lifecycle events
    /// are emitted from the registry with no other way back to that turn — so
    /// the binding happens here, where the turn is known, rather than at each
    /// `task.*` emission.
    pub fn bind_to_turn(&mut self, context: EventContext) {
        if let Some(registry) = self.session_task_registry.take() {
            self.session_task_registry = Some(TurnCorrelatedSessionTaskRegistry::wrap(
                registry,
                context.clone(),
            ));
        }
        self.event_context = Some(context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::session_task::{
        SessionTaskState, TASK_KIND_BACKGROUND_TOOL, TaskLinks, TaskWakePolicy, new_session_task,
    };
    use crate::runtime::typed_id::{MessageId, TurnId};
    use chrono::{TimeZone, Utc};

    fn task() -> SessionTask {
        new_session_task(
            CreateSessionTask {
                session_id: SessionId::from_uuid(uuid::Uuid::from_u128(1)),
                id: Some("task_fixed".into()),
                kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
                display_name: "Test".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Queued,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            },
            Utc.timestamp_opt(10, 0).unwrap(),
        )
    }

    #[test]
    fn task_origin_context_is_durable_but_not_public() {
        let mut task = task();
        let context = EventContext::turn(TurnId::from_seed(2), MessageId::from_seed(3));
        set_task_origin_event_context(&mut task.spec, &context);

        let stored = task_origin_event_context(&task).unwrap();
        assert_eq!(stored.turn_id, context.turn_id);
        assert_eq!(stored.input_message_id, context.input_message_id);
        assert_eq!(
            serde_json::to_value(&task).unwrap()["spec"],
            serde_json::json!({})
        );
    }
}
