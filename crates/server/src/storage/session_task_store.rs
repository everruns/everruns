// Database-backed session task registry.
//
// Implements the core SessionTaskRegistry trait over StorageBackend (both
// PostgreSQL and in-memory) and emits snapshot-carrying task.* events on the
// owning session's event stream. Event emission is best-effort: failures are
// logged and never fail the storage operation.

use async_trait::async_trait;
use chrono::Utc;
use everruns_core::events::{
    EventContext, EventData, EventRequest, SessionTaskEventData, TaskMessageEventData,
};
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TaskMessage, TaskMessageDirection,
    generate_task_message_id, new_session_task,
};
use everruns_core::traits::EventEmitter;
use everruns_core::{AgentLoopError, Result, SessionId};

use super::backend::StorageBackend;
use super::models::NewSessionTaskMessageRow;
use std::sync::Arc;

/// Database-backed session task registry.
#[derive(Clone)]
pub struct DbSessionTaskRegistry {
    db: Arc<StorageBackend>,
    emitter: Option<Arc<dyn EventEmitter>>,
}

impl DbSessionTaskRegistry {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db, emitter: None }
    }

    pub fn with_event_emitter(mut self, emitter: Arc<dyn EventEmitter>) -> Self {
        self.emitter = Some(emitter);
        self
    }

    async fn emit(&self, session_id: SessionId, data: EventData) {
        let Some(emitter) = &self.emitter else {
            return;
        };
        let request = EventRequest {
            event_type: data.event_type().to_string(),
            ts: Utc::now(),
            session_id,
            context: EventContext::default(),
            data,
            metadata: None,
            tags: None,
        };
        if let Err(e) = emitter.emit(request).await {
            tracing::warn!(session_id = %session_id, "Failed to emit task event: {e}");
        }
    }

    async fn emit_task_snapshot(&self, task: &SessionTask, created: bool) {
        let data = if created {
            EventData::TaskCreated(SessionTaskEventData { task: task.clone() })
        } else {
            EventData::TaskUpdated(SessionTaskEventData { task: task.clone() })
        };
        self.emit(task.session_id, data).await;
    }
}

