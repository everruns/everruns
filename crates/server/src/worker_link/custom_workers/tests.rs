use super::*;
use everruns_contracts::typed_id::{MessageId, TurnId};
use everruns_core::{EventContext, EventRequest, OutputMessageCompletedData, RuntimeMessage};
use everruns_internal_protocol::schema_event_request_to_proto;

pub(super) const TOKEN: &str = "custom_worker_test_credential_1234567890";

pub(super) fn request<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    request
}

pub(super) async fn fixture() -> (CustomWorkerServiceImpl, uuid::Uuid, SessionId) {
    let db = Arc::new(StorageBackend::test_database());
    let agent = db.create_test_agent(1, uuid::Uuid::now_v7()).await;
    let session = db.create_test_session().await;
    sqlx::query("UPDATE sessions SET agent_id = $1 WHERE id = $2")
        .bind(agent)
        .bind(session)
        .execute(db.pool())
        .await
        .unwrap();
    let deployment: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO custom_workers (org_id, agent_id, name, token_hash) VALUES (1, $1, 'data-agent', $2) RETURNING id",
    ).bind(agent).bind(Sha256::digest(TOKEN.as_bytes()).as_slice()).fetch_one(db.pool()).await.unwrap();
    db.mark_session_actor_runnable(session, 1, 1).await.unwrap();
    let events = Arc::new(EventService::with_listeners(
        db.clone(),
        crate::EventDelivery::in_memory(),
        vec![],
    ));
    (
        CustomWorkerServiceImpl::new(db, events),
        deployment,
        session,
    )
}

pub(super) async fn lease(svc: &CustomWorkerServiceImpl) -> proto::LeasedActor {
    svc.lease_actors(request(proto::LeaseActorsRequest {
        holder: "replica-1".into(),
        limit: 1,
        ..Default::default()
    }))
    .await
    .unwrap()
    .into_inner()
    .actors
    .remove(0)
}

pub(super) fn journal(session: SessionId, fence: i64) -> proto::CustomWorkerJournalRequest {
    proto::CustomWorkerJournalRequest {
        journal: Some(proto::ReadActorJournalRequest {
            session_id: Some(uuid_to_proto_uuid(session.uuid())),
            ..Default::default()
        }),
        actor_fence: Some(proto::ActorFence {
            holder: "replica-1".into(),
            fence,
            detached_tool_call_id: None,
        }),
    }
}

