use super::{GrpcAdapter, grpc_status_to_error, proto, proto_uuid_to_uuid, uuid_to_proto};
use async_trait::async_trait;
use everruns_provider::error::Result;
use everruns_provider::typed_id::SessionId;
use uuid::Uuid;

#[async_trait]
impl everruns_core::connection_services::UserConnectionResolver for GrpcAdapter {
    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::GetConnectionTokenRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            provider: provider.to_string(),
            acts_as: String::new(),
        };
        let response = client
            .get_connection_token(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().token)
    }

    async fn get_mcp_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
        acts_as: everruns_core::McpServerActsAs,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::GetConnectionTokenRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            provider: provider.to_string(),
            acts_as: acts_as.to_string(),
        };
        let response = client
            .get_connection_token(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().token)
    }

    async fn get_connection_user(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_connection_user(proto::GetConnectionUserRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                provider: provider.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().user_id {
            Some(user_id) => Ok(Some(proto_uuid_to_uuid(Some(&user_id))?)),
            None => Ok(None),
        }
    }

    async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_connection_token_for_user(proto::GetConnectionTokenForUserRequest {
                user_id: Some(uuid_to_proto(user_id)),
                provider: provider.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response.into_inner().token)
    }
}
