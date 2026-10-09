//! The worker's session secrets, spoken over the gRPC control plane.
//!
// Split out of `grpc_adapters.rs` to keep that file under the size guard.
// Decision: only the secret half of session storage is still an RPC; the
// key/value half runs internal commands (`internal_commands::session_storage`),
// which takes this as its `SessionSecretStorage`.

use crate::core::session_services::SecretInfo;
use crate::internal_commands::SessionSecretStorage;
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_internal_protocol::proto;

use super::{GrpcAdapter, grpc_status_to_error, proto_timestamp_or_now, uuid_to_proto};

#[async_trait]
impl SessionSecretStorage for GrpcAdapter {
    async fn set_secret(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
        name: &str,
        value: &str,
    ) -> Result<()> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageSetSecretRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
            value: value.to_string(),
        };
        client
            .session_storage_set_secret(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }

    async fn get_secret(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
        name: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageGetSecretRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
        };
        let response = client
            .session_storage_get_secret(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().value)
    }

    async fn delete_secret(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
        name: &str,
    ) -> Result<bool> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageDeleteSecretRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
        };
        let response = client
            .session_storage_delete_secret(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().deleted)
    }

    async fn list_secrets(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Result<Vec<SecretInfo>> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageListSecretsRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
        };
        let response = client
            .session_storage_list_secrets(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response
            .into_inner()
            .secrets
            .into_iter()
            .map(|s| SecretInfo {
                name: s.name,
                created_at: proto_timestamp_or_now(s.created_at.as_ref()),
                updated_at: proto_timestamp_or_now(s.updated_at.as_ref()),
            })
            .collect())
    }
}
