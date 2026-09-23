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
                .resolve_runtime_provider_config(req.org_id, &req.provider_id)
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
                    found: true,
                    api_key: api_key.unwrap_or_default(),
                    base_url: base_url.unwrap_or_default(),
                    provider_type,
                    request_options_json,
                }
            }
            None => GetDefaultProviderCredentialsResponse {
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

            if let Some(session) = self
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
                runtime_agent_id = session.agent_id;
                let mut agent = if let Some(agent_id) = session.agent_id {
                    crate::domains::agents::queries::get_by_public_id(
                        &self.db,
                        req.org_id,
                        None,
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
