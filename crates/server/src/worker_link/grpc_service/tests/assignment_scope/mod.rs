//! Real-network negatives for the preparatory assignment read boundary.
//!
//! The test interceptor is a trusted, fixed-scope identity-adapter stand-in. It
//! issues no credentials and does not test production workload authentication.

use super::super::assignment_scope::ActorJournalScope;
use super::*;

mod grant_records;
mod grants;
use everruns_contracts::typed_id::SessionId;
use everruns_core::{EventContext, EventRequest, OutputMessageCompletedData, RuntimeMessage};
use everruns_internal_protocol::{WorkerServiceClient, uuid_to_proto_uuid};
use std::time::Duration;

struct ScopedServer {
    address: String,
    client: WorkerServiceClient<tonic::transport::Channel>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl ScopedServer {
    async fn start(service: WorkerServiceImpl, scope: ActorJournalScope) -> Self {
        Self::start_shared(Arc::new(service), scope).await
    }

    async fn start_shared(service: Arc<WorkerServiceImpl>, scope: ActorJournalScope) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let incoming = futures::stream::unfold(listener, |listener| async move {
            let socket = listener.accept().await.map(|(socket, _)| socket);
            Some((socket, listener))
        });
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let auth = move |mut req: Request<()>| {
            // Remote metadata cannot change this server-owned assignment.
            req.extensions_mut()
                .insert(super::super::authority::WorkerAuthority::ActorJournal(
                    scope.clone(),
                ));
            Ok(req)
        };
        let service = WorkerServiceServer::from_arc(service);
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .layer(tonic::service::InterceptorLayer::new(auth))
                .add_service(service)
                .serve_with_incoming_shutdown(Box::pin(incoming), async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let client = WorkerServiceClient::connect(format!("http://{address}"))
            .await
            .unwrap();
        Self {
            address: format!("http://{address}"),
            client,
            shutdown: Some(shutdown),
            task,
        }
    }
}

impl Drop for ScopedServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

async fn journal(service: &WorkerServiceImpl, org_id: i64, text: &str) -> SessionId {
    let session = service
        .db
        .create_session(crate::storage::test_database::test_session_row(org_id))
        .await
        .unwrap()
        .id;
    service
        .event_service
        .emit(EventRequest::new(
            session,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant(text)),
        ))
        .await
        .unwrap();
    service
        .db
        .mark_session_actor_runnable(session, org_id, 1)
        .await
        .unwrap();
    session
}

async fn leased_scope(
    service: &WorkerServiceImpl,
    session: SessionId,
    org_id: i64,
) -> ActorJournalScope {
    let leases = service
        .db
        .lease_session_actors("executor-a", 100, Duration::from_secs(60), 32)
        .await
        .unwrap();
    let lease = leases
        .iter()
        .find(|lease| lease.session_id == session)
        .unwrap();
    super::journal_grants::seed_scope(&service.db, org_id, session, "executor-a", lease.fence).await
}

fn read(session: SessionId) -> Request<proto::ReadActorJournalRequest> {
    let mut req = Request::new(proto::ReadActorJournalRequest {
        session_id: Some(uuid_to_proto_uuid(session.uuid())),
        after_sequence: None,
        limit: 1,
    });
    req.set_timeout(Duration::from_secs(5));
    req
}

#[tokio::test]
async fn scoped_journal_refuses_foreign_session_even_when_the_executor_leases_both() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "tenant-a-journal").await;
    let foreign = journal(&service, 3, "tenant-b-journal").await;
    // Deliberately lease both to the same holder with the same numeric fence:
    // a holder/fence alone is not an assignment's tenant/session scope.
    let scope = leased_scope(&service, own, 2).await;
    let mut server = ScopedServer::start(service, scope).await;
    let own_page = server
        .client
        .read_actor_journal(read(own))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(own_page.events.len(), 1);
    let mut forged = read(foreign);
    forged
        .metadata_mut()
        .insert("x-everruns-org-id", "3".parse().unwrap());
    forged.metadata_mut().insert(
        "x-everruns-session-id",
        foreign.to_string().parse().unwrap(),
    );
    let error = server.client.read_actor_journal(forged).await.unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    let absent = server
        .client
        .read_actor_journal(read(SessionId::new()))
        .await
        .unwrap_err();
    assert_eq!(absent.code(), error.code());
    assert_eq!(absent.message(), error.message());
    let malformed = proto::ReadActorJournalRequest {
        session_id: Some(proto::Uuid {
            value: "events.>".into(),
        }),
        ..Default::default()
    };
    assert_eq!(
        server
            .client
            .read_actor_journal(malformed)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    // Refused selectors and forged metadata cannot change the server's scope.
    assert_eq!(
        server
            .client
            .read_actor_journal(read(own))
            .await
            .unwrap()
            .into_inner()
            .events
            .len(),
        1
    );
}

