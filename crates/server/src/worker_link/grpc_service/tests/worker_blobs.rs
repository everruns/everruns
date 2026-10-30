//! Image and file bytes reach a remote worker as signed HTTP paths on the
//! worker endpoint, never inside gRPC messages. End to end: a real worker
//! client resolves rows stored by the control plane and fetches their bytes.

use super::*;
use base64::Engine;
use everruns_worker::grpc_adapters::{GrpcClient, GrpcOrgAdapter};
use uuid::Uuid;

const ORG: i64 = everruns_core::DEFAULT_ORG_ID;

fn worker_service() -> (WorkerServiceImpl, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::test_database());
    let host_composition =
        crate::oss_host_composition_for_grade(everruns_core::DeploymentGrade::Dev);
    let event_service =
        EventService::with_listeners(db.clone(), crate::EventDelivery::in_memory(), vec![]);
    let service = WorkerServiceImpl::new(event_service, db.clone(), None, None, host_composition);
    (service, db)
}

async fn store_image(db: &StorageBackend, data: &[u8]) -> Uuid {
    let row = db
        .create_image(
            ORG,
            crate::storage::CreateImageRow {
                org_id: ORG,
                filename: "shot.png".into(),
                content_type: "image/png".into(),
                size_bytes: data.len() as i64,
                data: data.to_vec(),
                thumbnail_data: None,
                thumbnail_content_type: None,
                metadata: serde_json::json!({"source": "test"}),
            },
        )
        .await
        .unwrap();
    row.id.uuid()
}

async fn store_file(db: &StorageBackend, data: &[u8]) -> Uuid {
    let row = db
        .create_file(
            ORG,
            crate::storage::CreateFileRow {
                org_id: ORG,
                filename: Some("doc.pdf".into()),
                content_type: "application/pdf".into(),
                size_bytes: data.len() as i64,
                data: data.to_vec(),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
    row.id.uuid()
}

#[tokio::test]
async fn worker_fetches_image_and_file_bytes_over_the_worker_endpoint() {
    let (service, db) = worker_service();
    // Larger than one HTTP/2 data frame, so the body really streams.
    let image_bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let file_bytes = b"%PDF-1.7 test".to_vec();
    let image_id = store_image(&db, &image_bytes).await;
    let file_id = store_file(&db, &file_bytes).await;
    let missing = Uuid::new_v4();

    let (addr, shutdown, server) = start_grpc_test_server(service).await;
    let client = GrpcClient::connect(&addr).await.unwrap();
    let adapter = GrpcOrgAdapter::new(client.clone(), ORG);

    let images = adapter
        .resolve_images_batch(&[image_id, missing])
        .await
        .unwrap();
    assert_eq!(images.len(), 1);
    let image = &images[&image_id];
    assert_eq!(image.media_type, "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&image.base64)
            .unwrap(),
        image_bytes
    );

    let files = adapter.resolve_files_batch(&[file_id]).await.unwrap();
    let file = &files[&file_id];
    assert_eq!(file.media_type, "application/pdf");
    assert_eq!(file.filename.as_deref(), Some("doc.pdf"));
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&file.base64)
            .unwrap(),
        file_bytes
    );

    let artifact = client
        .get_image_artifact(
            ORG,
            everruns_contracts::typed_id::ImageId::from_uuid(image_id),
        )
        .await
        .unwrap()
        .expect("stored image");
    assert_eq!(artifact.data, image_bytes);
    assert_eq!(artifact.info.filename, "shot.png");

    // Another org sees nothing, so it is handed no path either.
    let other_org = GrpcOrgAdapter::new(client.clone(), ORG + 1_000_000);
    assert!(
        other_org
            .resolve_images_batch(&[image_id])
            .await
            .unwrap()
            .is_empty()
    );

    let _ = shutdown.send(());
    server.await.unwrap();
}

#[tokio::test]
async fn byte_paths_refuse_tampering() {
    let (service, db) = worker_service();
    let image_id = store_image(&db, b"png").await;
    let file_id = store_file(&db, b"pdf").await;
    let (addr, shutdown, server) = start_grpc_test_server(service).await;

    let signer = crate::api::internal_blobs::BlobLinkSigner::new(TEST_BLOB_SECRET);
    let good = signer.path(crate::api::internal_blobs::BlobKind::Image, image_id, ORG);
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let get = |path: String| {
        use tonic::codegen::Service as _;
        let mut channel = channel.clone();
        async move {
            std::future::poll_fn(|cx| channel.poll_ready(cx))
                .await
                .unwrap();
            let request = tonic::codegen::http::Request::get(path)
                .body(tonic::body::Body::empty())
                .unwrap();
            channel.call(request).await.unwrap().status().as_u16()
        }
    };

    assert_eq!(get(good.clone()).await, 200);
    // Same signature, other org: refused before any lookup.
    let other_org = good.replace(&format!("org_id={ORG}"), &format!("org_id={}", ORG + 1));
    assert_eq!(get(other_org).await, 403);
    // An image signature does not open a file with a swapped path.
    let as_file = good
        .replace("/images/", "/files/")
        .replace(&image_id.to_string(), &file_id.to_string());
    assert_eq!(get(as_file).await, 403);
    // A path signed with another secret.
    let foreign = crate::api::internal_blobs::BlobLinkSigner::new("other").path(
        crate::api::internal_blobs::BlobKind::Image,
        image_id,
        ORG,
    );
    assert_eq!(get(foreign).await, 403);

    let _ = shutdown.send(());
    server.await.unwrap();
}

#[tokio::test]
async fn worker_uploads_a_generated_image_over_the_worker_endpoint() {
    use everruns_contracts::runtime::image_services::{CreateStoredImage, ImageArtifactStore};
    let (service, db) = worker_service();
    let (addr, shutdown, server) = start_grpc_test_server(service).await;
    let client = GrpcClient::connect(&addr).await.unwrap();
    let adapter = GrpcOrgAdapter::new(client, ORG);
    // Larger than one HTTP/2 data frame, and with newlines in both the
    // metadata and the bytes.
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let metadata = serde_json::json!({"prompt": "a cat\nin two lines", "model": "gpt-image"});

    let stored = adapter
        .create_image(CreateStoredImage {
            filename: "cat.png".into(),
            content_type: "image/png".into(),
            data: data.clone(),
            metadata: metadata.clone(),
        })
        .await
        .unwrap();

    assert_eq!(stored.filename, "cat.png");
    assert_eq!(stored.size_bytes, data.len() as i64);
    assert_eq!(stored.metadata, metadata);
    let row = db
        .get_image(ORG, stored.id.uuid())
        .await
        .unwrap()
        .expect("image row");
    assert_eq!(row.data, data);
    assert_eq!(row.content_type, "image/png");

    let _ = shutdown.send(());
    server.await.unwrap();
}
