use super::*;
use crate::grpc_test_env::TlsEnvGuard;

mod tls;
use everruns_contracts::typed_id::SessionId;
use std::time::Instant;

/// The incident shape: a control plane that takes the connection and
/// never answers. The call errors at the deadline instead of hanging.
#[tokio::test]
async fn a_call_the_control_plane_never_answers_errors_at_the_deadline() {
    let env = TlsEnvGuard::new();
    clear_tls(&env);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    // Accept and hold every connection without reading or writing.
    let server = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let client =
        GrpcClient::connect_with_request_timeout(&addr.to_string(), Duration::from_millis(300))
            .await
            .expect("the TCP connection is accepted");
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        client.set_session_status(1, SessionId::new(), "active"),
    )
    .await
    .expect("the call ends at its deadline, not never");
    assert!(result.is_err(), "a call nobody answered cannot succeed");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "ended long after the deadline: {:?}",
        started.elapsed()
    );
    server.abort();
}
fn clear_tls(env: &TlsEnvGuard) {
    for key in [
        "WORKER_GRPC_TLS_CA_CERT",
        "WORKER_GRPC_TLS_CERT",
        "WORKER_GRPC_TLS_KEY",
        "WORKER_GRPC_TLS_DOMAIN",
    ] {
        env.remove_var(key);
    }
}
