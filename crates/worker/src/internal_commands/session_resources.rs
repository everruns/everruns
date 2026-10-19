//! `SessionResourceRegistry` over the `worker_*_session_resource(s)` commands.

use super::{InternalCommandTransport, call};
use crate::core::session_services::SessionResourceRegistry;
use crate::core::{
    RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use serde_json::json;

/// The session resource registry runtime capabilities use, on either transport.
pub struct CommandSessionResourceRegistry<T> {
    transport: T,
}

impl<T: InternalCommandTransport> CommandSessionResourceRegistry<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl<T: InternalCommandTransport> SessionResourceRegistry for CommandSessionResourceRegistry<T> {
    async fn register(&self, entry: RegisterSessionResource) -> Result<SessionResourceEntry> {
        call(
            &self.transport,
            "Register session resource",
            "worker_register_session_resource",
            json!({
                "session_id": entry.session_id.to_string(),
                "resource_id": entry.resource_id,
                "kind": entry.kind,
                "display_name": entry.display_name,
                "status": entry.status,
                "metadata": entry.metadata,
            }),
        )
        .await
    }

    async fn update_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: SessionResourceStatus,
    ) -> Result<Option<SessionResourceEntry>> {
        call(
            &self.transport,
            "Update session resource status",
            "worker_update_session_resource_status",
            json!({
                "session_id": session_id.to_string(),
                "resource_id": resource_id,
                "status": status,
            }),
        )
        .await
    }

    /// A filtered list, as it was over the RPCs: no command reads one entry.
    async fn get(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<SessionResourceEntry>> {
        let entries = self.list(session_id, None).await?;
        Ok(entries
            .into_iter()
            .find(|entry| entry.resource_id == resource_id))
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionResourceFilter>,
    ) -> Result<Vec<SessionResourceEntry>> {
        call(
            &self.transport,
            "List session resources",
            "worker_list_session_resources",
            json!({
                "session_id": session_id.to_string(),
                "kind": filter.and_then(|filter| filter.kind.clone()),
                "status": filter.and_then(|filter| filter.status),
            }),
        )
        .await
    }

    async fn deregister(&self, session_id: SessionId, resource_id: &str) -> Result<bool> {
        call(
            &self.transport,
            "Deregister session resource",
            "worker_deregister_session_resource",
            json!({
                "session_id": session_id.to_string(),
                "resource_id": resource_id,
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn entry(session: SessionId, resource_id: &str) -> Value {
        json!({
            "resource_id": resource_id,
            "session_id": session.to_string(),
            "kind": "sandbox",
            "display_name": "Sandbox",
            "status": "active",
            "metadata": {},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        })
    }

    #[tokio::test]
    async fn get_is_a_list_and_the_filter_travels_as_params() {
        let session = SessionId::new();
        let listed = recorded(Ok(json!([entry(session, "a"), entry(session, "b")])));
        let registry = CommandSessionResourceRegistry::new(&listed);
        let found = registry.get(session, "b").await.unwrap();
        assert_eq!(found.map(|entry| entry.resource_id).as_deref(), Some("b"));
        assert!(registry.get(session, "c").await.unwrap().is_none());

        registry
            .list(
                session,
                Some(&SessionResourceFilter {
                    kind: Some("sandbox".into()),
                    status: Some(SessionResourceStatus::Released),
                }),
            )
            .await
            .unwrap();
        let calls = listed.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .all(|(name, _)| name == "worker_list_session_resources")
        );
        assert_eq!(calls[0].1["session_id"], session.to_string());
        assert_eq!(calls[2].1["kind"], "sandbox");
        assert_eq!(calls[2].1["status"], "released");
    }

    #[tokio::test]
    async fn an_absent_resource_decodes_and_failures_name_the_operation() {
        let session = SessionId::new();
        let absent = recorded(Ok(Value::Null));
        let updated = CommandSessionResourceRegistry::new(&absent)
            .update_status(session, "gone", SessionResourceStatus::Failed)
            .await
            .unwrap();
        assert!(updated.is_none());
        assert_eq!(absent.calls.lock().unwrap()[0].1["status"], "failed");

        let missing = recorded(Err(proto::CommandError {
            kind: proto::command_error::Kind::NotFound as i32,
            message: "Session".to_string(),
        }));
        let error = CommandSessionResourceRegistry::new(&missing)
            .deregister(session, "x")
            .await
            .expect_err("not found");
        assert!(
            error
                .to_string()
                .contains("Deregister session resource: Session"),
            "{error}"
        );
    }
}
