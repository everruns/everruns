//! Real transport proof, not a synthetic identity header or fixed scope.
//! All tokens/registrations are test-only fixtures; no production issuer exists.
use super::super::authority::WorkerAuthority;
use super::super::journal_capability::{JournalCapabilityVerifier, journal_capability_digest};
use super::*;
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use everruns_core::{EventContext, EventRequest, OutputMessageCompletedData, RuntimeMessage};
use everruns_internal_protocol::{WorkerServiceClient, uuid_to_proto_uuid};
use sha2::{Digest, Sha256};
use std::{sync::Mutex, time::Duration};
use tonic::codegen::{Service as _, http};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity, ServerTlsConfig};

mod lifecycle;
mod negatives;
mod protection_floor;
mod records;

const SERVER_CERT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../worker/tests/fixtures/turn-server-cert.pem"
));
const SERVER_KEY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../worker/tests/fixtures/test-server-key.pem"
));
const CLIENT_A: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../worker/tests/fixtures/turn-client-cert.pem"
));
const KEY_A: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../worker/tests/fixtures/test-client-key.pem"
));
// Public test certificate reusing an existing checked-in synthetic private key.
const CLIENT_B: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/executor-b-cert.pem"
));
const KEY_B: &str = SERVER_KEY;

fn fingerprint(pem: &str) -> [u8; 32] {
    let encoded: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    Sha256::digest(STANDARD.decode(encoded).unwrap()).into()
}
fn token(byte: u8) -> String {
    format!("Bearer wjc1.{}", URL_SAFE_NO_PAD.encode([byte; 32]))
}

struct Assignment {
    session: SessionId,
    grant: uuid::Uuid,
    executor: uuid::Uuid,
    certificate: [u8; 32],
    digest: [u8; 32],
    byte: u8,
}

async fn assignment(service: &WorkerServiceImpl, org: i64, pem: &str, byte: u8) -> Assignment {
    assignment_window(service, org, pem, byte, (-1, 200), (-1, 180)).await
}

async fn assignment_window(
    service: &WorkerServiceImpl,
    org: i64,
    pem: &str,
    byte: u8,
    identity_window: (i64, i64),
    capability_window: (i64, i64),
) -> Assignment {
    let db = &service.db;
    // Test-only enrollment follows the irreversible wire protection floor.
    sqlx::query("INSERT INTO worker_ingress_protection_floor (minimum_version) VALUES (1) ON CONFLICT DO NOTHING")
        .execute(db.database().pool()).await.unwrap();
    let session = db
        .create_session(crate::storage::test_database::test_session_row(org))
        .await
        .unwrap()
        .id;
    service
        .event_service
        .emit(EventRequest::new(
            session,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant(format!(
                "tenant-{org}-journal"
            ))),
        ))
        .await
        .unwrap();
    db.mark_session_actor_runnable(session, org, 1)
        .await
        .unwrap();
    let lease = db
        .lease_session_actors("equal-holder", 100, Duration::from_secs(240), 32)
        .await
        .unwrap()
        .into_iter()
        .find(|lease| lease.session_id == session)
        .unwrap();
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(db.database().pool())
        .await
        .unwrap();
    let executor = uuid::Uuid::new_v4();
    let grant = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO worker_journal_read_grants \
        (id, executor_id, org_id, session_id, lease_holder, lease_fence, created_at, expires_at) \
        VALUES ($1,$2,$3,$4,'equal-holder',$5,$6,$7)",
    )
    .bind(grant)
    .bind(executor)
    .bind(org)
    .bind(session)
    .bind(lease.fence)
    .bind(now - chrono::Duration::seconds(30))
    .bind(now + chrono::Duration::seconds(240))
    .execute(db.database().pool())
    .await
    .unwrap();
    let certificate = fingerprint(pem);
    sqlx::query(
        "INSERT INTO worker_executor_identities \
        (certificate_sha256, executor_id, created_at, expires_at) VALUES ($1,$2,$3,$4)",
    )
    .bind(certificate.as_slice())
    .bind(executor)
    .bind(now + chrono::Duration::seconds(identity_window.0))
    .bind(now + chrono::Duration::seconds(identity_window.1))
    .execute(db.database().pool())
    .await
    .unwrap();
    let digest = journal_capability_digest(&[byte; 32]);
    sqlx::query(
        "INSERT INTO worker_journal_capabilities \
        (capability_sha256, grant_id, created_at, expires_at) VALUES ($1,$2,$3,$4)",
    )
    .bind(digest.as_slice())
    .bind(grant)
    .bind(now + chrono::Duration::seconds(capability_window.0))
    .bind(now + chrono::Duration::seconds(capability_window.1))
    .execute(db.database().pool())
    .await
    .unwrap();
    seed_dependency(db, grant, session, org).await;
    Assignment {
        session,
        grant,
        executor,
        certificate,
        digest,
        byte,
    }
}

