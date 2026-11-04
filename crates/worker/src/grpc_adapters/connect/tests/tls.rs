//! Actual TLS/HTTP2 calls through the public turn-client constructor.
use super::*;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use tonic::codegen::{Service, http};
use tonic::transport::{Identity, ServerTlsConfig};

const SERVER_CERT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/turn-server-cert.pem"
));
const SERVER_KEY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/test-server-key.pem"
));
const CLIENT_CERT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/turn-client-cert.pem"
));
const PROBE: &str = "turn TLS probe reached";

#[derive(Clone, Default)]
struct Probe {
    rpc: Arc<AtomicUsize>,
    blobs: Arc<AtomicUsize>,
    authorized: Arc<AtomicUsize>,
}
impl<B> Service<http::Request<B>> for Probe {
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = std::future::Ready<std::result::Result<Self::Response, Self::Error>>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        if request
            .headers()
            .get("authorization")
            .is_some_and(|value| value == "Bearer turn-tls-test-token")
        {
            self.authorized.fetch_add(1, Ordering::SeqCst);
        }
        let response = if request.uri().path().starts_with("/internal/blobs/") {
            self.blobs.fetch_add(1, Ordering::SeqCst);
            http::Response::new(tonic::body::Body::new(http_body_util::Full::new(
                bytes::Bytes::from_static(b"TLS blob"),
            )))
        } else {
            self.rpc.fetch_add(1, Ordering::SeqCst);
            tonic::Status::permission_denied(PROBE).into_http::<tonic::body::Body>()
        };
        std::future::ready(Ok(response))
    }
}
struct Incoming(tokio::net::TcpListener);
impl tonic::codegen::tokio_stream::Stream for Incoming {
    type Item = std::io::Result<tokio::net::TcpStream>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0
            .poll_accept(cx)
            .map(|accepted| Some(accepted.map(|(socket, _)| socket)))
    }
}
struct Server {
    addr: std::net::SocketAddr,
    probe: Probe,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start(tls: bool, mutual: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let probe = Probe::default();
        let routes = tonic::service::Routes::default()
            .into_axum_router()
            .fallback_service(probe.clone());
        let task = tokio::spawn(async move {
            let mut builder = tonic::transport::Server::builder();
            if tls {
                let mut config =
                    ServerTlsConfig::new().identity(Identity::from_pem(SERVER_CERT, SERVER_KEY));
                if mutual {
                    config =
                        config.client_ca_root(tonic::transport::Certificate::from_pem(CLIENT_CERT));
                }
                builder = builder.tls_config(config).unwrap();
            }
            builder
                .add_routes(routes.into())
                .serve_with_incoming(Incoming(listener))
                .await
                .unwrap();
        });
        Self { addr, probe, task }
    }
    fn hostname(&self) -> String {
        format!("localhost:{}", self.addr.port())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}
fn tls_env() -> TlsEnvGuard {
    let env = TlsEnvGuard::new();
    clear_tls(&env);
    env.set_var("WORKER_GRPC_TLS_CA_CERT", &fixture("turn-server-cert.pem"));
    env.set_var("WORKER_GRPC_AUTH_TOKEN", "turn-tls-test-token");
    env
}
async fn rpc(client: &GrpcClient) -> tonic::Status {
    tokio::time::timeout(
        Duration::from_secs(5),
        client.inner.client().set_session_status(
            everruns_internal_protocol::proto::SetSessionStatusRequest::default(),
        ),
    )
    .await
    .expect("probe call must terminate")
    .unwrap_err()
}
async fn succeeds(client: &GrpcClient, server: &Server) {
    let status = rpc(client).await;
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
    assert_eq!(status.message(), PROBE);
    let bytes = tokio::time::timeout(
        Duration::from_secs(5),
        client.blobs.fetch("/internal/blobs/files/probe", 128),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(bytes, b"TLS blob");
    assert_eq!(server.probe.rpc.load(Ordering::SeqCst), 1);
    assert_eq!(server.probe.blobs.load(Ordering::SeqCst), 1);
    assert_eq!(server.probe.authorized.load(Ordering::SeqCst), 2);
}

async fn refused(server: &Server) {
    // A TLS server may close after the client's handshake completes (for
    // example, when the server rejects a missing client certificate). Exercise
    // both consumers even if connect() returned a channel in that case.
    if let Ok(client) = tokio::time::timeout(
        Duration::from_secs(5),
        GrpcClient::connect_with_request_timeout(&server.hostname(), Duration::from_millis(500)),
    )
    .await
    .expect("TLS failure must terminate")
    {
        assert_ne!(rpc(&client).await.message(), PROBE);
        assert!(
            tokio::time::timeout(
                Duration::from_secs(5),
                client.blobs.fetch("/internal/blobs/files/probe", 128),
            )
            .await
            .expect("blob TLS failure must terminate")
            .is_err()
        );
    }
    assert_eq!(server.probe.rpc.load(Ordering::SeqCst), 0);
    assert_eq!(server.probe.blobs.load(Ordering::SeqCst), 0);
    assert_eq!(server.probe.authorized.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn public_turn_client_uses_configured_tls_for_rpc_and_blob_calls() {
    let _env = tls_env();
    let server = Server::start(true, false).await;
    // A real-handshake control proves the fixed test certificate is accepted
    // by the pre-existing TLS implementation, independently of the turn path.
    crate::grpc_durable_store::GrpcDurableStore::connect_with_timeout(
        &server.hostname(),
        Duration::from_secs(3),
    )
    .await
    .expect("existing durable TLS client must accept the fixture");
    let client =
        GrpcClient::connect_with_request_timeout(&server.hostname(), Duration::from_secs(2))
            .await
            .unwrap();
    succeeds(&client, &server).await;
}

#[tokio::test]
async fn configured_tls_never_sends_the_bearer_to_a_plaintext_peer() {
    let env = tls_env();
    env.set_var("WORKER_GRPC_TLS_DOMAIN", "localhost");
    let server = Server::start(false, false).await;
    if let Ok(client) =
        GrpcClient::connect_with_request_timeout(&server.addr.to_string(), Duration::from_secs(2))
            .await
    {
        let _ = rpc(&client).await;
    }
    assert_eq!(
        server.probe.authorized.load(Ordering::SeqCst),
        0,
        "configured TLS must not send credentials over a plaintext channel"
    );
    assert_eq!(server.probe.rpc.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn wrong_ca_and_hostname_and_malformed_ca_fail_without_application_requests() {
    let env = tls_env();
    let server = Server::start(true, false).await;
    env.set_var("WORKER_GRPC_TLS_CA_CERT", &fixture("test-ca.pem"));
    refused(&server).await;

    env.set_var("WORKER_GRPC_TLS_CA_CERT", &fixture("turn-server-cert.pem"));
    env.set_var("WORKER_GRPC_TLS_DOMAIN", "foreign-tenant.invalid");
    refused(&server).await;

    // This text fixture is deliberately not a PEM certificate.
    env.set_var("WORKER_GRPC_TLS_CA_CERT", &fixture("README.md"));
    env.remove_var("WORKER_GRPC_TLS_DOMAIN");
    refused(&server).await;
}

#[tokio::test]
async fn explicit_domain_is_required_when_the_address_does_not_match_the_certificate() {
    let env = tls_env();
    let server = Server::start(true, false).await;
    // Certificate has DNS:localhost, without an IP SAN.
    assert!(
        GrpcClient::connect_with_request_timeout(&server.addr.to_string(), Duration::from_secs(2))
            .await
            .is_err()
    );
    assert_eq!(server.probe.authorized.load(Ordering::SeqCst), 0);
    env.set_var("WORKER_GRPC_TLS_DOMAIN", "localhost");
    let client =
        GrpcClient::connect_with_request_timeout(&server.addr.to_string(), Duration::from_secs(2))
            .await
            .unwrap();
    succeeds(&client, &server).await;
}

#[tokio::test]
async fn mutual_tls_requires_a_matching_trusted_client_identity_for_rpc_and_blobs() {
    let env = tls_env();
    let server = Server::start(true, true).await;
    refused(&server).await; // no identity

    env.set_var("WORKER_GRPC_TLS_CERT", &fixture("turn-client-cert.pem"));
    refused(&server).await; // incomplete optional identity, no key

    env.set_var("WORKER_GRPC_TLS_CERT", &fixture("turn-server-cert.pem"));
    env.set_var("WORKER_GRPC_TLS_KEY", &fixture("test-server-key.pem"));
    refused(&server).await; // untrusted server-only identity

    env.set_var("WORKER_GRPC_TLS_CERT", &fixture("turn-client-cert.pem"));
    refused(&server).await; // trusted certificate with the wrong private key

    env.set_var("WORKER_GRPC_TLS_KEY", &fixture("test-client-key.pem"));
    let client =
        GrpcClient::connect_with_request_timeout(&server.hostname(), Duration::from_secs(2))
            .await
            .unwrap();
    succeeds(&client, &server).await;
}

#[tokio::test]
async fn tls_retains_the_hostname_and_does_not_use_plaintext_replica_addresses() {
    let _env = tls_env();
    let server = Server::start(true, false).await;
    let a = Server::start(false, false).await;
    let b = Server::start(false, false).await;
    let client = GrpcClient::connect_with_resolved(
        &server.hostname(),
        Duration::from_secs(2),
        grpc_client_tls_from_env(),
        BTreeSet::from([a.addr, b.addr]),
    )
    .await
    .unwrap();
    succeeds(&client, &server).await;
    for replica in [a, b] {
        assert_eq!(replica.probe.rpc.load(Ordering::SeqCst), 0);
        assert_eq!(replica.probe.blobs.load(Ordering::SeqCst), 0);
        assert_eq!(replica.probe.authorized.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn plaintext_single_endpoint_and_replica_balancing_still_work_without_tls() {
    let env = tls_env();
    clear_tls(&env);
    let server = Server::start(false, false).await;
    let client =
        GrpcClient::connect_with_request_timeout(&server.addr.to_string(), Duration::from_secs(2))
            .await
            .unwrap();
    succeeds(&client, &server).await;

    env.set_var("WORKER_GRPC_TLS_CA_CERT", "");
    let empty_ca_server = Server::start(false, false).await;
    let client = GrpcClient::connect_with_request_timeout(
        &empty_ca_server.addr.to_string(),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    succeeds(&client, &empty_ca_server).await;

    let a = Server::start(false, false).await;
    let b = Server::start(false, false).await;
    let client = GrpcClient::connect_with_resolved(
        "unused-in-this-test.invalid:9001",
        Duration::from_secs(2),
        None,
        BTreeSet::from([a.addr, b.addr]),
    )
    .await
    .unwrap();
    // Sequential completed requests allow either ready replica to be chosen.
    // Existing grpc_balance tests also verify in-flight calls reach both peers.
    for _ in 0..16 {
        assert_eq!(rpc(&client).await.message(), PROBE);
    }
    assert_eq!(
        a.probe.rpc.load(Ordering::SeqCst) + b.probe.rpc.load(Ordering::SeqCst),
        16
    );
    assert_eq!(
        a.probe.authorized.load(Ordering::SeqCst) + b.probe.authorized.load(Ordering::SeqCst),
        16
    );
}

#[tokio::test]
async fn a_stalled_tls_handshake_ends_at_the_configured_deadline() {
    let env = tls_env();
    env.set_var("WORKER_GRPC_TLS_DOMAIN", "localhost");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let _ = accepted_tx.send(());
        let _socket = socket;
        std::future::pending::<()>().await;
    });
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        GrpcClient::connect_with_request_timeout(&addr.to_string(), Duration::from_millis(200)),
    )
    .await;
    task.abort();
    assert!(
        accepted_rx.await.is_ok(),
        "must exercise an accepted TCP connection"
    );
    assert!(
        result
            .expect("handshake deadline must terminate connect")
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
#[should_panic(expected = "Failed to read WORKER_GRPC_TLS_CA_CERT")]
async fn unreadable_ca_retains_the_existing_fail_at_startup_contract() {
    let env = tls_env();
    env.set_var("WORKER_GRPC_TLS_CA_CERT", "/nonexistent/turn-client-ca.pem");
    let _ = GrpcClient::connect_with_request_timeout("127.0.0.1:1", Duration::from_secs(1)).await;
}
