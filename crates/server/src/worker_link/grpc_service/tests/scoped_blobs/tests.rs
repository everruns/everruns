use super::*;
use crate::api::internal_blobs::{AppState, BlobKind, BlobLinkSigner};

#[tokio::test]
async fn scoped_reader_cannot_fetch_even_valid_legacy_signed_paths() {
    let (service, db, scope, own) = fixture().await;
    let own_image = image(&db, 1).await;
    let foreign_image = image(&db, 2).await;
    let foreign_session = db
        .create_session(crate::storage::test_database::test_session_row(2))
        .await
        .unwrap()
        .id;
    service
        .event_service
        .emit(EventRequest::new(
            foreign_session,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant("foreign journal")),
        ))
        .await
        .unwrap();
    let foreign_file = db
        .create_file(
            2,
            crate::storage::CreateFileRow {
                org_id: 2,
                filename: Some("foreign.txt".into()),
                content_type: "text/plain".into(),
                size_bytes: 13,
                data: b"foreign-bytes".to_vec(),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .id
        .uuid();
    let signer = BlobLinkSigner::new(TEST_BLOB_SECRET);
    let server = CombinedServer::start(service, Some(scope)).await;
    // The scope survives the actual outer interceptor and combined router.
    let mut client = WorkerServiceClient::new(server.channel.clone());
    let mut read = Request::new(proto::ReadActorJournalRequest {
        session_id: Some(uuid_to_proto_uuid(own.uuid())),
        ..Default::default()
    });
    read.metadata_mut().insert(
        "authorization",
        format!("Bearer {TEST_BLOB_SECRET}").parse().unwrap(),
    );
    read.set_timeout(Duration::from_secs(5));
    assert_eq!(
        client
            .read_actor_journal(read)
            .await
            .unwrap()
            .into_inner()
            .events
            .len(),
        1
    );
    let mut foreign_read = Request::new(proto::ReadActorJournalRequest {
        session_id: Some(uuid_to_proto_uuid(foreign_session.uuid())),
        ..Default::default()
    });
    foreign_read.metadata_mut().insert(
        "authorization",
        format!("Bearer {TEST_BLOB_SECRET}").parse().unwrap(),
    );
    foreign_read
        .metadata_mut()
        .insert("x-everruns-role", "legacy".parse().unwrap());
    foreign_read.set_timeout(Duration::from_secs(5));
    assert_eq!(
        client
            .read_actor_journal(foreign_read)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied,
        "the outer layer must preserve assignment scope into gRPC as well as HTTP"
    );
    for (kind, id, org_id) in [
        (BlobKind::Image, foreign_image, 2),
        (BlobKind::File, foreign_file, 2),
        (BlobKind::Image, own_image, 1),
    ] {
        let path = signer.path(kind, id, org_id);
        let response = server
            .send("GET", &path, vec![], Some(TEST_BLOB_SECRET))
            .await;
        assert_eq!(
            response.status(),
            403,
            "a valid legacy signature cannot authorize a journal-only scope"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!body.windows(5).any(|value| value == b"image"));
        assert!(!body.windows(13).any(|value| value == b"foreign-bytes"));
    }
}

#[tokio::test]
async fn scoped_reader_cannot_upload_to_own_or_foreign_org() {
    let (service, db, scope, _) = fixture().await;
    let server = CombinedServer::start(service, Some(scope)).await;
    for org_id in [2, 1] {
        let before = image_count(&db, org_id).await;
        let path = format!("{}?org_id={org_id}", blob_wire::IMAGE_UPLOAD_PATH);
        let response = server
            .send("POST", &path, upload(), Some(TEST_BLOB_SECRET))
            .await;
        assert_eq!(response.status(), 403);
        assert_eq!(
            image_count(&db, org_id).await,
            before,
            "refused upload stores nothing"
        );
    }
}

#[tokio::test]
async fn scoped_upload_is_denied_before_body_buffering() {
    use axum::body::Body;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Poll;
    use tower::ServiceExt;

    let (_service, db, scope, _) = fixture().await;
    let app = crate::api::internal_blobs::routes(AppState {
        db: db.clone(),
        signing_secret: TEST_BLOB_SECRET.into(),
    })
    .layer(axum::Extension(
        super::super::super::authority::WorkerAuthority::ActorJournal(scope),
    ));
    for oversized in [false, true] {
        let polls = Arc::new(AtomicUsize::new(0));
        let observed = polls.clone();
        let stream = futures::stream::poll_fn(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Poll::<Option<Result<Bytes, std::convert::Infallible>>>::Pending
        });
        let mut request = http::Request::post(format!("{}?org_id=2", blob_wire::IMAGE_UPLOAD_PATH));
        if oversized {
            request = request.header(
                http::header::CONTENT_LENGTH,
                crate::api::images::MAX_IMAGE_SIZE + 2 * 1024 * 1024,
            );
        }
        let request = request.body(Body::from_stream(stream)).unwrap();
        let response =
            tokio::time::timeout(Duration::from_millis(250), app.clone().oneshot(request))
                .await
                .expect("authority refusal must not wait for a stalled body")
                .unwrap();
        assert_eq!(response.status(), 403);
        assert_eq!(
            polls.load(Ordering::SeqCst),
            0,
            "no body frame may be polled"
        );
    }
    assert_eq!(image_count(&db, 2).await, 0);
}

