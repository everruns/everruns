//! Actual customer transport must not bypass the protected worker plane.
use super::tests::{TOKEN, fixture, journal, lease, request};
use super::*;
use everruns_contracts::typed_id::{MessageId, TurnId};
use everruns_core::{EventContext, EventRequest, OutputMessageCompletedData, RuntimeMessage};
use everruns_internal_protocol::schema_event_request_to_proto;
use proto::custom_worker_service_client::CustomWorkerServiceClient;
use tonic::{Code, transport::Channel};

struct Server {
    endpoint: String,
    accept: tokio::task::JoinHandle<()>,
    serve: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn start(service: CustomWorkerServiceImpl) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let accept = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                if tx.send(Ok::<_, std::io::Error>(stream)).await.is_err() {
                    break;
                }
            }
        });
        let serve = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service.into_server())
                .serve_with_incoming(tokio_stream::wrappers::ReceiverStream::new(rx))
                .await
                .unwrap();
        });
        Self {
            endpoint,
            accept,
            serve,
        }
    }

    async fn connect(&self) -> CustomWorkerServiceClient<Channel> {
        CustomWorkerServiceClient::connect(self.endpoint.clone())
            .await
            .unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.serve.abort();
        self.accept.abort();
    }
}

async fn protect(db: &StorageBackend, version: i32) {
    sqlx::query("INSERT INTO worker_ingress_protection_floor (minimum_version) VALUES ($1) ON CONFLICT (singleton) DO UPDATE SET minimum_version=EXCLUDED.minimum_version")
        .bind(version).execute(db.pool()).await.unwrap();
}

async fn event(
    service: &CustomWorkerServiceImpl,
    session: SessionId,
    fence: i64,
) -> proto::EmitEventRequest {
    let input = MessageId::new();
    service
        .db
        .record_runtime_invocation(1, session, input.uuid(), None, None, None)
        .await
        .unwrap();
    let event = EventRequest::new(
        session,
        EventContext {
            turn_id: Some(TurnId::new()),
            input_message_id: Some(input),
            ..Default::default()
        },
        OutputMessageCompletedData::new(RuntimeMessage::assistant("customer answer")),
    );
    proto::EmitEventRequest {
        event: Some(schema_event_request_to_proto(&event)),
        actor_fence: journal(session, fence).actor_fence,
    }
}

async fn state(db: &StorageBackend, session: SessionId) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('actor', to_jsonb(a), 'session', to_jsonb(s), 'events', (SELECT count(*) FROM events e WHERE e.session_id=s.id)) FROM session_actors a JOIN sessions s ON s.id=a.session_id WHERE s.id=$1")
        .bind(session).fetch_one(db.pool()).await.unwrap()
}

async fn assert_denied(
    client: &mut CustomWorkerServiceClient<Channel>,
    session: SessionId,
    fence: i64,
    event: proto::EmitEventRequest,
    expected: Code,
) {
    assert_eq!(
        client
            .lease_actors(request(proto::LeaseActorsRequest {
                holder: "replacement".into(),
                limit: 1,
                ..Default::default()
            }))
            .await
            .unwrap_err()
            .code(),
        expected
    );
    assert_eq!(
        client
            .read_journal(request(journal(session, fence)))
            .await
            .unwrap_err()
            .code(),
        expected
    );
    assert_eq!(
        client.emit_event(request(event)).await.unwrap_err().code(),
        expected
    );
    assert_eq!(
        client
            .renew_actor_lease(request(proto::RenewActorLeaseRequest {
                session_id: Some(uuid_to_proto_uuid(session.uuid())),
                holder: "replica-1".into(),
                fence,
                ttl_ms: 300_000,
                consumed_seq: 1,
            }))
            .await
            .unwrap_err()
            .code(),
        expected
    );
    assert_eq!(
        client
            .release_actor(request(proto::ReleaseActorRequest {
                session_id: Some(uuid_to_proto_uuid(session.uuid())),
                holder: "replica-1".into(),
                fence,
                consumed_inbox_seq: 1,
            }))
            .await
            .unwrap_err()
            .code(),
        expected
    );
}

#[tokio::test]
async fn protection_floor_closes_existing_customer_channels_and_reconnects() {
    let (service, _, session) = fixture().await;
    let actor = lease(&service).await;
    let event = event(&service, session, actor.fence).await;
    let server = Server::start(service.clone()).await;
    let mut client = server.connect().await;
    client.emit_event(request(event.clone())).await.unwrap();
    assert_eq!(
        client
            .read_journal(request(journal(session, actor.fence)))
            .await
            .unwrap()
            .into_inner()
            .events
            .len(),
        1
    );
    let before = state(&service.db, session).await;
    for version in [1, 2] {
        protect(&service.db, version).await;
        for reconnect in [false, true] {
            if reconnect {
                client = server.connect().await;
            }
            assert_denied(
                &mut client,
                session,
                actor.fence,
                event.clone(),
                Code::PermissionDenied,
            )
            .await;
            assert_eq!(state(&service.db, session).await, before);
        }
    }
}

