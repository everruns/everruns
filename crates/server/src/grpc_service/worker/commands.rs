//! Generic domain command transport.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_execute_command(
        &self,
        request: Request<ExecuteCommandRequest>,
    ) -> Result<Response<ExecuteCommandResponse>, Status> {
        let req = request.into_inner();

        if req.api_version != COMMAND_API_VERSION_V1 {
            return Err(Status::invalid_argument(format!(
                "Unsupported command api_version: {}",
                req.api_version
            )));
        }

        if req.params_json.len() > MAX_EXECUTE_COMMAND_PARAMS_BYTES {
            return Err(Status::resource_exhausted(format!(
                "Command params payload too large: size exceeds {} byte limit",
                MAX_EXECUTE_COMMAND_PARAMS_BYTES
            )));
        }

        let params = if req.params_json.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice::<serde_json::Value>(&req.params_json)
                .map_err(|e| Status::invalid_argument(format!("Invalid params_json: {e}")))?
        };

        let Some(_) = params.as_object() else {
            return Err(Status::invalid_argument(
                "Command params must be a JSON object",
            ));
        };

        // gRPC ExecuteCommand is the auth boundary for worker-driven platform
        // tools. Respect the provided user_id; do not silently upgrade to
        // Caller::internal for user-owned sessions.
        let caller = match req.user_id.as_deref() {
            Some(user_id) => {
                let user_id = user_id
                    .parse()
                    .map_err(|e| Status::invalid_argument(format!("Invalid user_id: {e}")))?;
                crate::auth::caller_resolution::caller_for_user(&self.db, req.org_id, user_id)
                    .await
                    .map_err(|e| {
                        Status::permission_denied(format!("Failed to resolve caller: {e}"))
                    })?
            }
            None => everruns_core::Caller::internal(req.org_id),
        };
        let ctx = self.org_domain_ctx_for_caller(caller).await?;
        let response = match crate::domains::common::dispatch(&req.name, params, &ctx).await {
            Ok(ok_json) => ExecuteCommandResponse {
                result: Some(proto::execute_command_response::Result::OkJson(
                    ok_json.into_bytes(),
                )),
            },
            Err(error) => ExecuteCommandResponse {
                result: Some(proto::execute_command_response::Result::Error(
                    command_error_to_proto(error),
                )),
            },
        };

        Ok(Response::new(response))
    }

    pub(crate) async fn handle_list_commands(
        &self,
        _request: Request<ListCommandsRequest>,
    ) -> Result<Response<ListCommandsResponse>, Status> {
        let mut commands: Vec<CommandCatalogEntry> =
            inventory::iter::<crate::domains::common::CommandDescriptor>
                .into_iter()
                .map(|desc| {
                    let meta = (desc.meta)();
                    let positional_arg = (desc.positional_arg)();
                    CommandCatalogEntry {
                        name: meta.name.to_string(),
                        api_version: COMMAND_API_VERSION_V1.to_string(),
                        category: meta.category.to_string(),
                        description: meta.description.to_string(),
                        method: meta.method.to_string(),
                        path: meta.path.to_string(),
                        schema_hash: command_schema_hash(&meta, positional_arg),
                        positional_arg: positional_arg.map(str::to_string),
                    }
                })
                .collect();

        commands.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Response::new(ListCommandsResponse { commands }))
    }

    /// Whether the session's effective capability set carries `platform`.
    ///
    /// Resolves exactly as the worker does when it decides to install the
    /// catalog command source: fold harness -> agent -> session, then expand
    /// dependencies and canonicalise aliases. Any cheaper approximation (the
    /// union of the three declared lists, say) drifts from the worker, and a
    /// gate that disagrees with the surface it guards is worse than none.
    async fn session_has_platform_capability(
        &self,
        org_id: i64,
        session: &everruns_platform::Session,
    ) -> Result<bool, Status> {
        // The same inheritance fold the worker's harness store runs, so an
        // inherited `platform` resolves identically on both sides.
        let harness = crate::harness_chain::resolve_effective_harness(
            &self.db,
            org_id,
            session.harness_id.uuid(),
        )
        .await
        .map_err(|error| internal_status("Failed to load Platform command harness", error))?
        .ok_or_else(|| Status::not_found("Harness not found"))?;

        let agent = match session.agent_id {
            Some(agent_id) => crate::domains::agents::queries::get_by_public_id(
                &self.db,
                org_id,
                &agent_id.to_string(),
            )
            .await
            .map_err(|error| internal_status("Failed to load Platform command agent", error))?,
            None => None,
        };

        let agent_definition = agent.as_ref().map(|agent| agent.definition());
        let resolved = everruns_core::runtime_context::resolve_runtime_capabilities(
            &harness.definition(),
            agent_definition.as_ref(),
            &session.execution_session(),
            self.capability_service.registry(),
        );

        Ok(resolved
            .resolved_capability_configs
            .iter()
            .any(|capability| {
                capability.capability_id()
                    == everruns_platform::capabilities::PLATFORM_CAPABILITY_ID
            }))
    }

    pub(crate) async fn handle_invoke_platform_command_surface(
        &self,
        request: Request<InvokePlatformCommandSurfaceRequest>,
    ) -> Result<Response<InvokePlatformCommandSurfaceResponse>, Status> {
        // THREAT[TM-AGENT-017]: The worker supplies only the bound session and
        // org. Reload the session and resolve its persisted owner server-side;
        // never accept a caller identity across this trust boundary.
        let req = request.into_inner();
        if req.arguments_json.len() > MAX_EXECUTE_COMMAND_PARAMS_BYTES {
            return Err(Status::resource_exhausted(format!(
                "Platform command arguments exceed {} byte limit",
                MAX_EXECUTE_COMMAND_PARAMS_BYTES
            )));
        }
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let session = self
            .session_service
            .get(
                &everruns_core::Caller::internal(req.org_id),
                session_id,
                None,
            )
            .await
            .map_err(|error| {
                tracing::error!(%error, %session_id, org_id = req.org_id, "Failed to load Platform command session");
                Status::internal("Failed to load Platform command session")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        // This RPC is a worker trust boundary, not merely an RBAC boundary: the
        // worker installs the catalog only for sessions holding `platform`, but
        // a compromised worker can call this endpoint without going through
        // Bash at all. Re-derive the capability server-side.
        //
        // Resolved, not declared. `resolve_runtime_capabilities` folds
        // harness -> agent -> session and then expands dependencies and
        // canonicalises aliases, so `platform` can be present in the effective
        // set without appearing in any layer's declared list. Testing the
        // declared lists instead would deny the RPC for a session whose shell
        // legitimately carries the `everruns` builtin -- the two sides must
        // answer this question the same way, and this is the function the
        // worker-side answer comes from too.
        if !self
            .session_has_platform_capability(req.org_id, &session)
            .await?
        {
            return Err(Status::permission_denied(
                "Platform command execution requires the platform capability",
            ));
        }
        let user_id = session.resolved_owner_user_id.ok_or_else(|| {
            Status::permission_denied(
                "Platform command execution requires a user-owned session with a resolved owner",
            )
        })?;
        let caller = crate::auth::caller_resolution::caller_for_user(&self.db, req.org_id, user_id)
            .await
            .map_err(|error| {
                tracing::warn!(%error, %session_id, org_id = req.org_id, %user_id, "Failed to resolve Platform command caller");
                Status::permission_denied("Failed to resolve Platform command caller")
            })?;
        let operation = match PlatformCommandSurfaceOperation::try_from(req.operation) {
            Ok(PlatformCommandSurfaceOperation::Discover) => {
                crate::services::platform_command_surface::Operation::Discover
            }
            Ok(PlatformCommandSurfaceOperation::Query) => {
                crate::services::platform_command_surface::Operation::Query
            }
            Ok(PlatformCommandSurfaceOperation::Execute) => {
                crate::services::platform_command_surface::Operation::Execute
            }
            _ => {
                return Err(Status::invalid_argument(
                    "Invalid platform command operation",
                ));
            }
        };
        let arguments = if req.arguments_json.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice::<serde_json::Value>(&req.arguments_json).map_err(|error| {
                Status::invalid_argument(format!("Invalid arguments_json: {error}"))
            })?
        };
        if !arguments.is_object() {
            return Err(Status::invalid_argument(
                "Platform command arguments must be a JSON object",
            ));
        }

        let api_base = self
            .api_base_url
            .clone()
            .unwrap_or_else(|| "http://localhost:9300".to_string());
        let ui_base = std::env::var("PUBLIC_APP_URL")
            .or_else(|_| std::env::var("FRONTEND_URL"))
            .unwrap_or_else(|_| api_base.clone());
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            req.org_id,
            &everruns_platform::FeatureFlags::current(),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, org_id = req.org_id, "Failed to resolve Platform command feature flags");
            Status::internal("Failed to resolve organization feature flags")
        })?;
        let context = crate::api::mcp_endpoint::catalog::CatalogContext {
            domain_ctx: self
                .domain_ctx_for_caller(caller)
                .with_feature_flags(feature_flags),
            link_builder: crate::api::common::UrlBuilder::new(&api_base, &ui_base),
        };
        let result =
            crate::services::platform_command_surface::invoke(operation, &arguments, context).await;
        let result = match result {
            Ok(output) => {
                Some(proto::invoke_platform_command_surface_response::Result::Output(output))
            }
            Err(error) => {
                Some(proto::invoke_platform_command_surface_response::Result::Error(error))
            }
        };
        Ok(Response::new(InvokePlatformCommandSurfaceResponse {
            result,
        }))
    }
}

/// Helpers for driving registered commands over this module's RPC from tests in
/// the rest of `grpc_service`. It lives beside the handler it exercises so the
/// shared gRPC test file does not accumulate transport plumbing.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::grpc_service::*;
    use tonic::Request;

    /// Run one registered command over the gRPC command transport and return its
    /// JSON result, failing the test on a domain error rather than swallowing it.
    pub(crate) async fn execute_test_command(
        service: &WorkerServiceImpl,
        name: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let response = service
            .execute_command(Request::new(ExecuteCommandRequest {
                name: name.to_string(),
                api_version: "v1".to_string(),
                params_json: serde_json::to_vec(&params).expect("serialize params"),
                org_id: everruns_core::DEFAULT_ORG_ID,
                user_id: None,
                idempotency_key: None,
                metadata: std::collections::HashMap::new(),
            }))
            .await
            .unwrap_or_else(|status| panic!("{name} transport failure: {status:?}"))
            .into_inner();

        match response.result.expect("command result") {
            proto::execute_command_response::Result::OkJson(ok) => {
                serde_json::from_slice(&ok).expect("command output is json")
            }
            proto::execute_command_response::Result::Error(error) => {
                panic!("{name} failed: {error:?}")
            }
        }
    }
}
