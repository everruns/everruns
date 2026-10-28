use super::*;
use crate::api::internal_blobs::{BlobKind, BlobLinkSigner};

async fn protect(db: &StorageBackend) {
    sqlx::query("INSERT INTO worker_ingress_protection_floor (minimum_version) VALUES (1)")
        .execute(db.database().pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn legacy_read_and_signed_url_cannot_enter_a_protected_worker_plane() {
    let (service, db, _, session) = fixture().await;
    let image = image(&db, 2).await;
    let signed = BlobLinkSigner::new(TEST_BLOB_SECRET).path(BlobKind::Image, image, 2);
    let server = CombinedServer::start(service, None).await;
    let mut client = WorkerServiceClient::new(server.channel.clone());
    let request = || {
        let mut r = Request::new(proto::GetSessionRequest {
            org_id: 1,
            session_id: Some(uuid_to_proto_uuid(session.uuid())),
        });
        r.metadata_mut().insert(
            "authorization",
            format!("Bearer {TEST_BLOB_SECRET}").parse().unwrap(),
        );
        r
    };
    client.get_session(request()).await.unwrap();
    assert_eq!(
        server
            .send("GET", &signed, vec![], Some(TEST_BLOB_SECRET))
            .await
            .status(),
        http::StatusCode::OK
    );
    protect(&db).await;
    assert_eq!(
        client.get_session(request()).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        server
            .send("GET", &signed, vec![], Some(TEST_BLOB_SECRET))
            .await
            .status(),
        http::StatusCode::FORBIDDEN
    );
}

mod streams;

fn legacy<T>(message: T) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {TEST_BLOB_SECRET}").parse().unwrap(),
    );
    request
        .metadata_mut()
        .insert("x-everruns-org-id", "3".parse().unwrap());
    request
        .metadata_mut()
        .insert("x-everruns-role", "verified".parse().unwrap());
    request
}

