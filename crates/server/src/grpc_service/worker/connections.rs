//! User connection tokens.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_get_connection_token(
        &self,
        request: Request<GetConnectionTokenRequest>,
    ) -> Result<Response<GetConnectionTokenResponse>, Status> {
        let req = request.into_inner();
        if req.provider.starts_with("mcp_oauth_") {
            return Err(Status::permission_denied(
                "MCP credentials require attachment-scoped resolution",
            ));
        }
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let base = self.connection_resolver()?;
        let bound = req
            .input_message_id
            .as_ref()
            .map(|id| parse_uuid(Some(id)))
            .transpose()?
            .and_then(|id| base.for_execution(id));
        let resolver = bound.as_ref().unwrap_or(base);

        let token = resolver
            .get_connection_token(session_id.into(), &req.provider)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve connection token: {}", e);
                Status::internal("Failed to resolve connection token")
            })?;

        Ok(Response::new(GetConnectionTokenResponse { token }))
    }

    pub(crate) async fn handle_get_mcp_connection_token(
        &self,
        request: Request<GetMcpConnectionTokenRequest>,
    ) -> Result<Response<GetConnectionTokenResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        self.validate_mcp_operation(
            session_id,
            req.input_message_id.as_ref(),
            req.server_prefix.as_deref(),
            &req.provider,
            &req.acts_as,
        )
        .await?;
        let base = self.connection_resolver()?;
        let bound = req
            .input_message_id
            .as_ref()
            .map(|id| parse_uuid(Some(id)))
            .transpose()?
            .and_then(|id| base.for_execution(id));
        let resolver = bound.as_ref().unwrap_or(base);
        let token =
            resolve_mcp_connection_token(resolver, session_id.into(), &req.provider, &req.acts_as)
                .await?;

        Ok(Response::new(GetConnectionTokenResponse { token }))
    }

    pub(crate) async fn handle_get_connection_user(
        &self,
        request: Request<GetConnectionUserRequest>,
    ) -> Result<Response<GetConnectionUserResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let base = self.connection_resolver()?;
        let bound = req
            .input_message_id
            .as_ref()
            .map(|id| parse_uuid(Some(id)))
            .transpose()?
            .and_then(|id| base.for_execution(id));
        let resolver = bound.as_ref().unwrap_or(base);

        let user_id = resolver
            .get_connection_user(session_id.into(), &req.provider)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve connection owner: {}", e);
                Status::internal("Failed to resolve connection owner")
            })?;

        Ok(Response::new(GetConnectionUserResponse {
            user_id: user_id.map(|user_id| proto::Uuid {
                value: user_id.to_string(),
            }),
        }))
    }

    pub(crate) async fn handle_invalidate_mcp_connection(
        &self,
        request: Request<InvalidateMcpConnectionRequest>,
    ) -> Result<Response<InvalidateMcpConnectionResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        self.validate_mcp_operation(
            session_id,
            req.input_message_id.as_ref(),
            req.server_prefix.as_deref(),
            &req.provider,
            &req.acts_as,
        )
        .await?;
        let base = self.connection_resolver()?;
        let bound = req
            .input_message_id
            .as_ref()
            .map(|id| parse_uuid(Some(id)))
            .transpose()?
            .and_then(|id| base.for_execution(id));
        let resolver = bound.as_ref().unwrap_or(base);
        let acts_as = match req.acts_as.as_str() {
            value @ ("none" | "service" | "user" | "user_or_service") => {
                everruns_core::McpServerActsAs::from(value)
            }
            _ => return Err(Status::invalid_argument("Invalid MCP acts_as value")),
        };

        resolver
            .invalidate_mcp_connection(
                session_id.into(),
                &req.provider,
                acts_as,
                &req.rejected_credential_fingerprint,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to invalidate MCP connection: {}", e);
                Status::internal("Failed to invalidate MCP connection")
            })?;

        Ok(Response::new(InvalidateMcpConnectionResponse {}))
    }

    // THREAT[TM-TOOL-041]: a worker-supplied actsAs value cannot select a different credential owner.
    async fn validate_mcp_operation(
        &self,
        session: uuid::Uuid,
        input: Option<&proto::Uuid>,
        prefix: Option<&str>,
        provider: &str,
        acts_as: &str,
    ) -> Result<(), Status> {
        let Some(input) = input else {
            return Err(Status::permission_denied(
                "MCP operation requires an invocation",
            ));
        };
        let prefix =
            prefix.ok_or_else(|| Status::permission_denied("MCP attachment scope required"))?;
        let row = self
            .db
            .get_session_unscoped(session.into())
            .await
            .map_err(|_| Status::internal("Session unavailable"))?
            .ok_or_else(|| Status::not_found("Session not found"))?;
        let response = self
            .handle_get_mcp_server_by_prefix(Request::new(proto::GetMcpServerByPrefixRequest {
                org_id: row.org_id,
                session_id: Some(proto::Uuid {
                    value: session.to_string(),
                }),
                input_message_id: Some(input.clone()),
                server_prefix: prefix.into(),
            }))
            .await?;
        let server = response
            .into_inner()
            .server
            .ok_or_else(|| Status::permission_denied("MCP attachment unavailable"))?;
        // A `user_or_service` attachment resolves as the person, then as the
        // agent, so the worker asks for each concrete identity in turn.
        let acts_as_matches = server.acts_as == acts_as
            || (server.acts_as == "user_or_service" && matches!(acts_as, "user" | "service"));
        if server.oauth_provider_id.as_deref() != Some(provider) || !acts_as_matches {
            return Err(Status::permission_denied(
                "MCP operation does not match the configured attachment",
            ));
        }
        Ok(())
    }

    pub(crate) async fn handle_get_connection_token_for_user(
        &self,
        request: Request<GetConnectionTokenForUserRequest>,
    ) -> Result<Response<GetConnectionTokenForUserResponse>, Status> {
        let req = request.into_inner();
        let user_id = parse_uuid(req.user_id.as_ref())?;
        let resolver = self.connection_resolver()?;

        let token = resolver
            .get_connection_token_for_user(user_id, &req.provider)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve connection token for user: {}", e);
                Status::internal("Failed to resolve connection token for user")
            })?;

        Ok(Response::new(GetConnectionTokenForUserResponse { token }))
    }
}

