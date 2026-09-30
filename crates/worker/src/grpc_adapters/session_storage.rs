//! The worker's `SessionStorageStore`, spoken over the gRPC control plane.
//!
//! Split out of `grpc_adapters.rs` to keep that file under the size guard. The
//! implementation is unchanged by the move.

use async_trait::async_trait;
use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use everruns_internal_protocol::proto;
use everruns_provider::error::Result;

use super::{GrpcAdapter, grpc_status_to_error, proto_timestamp_or_now, uuid_to_proto};

#[async_trait]
impl SessionStorageStore for GrpcAdapter {
    async fn set_value(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let mut client = self.client.inner.lock().await;
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
        session_id: everruns_provider::typed_id::SessionId,
        key: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
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
        session_id: everruns_provider::typed_id::SessionId,
        key: &str,
    ) -> Result<bool> {
        let mut client = self.client.inner.lock().await;
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
        session_id: everruns_provider::typed_id::SessionId,
        key: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
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
        session_id: everruns_provider::typed_id::SessionId,
    ) -> Result<Vec<KeyInfo>> {
        let mut client = self.client.inner.lock().await;
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
        session_id: everruns_provider::typed_id::SessionId,
        name: &str,
        value: &str,
    ) -> Result<()> {
        let mut client = self.client.inner.lock().await;
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
        session_id: everruns_provider::typed_id::SessionId,
        name: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
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
        session_id: everruns_provider::typed_id::SessionId,
        name: &str,
    ) -> Result<bool> {
        let mut client = self.client.inner.lock().await;
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
        session_id: everruns_provider::typed_id::SessionId,
    ) -> Result<Vec<SecretInfo>> {
        let mut client = self.client.inner.lock().await;
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