#[tokio::test]
async fn protected_legacy_gate_refuses_global_rpcs_spoofed_selectors_and_reconnects() {
    let (service, db, _, session) = fixture().await;
    let foreign = db
        .create_session(crate::storage::test_database::test_session_row(2))
        .await
        .unwrap()
        .id;
    let mut server = CombinedServer::start(service, None).await;
    protect(&db).await;
    for reconnect in [false, true] {
        if reconnect {
            server.channel = tonic::transport::Endpoint::from_shared(server.url.clone())
                .unwrap()
                .connect()
                .await
                .unwrap();
        }
        let mut client = WorkerServiceClient::new(server.channel.clone());
        for (org, id) in [
            (1, Some(session)),
            (2, Some(foreign)),
            (3, Some(session)),
            (999, None),
        ] {
            assert_eq!(
                client
                    .get_session(legacy(proto::GetSessionRequest {
                        org_id: org,
                        session_id: id.map(|s| uuid_to_proto_uuid(s.uuid()))
                    }))
                    .await
                    .unwrap_err()
                    .code(),
                tonic::Code::PermissionDenied
            );
        }
        assert_eq!(
            client
                .get_default_provider_credentials(legacy(
                    proto::GetDefaultProviderCredentialsRequest {
                        org_id: 2,
                        ..Default::default()
                    }
                ))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
        assert_eq!(
            client
                .claim_durable_tasks(legacy(proto::ClaimDurableTasksRequest::default()))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
        assert_eq!(
            client
                .execute_command(legacy(proto::ExecuteCommandRequest::default()))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
        assert_eq!(
            client
                .lease_actors(legacy(proto::LeaseActorsRequest {
                    holder: "broad-worker".into(),
                    limit: 100,
                    ttl_ms: 60000,
                    max_leased_per_org: 0
                }))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
        let status = tokio::time::timeout(
            Duration::from_secs(3),
            client.emit_event_stream(legacy(futures::stream::pending::<proto::EmitEventRequest>())),
        )
        .await
        .expect("must refuse without draining messages")
        .unwrap_err();
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
    }
    // Public trait calls are also gated; transport middleware is not the only boundary.
    let service = test_worker_service().await;
    protect(&service.db).await;
    let mut req = Request::new(proto::GetSessionRequest {
        org_id: 1,
        session_id: None,
    });
    req.extensions_mut()
        .insert(crate::worker_link::grpc_service::authority::WorkerAuthority::LegacyWorker);
    assert_eq!(
        service.get_session(req).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn protection_refuses_development_and_unverified_selector_only_authority() {
    for selector_only in [false, true] {
        let (service, db, scope, session) = fixture().await;
        protect(&db).await;
        let server = if selector_only {
            CombinedServer::start(service, Some(scope)).await
        } else {
            CombinedServer::start_with_auth(service, GrpcAuthInterceptor::new(None)).await
        };
        let mut client = WorkerServiceClient::new(server.channel.clone());
        let req = legacy(proto::ReadActorJournalRequest {
            session_id: Some(uuid_to_proto_uuid(session.uuid())),
            after_sequence: Some(0),
            limit: 10,
        });
        assert_eq!(
            client.read_actor_journal(req).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
    }
}

#[tokio::test]
async fn protected_blob_plane_denies_reads_and_refuses_stalled_uploads() {
    use std::task::Poll;
    let (service, db, _, _) = fixture().await;
    let server = CombinedServer::start(service, None).await;
    protect(&db).await;
    for path in [
        "/internal/blobs/images/not-a-uuid?org_id=not-a-number",
        "/internal/blobs/files/not-a-uuid?org_id=3",
    ] {
        assert_eq!(
            server
                .send("GET", path, vec![], Some(TEST_BLOB_SECRET))
                .await
                .status(),
            400
        );
    }
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
    let response = tokio::time::timeout(Duration::from_secs(3), channel.call(request))
        .await
        .expect("floor refusal must not await upload bytes")
        .unwrap();
    assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn floor_is_empty_by_default_and_cannot_be_cleared_or_downgraded() {
    let (service, db, _, session) = fixture().await;
    assert_eq!(db.worker_ingress_minimum_version().await.unwrap(), None);
    db.require_legacy_worker_hosting().await.unwrap();
    protect(&db).await;
    assert!(db.require_legacy_worker_hosting().await.is_err());
    for sql in [
        "DELETE FROM worker_ingress_protection_floor",
        "TRUNCATE worker_ingress_protection_floor",
        "UPDATE worker_ingress_protection_floor SET minimum_version=0",
        "UPDATE worker_ingress_protection_floor SET created_at=created_at+INTERVAL '1 second'",
    ] {
        assert!(
            sqlx::query(sql)
                .execute(db.database().pool())
                .await
                .is_err()
        );
    }
    sqlx::query("UPDATE worker_ingress_protection_floor SET minimum_version=2")
        .execute(db.database().pool())
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE worker_ingress_protection_floor SET minimum_version=1")
            .execute(db.database().pool())
            .await
            .is_err()
    );
    db.delete_session(1, session).await.unwrap();
    assert_eq!(db.worker_ingress_minimum_version().await.unwrap(), Some(2));
    let server = CombinedServer::start(service, None).await;
    let mut client = WorkerServiceClient::new(server.channel.clone());
    assert_eq!(
        client
            .get_session(legacy(proto::GetSessionRequest {
                org_id: 1,
                session_id: Some(uuid_to_proto_uuid(session.uuid()))
            }))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn storage_failure_never_reopens_legacy_ingress_or_exposes_database_errors() {
    let (service, db, _, session) = fixture().await;
    let object = image(&db, 2).await;
    let signed = BlobLinkSigner::new(TEST_BLOB_SECRET).path(BlobKind::Image, object, 2);
    let server = CombinedServer::start(service, None).await;
    let mut client = WorkerServiceClient::new(server.channel.clone());
    db.database().pool().close().await;
    let status = client
        .get_session(legacy(proto::GetSessionRequest {
            org_id: 1,
            session_id: Some(uuid_to_proto_uuid(session.uuid())),
        }))
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::Internal);
    assert_eq!(status.message(), "Failed to validate worker ingress");
    assert_eq!(
        server
            .send("GET", &signed, vec![], Some(TEST_BLOB_SECRET))
            .await
            .status(),
        503
    );
    assert!(db.require_legacy_worker_hosting().await.is_err());
}

#[tokio::test]
async fn legacy_compatibility_and_refusal_mock_latency_benchmark() {
    let (service, db, _, session) = fixture().await;
    let server = CombinedServer::start(service, None).await;
    let mut client = WorkerServiceClient::new(server.channel.clone());
    let mut samples = [Vec::new(), Vec::new()];
    for (phase, times) in samples.iter_mut().enumerate() {
        if phase == 1 {
            protect(&db).await;
        }
        for _ in 0..100 {
            let start = std::time::Instant::now();
            let result = client
                .get_session(legacy(proto::GetSessionRequest {
                    org_id: 1,
                    session_id: Some(uuid_to_proto_uuid(session.uuid())),
                }))
                .await;
            times.push(start.elapsed().as_micros());
            if phase == 0 {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
            }
        }
        times.sort_unstable();
    }
    println!(
        "PROTECTED_INGRESS_BENCH samples=100 compatibility_p50_us={} compatibility_p95_us={} refusal_p50_us={} refusal_p95_us={}",
        samples[0][50], samples[0][95], samples[1][50], samples[1][95]
    );
}

#[tokio::test]
async fn opaque_capability_enrollment_cannot_omit_the_legacy_refusal_floor() {
    let (_service, db, _, session) = fixture().await;
    let grant: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM worker_journal_read_grants WHERE session_id=$1")
            .bind(session)
            .fetch_one(db.database().pool())
            .await
            .unwrap();
    let insert = "INSERT INTO worker_journal_capabilities (capability_sha256,grant_id,expires_at) SELECT $1,id,expires_at FROM worker_journal_read_grants WHERE id=$2";
    let digest = [0xc3u8; 32];
    assert!(
        sqlx::query(insert)
            .bind(digest.as_slice())
            .bind(grant)
            .execute(db.database().pool())
            .await
            .is_err()
    );
    protect(&db).await;
    sqlx::query(insert)
        .bind(digest.as_slice())
        .bind(grant)
        .execute(db.database().pool())
        .await
        .unwrap();
    sqlx::query("UPDATE worker_ingress_protection_floor SET minimum_version=2")
        .execute(db.database().pool())
        .await
        .unwrap();
    assert!(
        sqlx::query(insert)
            .bind([0xd4u8; 32].as_slice())
            .bind(grant)
            .execute(db.database().pool())
            .await
            .is_err()
    );
}
