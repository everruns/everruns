//! Platform capability and base-URL lookup.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_platform_list_capabilities(
        &self,
        request: Request<PlatformListCapabilitiesRequest>,
    ) -> Result<Response<PlatformListCapabilitiesResponse>, Status> {
        let req = request.into_inner();
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            req.org_id,
            &everruns_platform::FeatureFlags::current(),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, org_id = req.org_id, "Failed to resolve Platform capability feature flags");
            Status::internal("Failed to resolve organization feature flags")
        })?;
        let mut capabilities = self
            .capability_service
            .list_all(req.org_id)
            .await
            .map_err(|e| internal_status("Failed to list capabilities", e))?;
        capabilities
            .retain(|capability| feature_flags.is_capability_enabled(capability.id.as_str()));

        // Apply search filter if provided
        let filtered: Vec<_> = if let Some(ref q) = req.search {
            capabilities
                .into_iter()
                .filter(|c| c.matches_search(q))
                .collect()
        } else {
            capabilities
        };

        let proto_caps = filtered
            .iter()
            .map(|c| PlatformCapabilityInfo {
                id: c.id.as_str().to_string(),
                name: c.name.clone(),
                description: c.description.clone(),
                status: c.status.to_string(),
                category: c.category.clone(),
                icon: c.icon.clone(),
                is_mcp: c.is_mcp,
                is_skill: c.is_skill,
                is_guardrail: c.is_guardrail,
                tool_count: c.tool_definitions.len() as u32,
                tool_names: c
                    .tool_definitions
                    .iter()
                    .map(|t| t.name().to_string())
                    .collect(),
                dependencies: c.dependencies.clone(),
            })
            .collect();

        Ok(Response::new(PlatformListCapabilitiesResponse {
            capabilities: proto_caps,
        }))
    }

    pub(crate) async fn handle_platform_get_base_url(
        &self,
        _request: Request<PlatformGetBaseUrlRequest>,
    ) -> Result<Response<PlatformGetBaseUrlResponse>, Status> {
        let base_url = everruns_core::config::env_string_any(
            &["PUBLIC_APP_URL", "FRONTEND_URL", "APP_URL"],
            "http://localhost:9300",
        );

        Ok(Response::new(PlatformGetBaseUrlResponse { base_url }))
    }
}
