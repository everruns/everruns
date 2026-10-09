//! Leased resource cleanup.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.
//!
//! Decision: only the cleanup sweeper's operations remain RPCs. The sweeper
//! claims due leases across every org and settles each by resource id, outside
//! any one org's turn, so there is no org for an internal command to run as.
//! The tool-side upsert, release and list are internal commands
//! (`domains/session_resources/commands/leased`).

use crate::worker_link::grpc_service::*;

impl WorkerServiceImpl {
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
            .collect::<everruns_contracts::error::Result<Vec<_>>>()
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
                everruns_contracts::typed_id::LeasedResourceId::from_uuid(resource_id),
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
                everruns_contracts::typed_id::LeasedResourceId::from_uuid(resource_id),
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