// Revocation prerequisites only: no secret payload or resource-use authority
// is created for the executor. Fixtures use session-scoped service consent.
async fn seed_dependency(
    db: &StorageBackend,
    grant: uuid::Uuid,
    session: SessionId,
    org: i64,
) -> (uuid::Uuid, uuid::Uuid) {
    let resource = uuid::Uuid::new_v4();
    let source = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO resource_access (id,org_id,kind,scope,usage) VALUES ($1,$2,'credential_binding','organization','service')")
        .bind(resource).bind(org).execute(db.database().pool()).await.unwrap();
    sqlx::query("INSERT INTO resource_grants (id,org_id,resource_id,consumer_scope,consumer_session_id,usage) VALUES ($1,$2,$3,'session',$4,'service')")
        .bind(source).bind(org).bind(resource).bind(session).execute(db.database().pool()).await.unwrap();
    register_dependency(db, grant, source, resource)
        .await
        .unwrap();
    (source, resource)
}

async fn register_dependency(
    db: &StorageBackend,
    grant: uuid::Uuid,
    source: uuid::Uuid,
    resource: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO worker_journal_grant_dependencies \
        (grant_id,source_grant_id,source_resource_id,grant_revision,resource_revision,grant_epoch,resource_epoch) \
        SELECT $1,rg.id,ra.id,rg.revision,ra.revision,rg.worker_authorization_epoch,ra.worker_authorization_epoch \
        FROM resource_grants rg JOIN resource_access ra ON ra.id=rg.resource_id \
        WHERE rg.id=$2 AND ra.id=$3")
        .bind(grant).bind(source).bind(resource).execute(db.database().pool()).await?;
    Ok(())
}

#[derive(Clone)]
struct AuthState {
    retained: Arc<Mutex<Option<WorkerAuthority>>>,
}
async fn capture(
    axum::extract::State(state): axum::extract::State<AuthState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    assert!(
        !request.headers().contains_key(http::header::AUTHORIZATION),
        "proof must not reach handlers"
    );
    *state.retained.lock().unwrap() = request.extensions().get::<WorkerAuthority>().cloned();
    next.run(request).await
}

