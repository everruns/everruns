//! `LeasedResourceStore` over the `worker_*_leased_resource(s)` commands.
//!
//! Only the tool-side store moved. The cleanup sweeper's claim and settle calls
//! stay dedicated RPCs (`GrpcClient::claim_due_leased_resources` and friends):
//! they work across every org by resource id, so there is no org to run as.

use super::{InternalCommandTransport, call};
use crate::core::leased_resource::{LeasedResource, UpsertLeasedResource};
use crate::core::session_services::LeasedResourceStore;
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use serde_json::json;

/// The leased resource store tools use, on either transport.
pub struct CommandLeasedResourceStore<T> {
    transport: T,
}

impl<T: InternalCommandTransport> CommandLeasedResourceStore<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl<T: InternalCommandTransport> LeasedResourceStore for CommandLeasedResourceStore<T> {
    async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource> {
        call(
            &self.transport,
            "Upsert leased resource",
            "worker_upsert_leased_resource",
            json!({
                "session_id": input.session_id.to_string(),
                "provider": input.provider,
                "resource_type": input.resource_type,
                "external_id": input.external_id,
                "display_name": input.display_name,
                "owner_user_id": input.owner_user_id,
                "connection_id": input.connection_id,
                "lease_duration_seconds": input.lease_duration_seconds,
                "metadata": input.metadata,
            }),
        )
        .await
    }

    async fn release_resource(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) -> Result<Option<LeasedResource>> {
        call(
            &self.transport,
            "Release leased resource",
            "worker_release_leased_resource",
            json!({
                "session_id": session_id.to_string(),
                "provider": provider,
                "resource_type": resource_type,
                "external_id": external_id,
            }),
        )
        .await
    }

    async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>> {
        call(
            &self.transport,
            "List leased resources",
            "worker_list_session_leased_resources",
            json!({ "session_id": session_id.to_string() }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::leased_resource::LeasedResourceStatus;
    use everruns_contracts::typed_id::LeasedResourceId;
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

    fn lease(session: SessionId, id: LeasedResourceId) -> Value {
        json!({
            "id": id.to_string(),
            "session_id": session.to_string(),
            "provider": "daytona",
            "resource_type": "sandbox",
            "external_id": "sbx-1",
            "status": "active",
            "lease_duration_seconds": 900,
            "last_touched_at": "2026-01-01T00:00:00Z",
            "lease_expires_at": "2026-01-01T00:15:00Z",
            "cleanup_attempts": 0,
            "metadata": {},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        })
    }

    #[tokio::test]
    async fn the_lease_travels_as_params_and_decodes_back() {
        let session = SessionId::new();
        let id = LeasedResourceId::new();
        let answered = recorded(Ok(lease(session, id)));
        let owner = uuid::Uuid::new_v4();
        let resource = CommandLeasedResourceStore::new(&answered)
            .upsert_resource(UpsertLeasedResource {
                session_id: session,
                provider: "daytona".into(),
                resource_type: "sandbox".into(),
                external_id: "sbx-1".into(),
                display_name: None,
                owner_user_id: Some(owner),
                connection_id: None,
                lease_duration_seconds: 900,
                metadata: json!({ "region": "eu" }),
            })
            .await
            .unwrap();
        assert_eq!(resource.id, id);
        assert_eq!(resource.status, LeasedResourceStatus::Active);

        let calls = answered.calls.lock().unwrap();
        let (name, params) = &calls[0];
        assert_eq!(name, "worker_upsert_leased_resource");
        assert_eq!(params["session_id"], session.to_string());
        assert_eq!(params["owner_user_id"], owner.to_string());
        assert_eq!(params["lease_duration_seconds"], 900);
        assert_eq!(params["metadata"]["region"], "eu");
    }

    #[tokio::test]
    async fn an_absent_lease_decodes_and_failures_name_the_operation() {
        let session = SessionId::new();
        let absent = recorded(Ok(Value::Null));
        let released = CommandLeasedResourceStore::new(&absent)
            .release_resource(session, "daytona", "sandbox", "gone")
            .await
            .unwrap();
        assert!(released.is_none());
        assert_eq!(absent.calls.lock().unwrap()[0].1["external_id"], "gone");

        let missing = recorded(Err(proto::CommandError {
            kind: proto::command_error::Kind::NotFound as i32,
            message: "Session".to_string(),
        }));
        let error = CommandLeasedResourceStore::new(&missing)
            .list_resources(session)
            .await
            .expect_err("not found");
        assert!(
            error.to_string().contains("List leased resources: Session"),
            "{error}"
        );
    }
}
