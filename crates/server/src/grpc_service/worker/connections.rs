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
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let resolver = self.connection_resolver()?;

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
        let resolver = self.connection_resolver()?;
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
        let resolver = self.connection_resolver()?;

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
    session_id: everruns_provider::typed_id::SessionId,
    provider: &str,
    acts_as: &str,
) -> Result<Option<String>, Status> {
    let acts_as = match acts_as {
        value @ ("none" | "service" | "user") => everruns_core::McpServerActsAs::from(value),
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
    use everruns_core::McpServerActsAs;
    use everruns_core::connection_services::UserConnectionResolver;
    use everruns_provider::error::Result;
    use std::sync::Mutex;

    struct RecordingResolver {
        calls: Arc<Mutex<Vec<Option<McpServerActsAs>>>>,
    }

    #[async_trait::async_trait]
    impl UserConnectionResolver for RecordingResolver {
        async fn get_connection_token(
            &self,
            _session_id: everruns_provider::typed_id::SessionId,
            _provider: &str,
        ) -> Result<Option<String>> {
            self.calls.lock().unwrap().push(None);
            Ok(Some("legacy".to_string()))
        }

        async fn get_mcp_connection_token(
            &self,
            _session_id: everruns_provider::typed_id::SessionId,
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
        let session_id = everruns_provider::typed_id::SessionId::new();

        for (wire_value, expected_token) in
            [("none", "none"), ("service", "service"), ("user", "user")]
        {
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
            ]
        );

        let error = resolve_mcp_connection_token(&resolver, session_id, "github", "system")
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert_eq!(calls.lock().unwrap().len(), 3);
    }
}
