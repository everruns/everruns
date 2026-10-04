//! The worker's `SessionStorageStore`, spoken over the gRPC control plane.
//!
//! Split out of `grpc_adapters.rs` to keep that file under the size guard. The
//! implementation is unchanged by the move.

use crate::core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_internal_protocol::proto;

use super::{GrpcAdapter, grpc_status_to_error, proto_timestamp_or_now, uuid_to_proto};

#[async_trait]
impl SessionStorageStore for GrpcAdapter {
    async fn set_value(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageSetValueRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            key: key.to_string(),
            value: value.to_string(),
        };
        client
            .session_storage_set_value(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }

    async fn get_value(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
        key: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageGetValueRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            key: key.to_string(),
        };
        let response = client
            .session_storage_get_value(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().value)
    }

    async fn delete_value(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
        key: &str,
    ) -> Result<bool> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageDeleteValueRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            key: key.to_string(),
        };
        let response = client
            .session_storage_delete_value(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().deleted)
    }

    async fn take_value(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
        key: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageTakeValueRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            key: key.to_string(),
        };
        let response = client
            .session_storage_take_value(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().value)
    }

    async fn list_keys(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Result<Vec<KeyInfo>> {
        let mut client = self.client.inner.client();
        let request = proto::SessionStorageListKeysRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
        };
        let response = client
            .session_storage_list_keys(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response
            .into_inner()
            .keys
            .into_iter()
            .map(|k| KeyInfo {
                key: k.key,
                created_at: proto_timestamp_or_now(k.created_at.as_ref()),
                updated_at: proto_timestamp_or_now(k.updated_at.as_ref()),
            })
            .collect())
    }

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
        // The one secret read that goes through the `session_storage` domain
        // command instead of a bespoke RPC, so ownership verification, the
        // reserved-name rule and decryption live in one place rather than
        // being re-implemented server-side per transport. The value still
        // crosses the wire in plaintext, exactly as the RPC sent it; see
        // knowledge/security/session-secret-reads.md for what that does and
        // does not protect.
        let result = self
            .execute_session_command(
                "Session secrets",
                "get_session_secret",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "name": name,
                }),
                None,
            )
            .await?;

        match result {
            Ok(value) => serde_json::from_value(value).map_err(|error| {
                everruns_contracts::error::AgentLoopError::store(format!(
                    "get_session_secret returned unexpected shape: {error}"
                ))
            }),
            // A session or secret that is not there reads as absent, which is
            // what every caller of this trait method already expects.
            Err(error)
                if proto::command_error::Kind::try_from(error.kind)
                    == Ok(proto::command_error::Kind::NotFound) =>
            {
                Ok(None)
            }
            Err(error) => Err(everruns_contracts::error::AgentLoopError::store(format!(
                "read secret: {}",
                error.message
            ))),
        }
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
