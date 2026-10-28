//! Acknowledged server-body polling proves the RPC passed its initial gate.
use super::*;
use futures::StreamExt;
use prost::Message;

struct EventDecoder;
impl tonic::codec::Decoder for EventDecoder {
    type Item = proto::EmitEventRequest;
    type Error = tonic::Status;
    fn decode(
        &mut self,
        buf: &mut tonic::codec::DecodeBuf<'_>,
    ) -> Result<Option<Self::Item>, Self::Error> {
        proto::EmitEventRequest::decode(buf)
            .map(Some)
            .map_err(|_| tonic::Status::internal("invalid test frame"))
    }
}

#[tokio::test]
async fn legacy_stream_admitted_before_cutover_cannot_defer_event_writes_past_it() {
    let service = Arc::new(test_worker_service().await);
    let session = service.db.create_test_session().await;
    let event = EventRequest::new(
        session,
        EventContext::empty(),
        OutputMessageCompletedData::new(RuntimeMessage::assistant("must not be stored")),
    );
    let message = proto::EmitEventRequest {
        event: Some(everruns_internal_protocol::schema_event_request_to_proto(
            &event,
        )),
        actor_fence: None,
    };
    let encoded = message.encode_to_vec();
    let mut frame = vec![0];
    frame.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
    frame.extend_from_slice(&encoded);
    let (send, receive) = tokio::sync::mpsc::channel::<Result<Bytes, std::convert::Infallible>>(1);
    let (observed, polled) = tokio::sync::oneshot::channel();
    let body = futures::stream::unfold(
        (receive, Some(observed)),
        |(mut receiver, signal)| async move {
            let frame = receiver.recv().await?;
            if let Some(signal) = signal {
                let _ = signal.send(());
            }
            Some((frame, (receiver, None)))
        },
    );
    let streaming = tonic::Streaming::new_request(
        EventDecoder,
        axum::body::Body::from_stream(body),
        None,
        Some(1024 * 1024),
    );
    let mut request = Request::new(streaming);
    request
        .extensions_mut()
        .insert(crate::worker_link::grpc_service::authority::WorkerAuthority::LegacyWorker);
    let actor = service.clone();
    let call = tokio::spawn(async move { actor.emit_event_stream(request).await });
    send.send(Ok(Bytes::from(frame))).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), polled)
        .await
        .expect("initial admission must reach body polling")
        .unwrap();
    protect(&service.db).await;
    drop(send);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), call)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert!(
        service
            .db
            .read_session_actor_journal(session, Some(0), 10)
            .await
            .unwrap()
            .events
            .is_empty()
    );
}

#[tokio::test]
async fn an_open_legacy_notification_stream_cannot_yield_after_cutover() {
    use crate::live_updates::task_notifications::{TaskBroadcaster, TaskNotificationBroadcaster};
    let mut service = test_worker_service().await;
    let db = service.db.clone();
    let url = db.database().test_database().unwrap().url().to_owned();
    let broadcaster = Arc::new(TaskBroadcaster::Postgres(
        TaskNotificationBroadcaster::new(url).await,
    ));
    service.set_task_broadcaster(broadcaster.clone());
    let mut request = Request::new(proto::SubscribeTaskNotificationsRequest {
        worker_id: "legacy-stream".into(),
        activity_types: vec!["test".into()],
    });
    request
        .extensions_mut()
        .insert(crate::worker_link::grpc_service::authority::WorkerAuthority::LegacyWorker);
    let mut stream = service
        .subscribe_task_notifications(request)
        .await
        .unwrap()
        .into_inner();
    protect(&db).await;
    let status = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
    assert!(stream.next().await.is_none());
    broadcaster.shutdown().await;
}

#[tokio::test]
async fn an_upload_started_before_cutover_is_refused_before_storage_after_it() {
    use tower::ServiceExt;
    let (_service, db, _, _) = fixture().await;
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(db.database().pool())
        .await
        .unwrap();
    let router = crate::api::internal_blobs::routes(crate::api::internal_blobs::AppState {
        db: db.clone(),
        signing_secret: TEST_BLOB_SECRET.into(),
    });
    let encoded = blob_wire::encode_image_upload(
        &blob_wire::ImageUploadHead {
            content_type: "image/png".into(),
            filename: "upload.png".into(),
            metadata: serde_json::json!({}),
        },
        b"image bytes",
    )
    .unwrap();
    let (send, receive) = tokio::sync::mpsc::channel::<Result<Bytes, std::convert::Infallible>>(1);
    let (observed, polled) = tokio::sync::oneshot::channel();
    let body = futures::stream::unfold(
        (receive, Some(observed)),
        |(mut receiver, signal)| async move {
            let item = receiver.recv().await?;
            if let Some(signal) = signal {
                let _ = signal.send(());
            }
            Some((item, (receiver, None)))
        },
    );
    let mut request = http::Request::post("/internal/blobs/images?org_id=2")
        .header(
            http::header::AUTHORIZATION,
            format!("Bearer {TEST_BLOB_SECRET}"),
        )
        .body(axum::body::Body::from_stream(body))
        .unwrap();
    request
        .extensions_mut()
        .insert(crate::worker_link::grpc_service::authority::WorkerAuthority::LegacyWorker);
    let call = tokio::spawn(async move { router.oneshot(request).await.unwrap() });
    send.send(Ok(Bytes::from(encoded))).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), polled)
        .await
        .expect("upload initially passes admission")
        .unwrap();
    protect(&db).await;
    drop(send);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), call)
            .await
            .unwrap()
            .unwrap()
            .status(),
        403
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(db.database().pool())
        .await
        .unwrap();
    assert_eq!(after, before);
}
