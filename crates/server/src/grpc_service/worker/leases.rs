//! Leased resource lifecycle.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_upsert_leased_resource(
        &self,
        request: Request<UpsertLeasedResourceRequest>,
    ) -> Result<Response<UpsertLeasedResourceResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.leased_resource_store();

        let resource = store
            .upsert_resource(everruns_core::UpsertLeasedResource {
                session_id: session_id.into(),
                provider: req.provider,
                resource_type: req.resource_type,
                external_id: req.external_id,
                display_name: req.display_name,
                owner_user_id: req
                    .owner_user_id
                    .as_ref()
                    .map(|id| parse_uuid(Some(id)))
                    .transpose()?,
                lease_duration_seconds: req.lease_duration_seconds,
                metadata: req
                    .metadata
                    .as_ref()
                    .map(everruns_internal_protocol::proto_struct_to_json)
                    .unwrap_or_else(|| serde_json::json!({})),
            })
            .await
            .map_err(|e| {
                tracing::error!("Failed to upsert leased resource: {}", e);
                Status::internal("Failed to upsert leased resource")
            })?;

        Ok(Response::new(UpsertLeasedResourceResponse {
            resource: Some(leased_resource_to_proto(&resource)),
        }))
    }

    pub(crate) async fn handle_release_leased_resource(
        &self,
        request: Request<ReleaseLeasedResourceRequest>,
    ) -> Result<Response<ReleaseLeasedResourceResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.leased_resource_store();

        let resource = store
            .release_resource(
                session_id.into(),
                &req.provider,
                &req.resource_type,
                &req.external_id,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to release leased resource: {}", e);
                Status::internal("Failed to release leased resource")
            })?;

        Ok(Response::new(ReleaseLeasedResourceResponse {
            resource: resource.as_ref().map(leased_resource_to_proto),
        }))
    }

    pub(crate) async fn handle_list_session_leased_resources(
        &self,
        request: Request<ListSessionLeasedResourcesRequest>,
    ) -> Result<Response<ListSessionLeasedResourcesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.leased_resource_store();

        let resources = store.list_resources(session_id.into()).await.map_err(|e| {
            tracing::error!("Failed to list leased resources: {}", e);
            Status::internal("Failed to list leased resources")
        })?;

        Ok(Response::new(ListSessionLeasedResourcesResponse {
            resources: resources.iter().map(leased_resource_to_proto).collect(),
        }))
    }

    pub(crate) async fn handle_claim_due_leased_resources(
        &self,
        request: Request<ClaimDueLeasedResourcesRequest>,
    ) -> Result<Response<ClaimDueLeasedResourcesResponse>, Status> {
        let req = request.into_inner();
        let rows = self
            .db
            .claim_due_leased_resources(req.limit as i32, req.stale_after_seconds as i32)
            .await
            .map_err(|e| {
                tracing::error!("Failed to claim leased resources: {}", e);
                Status::internal("Failed to claim leased resources")
            })?;

        let resources = rows
            .iter()
            .map(crate::storage::leased_resource_row_to_domain)
            .collect::<everruns_provider::error::Result<Vec<_>>>()
            .map_err(|e| {
                tracing::error!("Failed to map leased resources: {}", e);
                Status::internal("Failed to map leased resources")
            })?;

        Ok(Response::new(ClaimDueLeasedResourcesResponse {
            resources: resources.iter().map(leased_resource_to_proto).collect(),
        }))
    }

    pub(crate) async fn handle_mark_leased_resource_released(
        &self,
        request: Request<MarkLeasedResourceReleasedRequest>,
    ) -> Result<Response<MarkLeasedResourceReleasedResponse>, Status> {
        let req = request.into_inner();
        let resource_id = parse_uuid(req.resource_id.as_ref())?;
        let expected_cleanup_started_at = req
            .expected_cleanup_started_at
            .as_ref()
            .map(everruns_internal_protocol::proto_timestamp_to_datetime)
            .ok_or_else(|| Status::invalid_argument("Missing expected_cleanup_started_at"))?;

        let updated = self
            .db
            .mark_leased_resource_released(
                everruns_provider::typed_id::LeasedResourceId::from_uuid(resource_id),
                expected_cleanup_started_at,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to mark leased resource released: {}", e);
                Status::internal("Failed to mark leased resource released")
            })?
            .is_some();

        Ok(Response::new(MarkLeasedResourceReleasedResponse {
            updated,
        }))
    }

    pub(crate) async fn handle_mark_leased_resource_cleanup_failed(
        &self,
        request: Request<MarkLeasedResourceCleanupFailedRequest>,
    ) -> Result<Response<MarkLeasedResourceCleanupFailedResponse>, Status> {
        let req = request.into_inner();
        let resource_id = parse_uuid(req.resource_id.as_ref())?;
        let expected_cleanup_started_at = req
            .expected_cleanup_started_at
            .as_ref()
            .map(everruns_internal_protocol::proto_timestamp_to_datetime)
            .ok_or_else(|| Status::invalid_argument("Missing expected_cleanup_started_at"))?;

        let updated = self
            .db
            .mark_leased_resource_cleanup_failed(
                everruns_provider::typed_id::LeasedResourceId::from_uuid(resource_id),
                expected_cleanup_started_at,
                req.retry_after_seconds as i32,
                &req.error,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to mark leased resource cleanup failed: {}", e);
                Status::internal("Failed to mark leased resource cleanup failed")
            })?
            .is_some();

        Ok(Response::new(MarkLeasedResourceCleanupFailedResponse {
            updated,
        }))
    }
}
