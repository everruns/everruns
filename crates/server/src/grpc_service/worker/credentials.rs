//! Provider credentials and MCP server resolution.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_get_default_provider_credentials(
        &self,
        request: Request<GetDefaultProviderCredentialsRequest>,
    ) -> Result<Response<GetDefaultProviderCredentialsResponse>, Status> {
        let req = request.into_inner();
        if let Some(model_id) = req.decision_model_id.as_deref() {
            let session_id = req
                .session_id
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Session is required"))?;
            let binding = self
                .provider_resolver_service
                .resolve_decision_model(
                    req.org_id,
                    (!model_id.is_empty()).then_some(model_id),
                    parse_uuid(Some(session_id))?,
                )
                .await
                .map_err(|_| Status::failed_precondition("Decision model is unavailable"))?;
            return Ok(Response::new(GetDefaultProviderCredentialsResponse {
                found: binding.is_some(),
                decision_binding_json: binding
                    .map(|b| serde_json::to_string(&b))
                    .transpose()
                    .map_err(|_| Status::internal("Invalid binding"))?
                    .unwrap_or_default(),
                ..Default::default()
            }));
        }

        let resolved = if req.provider_id.is_empty() {
            self.provider_resolver_service
                .resolve_provider_credentials(req.org_id, &req.provider_type)
                .await
                .map(|value| {
                    value.map(|credentials| {
                        (
                            req.provider_type.clone(),
                            Some(credentials.api_key),
                            credentials.base_url,
                            Default::default(),
                        )
                    })
                })
        } else {
            self.provider_resolver_service
                .resolve_runtime_provider_config_for_session(
                    req.org_id,
                    &req.provider_id,
                    req.session_id
                        .as_ref()
                        .map(|id| parse_uuid(Some(id)))
                        .transpose()?,
                )
                .await
                .map(|value| {
                    value.map(|provider| {
                        (
                            provider.provider_type,
                            provider.api_key,
                            provider.base_url,
                            provider.request_options,
                        )
                    })
                })
        }
        .map_err(|e| {
            tracing::error!(
                provider_type = %req.provider_type,
                error = %e,
                "Failed to resolve provider credentials"
            );
            Status::internal("Failed to resolve provider credentials")
        })?;

        Ok(Response::new(match resolved {
            Some((provider_type, api_key, base_url, request_options)) => {
                // Empty when the connection configures nothing, so the worker
                // applies no options rather than an empty JSON object.
                let request_options_json = if request_options.is_empty() {
                    String::new()
                } else {
                    serde_json::to_string(&request_options).unwrap_or_default()
                };
                GetDefaultProviderCredentialsResponse {
                    decision_binding_json: String::new(),
                    found: true,
                    api_key: api_key.unwrap_or_default(),
                    base_url: base_url.unwrap_or_default(),
                    provider_type,
                    request_options_json,
                }
            }
            None => GetDefaultProviderCredentialsResponse {
                decision_binding_json: String::new(),
                found: false,
                api_key: String::new(),
                base_url: String::new(),
                provider_type: String::new(),
                request_options_json: String::new(),
            },
        }))
    }

    pub(crate) async fn handle_get_mcp_server_by_prefix(
        &self,
        request: Request<GetMcpServerByPrefixRequest>,
    ) -> Result<Response<GetMcpServerByPrefixResponse>, Status> {
        let req = request.into_inner();
        let mut runtime_agent_id = None;

        if let Some(session_id) = req.session_id.as_ref() {
            let session_id = parse_uuid(Some(session_id))?;
            let internal_caller = everruns_core::Caller::internal(req.org_id);

            if let Some(mut session) = self
                .session_service
                .get(&internal_caller, session_id, None)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to get session for scoped MCP lookup: {}", e);
                    Status::internal("Failed to resolve scoped MCP server")
                })?
                && let Some(harness) = crate::domains::harnesses::queries::resolve_effective(
                    &self.db,
                    req.org_id,
                    session.harness_id,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to get harness for scoped MCP lookup: {}", e);
                    Status::internal("Failed to resolve scoped MCP server")
                })?
            {
                if let Some(message) = req.input_message_id.as_ref() {
                    let message = parse_uuid(Some(message))?;
                    if !self
                        .db
                        .runtime_invocation_exists(session.id, message)
                        .await
                        .map_err(|_| Status::internal("Invocation unavailable"))?
                    {
                        return Err(Status::permission_denied("Unknown invocation"));
                    }
                    let responder = self
                        .db
                        .runtime_invocation_responder(session.id, message)
                        .await
                        .map_err(|_| Status::internal("Invocation unavailable"))?
                        .map(everruns_contracts::typed_id::AgentId::from_uuid);

                    if session.agent_id != responder {
                        session.agent_version_id = None;
                    }

                    session.agent_id = responder;
                }
                runtime_agent_id = session.agent_id;
                let mut agent = if let Some(agent_id) = session.agent_id {
                    crate::domains::agents::queries::get_by_public_id(
                        &self.db,
                        req.org_id,
                        &agent_id.to_string(),
                    )
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to get agent for scoped MCP lookup: {}", e);
                        Status::internal("Failed to resolve scoped MCP server")
                    })?
                } else {
                    None
                };
                if let (Some(agent), Some(version_id)) = (agent.as_mut(), session.agent_version_id)
                    && let Some(version_row) = self
                        .db
                        .get_agent_version(req.org_id, version_id)
                        .await
                        .map_err(|e| {
                            tracing::error!(
                                "Failed to get agent version for scoped MCP lookup: {}",
                                e
                            );
                            Status::internal("Failed to resolve scoped MCP server")
                        })?
                {
                    let version =
                        crate::domains::agents::queries::row_to_agent_version(version_row);
                    *agent = crate::domains::agents::queries::version_to_agent(agent, &version);
                }

                if let Some(r) = crate::domains::mcp_servers::scoped_mcp::resolve_scoped_mcp_server_with_capabilities(
                    &self.mcp_server_service,
                    req.org_id,
                    &harness,
                    agent.as_ref(),
                    &session,
                    &req.server_prefix,
                    self.capability_service.registry(),
                )
                .await
                .map_err(|error| {
                    tracing::error!(%error, "Failed to resolve scoped MCP server");
                    Status::internal("Failed to resolve scoped MCP server")
                })?
                {
                    let secret_bindings = crate::domains::agents::credentials::resolve_runtime_secret_bindings(
                        self.db.as_ref(),
                        self.encryption.as_deref(),
                        req.org_id,
                        runtime_agent_id,
                        &r.name,
                        &r.url,
                    )
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, "Failed to resolve Agent MCP credentials");
                        Status::internal("Failed to resolve MCP credentials")
                    })?;
                    return Ok(Response::new(GetMcpServerByPrefixResponse {
                        server: Some(resolved_mcp_server_to_proto(r, secret_bindings)),
                    }));
                }
            }
        }

        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let resolved = self
            .mcp_server_service
            .resolve_by_prefix(&internal_caller, &req.server_prefix)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve MCP server: {}", e);
                Status::internal("Failed to resolve MCP server")
            })?;

        let server_info = if let Some(r) = resolved {
            let secret_bindings =
                crate::domains::agents::credentials::resolve_runtime_secret_bindings(
                    self.db.as_ref(),
                    self.encryption.as_deref(),
                    req.org_id,
                    runtime_agent_id,
                    &r.name,
                    &r.url,
                )
                .await
                .map_err(|error| {
                    tracing::error!(%error, "Failed to resolve Agent MCP credentials");
                    Status::internal("Failed to resolve MCP credentials")
                })?;
            Some(resolved_mcp_server_to_proto(r, secret_bindings))
        } else {
            None
        };

        Ok(Response::new(GetMcpServerByPrefixResponse {
            server: server_info,
        }))
    }
}
