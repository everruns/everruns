//! Session storage secrets.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.
//!
//! Decision: only the secret operations remain RPCs; they are secret-bearing
//! and move with connections and credentials. Key/value storage is served by
//! internal commands (`domains/session_storage/commands/worker`).

use crate::worker_link::grpc_service::*;

impl WorkerServiceImpl {
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
