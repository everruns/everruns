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
