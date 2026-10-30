//! Real combined-listener scope tests. The fixed server-side scope adapter is
//! synthetic; no production identity or credential issuer is enabled.

mod authority_context;
mod protection_floor;
mod tests;

use super::super::assignment_scope::ActorJournalScope;
use super::*;
use bytes::Bytes;
use everruns_contracts::typed_id::SessionId;
use everruns_core::{EventContext, EventRequest, OutputMessageCompletedData, RuntimeMessage};
use everruns_internal_protocol::{WorkerServiceClient, blob_wire, uuid_to_proto_uuid};
use http_body_util::BodyExt;
use std::time::Duration;
use tonic::codegen::{Service as _, http};
use tonic::transport::Channel;

struct CombinedServer {
    channel: Channel,
    url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl CombinedServer {
    async fn start(service: WorkerServiceImpl, scope: Option<ActorJournalScope>) -> Self {
        let mut bearer = GrpcAuthInterceptor::new(Some(TEST_BLOB_SECRET.into()));
        let auth = move |request: Request<()>| {
            let mut request = bearer.call(request)?;
            if let Some(scope) = &scope {
                request.extensions_mut().insert(
                    super::super::authority::WorkerAuthority::ActorJournal(scope.clone()),
                );
            }
            Ok(request)
        };
        Self::start_with_auth(service, auth).await
    }

    async fn start_with_auth<I>(service: WorkerServiceImpl, auth: I) -> Self
    where
        I: tonic::service::Interceptor + Clone + Send + Sync + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let incoming = futures::stream::unfold(listener, |listener| async move {
            let socket = listener.accept().await.map(|(socket, _)| socket);
            Some((socket, listener))
        });
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .layer(tonic::service::interceptor::InterceptorLayer::new(auth))
                .add_routes(service.into_routes(TEST_BLOB_SECRET.into()))
                .serve_with_incoming_shutdown(Box::pin(incoming), async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        Self {
            channel,
            url: format!("http://{address}"),
            shutdown: Some(shutdown),
            task,
        }
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        data: Vec<u8>,
        token: Option<&str>,
    ) -> http::Response<tonic::body::Body> {
        let mut channel = self.channel.clone();
        std::future::poll_fn(|cx| channel.poll_ready(cx))
            .await
            .unwrap();
        let mut request = http::Request::builder()
            .method(method)
            .uri(path)
            .header("x-everruns-org-id", "2")
            .header("x-everruns-role", "legacy");
        if let Some(token) = token {
            request = request.header(http::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = request
            .body(tonic::body::Body::new(http_body_util::Full::new(
                Bytes::from(data),
            )))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), channel.call(request))
            .await
            .unwrap()
            .unwrap()
    }
}

impl Drop for CombinedServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

async fn fixture() -> (
    WorkerServiceImpl,
    Arc<StorageBackend>,
    ActorJournalScope,
    SessionId,
) {
    let service = test_worker_service().await;
    let db = service.db.clone();
    let own = db.create_test_session().await;
    service
        .event_service
        .emit(EventRequest::new(
            own,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant("own journal")),
        ))
        .await
        .unwrap();
    db.mark_session_actor_runnable(own, 1, 1).await.unwrap();
    let lease = db
        .lease_session_actors("reader-a", 1, Duration::from_secs(60), 32)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(lease.session_id, own);
    let scope = super::journal_grants::seed_scope(&db, 1, own, "reader-a", lease.fence).await;
    (service, db, scope, own)
}

async fn image(db: &StorageBackend, org_id: i64) -> uuid::Uuid {
    db.create_image(
        org_id,
        crate::storage::CreateImageRow {
            org_id,
            filename: "fixture.png".into(),
            content_type: "image/png".into(),
            size_bytes: 5,
            data: b"image".to_vec(),
            thumbnail_data: None,
            thumbnail_content_type: None,
            metadata: serde_json::json!({}),
        },
    )
    .await
    .unwrap()
    .id
    .uuid()
}

fn upload() -> Vec<u8> {
    blob_wire::encode_image_upload(
        &blob_wire::ImageUploadHead {
            filename: "uploaded.png".into(),
            content_type: "image/png".into(),
            metadata: serde_json::json!({}),
        },
        b"upload",
    )
    .unwrap()
}

async fn image_count(db: &StorageBackend, org_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM images WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(db.database().pool())
        .await
        .unwrap()
}
