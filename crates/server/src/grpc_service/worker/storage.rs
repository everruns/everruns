//! Session key/value storage and secrets.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_session_storage_set_value(
        &self,
        request: Request<SessionStorageSetValueRequest>,
    ) -> Result<Response<SessionStorageSetValueResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        store
            .set_value(session_id.into(), &req.key, &req.value)
            .await
            .map_err(|e| {
                tracing::error!("Failed to set storage value: {}", e);
                Status::internal("Failed to set storage value")
            })?;

        Ok(Response::new(SessionStorageSetValueResponse {}))
    }

    pub(crate) async fn handle_session_storage_get_value(
        &self,
        request: Request<SessionStorageGetValueRequest>,
    ) -> Result<Response<SessionStorageGetValueResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let value = store
            .get_value(session_id.into(), &req.key)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get storage value: {}", e);
                Status::internal("Failed to get storage value")
            })?;

        Ok(Response::new(SessionStorageGetValueResponse { value }))
    }

    pub(crate) async fn handle_session_storage_delete_value(
        &self,
        request: Request<SessionStorageDeleteValueRequest>,
    ) -> Result<Response<SessionStorageDeleteValueResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let deleted = store
            .delete_value(session_id.into(), &req.key)
            .await
            .map_err(|e| {
                tracing::error!("Failed to delete storage value: {}", e);
                Status::internal("Failed to delete storage value")
            })?;

        Ok(Response::new(SessionStorageDeleteValueResponse { deleted }))
    }

    pub(crate) async fn handle_session_storage_list_keys(
        &self,
        request: Request<SessionStorageListKeysRequest>,
    ) -> Result<Response<SessionStorageListKeysResponse>, Status> {
        use everruns_internal_protocol::datetime_to_proto_timestamp;

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let keys = store.list_keys(session_id.into()).await.map_err(|e| {
            tracing::error!("Failed to list storage keys: {}", e);
            Status::internal("Failed to list storage keys")
        })?;

        let proto_keys = keys
            .into_iter()
            .map(|k| proto::StorageKeyInfo {
                key: k.key,
                created_at: Some(datetime_to_proto_timestamp(k.created_at)),
                updated_at: Some(datetime_to_proto_timestamp(k.updated_at)),
            })
            .collect();

        Ok(Response::new(SessionStorageListKeysResponse {
            keys: proto_keys,
        }))
    }

    pub(crate) async fn handle_session_storage_set_secret(
        &self,
        request: Request<SessionStorageSetSecretRequest>,
    ) -> Result<Response<SessionStorageSetSecretResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        store
            .set_secret(session_id.into(), &req.name, &req.value)
            .await
            .map_err(|e| {
                tracing::error!("Failed to set secret: {}", e);
                Status::internal("Failed to set secret")
            })?;

        Ok(Response::new(SessionStorageSetSecretResponse {}))
    }

    pub(crate) async fn handle_session_storage_get_secret(
        &self,
        request: Request<SessionStorageGetSecretRequest>,
    ) -> Result<Response<SessionStorageGetSecretResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let value = store
            .get_secret(session_id.into(), &req.name)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get secret: {}", e);
                Status::internal("Failed to get secret")
            })?;

        Ok(Response::new(SessionStorageGetSecretResponse { value }))
    }

    pub(crate) async fn handle_session_storage_delete_secret(
        &self,
        request: Request<SessionStorageDeleteSecretRequest>,
    ) -> Result<Response<SessionStorageDeleteSecretResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let deleted = store
            .delete_secret(session_id.into(), &req.name)
            .await
            .map_err(|e| {
                tracing::error!("Failed to delete secret: {}", e);
                Status::internal("Failed to delete secret")
            })?;

        Ok(Response::new(SessionStorageDeleteSecretResponse {
            deleted,
        }))
    }

    pub(crate) async fn handle_session_storage_list_secrets(
        &self,
        request: Request<SessionStorageListSecretsRequest>,
    ) -> Result<Response<SessionStorageListSecretsResponse>, Status> {
        use everruns_internal_protocol::datetime_to_proto_timestamp;

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let secrets = store.list_secrets(session_id.into()).await.map_err(|e| {
            tracing::error!("Failed to list secrets: {}", e);
            Status::internal("Failed to list secrets")
        })?;

        let proto_secrets = secrets
            .into_iter()
            .map(|s| proto::StorageSecretInfo {
                name: s.name,
                created_at: Some(datetime_to_proto_timestamp(s.created_at)),
                updated_at: Some(datetime_to_proto_timestamp(s.updated_at)),
            })
            .collect();

        Ok(Response::new(SessionStorageListSecretsResponse {
            secrets: proto_secrets,
        }))
    }
}
