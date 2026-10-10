//! Provider credentials and MCP server resolution.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.
//!
//! Decision: these two stay RPCs while connection tokens, MCP grants and
//! session secrets moved to internal commands. `GetDefaultProviderCredentials`
//! runs on every reason step and answers deployment-level provider keys and
//! environment fallbacks, which no org owns; `GetMcpServerByPrefix` runs on
//! every MCP tool call. Its resolution is `mcp_servers::worker_lookup`, which
//! the MCP grant commands reuse to check the attachment a token request names.

use super::support::*;
use crate::worker_link::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_get_default_provider_credentials(
        &self,
        request: Request<GetDefaultProviderCredentialsRequest>,
    ) -> Result<Response<GetDefaultProviderCredentialsResponse>, Status> {
        let req = request.into_inner();
        if req.system_decisions {
            let session_id = req
                .session_id
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Session is required"))?;
            let source = self
                .provider_resolver_service
                .resolve_system_decision_model(req.org_id, Some(parse_uuid(Some(session_id))?))
                .await
                .map_err(|_| Status::failed_precondition("Decision model is unavailable"))?;
            let binding = match source {
                everruns_core::connection_services::SystemDecisionModel::Deployment => None,
                everruns_core::connection_services::SystemDecisionModel::Organization(b) => Some(b),
            };
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
        use crate::domains::mcp_servers::worker_lookup::{self, WorkerMcpLookupError};

        let req = request.into_inner();
        let session_id = req
            .session_id
            .as_ref()
            .map(|id| parse_uuid(Some(id)))
            .transpose()?;
        let input_message = req
            .input_message_id
            .as_ref()
            .map(|id| parse_uuid(Some(id)))
            .transpose()?;
        let storage = self.storage_store().ok();
        let lookup = worker_lookup::WorkerMcpLookup {
            db: &self.db,
            session_service: &self.session_service,
            mcp_server_service: &self.mcp_server_service,
            registry: self.capability_service.registry(),
            encryption: self.encryption.as_deref(),
            storage: storage.map(|store| store.as_ref()),
        };
        let resolved = worker_lookup::resolve(
            &lookup,
            req.org_id,
            session_id,
            input_message,
            &req.server_prefix,
        )
        .await
        .map_err(|error| match error {
            WorkerMcpLookupError::UnknownInvocation => {
                Status::permission_denied("Unknown invocation")
            }
            WorkerMcpLookupError::Internal(context) => Status::internal(context),
        })?;

        let server = match resolved {
            Some(found) => {
                let secret_bindings =
                    crate::domains::agents::credentials::resolve_runtime_secret_bindings(
                        self.db.as_ref(),
                        self.encryption.as_deref(),
                        req.org_id,
                        found.runtime_agent_id,
                        &found.server.name,
                        &found.server.url,
                    )
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, "Failed to resolve Agent MCP credentials");
                        Status::internal("Failed to resolve MCP credentials")
                    })?;
                Some(resolved_mcp_server_to_proto(found.server, secret_bindings))
            }
            None => None,
        };

        Ok(Response::new(GetMcpServerByPrefixResponse { server }))
    }
}