#[tokio::test]
async fn protected_customer_refusal_precedes_payload_and_deployment_lookup() {
    let (service, _, _) = fixture().await;
    protect(&service.db, 1).await;
    // If authentication consults this table first it fails Internal instead of
    // applying the protection floor. No deployment lookup is needed to refuse.
    sqlx::query("ALTER TABLE custom_workers RENAME TO unavailable_custom_workers")
        .execute(service.db.pool())
        .await
        .unwrap();
    let server = Server::start(service).await;
    let mut client = server.connect().await;
    assert_eq!(
        client
            .lease_actors(request(proto::LeaseActorsRequest::default()))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        client
            .read_journal(request(proto::CustomWorkerJournalRequest::default()))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        client
            .emit_event(request(proto::EmitEventRequest::default()))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        client
            .renew_actor_lease(request(proto::RenewActorLeaseRequest::default()))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        client
            .release_actor(request(proto::ReleaseActorRequest::default()))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
}

#[tokio::test]
async fn unavailable_floor_never_defaults_to_unprotected_customer_authority() {
    let (service, _, session) = fixture().await;
    let actor = lease(&service).await;
    let event = event(&service, session, actor.fence).await;
    let server = Server::start(service.clone()).await;
    let mut client = server.connect().await;
    client
        .read_journal(request(journal(session, actor.fence)))
        .await
        .unwrap();
    let before = state(&service.db, session).await;
    sqlx::query("ALTER TABLE worker_ingress_protection_floor RENAME TO unavailable_ingress_floor")
        .execute(service.db.pool())
        .await
        .unwrap();
    for reconnect in [false, true] {
        if reconnect {
            client = server.connect().await;
        }
        assert_denied(
            &mut client,
            session,
            actor.fence,
            event.clone(),
            Code::Internal,
        )
        .await;
        assert_eq!(state(&service.db, session).await, before);
    }
}

#[tokio::test]
async fn floor_refuses_own_foreign_and_spoofed_customer_selectors() {
    let (service, _, session) = fixture().await;
    let actor = lease(&service).await;
    let other_agent = service.db.create_test_agent(2, uuid::Uuid::now_v7()).await;
    let other = service
        .db
        .create_session(crate::storage::test_database::test_session_row(2))
        .await
        .unwrap()
        .id;
    sqlx::query("UPDATE sessions SET agent_id=$1 WHERE id=$2")
        .bind(other_agent)
        .bind(other)
        .execute(service.db.pool())
        .await
        .unwrap();
    // Reuse an existing synthetic fixture value, never a real credential.
    let other_token = "different_customer_worker_credential_1234567890";
    sqlx::query(
        "INSERT INTO custom_workers (org_id,agent_id,name,token_hash) VALUES (2,$1,'other',$2)",
    )
    .bind(other_agent)
    .bind(Sha256::digest(other_token.as_bytes()).as_slice())
    .execute(service.db.pool())
    .await
    .unwrap();
    service
        .db
        .mark_session_actor_runnable(other, 2, 1)
        .await
        .unwrap();
    let server = Server::start(service.clone()).await;
    let mut client = server.connect().await;
    let authorize = |value| {
        let mut req = request(value);
        req.metadata_mut().insert(
            "authorization",
            format!("Bearer {other_token}").parse().unwrap(),
        );
        req
    };
    let mut claim = request(proto::LeaseActorsRequest {
        holder: "replica-1".into(),
        limit: 1,
        ..Default::default()
    });
    claim.metadata_mut().insert(
        "authorization",
        format!("Bearer {other_token}").parse().unwrap(),
    );
    let other_actor = client
        .lease_actors(claim)
        .await
        .unwrap()
        .into_inner()
        .actors
        .remove(0);
    client
        .read_journal(authorize(journal(other, other_actor.fence)))
        .await
        .unwrap();
    assert_eq!(
        client
            .read_journal(request(journal(other, other_actor.fence)))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        client
            .read_journal(authorize(journal(session, actor.fence)))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    protect(&service.db, 1).await;
    for token in [
        TOKEN,
        other_token,
        "wjc1.forged_assignment_proof_12345678901234567890",
    ] {
        for target in [session, other, SessionId::new()] {
            let mut req = request(journal(target, actor.fence));
            req.metadata_mut()
                .insert("authorization", format!("Bearer {token}").parse().unwrap());
            req.metadata_mut()
                .insert("x-everruns-org-id", "2".parse().unwrap());
            req.metadata_mut()
                .insert("x-everruns-role", "verified".parse().unwrap());
            assert_eq!(
                client.read_journal(req).await.unwrap_err().code(),
                Code::PermissionDenied
            );
        }
    }
}