#[allow(clippy::result_large_err)]
async fn resolve_mcp_connection_token(
    resolver: &Arc<dyn everruns_core::connection_services::UserConnectionResolver>,
    session_id: everruns_contracts::typed_id::SessionId,
    provider: &str,
    acts_as: &str,
) -> Result<Option<String>, Status> {
    let acts_as = match acts_as {
        value @ ("none" | "service" | "user" | "user_or_service") => {
            everruns_core::McpServerActsAs::from(value)
        }
        _ => return Err(Status::invalid_argument("Invalid MCP acts_as value")),
    };
    resolver
        .get_mcp_connection_token(session_id, provider, acts_as)
        .await
        .map_err(|error| {
            tracing::error!(%error, "Failed to resolve MCP connection token");
            Status::internal("Failed to resolve MCP connection token")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::error::Result;
    use everruns_core::McpServerActsAs;
    use everruns_core::connection_services::UserConnectionResolver;
    use std::sync::Mutex;

    struct RecordingResolver {
        calls: Arc<Mutex<Vec<Option<McpServerActsAs>>>>,
    }

    #[async_trait::async_trait]
    impl UserConnectionResolver for RecordingResolver {
        async fn get_connection_token(
            &self,
            _session_id: everruns_contracts::typed_id::SessionId,
            _provider: &str,
        ) -> Result<Option<String>> {
            self.calls.lock().unwrap().push(None);
            Ok(Some("legacy".to_string()))
        }

        async fn get_mcp_connection_token(
            &self,
            _session_id: everruns_contracts::typed_id::SessionId,
            _provider: &str,
            acts_as: McpServerActsAs,
        ) -> Result<Option<String>> {
            self.calls.lock().unwrap().push(Some(acts_as));
            Ok(Some(acts_as.to_string()))
        }
    }

    #[tokio::test]
    async fn mcp_connection_token_dispatch_uses_only_exact_identity_paths() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resolver: Arc<dyn UserConnectionResolver> = Arc::new(RecordingResolver {
            calls: calls.clone(),
        });
        let session_id = everruns_contracts::typed_id::SessionId::new();

        for (wire_value, expected_token) in [
            ("none", "none"),
            ("service", "service"),
            ("user", "user"),
            ("user_or_service", "user_or_service"),
        ] {
            let token = resolve_mcp_connection_token(&resolver, session_id, "github", wire_value)
                .await
                .unwrap();
            assert_eq!(token.as_deref(), Some(expected_token));
        }

        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                Some(McpServerActsAs::None),
                Some(McpServerActsAs::Service),
                Some(McpServerActsAs::User),
                Some(McpServerActsAs::UserOrService),
            ]
        );

        let error = resolve_mcp_connection_token(&resolver, session_id, "github", "system")
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert_eq!(calls.lock().unwrap().len(), 4);
    }
}
