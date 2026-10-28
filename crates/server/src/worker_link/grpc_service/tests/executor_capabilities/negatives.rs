use super::*;
use http_body_util::BodyExt;

#[tokio::test]
async fn future_routes_cannot_bypass_the_journal_purpose_even_with_grpc_content_type() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let server = Server::start(service, true, Some((CLIENT_A, KEY_A))).await;
    for (method, path, grpc) in [
        ("GET", "/future/unsafe", false),
        ("POST", "/future/unsafe", true),
        (
            "POST",
            "/everruns.internal.WorkerService/ReadActorJournalExtra",
            true,
        ),
        (
            "GET",
            "/everruns.internal.WorkerService/ReadActorJournal",
            true,
        ),
    ] {
        let mut channel = server.channel.clone();
        std::future::poll_fn(|cx| channel.poll_ready(cx))
            .await
            .unwrap();
        let mut request = http::Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", token(a.byte));
        if grpc {
            request = request.header("content-type", "application/grpc");
        }
        let response = channel
            .call(request.body(tonic::body::Body::empty()).unwrap())
            .await
            .unwrap();
        if grpc {
            assert_eq!(response.headers().get("grpc-status").unwrap(), "7");
        } else {
            assert_eq!(response.status(), 403);
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            !bytes
                .windows(19)
                .any(|value| value == b"future-route-secret")
        );
    }
    assert!(server.retained.lock().unwrap().is_none());
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap().events.len(),
        1
    );
}

#[tokio::test]
async fn verified_context_cannot_be_promoted_reconstructed_or_used_after_storage_failure() {
    use tonic::service::Interceptor as _;
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    server.read(a.session, a.byte).await.unwrap();
    let context = server.context();
    let mut future = Request::new(());
    future.extensions_mut().insert(context.clone());
    assert_eq!(
        super::super::super::authority::check_rpc(&future, "future_rpc")
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let WorkerAuthority::VerifiedJournal(scope) = &context else {
        panic!("must resolve verified proof");
    };
    for token in [None, Some(TEST_BLOB_SECRET.into())] {
        let mut auth = GrpcAuthInterceptor::new(token);
        let mut request = Request::new(());
        request.extensions_mut().insert(context.clone());
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {TEST_BLOB_SECRET}").parse().unwrap(),
        );
        assert_eq!(
            auth.call(request).unwrap_err().code(),
            tonic::Code::Unauthenticated
        );
    }
    let mut standalone = http::Extensions::new();
    standalone.insert(scope.clone());
    standalone.insert(WorkerAuthority::LegacyWorker);
    assert_eq!(
        super::super::super::authority::resolve(&standalone)
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        service
            .read_actor_journal(read(a.session, a.byte))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    service.db.database().pool().close().await;
    assert_eq!(
        retained_read(&service, context, a.session)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Internal
    );
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap_err().code(),
        tonic::Code::Internal
    );
}

