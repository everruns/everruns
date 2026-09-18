//! Platform agent management.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_platform_list_agents(
        &self,
        request: Request<PlatformListAgentsRequest>,
    ) -> Result<Response<PlatformListAgentsResponse>, Status> {
        let req = request.into_inner();
        let pagination = crate::api::common::Pagination::new(0, 1000);
        let (rows, _total) = self
            .db
            .list_agents(req.org_id, None, false, pagination)
            .await
            .map_err(|e| internal_status("Failed to list agents", e))?;
        let agents = crate::domains::agents::queries::load_agents_list(&self.db, rows)
            .await
            .map_err(|e| internal_status("Failed to list agents", e))?;

        let proto_agents = agents.iter().map(schema_agent_to_proto).collect();
        Ok(Response::new(PlatformListAgentsResponse {
            agents: proto_agents,
        }))
    }

    pub(crate) async fn handle_platform_create_agent(
        &self,
        request: Request<PlatformCreateAgentRequest>,
    ) -> Result<Response<PlatformCreateAgentResponse>, Status> {
        let req = request.into_inner();
        let capabilities: Vec<everruns_capability::CapabilityRef> = req
            .capabilities
            .iter()
            .map(|id| everruns_capability::CapabilityRef::new(id.as_str()))
            .collect();

        let create_req = crate::domains::agents::types::CreateAgentRequest {
            id: None,
            name: req.name.clone(),
            display_name: req.display_name,
            description: req.description,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: req.system_prompt,
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: vec![],
            capabilities,
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        };

        use crate::domains::common::Command;
        let ctx = self.org_domain_ctx(req.org_id).await?;
        let agent = crate::domains::agents::CreateAgent(create_req)
            .run(&ctx)
            .await
            .map_err(|e| internal_status("Failed to create agent", e))?;

        Ok(Response::new(PlatformCreateAgentResponse {
            agent: Some(schema_agent_to_proto(&agent)),
        }))
    }

    pub(crate) async fn handle_platform_update_agent(
        &self,
        request: Request<PlatformUpdateAgentRequest>,
    ) -> Result<Response<PlatformUpdateAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = parse_uuid(req.agent_id.as_ref())?;

        let public_id = everruns_provider::typed_id::AgentId::from_uuid(agent_id).to_string();

        let update_req = crate::domains::agents::types::UpdateAgentRequest {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            intro_markdown: None,
            short_description: None,
            starters: None,
            system_prompt: req.system_prompt,
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            tools: None,
            mcp_servers: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: None,
        };

        use crate::domains::common::Command;
        let ctx = self.org_domain_ctx(req.org_id).await?;
        let updated = crate::domains::agents::UpdateAgentCmd {
            id: public_id,
            req: update_req,
        }
        .run(&ctx)
        .await
        .map_err(|e| internal_status("Failed to update agent", e))?;

        Ok(Response::new(PlatformUpdateAgentResponse {
            agent: Some(schema_agent_to_proto(&updated)),
        }))
    }

    pub(crate) async fn handle_platform_delete_agent(
        &self,
        request: Request<PlatformDeleteAgentRequest>,
    ) -> Result<Response<PlatformDeleteAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = parse_uuid(req.agent_id.as_ref())?;

        use crate::domains::common::Command;
        let public_id = everruns_provider::typed_id::AgentId::from_uuid(agent_id).to_string();
        let ctx = self.org_domain_ctx(req.org_id).await?;
        crate::domains::agents::DeleteAgent { id: public_id }
            .run(&ctx)
            .await
            .map_err(|e| internal_status("Failed to delete agent", e))?;

        Ok(Response::new(PlatformDeleteAgentResponse {}))
    }
}
