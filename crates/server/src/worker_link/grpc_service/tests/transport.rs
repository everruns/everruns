//! Explicit trusted development adapters for transport and direct RPC tests.
use super::*;

pub(crate) fn trusted_request<T>(message: T) -> Request<T> {
    let mut request = Request::new(message);
    request
        .extensions_mut()
        .insert(authority::WorkerAuthority::Development);
    request
}

pub(crate) async fn start_grpc_test_server(
    service: WorkerServiceImpl,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind grpc listener");
    let addr = listener.local_addr().expect("grpc listener address");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let (incoming_tx, incoming_rx) = tokio::sync::mpsc::channel(8);
    let server = tokio::spawn(async move {
        let accept_task = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                if incoming_tx.send(Ok(stream)).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                let _ = incoming_tx.send(Err(error)).await;
                                break;
                            }
                        }
                    }
                }
            }
        });
        tonic::transport::Server::builder()
            .layer(tonic::service::InterceptorLayer::new(
                GrpcAuthInterceptor::new(None),
            ))
            .add_routes(service.into_routes(TEST_BLOB_SECRET.to_string()))
            .serve_with_incoming(tokio_stream::wrappers::ReceiverStream::new(incoming_rx))
            .await
            .expect("grpc test server should run");
        accept_task.await.expect("grpc accept task should join");
    });

    (addr.to_string(), shutdown_tx, server)
}
