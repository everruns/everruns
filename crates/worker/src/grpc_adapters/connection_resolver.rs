use async_trait::async_trait;
use everruns_core::connection_services::UserConnectionResolver;
use everruns_internal_protocol::proto;
use everruns_provider::error::Result;
use everruns_provider::typed_id::SessionId;
use uuid::Uuid;

use super::{GrpcAdapter, grpc_status_to_error, proto_uuid_to_uuid, uuid_to_proto};

#[async_trait]
impl UserConnectionResolver for GrpcAdapter {
    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_connection_token(proto::GetConnectionTokenRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                provider: provider.to_string(),
            })
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
        let response = client
            .get_mcp_connection_token(proto::GetMcpConnectionTokenRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                provider: provider.to_string(),
                acts_as: acts_as.to_string(),
            })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_adapters::GrpcClient;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use tonic::body::Body;
    use tonic::codegen::Service;
    use tonic::codegen::http::{Request, Response};

    #[derive(Clone)]
    struct OldControlPlane {
        paths: Arc<Mutex<Vec<String>>>,
    }

    impl Service<Request<Body>> for OldControlPlane {
        type Response = Response<Body>;
        type Error = Infallible;
        type Future = std::future::Ready<std::result::Result<Self::Response, Self::Error>>;

        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, request: Request<Body>) -> Self::Future {
            self.paths
                .lock()
                .unwrap()
                .push(request.uri().path().to_string());
            std::future::ready(Ok(Response::builder()
                .status(200)
                .header("content-type", "application/grpc")
                .header("grpc-status", tonic::Code::Unimplemented as i32)
                .body(Body::empty())
                .unwrap()))
        }
    }

    impl tonic::server::NamedService for OldControlPlane {
        const NAME: &'static str = "everruns.internal.WorkerService";
    }

    #[tokio::test]
    async fn mcp_lookup_fails_closed_against_an_old_control_plane() {
        let paths = Arc::new(Mutex::new(Vec::new()));
        let service = OldControlPlane {
            paths: paths.clone(),
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_shutdown(address, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let client = loop {
            match GrpcClient::connect(&address.to_string()).await {
                Ok(client) => break client,
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => panic!("old control plane did not start: {error}"),
            }
        };
        let adapter = GrpcAdapter::new(client);
        adapter
            .get_mcp_connection_token(
                SessionId::new(),
                "mcp_oauth_example",
                everruns_core::McpServerActsAs::User,
            )
            .await
            .unwrap_err();

        assert_eq!(
            *paths.lock().unwrap(),
            vec!["/everruns.internal.WorkerService/GetMcpConnectionToken"]
        );
        let _ = shutdown_tx.send(());
        server.await.unwrap();
    }
}
