//! `SessionScheduleStore` over the `worker_*_session_schedule(s)` commands.

use super::{InternalCommandTransport, decode, failure, is_bad_request};
use crate::core::session_schedule::{ScheduleLimitError, SessionSchedule};
use crate::core::session_services::SessionScheduleStore;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{ScheduleId, SessionId};
use serde_json::{Value, json};

/// The session schedule store the runtime's scheduling tools use, on either
/// transport.
pub struct CommandSessionScheduleStore<T> {
    transport: T,
}

impl<T: InternalCommandTransport> CommandSessionScheduleStore<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    async fn call<R: serde::de::DeserializeOwned>(
        &self,
        operation: &str,
        name: &str,
        params: Value,
    ) -> Result<R> {
        super::call(&self.transport, operation, name, params).await
    }
}

#[async_trait]
impl<T: InternalCommandTransport> SessionScheduleStore for CommandSessionScheduleStore<T> {
    /// The command always enforces create-time limits; nothing creates a
    /// schedule on a worker's behalf without them.
    async fn create_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> Result<SessionSchedule> {
        self.create_schedule_enforcing_limits(
            session_id,
            description,
            cron_expression,
            scheduled_at,
            timezone,
        )
        .await
        .map_err(|error| match error {
            ScheduleLimitError::Rejected(message) => AgentLoopError::store(message),
            ScheduleLimitError::Store(error) => error,
        })
    }

    async fn create_schedule_enforcing_limits(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> std::result::Result<SessionSchedule, ScheduleLimitError> {
        const OPERATION: &str = "Create schedule";
        let outcome = self
            .transport
            .execute_internal_command(
                "worker_create_session_schedule",
                json!({
                    "session_id": session_id.to_string(),
                    "description": description,
                    "cron_expression": cron_expression,
                    "scheduled_at": scheduled_at,
                    "timezone": timezone,
                }),
            )
            .await
            .map_err(ScheduleLimitError::Store)?;
        match outcome {
            Ok(value) => decode(OPERATION, value).map_err(ScheduleLimitError::Store),
            // A limit or interval rejection: the tool reports it to the model.
            Err(error) if is_bad_request(&error) => {
                Err(ScheduleLimitError::Rejected(error.message))
            }
            Err(error) => Err(ScheduleLimitError::Store(failure(OPERATION, error))),
        }
    }

    async fn cancel_schedule(
        &self,
        session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> Result<SessionSchedule> {
        self.call(
            "Cancel schedule",
            "worker_cancel_session_schedule",
            json!({
                "session_id": session_id.to_string(),
                "schedule_id": schedule_id.to_string(),
            }),
        )
        .await
    }

    async fn list_schedules(&self, session_id: SessionId) -> Result<Vec<SessionSchedule>> {
        self.call(
            "List schedules",
            "worker_list_session_schedules",
            json!({ "session_id": session_id.to_string() }),
        )
        .await
    }

    async fn count_active_schedules(&self, session_id: SessionId) -> Result<u32> {
        self.call(
            "Count active schedules",
            "worker_count_active_session_schedules",
            json!({ "session_id": session_id.to_string() }),
        )
        .await
    }

    async fn count_active_org_schedules(&self) -> Result<u32> {
        self.call(
            "Count active org schedules",
            "worker_count_active_org_session_schedules",
            json!({}),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_internal_protocol::proto;
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

    fn error(kind: proto::command_error::Kind, message: &str) -> proto::CommandError {
        proto::CommandError {
            kind: kind as i32,
            message: message.to_string(),
        }
    }

    fn recorded(answer: std::result::Result<Value, proto::CommandError>) -> Recorded {
        Recorded {
            calls: Mutex::new(Vec::new()),
            answer,
        }
    }

    #[tokio::test]
    async fn a_bad_request_is_a_limit_rejection_and_anything_else_a_store_failure() {
        let session = SessionId::new();
        let rejected = recorded(Err(error(
            proto::command_error::Kind::BadRequest,
            "Maximum 5 active schedules per session.",
        )));
        let result = CommandSessionScheduleStore::new(&rejected)
            .create_schedule_enforcing_limits(session, "x".into(), None, None, "UTC".into())
            .await;
        assert!(
            matches!(&result, Err(ScheduleLimitError::Rejected(message)) if message.starts_with("Maximum 5"))
        );
        let (name, params) = rejected.calls.lock().unwrap().remove(0);
        assert_eq!(name, "worker_create_session_schedule");
        assert_eq!(params["session_id"], session.to_string());

        let broken = recorded(Err(error(
            proto::command_error::Kind::Internal,
            "Internal server error",
        )));
        let result = CommandSessionScheduleStore::new(&broken)
            .create_schedule_enforcing_limits(session, "x".into(), None, None, "UTC".into())
            .await;
        assert!(matches!(result, Err(ScheduleLimitError::Store(_))));
    }

    #[tokio::test]
    async fn counts_decode_and_failures_name_the_operation() {
        let counted = recorded(Ok(json!(3)));
        let store = CommandSessionScheduleStore::new(&counted);
        assert_eq!(store.count_active_org_schedules().await.unwrap(), 3);
        assert_eq!(
            counted.calls.lock().unwrap()[0].0,
            "worker_count_active_org_session_schedules"
        );

        let missing = recorded(Err(error(proto::command_error::Kind::NotFound, "Session")));
        let error = CommandSessionScheduleStore::new(&missing)
            .list_schedules(SessionId::new())
            .await
            .expect_err("not found");
        assert!(
            error.to_string().contains("List schedules: Session"),
            "{error}"
        );
    }
}
