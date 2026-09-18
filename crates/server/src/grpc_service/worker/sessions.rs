//! Agent, harness, session lookup and mutation, model resolution.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<GetAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = parse_uuid(req.agent_id.as_ref())?;

        // Get agent with capabilities via domain query
        let public_id = everruns_provider::typed_id::AgentId::from_uuid(agent_id).to_string();
        let agent =
            crate::domains::agents::queries::get_by_public_id(&self.db, req.org_id, &public_id)
                .await
                .map_err(|e| internal_status("Failed to get agent", e))?;

        let proto_agent = agent.map(|a| schema_agent_to_proto(&a));

        Ok(Response::new(GetAgentResponse { agent: proto_agent }))
    }

    pub(crate) async fn handle_get_harness(
        &self,
        request: Request<GetHarnessRequest>,
    ) -> Result<Response<GetHarnessResponse>, Status> {
        let req = request.into_inner();
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        let harness = crate::domains::harnesses::queries::resolve_effective(
            &self.db,
            req.org_id,
            everruns_provider::typed_id::HarnessId::from_uuid(harness_id),
        )
        .await
        .map_err(|e| internal_status("Failed to get harness", e))?;

        let proto_harness = harness.map(|h| schema_harness_to_proto(&h));

        Ok(Response::new(GetHarnessResponse {
            harness: proto_harness,
        }))
    }

    pub(crate) async fn handle_get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        // Get session via SessionService
        let session = self
            .session_service
            .get(&internal_caller, session_id, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                Status::internal("Failed to get session")
            })?;

        use everruns_internal_protocol::schema_session_to_proto;

        let proto_session = session.map(|s| schema_session_to_proto(&s));

        Ok(Response::new(GetSessionResponse {
            session: proto_session,
        }))
    }

    pub(crate) async fn handle_set_session_status(
        &self,
        request: Request<SetSessionStatusRequest>,
    ) -> Result<Response<SetSessionStatusResponse>, Status> {
        use everruns_internal_protocol::schema_session_to_proto;

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        // Validate status value
        let valid_statuses = ["started", "active", "idle", "waiting_for_tool_results"];
        if !valid_statuses.contains(&req.status.as_str()) {
            return Err(Status::invalid_argument(format!(
                "Invalid status '{}'. Must be one of: started, active, idle, waiting_for_tool_results",
                req.status
            )));
        }

        let session = self
            .session_service
            .update_status(&internal_caller, session_id, req.status)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session status: {}", e);
                Status::internal("Failed to update session status")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        let proto_session = schema_session_to_proto(&session);

        Ok(Response::new(SetSessionStatusResponse {
            session: Some(proto_session),
        }))
    }

    pub(crate) async fn handle_set_session_title(
        &self,
        request: Request<SetSessionTitleRequest>,
    ) -> Result<Response<SetSessionTitleResponse>, Status> {
        use everruns_internal_protocol::schema_session_to_proto;

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        let session = self
            .session_service
            .update(
                &internal_caller,
                session_id,
                crate::api::sessions::UpdateSessionRequest {
                    title: Some(req.title),
                    goal: None,
                    agent_identity_id: everruns_durable::UpdateField::Unchanged,
                    locale: None,
                    tags: None,
                },
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session title: {}", e);
                Status::internal("Failed to update session title")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        let proto_session = schema_session_to_proto(&session);

        Ok(Response::new(SetSessionTitleResponse {
            session: Some(proto_session),
        }))
    }

    pub(crate) async fn handle_get_resolved_model(
        &self,
        request: Request<GetResolvedModelRequest>,
    ) -> Result<Response<GetResolvedModelResponse>, Status> {
        let req = request.into_inner();
        let model_id = parse_uuid(req.model_id.as_ref())?;

        // Resolve model via ProviderResolverService
        let resolved = self
            .provider_resolver_service
            .resolve_model(req.org_id, model_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve model: {}", e);
                Status::internal("Failed to resolve model")
            })?;

        Ok(Response::new(GetResolvedModelResponse {
            model: resolved.map(Self::resolved_model_to_proto),
        }))
    }

    pub(crate) async fn handle_get_default_model(
        &self,
        request: Request<GetDefaultModelRequest>,
    ) -> Result<Response<GetDefaultModelResponse>, Status> {
        let req = request.into_inner();
        // Resolve default model via ProviderResolverService
        let resolved = self
            .provider_resolver_service
            .resolve_default_model(req.org_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve default model: {}", e);
                Status::internal("Failed to resolve default model")
            })?;

        // Model resolution is credential-free; provider config resolves later.
        if let Some(ref model) = resolved {
            tracing::debug!(
                model_id = %model.model_id,
                provider_type = %model.provider_type,
                provider_id = %model.provider_id,
                "gRPC get_default_model: resolved model"
            );
        } else {
            tracing::debug!("gRPC get_default_model: no default model configured");
        }

        Ok(Response::new(GetDefaultModelResponse {
            model: resolved.map(Self::resolved_model_to_proto),
        }))
    }
}