struct Server {
    channel: Channel,
    address: String,
    retained: Arc<Mutex<Option<WorkerAuthority>>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start(
        service: Arc<WorkerServiceImpl>,
        tls: bool,
        identity: Option<(&str, &str)>,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let incoming = futures::stream::unfold(listener, |listener| async move {
            let socket = listener.accept().await.map(|(socket, _)| socket);
            Some((socket, listener))
        });
        let retained = Arc::new(Mutex::new(None));
        let state = AuthState {
            retained: retained.clone(),
        };
        // The same RPC + raw HTTP route composition as the production listener.
        let router = tonic::service::Routes::new(WorkerServiceServer::from_arc(service.clone()))
            .into_axum_router()
            .merge(crate::api::internal_blobs::routes(
                crate::api::internal_blobs::AppState {
                    db: service.db.clone(),
                    signing_secret: TEST_BLOB_SECRET.into(),
                },
            ))
            // A future handler with no scope check must still be unreachable.
            .route(
                "/future/unsafe",
                axum::routing::any(|| async { "future-route-secret" }),
            )
            .layer(axum::middleware::from_fn_with_state(state, capture))
            .layer(axum::middleware::from_fn_with_state(
                JournalCapabilityVerifier::new(service.db.clone()),
                super::super::journal_capability::authenticate_request,
            ));
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut server = tonic::transport::Server::builder();
            if tls {
                server = server
                    .tls_config(
                        ServerTlsConfig::new()
                            .identity(Identity::from_pem(SERVER_CERT, SERVER_KEY))
                            .client_ca_root(Certificate::from_pem(format!(
                                "{CLIENT_A}\n{CLIENT_B}"
                            )))
                            // Test missing certificate evidence even on encrypted connections.
                            .client_auth_optional(true),
                    )
                    .unwrap();
            }
            server
                .add_routes(router.into())
                .serve_with_incoming_shutdown(Box::pin(incoming), async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let address = format!("{}://{address}", if tls { "https" } else { "http" });
        let channel = Self::connect(&address, identity).await;
        Self {
            channel,
            address,
            retained,
            shutdown: Some(shutdown),
            task,
        }
    }
    async fn connect(address: &str, identity: Option<(&str, &str)>) -> Channel {
        let mut endpoint = tonic::transport::Endpoint::from_shared(address.to_string())
            .unwrap()
            .timeout(Duration::from_secs(5));
        if address.starts_with("https:") {
            let mut tls = ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(SERVER_CERT))
                .domain_name("localhost");
            if let Some((cert, key)) = identity {
                tls = tls.identity(Identity::from_pem(cert, key));
            }
            endpoint = endpoint.tls_config(tls).unwrap();
        }
        endpoint.connect().await.unwrap()
    }
    async fn read(
        &self,
        session: SessionId,
        byte: u8,
    ) -> Result<proto::ReadActorJournalResponse, Status> {
        WorkerServiceClient::new(self.channel.clone())
            .read_actor_journal(read(session, byte))
            .await
            .map(Response::into_inner)
    }
    fn context(&self) -> WorkerAuthority {
        self.retained.lock().unwrap().clone().unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}
fn read(session: SessionId, byte: u8) -> Request<proto::ReadActorJournalRequest> {
    let mut request = Request::new(proto::ReadActorJournalRequest {
        session_id: Some(uuid_to_proto_uuid(session.uuid())),
        after_sequence: None,
        limit: 1,
    });
    request
        .metadata_mut()
        .insert("authorization", token(byte).parse().unwrap());
    request.set_timeout(Duration::from_secs(5));
    request
}
async fn retained_read(
    service: &WorkerServiceImpl,
    context: WorkerAuthority,
    session: SessionId,
) -> Result<Response<proto::ReadActorJournalResponse>, Status> {
    let mut request = read(session, 0);
    request.extensions_mut().insert(context);
    service.read_actor_journal(request).await
}

#[tokio::test]
async fn verified_peers_cannot_redeem_another_executors_capability_or_select_its_tenant() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let b = assignment(&service, 3, CLIENT_B, 0xb2).await;
    let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap().events.len(),
        1
    );
    assert_eq!(
        server.read(b.session, a.byte).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        server.read(b.session, b.byte).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    let mut forged = read(b.session, b.byte);
    for (key, value) in [
        ("x-everruns-org-id", "3".into()),
        ("x-everruns-executor-id", b.executor.to_string()),
        ("x-everruns-grant-id", b.grant.to_string()),
        ("x-forwarded-client-cert", CLIENT_B.replace('\n', "")),
    ] {
        forged.metadata_mut().insert(key, value.parse().unwrap());
    }
    assert_eq!(
        WorkerServiceClient::new(server.channel.clone())
            .read_actor_journal(forged)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    // B's proof is valid for B with its own verified private-key possession.
    server.channel = Server::connect(&server.address, Some((CLIENT_B, KEY_B))).await;
    assert_eq!(
        server.read(b.session, b.byte).await.unwrap().events.len(),
        1
    );
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
}
