//! `SessionTaskRegistry` over the `worker_*_session_task(s)` and
//! `worker_*_session_task_message(s)` commands.
//!
//! The registry is built per org: every command first checks the session
//! belongs to it. The reaper's cross-org orphan scan and retention prune stay
//! dedicated RPCs (`GrpcClient::list_orphaned_session_tasks` and
//! `prune_terminal_session_tasks`); the scan returns each task's org, and the
//! reaper reconciles the task through that org's registry.
//!
//! Decision: tasks cross as the core structs' JSON. The server sends `spec`
//! unredacted (`WorkerSessionTask`), so executors still see push-config
//! secrets and the origin event context.

use super::{InternalCommandTransport, call};
use crate::core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskUpdate, TaskMessage,
};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use serde_json::json;

/// The session task registry tools and executors use, on either transport.
pub struct CommandSessionTaskRegistry<T> {
    transport: T,
}

impl<T: InternalCommandTransport> CommandSessionTaskRegistry<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

fn task(session_id: SessionId, task_id: &str) -> serde_json::Value {
    json!({ "session_id": session_id.to_string(), "task_id": task_id })
}

#[async_trait]
impl<T: InternalCommandTransport> SessionTaskRegistry for CommandSessionTaskRegistry<T> {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        call(
            &self.transport,
            "Create session task",
            "worker_create_session_task",
            json!({ "session_id": input.session_id.to_string(), "task": input }),
        )
        .await
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        call(
            &self.transport,
            "Update session task",
            "worker_update_session_task",
            json!({
                "session_id": session_id.to_string(),
                "task_id": task_id,
                "update": update,
            }),
        )
        .await
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        call(
            &self.transport,
            "Get session task",
            "worker_get_session_task",
            task(session_id, task_id),
        )
        .await
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        call(
            &self.transport,
            "List session tasks",
            "worker_list_session_tasks",
            json!({
                "session_id": session_id.to_string(),
                "kind": filter.and_then(|f| f.kind.clone()),
                "state": filter.and_then(|f| f.state),
            }),
        )
        .await
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        call(
            &self.transport,
            "Request session task cancel",
            "worker_request_cancel_session_task",
            task(session_id, task_id),
        )
        .await
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        call(
            &self.transport,
            "Record session task message",
            "worker_record_session_task_message",
            json!({
                "session_id": session_id.to_string(),
                "task_id": task_id,
                "message": message,
            }),
        )
        .await
    }

    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        call(
            &self.transport,
            "List session task messages",
            "worker_list_session_task_messages",
            json!({
                "session_id": session_id.to_string(),
                "task_id": task_id,
                "limit": limit,
                "after_id": after_id,
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session_task::{SessionTaskState, TaskMessageDirection};
    use everruns_internal_protocol::proto;
    use serde_json::Value;
    use std::sync::Mutex;

    /// Records each call and answers with a canned result.
    struct Recorded {
        calls: Mutex<Vec<(String, Value)>>,
        answer: std::result::Result<Value, proto::CommandError>,
    }

    #[async_trait]
    impl InternalCommandTransport for &Recorded {
        async fn execute_internal_command(
            &self,
            name: &str,
            params: Value,
        ) -> Result<std::result::Result<Value, proto::CommandError>> {
            self.calls.lock().unwrap().push((name.to_string(), params));
            Ok(self.answer.clone())
        }
    }

    fn recorded(answer: std::result::Result<Value, proto::CommandError>) -> Recorded {
        Recorded {
            calls: Mutex::new(Vec::new()),
            answer,
        }
    }

    fn stored_task(session: SessionId) -> Value {
        json!({
            "id": "task_1",
            "session_id": session.to_string(),
            "kind": "background_tool",
            "display_name": "Build",
            "spec": { "push_configs": [{ "url": "https://h.example", "secret": "s" }] },
            "state": "running",
            "attempt": 2,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
        })
    }

    #[tokio::test]
    async fn the_task_travels_as_params_and_decodes_back() {
        let session = SessionId::new();
        let answered = recorded(Ok(stored_task(session)));
        let registry = CommandSessionTaskRegistry::new(&answered);
        let created = registry
            .create(CreateSessionTask {
                session_id: session,
                id: Some("task_1".into()),
                kind: "background_tool".into(),
                display_name: "Build".into(),
                spec: json!({ "tool": "build" }),
                state: SessionTaskState::Queued,
                links: Default::default(),
                wake_policy: Default::default(),
            })
            .await
            .unwrap();
        assert_eq!(created.id, "task_1");
        assert_eq!(created.state, SessionTaskState::Running);
        assert_eq!(created.attempt, 2);
        assert_eq!(created.spec["push_configs"][0]["secret"], "s");

        let calls = answered.calls.lock().unwrap();
        let (name, params) = &calls[0];
        assert_eq!(name, "worker_create_session_task");
        assert_eq!(params["session_id"], session.to_string());
        assert_eq!(params["task"]["session_id"], session.to_string());
        assert_eq!(params["task"]["spec"]["tool"], "build");
    }

    #[tokio::test]
    async fn updates_filters_and_cursors_reach_the_command() {
        let session = SessionId::new();
        let answered = recorded(Ok(Value::Null));
        let registry = CommandSessionTaskRegistry::new(&answered);
        let update = SessionTaskUpdate {
            state: Some(SessionTaskState::Failed),
            expected_attempt: Some(2),
            increment_attempt: true,
            ..Default::default()
        };
        assert!(
            registry
                .update(session, "task_1", update)
                .await
                .unwrap()
                .is_none()
        );
        assert!(registry.get(session, "task_1").await.unwrap().is_none());
        assert!(
            registry
                .request_cancel(session, "task_1")
                .await
                .unwrap()
                .is_none()
        );

        {
            let calls = answered.calls.lock().unwrap();
            assert_eq!(calls[0].0, "worker_update_session_task");
            assert_eq!(calls[0].1["update"]["state"], "failed");
            assert_eq!(calls[0].1["update"]["expected_attempt"], 2);
            assert_eq!(calls[0].1["update"]["increment_attempt"], true);
            assert_eq!(calls[1].0, "worker_get_session_task");
            assert_eq!(calls[2].0, "worker_request_cancel_session_task");
            assert_eq!(calls[2].1["task_id"], "task_1");
        }

        let listed = recorded(Ok(json!([])));
        let registry = CommandSessionTaskRegistry::new(&listed);
        let filter = SessionTaskFilter {
            kind: Some("monitor".into()),
            state: Some(SessionTaskState::AwaitingInput),
        };
        assert!(
            registry
                .list(session, Some(&filter))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            registry
                .list_messages(session, "task_1", Some(5), Some("tmsg_1"))
                .await
                .unwrap()
                .is_empty()
        );
        let calls = listed.calls.lock().unwrap();
        assert_eq!(calls[0].1["kind"], "monitor");
        assert_eq!(calls[0].1["state"], "awaiting_input");
        assert_eq!(calls[1].0, "worker_list_session_task_messages");
        assert_eq!(calls[1].1["limit"], 5);
        assert_eq!(calls[1].1["after_id"], "tmsg_1");
    }

    #[tokio::test]
    async fn messages_decode_and_failures_name_the_operation() {
        let session = SessionId::new();
        let answered = recorded(Ok(json!({
            "id": "tmsg_1",
            "task_id": "task_1",
            "direction": "outbound",
            "content": [{ "type": "text", "text": "done" }],
            "created_at": "2026-01-01T00:00:00Z",
        })));
        let message = CommandSessionTaskRegistry::new(&answered)
            .record_message(
                session,
                "task_1",
                NewTaskMessage::outbound_text("done").with_expected_attempt(3),
            )
            .await
            .unwrap();
        assert_eq!(message.direction, TaskMessageDirection::Outbound);
        {
            let calls = answered.calls.lock().unwrap();
            assert_eq!(calls[0].1["message"]["direction"], "outbound");
            assert_eq!(calls[0].1["message"]["expected_attempt"], 3);
        }

        let missing = recorded(Err(proto::CommandError {
            kind: proto::command_error::Kind::NotFound as i32,
            message: "Session task".to_string(),
        }));
        let error = CommandSessionTaskRegistry::new(&missing)
            .record_message(session, "task_x", NewTaskMessage::inbound_text("hi"))
            .await
            .expect_err("not found");
        assert!(
            error
                .to_string()
                .contains("Record session task message: Session task"),
            "{error}"
        );
    }
}