#[async_trait]
impl SessionTaskRegistry for DbSessionTaskRegistry {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        let task = new_session_task(input, Utc::now());
        let (row, inserted) =
            self.db.create_session_task(&task).await.map_err(|e| {
                AgentLoopError::store(format!("Failed to create session task: {e}"))
            })?;
        let task = row
            .to_task()
            .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))?;
        if inserted {
            self.emit_task_snapshot(&task, true).await;
        }
        Ok(task)
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        let row = self
            .db
            .update_session_task(session_id, task_id, update)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to update session task: {e}")))?;
        let task = row
            .as_ref()
            .map(|r| r.to_task())
            .transpose()
            .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))?;
        if let Some(task) = &task {
            self.emit_task_snapshot(task, false).await;
        }
        Ok(task)
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        let row = self
            .db
            .get_session_task(session_id, task_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to get session task: {e}")))?;
        row.as_ref()
            .map(|r| r.to_task())
            .transpose()
            .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        let kind = filter.and_then(|f| f.kind.as_deref());
        let state = filter.and_then(|f| f.state.map(|s| s.to_string()));
        let rows = self
            .db
            .list_session_tasks(session_id, kind, state.as_deref())
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to list session tasks: {e}")))?;
        rows.iter()
            .map(|r| {
                r.to_task()
                    .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))
            })
            .collect()
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        let row = self
            .db
            .request_cancel_session_task(session_id, task_id)
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to request session task cancel: {e}"))
            })?;
        let Some((row, changed)) = row else {
            return Ok(None);
        };
        let task = row
            .to_task()
            .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))?;
        if changed {
            self.emit_task_snapshot(&task, false).await;
        }
        Ok(Some(task))
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        let task = self
            .get(session_id, task_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("Session task not found: {task_id}")))?;

        let row = self
            .db
            .insert_session_task_message(NewSessionTaskMessageRow {
                id: generate_task_message_id(),
                task_id: task_id.to_string(),
                session_id,
                direction: message.direction.to_string(),
                content: serde_json::to_value(&message.content).map_err(|e| {
                    AgentLoopError::store(format!("Invalid task message content: {e}"))
                })?,
                in_reply_to: message.in_reply_to.clone(),
            })
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to record task message: {e}")))?;
        let stored = row
            .to_message()
            .map_err(|e| AgentLoopError::store(format!("Invalid task message row: {e}")))?;

        let data = match stored.direction {
            TaskMessageDirection::Inbound => EventData::TaskMessageSent(TaskMessageEventData {
                task_id: task_id.to_string(),
                message: stored.clone(),
            }),
            TaskMessageDirection::Outbound => {
                EventData::TaskMessageReceived(TaskMessageEventData {
                    task_id: task_id.to_string(),
                    message: stored.clone(),
                })
            }
        };
        self.emit(session_id, data).await;

        // An inbound answer to the pending input request resumes the task.
        if stored.direction == TaskMessageDirection::Inbound
            && let (Some(in_reply_to), Some(pending)) = (&stored.in_reply_to, &task.input_request)
            && in_reply_to == &pending.id
        {
            self.update(
                session_id,
                task_id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await?;
        }

        Ok(stored)
    }

    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<TaskMessage>> {
        let rows = self
            .db
            .list_session_task_messages(session_id, task_id, limit)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to list task messages: {e}")))?;
        rows.iter()
            .map(|r| {
                r.to_message()
                    .map_err(|e| AgentLoopError::store(format!("Invalid task message row: {e}")))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::session_task::{TaskInputRequest, TaskLinks, TaskWakePolicy};

    fn registry() -> DbSessionTaskRegistry {
        DbSessionTaskRegistry::new(Arc::new(StorageBackend::in_memory()))
    }

    fn create_input(session_id: SessionId) -> CreateSessionTask {
        CreateSessionTask {
            session_id,
            id: None,
            kind: "background_tool".to_string(),
            display_name: "Test run".to_string(),
            spec: serde_json::json!({"tool": "demo"}),
            state: SessionTaskState::Queued,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        }
    }

    #[tokio::test]
    async fn create_is_idempotent_on_id() {
        let registry = registry();
        let session_id = SessionId::new();
        let mut input = create_input(session_id);
        input.id = Some("task_fixed".to_string());

        let first = registry.create(input.clone()).await.unwrap();
        let mut second_input = input.clone();
        second_input.display_name = "Changed".to_string();
        let second = registry.create(second_input).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.display_name, "Test run");
    }

    #[tokio::test]
    async fn update_applies_lifecycle_invariants() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry.create(create_input(session_id)).await.unwrap();
        assert!(task.started_at.is_none());

        let task = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::Running);
        assert!(task.started_at.is_some());

        let task = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    summary: Some("done".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert!(task.finished_at.is_some());

        // Terminal is final.
        let task = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::Succeeded);
    }

    #[tokio::test]
    async fn request_cancel_is_idempotent() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry.create(create_input(session_id)).await.unwrap();

        let first = registry
            .request_cancel(session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        let stamp = first.cancel_requested_at.unwrap();
        let second = registry
            .request_cancel(session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.cancel_requested_at, Some(stamp));
        assert_eq!(second.state, SessionTaskState::Queued);
    }

    #[tokio::test]
    async fn answering_input_request_resumes_task() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry.create(create_input(session_id)).await.unwrap();

        let task = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "req_1".to_string(),
                        prompt: "Proceed?".to_string(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::AwaitingInput);

        let mut answer = NewTaskMessage::inbound_text("yes");
        answer.in_reply_to = Some("req_1".to_string());
        registry
            .record_message(session_id, &task.id, answer)
            .await
            .unwrap();

        let task = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(task.state, SessionTaskState::Running);
        assert!(task.input_request.is_none());

        let messages = registry
            .list_messages(session_id, &task.id, None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].in_reply_to.as_deref(), Some("req_1"));
    }

    #[tokio::test]
    async fn list_filters_by_kind_and_state() {
        let registry = registry();
        let session_id = SessionId::new();
        let mut a = create_input(session_id);
        a.kind = "subagent".to_string();
        let mut b = create_input(session_id);
        b.kind = "background_tool".to_string();
        b.state = SessionTaskState::Running;
        registry.create(a).await.unwrap();
        registry.create(b).await.unwrap();

        let subagents = registry
            .list(
                session_id,
                Some(&SessionTaskFilter {
                    kind: Some("subagent".to_string()),
                    state: None,
                }),
            )
            .await
            .unwrap();
        assert_eq!(subagents.len(), 1);

        let running = registry
            .list(
                session_id,
                Some(&SessionTaskFilter {
                    kind: None,
                    state: Some(SessionTaskState::Running),
                }),
            )
            .await
            .unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].kind, "background_tool");
    }

    #[tokio::test]
    async fn message_limit_returns_most_recent_oldest_first() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry.create(create_input(session_id)).await.unwrap();
        for i in 0..5 {
            registry
                .record_message(
                    session_id,
                    &task.id,
                    NewTaskMessage::inbound_text(format!("m{i}")),
                )
                .await
                .unwrap();
        }
        let messages = registry
            .list_messages(session_id, &task.id, Some(2))
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        let texts: Vec<String> = messages
            .iter()
            .map(|m| everruns_core::session_task::task_message_text(&m.content))
            .collect();
        assert_eq!(texts, vec!["m3", "m4"]);
    }
}
