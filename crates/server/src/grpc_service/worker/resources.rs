//! Session resource registry.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_register_session_resource(
        &self,
        request: Request<RegisterSessionResourceRequest>,
    ) -> Result<Response<RegisterSessionResourceResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let registry = self.session_resource_registry();

        let entry = registry
            .register(everruns_core::RegisterSessionResource {
                session_id: session_id.into(),
                resource_id: req.resource_id,
                kind: req.kind,
                display_name: req.display_name,
                status: everruns_core::SessionResourceStatus::from(req.status.as_str()),
                metadata: req
                    .metadata
                    .as_ref()
                    .map(everruns_internal_protocol::proto_struct_to_json)
                    .unwrap_or_else(|| serde_json::json!({})),
            })
            .await
            .map_err(|e| {
                tracing::error!("Failed to register session resource: {e}");
                Status::internal("Failed to register session resource")
            })?;

        Ok(Response::new(RegisterSessionResourceResponse {
            entry: Some(session_resource_entry_to_proto(&entry)),
        }))
    }

    pub(crate) async fn handle_update_session_resource_status(
        &self,
        request: Request<UpdateSessionResourceStatusRequest>,
    ) -> Result<Response<UpdateSessionResourceStatusResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let registry = self.session_resource_registry();

        let entry = registry
            .update_status(
                session_id.into(),
                &req.resource_id,
                everruns_core::SessionResourceStatus::from(req.status.as_str()),
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session resource status: {e}");
                Status::internal("Failed to update session resource status")
            })?;

        Ok(Response::new(UpdateSessionResourceStatusResponse {
            entry: entry.as_ref().map(session_resource_entry_to_proto),
        }))
    }

    pub(crate) async fn handle_list_session_resources(
        &self,
        request: Request<ListSessionResourcesRequest>,
    ) -> Result<Response<ListSessionResourcesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let registry = self.session_resource_registry();

        let filter = if req.kind.is_some() || req.status.is_some() {
            Some(everruns_core::SessionResourceFilter {
                kind: req.kind,
                status: req
                    .status
                    .as_deref()
                    .map(everruns_core::SessionResourceStatus::from),
            })
        } else {
            None
        };

        let entries = registry
            .list(session_id.into(), filter.as_ref())
            .await
            .map_err(|e| {
                tracing::error!("Failed to list session resources: {e}");
                Status::internal("Failed to list session resources")
            })?;

        Ok(Response::new(ListSessionResourcesResponse {
            entries: entries
                .iter()
                .map(session_resource_entry_to_proto)
                .collect(),
        }))
    }

    pub(crate) async fn handle_deregister_session_resource(
        &self,
        request: Request<DeregisterSessionResourceRequest>,
    ) -> Result<Response<DeregisterSessionResourceResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let registry = self.session_resource_registry();

        let removed = registry
            .deregister(session_id.into(), &req.resource_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to deregister session resource: {e}");
                Status::internal("Failed to deregister session resource")
            })?;

        Ok(Response::new(DeregisterSessionResourceResponse { removed }))
    }
}
