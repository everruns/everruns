use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use tonic::body::Body;
use tonic::codegen::http::{Request, Response};
use tower::service_fn;

use crate::{WorkerServiceClient, proto};

#[tokio::test]
async fn mcp_lookup_uses_a_distinct_rpc_and_old_servers_fail_closed() {
    let paths = Arc::new(Mutex::new(Vec::new()));
    let observed = paths.clone();
    let service = service_fn(move |request: Request<Body>| {
        observed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(request.uri().path().to_string());
        async {
            Ok::<_, Infallible>(
                Response::builder()
                    .status(200)
                    .header("content-type", "application/grpc")
                    .header("grpc-status", "12")
                    .body(Body::empty())
                    .expect("static gRPC response"),
            )
        }
    });
    let mut client = WorkerServiceClient::new(service);
    let mcp = client
        .get_mcp_connection_token(proto::GetMcpConnectionTokenRequest {
            session_id: Some(proto::Uuid {
                value: uuid::Uuid::new_v4().to_string(),
            }),
            provider: "mcp_oauth_test".to_string(),
            acts_as: "user".to_string(),
        })
        .await
        .expect_err("old control plane must reject the new MCP-specific RPC");

    assert_eq!(mcp.code(), tonic::Code::Unimplemented);
    assert_eq!(
        *paths.lock().unwrap_or_else(|error| error.into_inner()),
        ["/everruns.internal.WorkerService/GetMcpConnectionToken"]
    );
}