#[tokio::test]
async fn combined_listener_preserves_legacy_bearer_signature_and_upload_behavior() {
    let (service, db, _scope, _) = fixture().await;
    let id = image(&db, 1).await;
    let path = BlobLinkSigner::new(TEST_BLOB_SECRET).path(BlobKind::Image, id, 1);
    let server = CombinedServer::start(service, None).await;
    for token in [None, Some("incorrect-fixture-token")] {
        let response = server.send("GET", &path, vec![], token).await;
        assert_eq!(response.headers().get("grpc-status").unwrap(), "16");
    }
    let response = server
        .send("GET", &path, vec![], Some(TEST_BLOB_SECRET))
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "image"
    );
    let response = server
        .send(
            "POST",
            &format!("{}?org_id=1", blob_wire::IMAGE_UPLOAD_PATH),
            upload(),
            Some(TEST_BLOB_SECRET),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(image_count(&db, 1).await, 2);
    assert_eq!(
        server
            .send("GET", &format!("{path}x"), vec![], Some(TEST_BLOB_SECRET))
            .await
            .status(),
        403,
        "legacy signatures are still checked"
    );
}

#[tokio::test]
async fn scoped_refusal_precedes_path_query_and_upload_decoding() {
    let (service, _db, scope, _) = fixture().await;
    let server = CombinedServer::start(service, Some(scope)).await;
    for (method, path) in [
        ("GET", "/internal/blobs/images/not-a-uuid?org_id=2"),
        (
            "GET",
            "/internal/blobs/unknown/00000000-0000-0000-0000-000000000001?org_id=bad",
        ),
        ("POST", "/internal/blobs/images?org_id=bad"),
        ("POST", "/internal/blobs/images?org_id=2"),
    ] {
        let response = server
            .send(method, path, b"malformed".to_vec(), Some(TEST_BLOB_SECRET))
            .await;
        assert_eq!(response.status(), 403);
    }
}

#[tokio::test]
async fn scoped_refusal_does_not_access_blob_storage() {
    let (service, db, scope, _) = fixture().await;
    let path = BlobLinkSigner::new(TEST_BLOB_SECRET).path(BlobKind::Image, image(&db, 2).await, 2);
    let server = CombinedServer::start(service, Some(scope)).await;
    db.database().pool().close().await;
    assert_eq!(
        server
            .send("GET", &path, vec![], Some(TEST_BLOB_SECRET))
            .await
            .status(),
        403
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
        403
    );
}

#[tokio::test]
async fn combined_listener_refuses_a_stalled_upload_without_awaiting_its_body() {
    use std::task::Poll;
    let (service, _db, scope, _) = fixture().await;
    let server = CombinedServer::start(service, Some(scope)).await;
    let mut channel = server.channel.clone();
    std::future::poll_fn(|cx| channel.poll_ready(cx))
        .await
        .unwrap();
    let stream = futures::stream::poll_fn(|_| {
        Poll::<Option<Result<Bytes, std::convert::Infallible>>>::Pending
    });
    let request = http::Request::post("/internal/blobs/images?org_id=2")
        .header(
            http::header::AUTHORIZATION,
            format!("Bearer {TEST_BLOB_SECRET}"),
        )
        .body(tonic::body::Body::new(axum::body::Body::from_stream(
            stream,
        )))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), channel.call(request))
        .await
        .expect("scope refusal arrives without a request body")
        .unwrap();
    assert_eq!(response.status(), 403);
}
