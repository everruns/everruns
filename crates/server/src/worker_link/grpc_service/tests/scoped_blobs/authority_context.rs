//! Faulty trusted adapters must not turn lost context into legacy authority.

use super::*;
use crate::api::internal_blobs::{AppState, BlobKind, BlobLinkSigner};

fn read(session: SessionId) -> Request<proto::ReadActorJournalRequest> {
    let mut request = Request::new(proto::ReadActorJournalRequest {
        session_id: Some(uuid_to_proto_uuid(session.uuid())),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {TEST_BLOB_SECRET}").parse().unwrap(),
    );
    request
        .metadata_mut()
        .insert("x-everruns-role", "legacy".parse().unwrap());
    request
        .metadata_mut()
        .insert("x-everruns-org-id", "2".parse().unwrap());
    request.set_timeout(Duration::from_secs(5));
    request
}

async fn losing_context(service: WorkerServiceImpl) -> CombinedServer {
    let mut bearer = GrpcAuthInterceptor::new(Some(TEST_BLOB_SECRET.into()));
    CombinedServer::start_with_auth(service, move |request: Request<()>| {
        let mut request = bearer.call(request)?;
        request.extensions_mut().clear();
        Ok(request)
    })
    .await
}

#[tokio::test]
async fn lost_context_refuses_own_and_foreign_journals() {
    let (service, db, _scope, own) = fixture().await;
    let foreign = db
        .create_session(crate::storage::test_database::test_session_row(2))
        .await
        .unwrap()
        .id;
    service
        .event_service
        .emit(EventRequest::new(
            foreign,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant("foreign journal")),
        ))
        .await
        .unwrap();
    let server = losing_context(service).await;
    let mut client = WorkerServiceClient::new(server.channel.clone());
    for session in [own, foreign] {
        assert_eq!(
            client
                .read_actor_journal(read(session))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated,
            "a valid bearer or forged role headers cannot replace lost server authority"
        );
    }
}

#[tokio::test]
async fn lost_context_refuses_valid_signed_paths_and_uploads() {
    let (service, db, _scope, _) = fixture().await;
    let path = BlobLinkSigner::new(TEST_BLOB_SECRET).path(BlobKind::Image, image(&db, 2).await, 2);
    let before = image_count(&db, 2).await;
    let server = losing_context(service).await;
    assert_eq!(
        server
            .send("GET", &path, vec![], Some(TEST_BLOB_SECRET))
            .await
            .status(),
        401
    );
    assert_eq!(
        server
            .send(
                "POST",
                "/internal/blobs/images?org_id=2",
                upload(),
                Some(TEST_BLOB_SECRET)
            )
            .await
            .status(),
        401
    );
    assert_eq!(image_count(&db, 2).await, before);
}

#[tokio::test]
async fn missing_context_refuses_upload_before_body_or_storage() {
    use axum::body::Body;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Poll;
    use tower::ServiceExt;

    let (_service, db, _scope, _) = fixture().await;
    let app = crate::api::internal_blobs::routes(AppState {
        db: db.clone(),
        signing_secret: TEST_BLOB_SECRET.into(),
    });
    db.database().pool().close().await;
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = polls.clone();
    let stream = futures::stream::poll_fn(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        Poll::<Option<Result<Bytes, std::convert::Infallible>>>::Pending
    });
    let request = http::Request::post("/internal/blobs/images?org_id=2")
        .body(Body::from_stream(stream))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_millis(250), app.oneshot(request))
        .await
        .expect("missing authority must be refused before body polling")
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_obsolete_scope_extension_does_not_become_legacy_authority() {
    let (service, _db, scope, own) = fixture().await;
    let mut bearer = GrpcAuthInterceptor::new(Some(TEST_BLOB_SECRET.into()));
    let server = CombinedServer::start_with_auth(service, move |request: Request<()>| {
        let mut request = bearer.call(request)?;
        request.extensions_mut().insert(scope.clone());
        Ok(request)
    })
    .await;
    let mut client = WorkerServiceClient::new(server.channel.clone());
    assert_eq!(
        client
            .read_actor_journal(read(own))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated,
        "a stale adapter must fail closed rather than retain broad legacy authority"
    );
    assert_eq!(
        server
            .send(
                "POST",
                "/internal/blobs/images?org_id=2",
                upload(),
                Some(TEST_BLOB_SECRET)
            )
            .await
            .status(),
        401
    );
}

#[tokio::test]
async fn lost_context_refuses_streaming_ingress_without_waiting_for_eof() {
    let (service, _db, _scope, _) = fixture().await;
    let server = losing_context(service).await;
    let mut client = WorkerServiceClient::new(server.channel.clone());
    let mut request = Request::new(futures::stream::pending::<proto::EmitEventRequest>());
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {TEST_BLOB_SECRET}").parse().unwrap(),
    );
    let status = tokio::time::timeout(Duration::from_secs(5), client.emit_event_stream(request))
        .await
        .expect("authority refusal must not await an unending event stream")
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn public_rpc_without_an_auth_adapter_refuses_before_domain_validation() {
    let service = test_worker_service().await;
    let status = service
        .execute_command(Request::new(proto::ExecuteCommandRequest::default()))
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::Unauthenticated);
}