#[tokio::test]
async fn verified_assignment_cannot_outlive_its_sessions_actual_tenant_binding() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    server.read(a.session, a.byte).await.unwrap();
    let context = server.context();
    // Deliberately contradict the stored grant/actor org; selectors and equal
    // holder/fence values must never replace the actual session owner.
    sqlx::query("UPDATE sessions SET org_id=3 WHERE id=$1")
        .bind(a.session)
        .execute(service.db.database().pool())
        .await
        .unwrap();
    assert_eq!(
        retained_read(&service, context, a.session)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn retained_verified_context_and_reconnect_recheck_every_revocation_boundary() {
    for boundary in ["capability", "identity", "grant", "lease"] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
        let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
        assert_eq!(
            server.read(a.session, a.byte).await.unwrap().events.len(),
            1
        );
        let context = server.context();
        let debug = format!("{context:?}");
        assert!(!debug.contains(&token(a.byte)));
        assert!(!debug.contains(&hex::encode(a.digest)));
        match boundary {
            "capability" => {
                assert!(
                    !service
                        .db
                        .revoke_worker_journal_capability(3, &a.digest)
                        .await
                        .unwrap()
                );
                assert!(
                    service
                        .db
                        .revoke_worker_journal_capability(2, &a.digest)
                        .await
                        .unwrap()
                );
                assert!(
                    !service
                        .db
                        .revoke_worker_journal_capability(2, &a.digest)
                        .await
                        .unwrap()
                );
            }
            "identity" => {
                assert!(
                    !service
                        .db
                        .revoke_worker_executor_identity(uuid::Uuid::new_v4(), &a.certificate)
                        .await
                        .unwrap()
                );
                assert!(
                    service
                        .db
                        .revoke_worker_executor_identity(a.executor, &a.certificate)
                        .await
                        .unwrap()
                );
                assert!(
                    !service
                        .db
                        .revoke_worker_executor_identity(a.executor, &a.certificate)
                        .await
                        .unwrap()
                );
            }
            "grant" => {
                assert!(
                    service
                        .db
                        .revoke_worker_journal_read_grant(2, a.grant)
                        .await
                        .unwrap()
                );
            }
            "lease" => {
                assert!(
                    service
                        .db
                        .release_session_actor(a.session, "equal-holder", 1, 0)
                        .await
                        .unwrap()
                        .is_some()
                );
                let replacement = service
                    .db
                    .lease_session_actors("equal-holder", 100, Duration::from_secs(240), 32)
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|lease| lease.session_id == a.session)
                    .unwrap();
                assert!(replacement.fence > 1);
            }
            _ => unreachable!(),
        }
        // Bypass authentication middleware using the already resolved context:
        // handler admission must independently recheck its mandatory proof.
        assert_eq!(
            retained_read(&service, context.clone(), a.session)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied,
            "{boundary}"
        );
        assert_eq!(
            server.read(a.session, a.byte).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
        server.channel = Server::connect(&server.address, Some((CLIENT_A, KEY_A))).await;
        assert_eq!(
            server.read(a.session, a.byte).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
        if boundary == "capability" {
            // A replacement proof must not revive an old retained context.
            sqlx::query(
                "INSERT INTO worker_journal_capabilities (capability_sha256, grant_id, expires_at) \
                SELECT $1, id, expires_at FROM worker_journal_read_grants WHERE id=$2",
            )
            .bind(journal_capability_digest(&[0xc3; 32]).as_slice())
            .bind(a.grant)
            .execute(service.db.database().pool())
            .await
            .unwrap();
            assert_eq!(server.read(a.session, 0xc3).await.unwrap().events.len(), 1);
            assert_eq!(
                retained_read(&service, context, a.session)
                    .await
                    .unwrap_err()
                    .code(),
                tonic::Code::PermissionDenied
            );
        }
    }
}

#[tokio::test]
async fn expired_and_future_identity_or_capability_records_are_refused() {
    for (identity, capability) in [
        ((-20, -10), (-1, 180)),
        ((60, 120), (-1, 180)),
        ((-1, 200), (-20, -10)),
        ((-1, 200), (60, 120)),
    ] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment_window(&service, 2, CLIENT_A, 0xa1, identity, capability).await;
        let server = Server::start(service, true, Some((CLIENT_A, KEY_A))).await;
        assert_eq!(
            server.read(a.session, a.byte).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
        assert!(server.retained.lock().unwrap().is_none());
    }
}

#[tokio::test]
async fn plaintext_encrypted_without_peer_and_unregistered_peer_never_authenticate() {
    for (tls, identity) in [(false, None), (true, None), (true, Some((CLIENT_B, KEY_B)))] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
        let server = Server::start(service, tls, identity).await;
        let mut request = read(a.session, a.byte);
        request.metadata_mut().insert(
            "x-everruns-executor-id",
            a.executor.to_string().parse().unwrap(),
        );
        request.metadata_mut().insert(
            "x-forwarded-client-cert",
            CLIENT_A.replace('\n', "").parse().unwrap(),
        );
        assert_eq!(
            WorkerServiceClient::new(server.channel.clone())
                .read_actor_journal(request)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
        assert!(server.retained.lock().unwrap().is_none());
    }
}

#[tokio::test]
async fn malformed_duplicate_missing_and_legacy_authorization_never_fall_back() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let server = Server::start(service, true, Some((CLIENT_A, KEY_A))).await;
    let good = token(a.byte);
    let mut noncanonical = good.clone().into_bytes();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let last = noncanonical.last_mut().unwrap();
    *last = alphabet[alphabet.iter().position(|byte| byte == last).unwrap() + 1];
    for value in [
        None,
        Some(format!("Bearer {TEST_BLOB_SECRET}")),
        Some(format!("{good}=")),
        Some(format!(" {good}")),
        Some(format!("{good} ")),
        Some("Bearer wjc1.".into()),
        Some(format!("Bearer wjc1.{}", "/".repeat(43))),
        Some(format!("Bearer wjc1.{}", "A".repeat(512))),
        Some(String::from_utf8(noncanonical).unwrap()),
        Some(token(0xff)),
    ] {
        let mut request = read(a.session, a.byte);
        request.metadata_mut().remove("authorization");
        if let Some(value) = value {
            request
                .metadata_mut()
                .insert("authorization", value.parse().unwrap());
        }
        assert_eq!(
            WorkerServiceClient::new(server.channel.clone())
                .read_actor_journal(request)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
    }
    let mut request = read(a.session, a.byte);
    request
        .metadata_mut()
        .append("authorization", good.parse().unwrap());
    assert_eq!(
        WorkerServiceClient::new(server.channel.clone())
            .read_actor_journal(request)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert!(server.retained.lock().unwrap().is_none());
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap().events.len(),
        1
    );
}

#[tokio::test]
async fn verified_journal_scope_denies_commands_and_does_not_drain_an_infinite_event_stream() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let server = Server::start(service, true, Some((CLIENT_A, KEY_A))).await;
    let mut command = Request::new(proto::ExecuteCommandRequest::default());
    command
        .metadata_mut()
        .insert("authorization", token(a.byte).parse().unwrap());
    assert_eq!(
        WorkerServiceClient::new(server.channel.clone())
            .execute_command(command)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let mut stream = Request::new(futures::stream::pending::<proto::EmitEventRequest>());
    stream
        .metadata_mut()
        .insert("authorization", token(a.byte).parse().unwrap());
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        WorkerServiceClient::new(server.channel.clone()).emit_event_stream(stream),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
}

#[tokio::test]
async fn verified_scope_cannot_use_a_valid_foreign_legacy_blob_url_or_buffer_uploads() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let file = service
        .db
        .create_file(
            3,
            crate::storage::CreateFileRow {
                org_id: 3,
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
    let path = crate::api::internal_blobs::BlobLinkSigner::new(TEST_BLOB_SECRET).path(
        crate::api::internal_blobs::BlobKind::File,
        file,
        3,
    );
    let server = Server::start(service, true, Some((CLIENT_A, KEY_A))).await;
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap().events.len(),
        1
    );
    for (method, path, body) in [
        ("GET", path.as_str(), tonic::body::Body::empty()),
        (
            "POST",
            "/internal/blobs/images?org_id=3",
            tonic::body::Body::new(axum::body::Body::from_stream(futures::stream::pending::<
                Result<bytes::Bytes, std::convert::Infallible>,
            >())),
        ),
    ] {
        let mut channel = server.channel.clone();
        std::future::poll_fn(|cx| channel.poll_ready(cx))
            .await
            .unwrap();
        let request = http::Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", token(a.byte))
            .header("x-everruns-role", "legacy")
            .body(body)
            .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(5), channel.call(request))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), 403);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!bytes.windows(13).any(|bytes| bytes == b"foreign-bytes"));
    }
}

#[tokio::test]
async fn invalid_http_proof_never_runs_routes_or_buffers_an_unending_body() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let server = Server::start(service, true, Some((CLIENT_A, KEY_A))).await;
    for proof in [
        None,
        Some(format!("Bearer {TEST_BLOB_SECRET}")),
        Some(token(0xff)),
    ] {
        let mut channel = server.channel.clone();
        std::future::poll_fn(|cx| channel.poll_ready(cx))
            .await
            .unwrap();
        let mut request = http::Request::builder()
            .method("POST")
            .uri("/internal/blobs/images?org_id=3");
        if let Some(proof) = proof {
            request = request.header("authorization", proof);
        }
        let body =
            tonic::body::Body::new(axum::body::Body::from_stream(futures::stream::pending::<
                Result<bytes::Bytes, std::convert::Infallible>,
            >()));
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            channel.call(request.body(body).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), 401);
    }
    assert!(server.retained.lock().unwrap().is_none());
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap().events.len(),
        1
    );
}