#[tokio::test]
async fn scoped_journal_rechecks_expiry_and_generation_after_reconnect() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "own-journal").await;
    let scope = leased_scope(&service, own, 2).await;
    let old_fence = scope.fence;
    let db = service.db.clone();
    let mut server = ScopedServer::start(service, scope).await;
    server.client.read_actor_journal(read(own)).await.unwrap();
    sqlx::query("UPDATE session_actors SET lease_expires_at = clock_timestamp() - INTERVAL '1 second' WHERE session_id = $1")
        .bind(own).execute(db.database().pool()).await.unwrap();
    let expired = server
        .client
        .read_actor_journal(read(own))
        .await
        .unwrap_err();
    assert_eq!(expired.code(), tonic::Code::PermissionDenied);
    let leases = db
        .lease_session_actors("executor-a", 100, Duration::from_secs(60), 32)
        .await
        .unwrap();
    assert!(
        leases
            .iter()
            .any(|lease| lease.session_id == own && lease.fence > old_fence)
    );
    // Even the same holder cannot reuse a previous generation on a new channel.
    let mut reconnected = WorkerServiceClient::connect(server.address.clone())
        .await
        .unwrap();
    let stale = reconnected.read_actor_journal(read(own)).await.unwrap_err();
    assert_eq!(stale.code(), tonic::Code::PermissionDenied);
    assert_eq!(stale.message(), expired.message());
}

#[tokio::test]
async fn scoped_journal_rechecks_release_without_waiting_for_expiry() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "own-journal").await;
    let scope = leased_scope(&service, own, 2).await;
    let fence = scope.fence;
    let db = service.db.clone();
    let mut server = ScopedServer::start(service, scope).await;
    server.client.read_actor_journal(read(own)).await.unwrap();
    assert!(
        db.release_session_actor(own, "executor-a", fence, 0)
            .await
            .unwrap()
            .is_some()
    );
    let released = server
        .client
        .read_actor_journal(read(own))
        .await
        .unwrap_err();
    assert_eq!(released.code(), tonic::Code::PermissionDenied);
}

#[tokio::test]
async fn scoped_journal_refuses_a_scope_whose_org_disagrees_with_the_session() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "own-journal").await;
    let actual = leased_scope(&service, own, 2).await;
    let db = service.db.clone();
    let inconsistent =
        super::journal_grants::seed_scope(&db, 3, own, &actual.holder, actual.fence).await;
    let mut server = ScopedServer::start(service, inconsistent).await;
    assert_eq!(
        server
            .client
            .read_actor_journal(read(own))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    // Even a corrupt actor index cannot override the session's actual owner.
    sqlx::query("UPDATE session_actors SET org_id = 3 WHERE session_id = $1")
        .bind(own)
        .execute(db.database().pool())
        .await
        .unwrap();
    assert_eq!(
        server
            .client
            .read_actor_journal(read(own))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn scoped_journal_refuses_another_holder_with_the_current_fence() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "own-journal").await;
    let actual = leased_scope(&service, own, 2).await;
    let wrong_holder =
        super::journal_grants::seed_scope(&service.db, 2, own, "another-executor", actual.fence)
            .await;
    let mut server = ScopedServer::start(service, wrong_holder).await;
    assert_eq!(
        server
            .client
            .read_actor_journal(read(own))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn scoped_journal_does_not_fall_back_when_online_admission_is_unavailable() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "own-journal").await;
    let scope = leased_scope(&service, own, 2).await;
    let db = service.db.clone();
    let mut server = ScopedServer::start(service, scope.clone()).await;
    server.client.read_actor_journal(read(own)).await.unwrap();
    db.database().pool().close().await;
    // The grant admission remains independently fail-closed; wire calls now
    // stop at the earlier, uncached ingress-floor lookup.
    let grant_failure = scope.admit_read(&db, own).await.unwrap_err();
    assert_eq!(grant_failure.code(), tonic::Code::Internal);
    assert_eq!(
        grant_failure.message(),
        "Failed to authorize worker assignment"
    );
    let unavailable = server
        .client
        .read_actor_journal(read(own))
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), tonic::Code::Internal);
    assert_eq!(unavailable.message(), "Failed to validate worker ingress");
}

#[tokio::test]
async fn scoped_reader_cannot_publish_claim_work_fetch_credentials_or_dispatch_commands() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "own-journal").await;
    let scope = leased_scope(&service, own, 2).await;
    let foreign = journal(&service, 3, "foreign-journal").await;
    let db = service.db.clone();
    let events = service.event_service.clone();
    let mut server = ScopedServer::start(service, scope).await;
    for session in [own, foreign] {
        let event = EventRequest::new(
            session,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant("forged-output")),
        );
        let request = proto::EmitEventRequest {
            event: Some(everruns_internal_protocol::schema_event_request_to_proto(
                &event,
            )),
            actor_fence: None,
        };
        assert_eq!(
            server
                .client
                .emit_event(request.clone())
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
        assert_eq!(
            server
                .client
                .emit_event_stream(tokio_stream::iter([request]))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
    }
    assert_eq!(
        server
            .client
            .lease_actors(proto::LeaseActorsRequest {
                holder: "executor-a".into(),
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        server
            .client
            .get_default_provider_credentials(GetDefaultProviderCredentialsRequest {
                org_id: 3,
                provider_type: "openai".into(),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        server
            .client
            .execute_command(ExecuteCommandRequest::default())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        server
            .client
            .commit_exec(CommitExecRequest::default())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert!(
        db.get_session_actor(foreign)
            .await
            .unwrap()
            .unwrap()
            .lease_holder
            .is_none()
    );
    for session in [own, foreign] {
        assert_eq!(
            events
                .list(session.uuid(), None, None, &[], &[], None, None)
                .await
                .unwrap()
                .len(),
            1,
            "no refused unary or stream publish reaches storage"
        );
    }
}
