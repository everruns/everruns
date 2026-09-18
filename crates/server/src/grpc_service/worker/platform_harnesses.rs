//! Platform harness management.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_platform_list_harnesses(
        &self,
        request: Request<PlatformListHarnessesRequest>,
    ) -> Result<Response<PlatformListHarnessesResponse>, Status> {
        let req = request.into_inner();
        let rows = self
            .db
            .list_harnesses(req.org_id, None, false)
            .await
            .map_err(|e| internal_status("Failed to list harnesses", e))?;
        let harnesses = crate::domains::harnesses::queries::load_harnesses_list(&self.db, rows)
            .await
            .map_err(|e| internal_status("Failed to list harnesses", e))?;

        let proto_harnesses = harnesses.iter().map(schema_harness_to_proto).collect();
        Ok(Response::new(PlatformListHarnessesResponse {
            harnesses: proto_harnesses,
        }))
    }

    pub(crate) async fn handle_platform_create_harness(
        &self,
        request: Request<PlatformCreateHarnessRequest>,
    ) -> Result<Response<PlatformCreateHarnessResponse>, Status> {
        let req = request.into_inner();
        let capabilities: Vec<everruns_capability::CapabilityRef> = req
            .capabilities
            .iter()
            .map(|id| everruns_capability::CapabilityRef::new(id.as_str()))
            .collect();

        let create_req = crate::domains::harnesses::types::CreateHarnessRequest {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            // Worker proto carries a plain string; empty/whitespace means no base prompt.
            system_prompt: (!req.system_prompt.trim().is_empty()).then_some(req.system_prompt),
            parent_harness_id: req
                .parent_harness_id
                .as_ref()
                .map(|parent_id| parse_uuid(Some(parent_id)))
                .transpose()?
                .map(everruns_provider::typed_id::HarnessId::from_uuid),
            default_model_id: None,
            tags: vec![],
            capabilities,
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        };

        use crate::domains::common::Command;
        let ctx = self.org_domain_ctx(req.org_id).await?;
        let harness = crate::domains::harnesses::CreateHarness(create_req)
            .run(&ctx)
            .await
            .map_err(|e| internal_status("Failed to create harness", e))?;

        Ok(Response::new(PlatformCreateHarnessResponse {
            harness: Some(schema_harness_to_proto(&harness)),
        }))
    }

    pub(crate) async fn handle_platform_update_harness(
        &self,
        request: Request<PlatformUpdateHarnessRequest>,
    ) -> Result<Response<PlatformUpdateHarnessResponse>, Status> {
        let req = request.into_inner();
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        let update_req = crate::domains::harnesses::types::UpdateHarnessRequest {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            intro_markdown: None,
            short_description: None,
            starters: None,
            system_prompt: req.system_prompt,
            parent_harness_id: if req.clear_parent_harness_id.unwrap_or(false) {
                Some(None)
            } else {
                req.parent_harness_id
                    .as_ref()
                    .map(|parent_id| parse_uuid(Some(parent_id)))
                    .transpose()?
                    .map(everruns_provider::typed_id::HarnessId::from_uuid)
                    .map(Some)
            },
            default_model_id: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            mcp_servers: None,
            network_access: None,
            status: None,
            embedder_metadata: None,
        };

        use crate::domains::common::Command;
        let harness_public_id =
            everruns_provider::typed_id::HarnessId::from_uuid(harness_id).to_string();
        let ctx = self.org_domain_ctx(req.org_id).await?;
        let harness = crate::domains::harnesses::UpdateHarnessCmd {
            id: harness_public_id,
            req: update_req,
        }
        .run(&ctx)
        .await
        .map_err(|e| internal_status("Failed to update harness", e))?;

        Ok(Response::new(PlatformUpdateHarnessResponse {
            harness: Some(schema_harness_to_proto(&harness)),
        }))
    }

    pub(crate) async fn handle_platform_delete_harness(
        &self,
        request: Request<PlatformDeleteHarnessRequest>,
    ) -> Result<Response<PlatformDeleteHarnessResponse>, Status> {
        let req = request.into_inner();
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        use crate::domains::common::Command;
        let harness_public_id =
            everruns_provider::typed_id::HarnessId::from_uuid(harness_id).to_string();
        let ctx = self.org_domain_ctx(req.org_id).await?;
        crate::domains::harnesses::DeleteHarness {
            id: harness_public_id,
        }
        .run(&ctx)
        .await
        .map_err(|e| internal_status("Failed to delete harness", e))?;

        Ok(Response::new(PlatformDeleteHarnessResponse {}))
    }

    pub(crate) async fn handle_platform_copy_harness(
        &self,
        request: Request<PlatformCopyHarnessRequest>,
    ) -> Result<Response<PlatformCopyHarnessResponse>, Status> {
        let req = request.into_inner();
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        use crate::domains::common::Command;
        let harness_public_id =
            everruns_provider::typed_id::HarnessId::from_uuid(harness_id).to_string();
        let ctx = self.org_domain_ctx(req.org_id).await?;
        let harness = crate::domains::harnesses::CopyHarness {
            id: harness_public_id,
        }
        .run(&ctx)
        .await
        .map_err(|e| internal_status("Failed to copy harness", e))?;

        // If a new_name was provided, update the copy with the new name
        let harness = if let Some(new_name) = req.new_name {
            let update_req = crate::domains::harnesses::types::UpdateHarnessRequest {
                name: Some(new_name),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: None,
                system_prompt: None,
                parent_harness_id: None,
                default_model_id: None,
                tags: None,
                capabilities: None,
                initial_files: None,
                mcp_servers: None,
                network_access: None,
                status: None,
                embedder_metadata: None,
            };
            crate::domains::harnesses::UpdateHarnessCmd {
                id: harness.id.to_string(),
                req: update_req,
            }
            .run(&ctx)
            .await
            .map_err(|e| internal_status("Failed to rename copied harness", e))
            .unwrap_or(harness)
        } else {
            harness
        };

        Ok(Response::new(PlatformCopyHarnessResponse {
            harness: Some(schema_harness_to_proto(&harness)),
        }))
    }
}