#[tokio::test]
async fn custom_workers_route_without_fallback_and_renew_release() {
    let (svc, deployment, session) = fixture().await;
    let regular = svc.db.create_test_session().await;
    svc.db
        .mark_session_actor_runnable(regular, 1, 1)
        .await
        .unwrap();
    let builtin = svc
        .db
        .lease_session_actors("builtin", 10, Duration::from_secs(30), 32)
        .await
        .unwrap();
    assert_eq!(builtin.len(), 1);
    assert_eq!(builtin[0].session_id, regular);
    let actor = lease(&svc).await;
    assert_eq!(actor.session_id, Some(uuid_to_proto_uuid(session.uuid())));
    assert!(
        svc.read_journal(request(journal(session, actor.fence)))
            .await
            .is_ok()
    );
    let renewed = svc
        .renew_actor_lease(request(proto::RenewActorLeaseRequest {
            session_id: actor.session_id.clone(),
            holder: "replica-1".into(),
            fence: actor.fence,
            ttl_ms: 300_000,
            consumed_seq: 0,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(renewed.held);
    assert!(
        svc.release_actor(request(proto::ReleaseActorRequest {
            session_id: actor.session_id,
            holder: "replica-1".into(),
            fence: actor.fence,
            consumed_inbox_seq: 0,
        }))
        .await
        .unwrap()
        .into_inner()
        .still_runnable
    );
    sqlx::query("UPDATE custom_workers SET enabled = FALSE WHERE id = $1")
        .bind(deployment)
        .execute(svc.db.pool())
        .await
        .unwrap();
    assert_eq!(
        svc.lease_actors(request(proto::LeaseActorsRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    assert!(
        svc.db
            .lease_session_actors("builtin", 10, Duration::from_secs(30), 32)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn credentials_fences_and_session_scope_are_required() {
    let (svc, _, session) = fixture().await;
    assert_eq!(
        svc.lease_actors(Request::new(proto::LeaseActorsRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    let mut bad = request(proto::LeaseActorsRequest::default());
    bad.metadata_mut().insert(
        "authorization",
        "Bearer a_trusted_fleet_token_is_not_a_deployment_token"
            .parse()
            .unwrap(),
    );
    assert_eq!(
        svc.lease_actors(bad).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let actor = lease(&svc).await;
    let other = svc.db.create_test_session().await;
    assert_eq!(
        svc.read_journal(request(journal(other, actor.fence)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        svc.read_journal(request(journal(session, actor.fence + 1)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let mut no_fence = journal(session, actor.fence);
    no_fence.actor_fence = None;
    assert_eq!(
        svc.read_journal(request(no_fence))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    sqlx::query("UPDATE session_actors SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE session_id = $1")
        .bind(session).execute(svc.db.pool()).await.unwrap();
    assert_eq!(
        svc.read_journal(request(journal(session, actor.fence)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let replacement = lease(&svc).await;
    assert!(replacement.fence > actor.fence);
}

#[tokio::test]
async fn custom_events_use_canonical_storage_and_require_the_owning_invocation() {
    let (svc, _, session) = fixture().await;
    let actor = lease(&svc).await;
    let input = MessageId::new();
    svc.db
        .record_runtime_invocation(1, session, input.uuid(), None, None, None)
        .await
        .unwrap();
    let context = EventContext {
        turn_id: Some(TurnId::new()),
        input_message_id: Some(input),
        ..Default::default()
    };
    let event = EventRequest::new(
        session,
        context,
        OutputMessageCompletedData::new(RuntimeMessage::assistant("Database answer")),
    );
    let req = proto::EmitEventRequest {
        event: Some(schema_event_request_to_proto(&event)),
        actor_fence: journal(session, actor.fence).actor_fence,
    };
    let stored = svc
        .emit_event(request(req.clone()))
        .await
        .unwrap()
        .into_inner()
        .event
        .unwrap();
    assert!(stored.sequence > 0);
    assert_eq!(stored.event_type, "output.message.completed");
    let messages = svc
        .db
        .read_session_actor_journal(session, Some(0), 100)
        .await
        .unwrap();
    assert!(
        messages
            .events
            .iter()
            .any(|e| e.sequence == stored.sequence)
    );
    let mut forged = req.clone();
    forged
        .event
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .input_message_id = Some(uuid_to_proto_uuid(uuid::Uuid::now_v7()));
    assert_eq!(
        svc.emit_event(request(forged)).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    let mut missing = req.clone();
    missing.actor_fence = None;
    assert_eq!(
        svc.emit_event(request(missing)).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    let stale = req.clone();
    let mut intake = req;
    intake.event.as_mut().unwrap().event_type = "input.message".into();
    assert!(svc.emit_event(request(intake)).await.is_err());
    sqlx::query("UPDATE session_actors SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE session_id = $1")
        .bind(session).execute(svc.db.pool()).await.unwrap();
    assert_eq!(
        svc.emit_event(request(stale)).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn tenant_and_other_deployment_cannot_access_a_lease() {
    let (svc, _, session) = fixture().await;
    let actor = lease(&svc).await;
    let other_agent = svc.db.create_test_agent(2, uuid::Uuid::now_v7()).await;
    let other_token = "different_customer_worker_credential_1234567890";
    sqlx::query("INSERT INTO custom_workers (org_id, agent_id, name, token_hash) VALUES (2, $1, 'other', $2)")
        .bind(other_agent).bind(Sha256::digest(other_token.as_bytes()).as_slice()).execute(svc.db.pool()).await.unwrap();
    let mut req = journal(session, actor.fence);
    req.actor_fence.as_mut().unwrap().holder = "replica-1".into();
    let mut req = request(req);
    req.metadata_mut().insert(
        "authorization",
        format!("Bearer {other_token}").parse().unwrap(),
    );
    assert_eq!(
        svc.read_journal(req).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn custom_protocol_round_trips_and_does_not_serve_trusted_rpcs() {
    let (svc, _, _) = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let accept = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            if tx.send(Ok::<_, std::io::Error>(stream)).await.is_err() {
                break;
            }
        }
    });
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(svc.into_server())
            .serve_with_incoming(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap();
    });
    let endpoint = format!("http://{address}");
    let mut client =
        proto::custom_worker_service_client::CustomWorkerServiceClient::connect(endpoint.clone())
            .await
            .unwrap();
    assert_eq!(
        client
            .lease_actors(proto::LeaseActorsRequest::default())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    let leased = client
        .lease_actors(request(proto::LeaseActorsRequest {
            holder: "network-replica".into(),
            limit: 1,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(leased.actors.len(), 1);
    let mut trusted = proto::worker_service_client::WorkerServiceClient::connect(endpoint)
        .await
        .unwrap();
    assert_eq!(
        trusted
            .get_turn_context(request(proto::GetTurnContextRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    server.abort();
    accept.abort();
}

#[derive(Default)]
struct Observed(std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl everruns_core::EventListener for Observed {
    async fn on_event(&self, event: &everruns_core::Event) {
        self.0.lock().unwrap().push(event.event_type.clone());
    }
}

#[tokio::test]
async fn lifecycle_events_reach_live_delivery_and_trace_listeners() {
    let (mut svc, _, session) = fixture().await;
    let listener = Arc::new(Observed::default());
    let delivery = crate::EventDelivery::in_memory();
    let mut subscription = delivery.subscribe(session.uuid()).await.unwrap();
    svc.events = Arc::new(EventService::with_listeners(
        svc.db.clone(),
        delivery,
        vec![listener.clone()],
    ));
    let actor = lease(&svc).await;
    let input = MessageId::new();
    svc.db
        .record_runtime_invocation(1, session, input.uuid(), None, None, None)
        .await
        .unwrap();
    let turn_id = TurnId::new();
    let context = EventContext {
        turn_id: Some(turn_id),
        input_message_id: Some(input),
        ..Default::default()
    };
    let event = EventRequest::new(
        session,
        context.clone(),
        everruns_core::TurnStartedData {
            turn_id,
            input_message_id: input,
            input_content: None,
            agent_id: None,
            agent_name: None,
            agent_description: None,
            origin: None,
        },
    );
    let emit = |event: EventRequest| {
        request(proto::EmitEventRequest {
            event: Some(schema_event_request_to_proto(&event)),
            actor_fence: journal(session, actor.fence).actor_fence,
        })
    };
    svc.emit_event(emit(event)).await.unwrap();
    assert_eq!(
        svc.db
            .get_session(1, session)
            .await
            .unwrap()
            .unwrap()
            .status,
        "active"
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), subscription.recv())
            .await
            .unwrap()
            .unwrap()
            .event_type,
        "session.activated"
    );
    svc.emit_event(emit(EventRequest::new(
        session,
        context,
        everruns_core::TurnCompletedData {
            turn_id,
            ..Default::default()
        },
    )))
    .await
    .unwrap();
    assert_eq!(
        svc.db
            .get_session(1, session)
            .await
            .unwrap()
            .unwrap()
            .status,
        "idle"
    );
    assert_eq!(
        *listener.0.lock().unwrap(),
        [
            "session.activated",
            "turn.started",
            "session.idled",
            "turn.completed"
        ]
    );
}

#[test]
fn event_payloads_cannot_forge_identity_or_bypass_typed_validation() {
    let session = SessionId::new();
    let turn_id = TurnId::new();
    let input = MessageId::new();
    let context = EventContext {
        turn_id: Some(turn_id),
        input_message_id: Some(input),
        ..Default::default()
    };
    let mismatch = EventRequest::new(
        session,
        context.clone(),
        everruns_core::TurnCompletedData {
            turn_id: TurnId::new(),
            ..Default::default()
        },
    );
    assert!(validate_event(&mismatch).is_err());
    let mut impersonated = RuntimeMessage::assistant("forged input");
    impersonated.role = everruns_core::RuntimeMessageRole::User;
    assert!(
        validate_event(&EventRequest::new(
            session,
            context.clone(),
            OutputMessageCompletedData::new(impersonated)
        ))
        .is_err()
    );
    let mut unsupported = EventRequest::new(
        session,
        context,
        OutputMessageCompletedData::new(RuntimeMessage::assistant("x")),
    );
    unsupported.data = everruns_core::EventData::unsupported(
        "output.message.completed".into(),
        serde_json::json!({}),
    );
    assert!(validate_event(&unsupported).is_err());
}

#[test]
fn global_trace_keys_are_namespaced_and_restart_stable() {
    let deployment = Deployment {
        id: uuid::Uuid::now_v7(),
        org_id: 1,
        agent_id: uuid::Uuid::now_v7(),
    };
    let other = Deployment {
        id: uuid::Uuid::now_v7(),
        org_id: 2,
        agent_id: uuid::Uuid::now_v7(),
    };
    let turn_id = TurnId::new();
    let input = MessageId::new();
    let context = EventContext {
        turn_id: Some(turn_id),
        input_message_id: Some(input),
        span_id: Some("guessed-shared-span".into()),
        parent_span_id: Some("guessed-shared-parent".into()),
        ..Default::default()
    };
    let source = EventRequest::new(
        SessionId::new(),
        context,
        everruns_core::TurnCompletedData {
            turn_id,
            ..Default::default()
        },
    );
    let mut first = source.clone();
    let mut second = source.clone();
    let mut tenant = source;
    normalize_event(&deployment, &mut first).unwrap();
    normalize_event(&deployment, &mut second).unwrap();
    normalize_event(&other, &mut tenant).unwrap();
    assert_eq!(first.context.turn_id, second.context.turn_id);
    assert_eq!(first.context.span_id, second.context.span_id);
    assert_ne!(first.context.turn_id, tenant.context.turn_id);
    assert_ne!(first.context.span_id, tenant.context.span_id);
    assert_ne!(first.context.parent_span_id, tenant.context.parent_span_id);
    assert_eq!(
        serde_json::to_value(&first.data).unwrap()["turn_id"],
        serde_json::json!(first.context.turn_id.unwrap())
    );
}

#[test]
fn customer_model_reports_cannot_claim_a_managed_provider_account() {
    let deployment = Deployment {
        id: uuid::Uuid::now_v7(),
        org_id: 1,
        agent_id: uuid::Uuid::now_v7(),
    };
    let generation = everruns_core::LlmGenerationData::success(
        vec![],
        vec![],
        Some("answer".into()),
        vec![],
        "custom-model".into(),
        Some("openai".into()),
        None,
        None,
        None,
    )
    .with_provider_id(Some("provider_untrusted".into()));
    let context = EventContext {
        turn_id: Some(TurnId::new()),
        input_message_id: Some(MessageId::new()),
        ..Default::default()
    };
    let mut event = EventRequest::new(SessionId::new(), context, generation);
    event.metadata = Some(
        serde_json::json!({"everruns_provider_id": "provider_untrusted", "custom_worker_id": "forged"}),
    );
    normalize_event(&deployment, &mut event).unwrap();
    let data = serde_json::to_value(&event.data).unwrap();
    assert!(data["metadata"]["provider_id"].is_null());
    assert!(
        event
            .metadata
            .as_ref()
            .unwrap()
            .get("everruns_provider_id")
            .is_none()
    );
    assert_eq!(
        event.metadata.unwrap()["custom_worker_id"],
        deployment.id.to_string()
    );
}
