use super::*;
use crate::domains::sessions::limits::OrgCaps;
use crate::errors::BadRequestError;
use crate::storage::{CreateUserRow, RESOLVING_TOOL_RESULTS_STATUS, StorageBackend, UpdateSession};
use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
use std::sync::atomic::{AtomicUsize, Ordering};

struct NoopRunner;

#[async_trait]
impl TurnBackend for NoopRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        // The server drops its tickets; this one never resolves.
        Ok(everruns_core::host::TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

struct FailOnceResumeRunner {
    calls: AtomicUsize,
}

#[async_trait]
impl TurnBackend for FailOnceResumeRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        if matches!(
            request.input,
            everruns_core::host::TurnInput::RecordedToolResults { .. }
        ) && self.calls.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Err(everruns_contracts::error::AgentLoopError::store(
                "durable resume enqueue failed",
            ));
        }
        // The server drops its tickets; this one never resolves.
        Ok(everruns_core::host::TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

async fn create_test_session(db: &StorageBackend, org_id: i64) -> crate::storage::SessionRow {
    db.create_session(crate::storage::CreateSessionRow {
        playground_user_id: None,
        source: crate::domains::sessions::record::SessionSource::Api,
        workspace_id: None,
        org_id,
        harness_id: None,
        app_id: None,
        channel_id: None,
        trigger_id: None,
        agent_id: None,
        agent_revision: None,
        virtual_user_id: None,
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(org_id as u128),
        resolved_owner_user_id: None,
        title: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::json!([]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    })
    .await
    .unwrap()
}

async fn park_test_session(db: &StorageBackend, session: &crate::storage::SessionRow) {
    db.update_session(
        session.org_id,
        session.id,
        UpdateSession {
            status: Some("waiting_for_tool_results".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db.create_event(crate::storage::CreateEventRow {
        session_id: session.id,
        event_type: "tool.call_requested".to_string(),
        ts: Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({
            "tool_calls": [{
                "id": "call_parked",
                "name": "ask_user",
                "arguments": {}
            }]
        }),
        metadata: None,
        tags: None,
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn active_turn_cap_enforced() {
    let db = Arc::new(StorageBackend::test_database());
    let runner: Arc<dyn TurnBackend> = Arc::new(NoopRunner);
    let delivery = crate::live_updates::event_delivery::EventDelivery::in_memory();

    let svc = MessageService::new(db.clone(), runner, delivery).with_caps(OrgCaps {
        max_concurrent_sessions: 10_000,
        max_active_turns: 1,
    });

    // Seed an 'active' session so count_active_turns_for_org returns 1.
    // max_active_turns = 1 so 1 active turn exactly hits the cap.
    let session = create_test_session(&db, 1).await;
    db.update_session(
        1,
        session.id,
        UpdateSession {
            status: Some("active".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let ctx = CreateMessageContext {
        runtime_subject_principal_id: None,
        org_id: 1,
        user_id: None,
        harness_id: session.id.uuid(),
        agent_id: None,
        session_id: session.id.uuid(),
        event_metadata: None,
        request_id: None,
    };

    let err = svc
        .create(ctx, CreateMessageRequest::user("hello"))
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<BadRequestError>().is_some(),
        "expected BadRequestError, got: {err}"
    );
    assert!(
        err.to_string().contains("Too many active turns"),
        "got: {err}"
    );

    // Org members see the refusal in Settings -> Health.
    let mut recorded = false;
    for _ in 0..100 {
        if db
            .list_health_issues(1, 0, 10, None)
            .await
            .unwrap()
            .iter()
            .any(|row| row.code == crate::domains::health_issues::active_turns::ACTIVE_TURN_LIMIT)
        {
            recorded = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(recorded, "the cap hit must open an org health issue");
}

#[tokio::test]
async fn parked_turn_resumes_at_new_turn_capacity() {
    let db = Arc::new(StorageBackend::test_database());
    let runner: Arc<dyn TurnBackend> = Arc::new(NoopRunner);
    let delivery = crate::live_updates::event_delivery::EventDelivery::in_memory();
    let svc = MessageService::new(db.clone(), runner, delivery).with_caps(OrgCaps {
        max_concurrent_sessions: 10_000,
        max_active_turns: 1,
    });
    let active = create_test_session(&db, 1).await;
    db.update_session(
        1,
        active.id,
        UpdateSession {
            status: Some("active".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let parked = create_test_session(&db, 1).await;
    park_test_session(&db, &parked).await;

    let message = svc
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: 1,
                user_id: None,
                harness_id: parked.id.uuid(),
                agent_id: None,
                session_id: parked.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("typed answer"),
        )
        .await
        .unwrap();

    assert_eq!(message.session_id, parked.id);
    assert_eq!(
        db.get_session(1, parked.id).await.unwrap().unwrap().status,
        "active"
    );
}

#[tokio::test]
async fn parked_turn_event_write_failure_preserves_plan_for_retry() {
    let db = Arc::new(StorageBackend::test_database());
    let svc = MessageService::new(
        db.clone(),
        Arc::new(NoopRunner),
        crate::live_updates::event_delivery::EventDelivery::in_memory(),
    );
    let session = create_test_session(&db, 1).await;
    park_test_session(&db, &session).await;
    db.force_storage_failure("create_event");

    let error = svc
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: 1,
                user_id: None,
                harness_id: session.id.uuid(),
                agent_id: None,
                session_id: session.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("typed answer"),
        )
        .await
        .expect_err("event write must fail");

    assert!(error.to_string().contains("relation"));
    assert_eq!(
        db.get_session(1, session.id).await.unwrap().unwrap().status,
        RESOLVING_TOOL_RESULTS_STATUS
    );

    let recovered = svc
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: 1,
                user_id: None,
                harness_id: session.id.uuid(),
                agent_id: None,
                session_id: session.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("replacement must not win"),
        )
        .await
        .expect("expired resolution claim should recover");
    assert_eq!(
        db.get_session(1, session.id).await.unwrap().unwrap().status,
        "active"
    );
    let input_events = db
        .list_events(
            session.id,
            None,
            None,
            &["input.message".to_string()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(input_events.len(), 1);
    assert_eq!(
        input_events[0].data["message"]["content"][0]["text"],
        "typed answer"
    );
    assert_eq!(
        recovered.id.to_string(),
        input_events[0].data["message"]["id"]
    );
}

#[tokio::test]
async fn parked_turn_partial_commit_retry_is_idempotent() {
    let db = Arc::new(StorageBackend::test_database());
    let svc = MessageService::new(
        db.clone(),
        Arc::new(FailOnceResumeRunner {
            calls: AtomicUsize::new(0),
        }),
        crate::live_updates::event_delivery::EventDelivery::in_memory(),
    );
    let session = create_test_session(&db, 1).await;
    park_test_session(&db, &session).await;

    let error = svc
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: 1,
                user_id: None,
                harness_id: session.id.uuid(),
                agent_id: None,
                session_id: session.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("typed answer"),
        )
        .await
        .expect_err("durable enqueue must fail");

    assert_eq!(error.to_string(), "durable resume enqueue failed");
    assert_eq!(
        db.get_session(1, session.id).await.unwrap().unwrap().status,
        RESOLVING_TOOL_RESULTS_STATUS
    );

    svc.create(
        CreateMessageContext {
            runtime_subject_principal_id: None,
            org_id: 1,
            user_id: None,
            harness_id: session.id.uuid(),
            agent_id: None,
            session_id: session.id.uuid(),
            event_metadata: None,
            request_id: None,
        },
        CreateMessageRequest::user("replacement must not duplicate"),
    )
    .await
    .expect("retry should finish the persisted plan");

    let events = db
        .list_events(session.id, None, None, &[], &[], None, None)
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "tool.completed")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "input.message")
            .count(),
        1
    );
    assert!(events.iter().any(|event| {
        event.event_type == "input.message"
            && event.data["message"]["content"][0]["text"] == "typed answer"
    }));
    assert_eq!(
        db.get_session(1, session.id).await.unwrap().unwrap().status,
        "active"
    );
}

#[tokio::test]
async fn create_message_without_user_id_uses_session_owner_participant_metadata() {
    let db = Arc::new(StorageBackend::test_database());
    let runner: Arc<dyn TurnBackend> = Arc::new(NoopRunner);
    let delivery = crate::live_updates::event_delivery::EventDelivery::in_memory();

    let svc = MessageService::new(db.clone(), runner, delivery).with_caps(OrgCaps {
        max_concurrent_sessions: 10_000,
        max_active_turns: 10_000,
    });

    let session = create_test_session(&db, 1).await;
    let owner_participant = db
        .list_session_participants(1, session.id)
        .await
        .unwrap()
        .into_iter()
        .find(|row| {
            row.kind == "user"
                && row.principal_id == session.owner_principal_id
                && row.left_at.is_none()
        })
        .expect("session owner user participant");

    let message = svc
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: 1,
                user_id: None,
                harness_id: session.id.uuid(),
                agent_id: None,
                session_id: session.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("owner provenance"),
        )
        .await
        .unwrap();
    assert_eq!(message.session_id, session.id);

    let events = db
        .list_message_events_limited(session.id, None)
        .await
        .unwrap();
    let input = events
        .into_iter()
        .find(|row| row.event_type == "input.message")
        .expect("input message event");
    let metadata = input.metadata.expect("input message metadata");
    assert_eq!(
        metadata
            .get("initiator_principal_id")
            .and_then(|value| value.as_str()),
        Some(session.owner_principal_id.to_string().as_str())
    );
    assert_eq!(
        metadata
            .get("participant_id")
            .and_then(|value| value.as_str()),
        Some(owner_participant.id.to_string().as_str())
    );
}

#[tokio::test]
async fn create_message_rejoins_user_who_left_session() {
    let db = Arc::new(StorageBackend::test_database());
    let runner: Arc<dyn TurnBackend> = Arc::new(NoopRunner);
    let delivery = crate::live_updates::event_delivery::EventDelivery::in_memory();
    let svc = MessageService::new(db.clone(), runner, delivery).with_caps(OrgCaps {
        max_concurrent_sessions: 10_000,
        max_active_turns: 10_000,
    });

    let user = db
        .create_user(CreateUserRow {
            email: "returning-user@example.com".to_string(),
            name: "Returning User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    db.add_organization_member(everruns_core::DEFAULT_ORG_ID, user.id, "member")
        .await
        .unwrap();
    let principal = PrincipalService::new(db.clone())
        .ensure_default_virtual_user_principal(1, user.id)
        .await
        .unwrap();
    let session = create_test_session(&db, 1).await;
    let original_participant = db
        .ensure_active_user_session_participant(CreateSessionParticipantRow {
            org_id: 1,
            session_id: session.id,
            kind: SessionParticipantKind::User,
            agent_id: None,
            principal_id: principal.id,
            display_name: Some("Returning User".to_string()),
            role: SessionParticipantRole::Member,
            joined_at: None,
        })
        .await
        .unwrap();
    let runtime_user = db.default_virtual_user(1, user.id).await.unwrap();
    db.update_virtual_user(
        1,
        runtime_user.id,
        crate::storage::UpdateVirtualUser {
            name: Some("Returning runtime user".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db.leave_session_participant(1, session.id, original_participant.id)
        .await
        .unwrap()
        .expect("leave initial participant");

    svc.create(
        CreateMessageContext {
            runtime_subject_principal_id: None,
            org_id: 1,
            user_id: Some(user.id),
            harness_id: session.id.uuid(),
            agent_id: None,
            session_id: session.id.uuid(),
            event_metadata: None,
            request_id: None,
        },
        CreateMessageRequest::user("I am back"),
    )
    .await
    .unwrap();

    let participants = db.list_session_participants(1, session.id).await.unwrap();
    let active_participant = participants
        .iter()
        .find(|row| row.principal_id == principal.id && row.left_at.is_none())
        .expect("returning user rejoins");
    assert_ne!(active_participant.id, original_participant.id);
    assert_eq!(active_participant.principal_id, principal.id);
    assert_eq!(
        active_participant.display_name.as_deref(),
        Some("Returning runtime user")
    );
    assert_eq!(
        db.get_user(user.id).await.unwrap().unwrap().name,
        "Returning User"
    );

    let events = db
        .list_message_events_limited(session.id, None)
        .await
        .unwrap();
    let input = events
        .into_iter()
        .find(|row| row.event_type == "input.message")
        .expect("input message event");
    assert_eq!(
        input
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("participant_id"))
            .and_then(|value| value.as_str()),
        Some(active_participant.id.to_string().as_str())
    );
}

fn send_ctx(session: &crate::storage::SessionRow) -> CreateMessageContext {
    CreateMessageContext {
        runtime_subject_principal_id: None,
        org_id: 1,
        user_id: None,
        harness_id: session.id.uuid(),
        agent_id: None,
        session_id: session.id.uuid(),
        event_metadata: None,
        request_id: None,
    }
}

#[tokio::test]
async fn send_reports_started_then_steered_and_retry_is_idempotent() {
    let db = Arc::new(StorageBackend::test_database());
    let runner: Arc<dyn TurnBackend> = Arc::new(NoopRunner);
    let delivery = crate::live_updates::event_delivery::EventDelivery::in_memory();
    let svc = MessageService::new(db.clone(), runner, delivery);
    let session = create_test_session(&db, 1).await;

    let first_id = Uuid::now_v7();
    let mut first_req = CreateMessageRequest::user("yes");
    first_req.client_message_id = Some(first_id);
    let first = svc
        .create(send_ctx(&session), first_req.clone())
        .await
        .unwrap();
    assert_eq!(first.delivery, Some(MessageDelivery::Started));
    assert_eq!(
        first
            .metadata
            .as_ref()
            .and_then(
                |metadata| metadata.get(everruns_core::message::CLIENT_MESSAGE_ID_METADATA_KEY)
            )
            .and_then(|value| value.as_str()),
        Some(first_id.to_string().as_str()),
        "the stored message echoes the client id"
    );

    // The same text again, with its own id, is a second message that joins
    // the running turn.
    let mut second_req = CreateMessageRequest::user("yes");
    second_req.client_message_id = Some(Uuid::now_v7());
    let second = svc.create(send_ctx(&session), second_req).await.unwrap();
    assert_eq!(second.delivery, Some(MessageDelivery::Steered));
    assert_ne!(second.id, first.id);

    // Retrying the first send returns the stored message and stores nothing.
    let retried = svc.create(send_ctx(&session), first_req).await.unwrap();
    assert_eq!(retried.delivery, Some(MessageDelivery::Duplicate));
    assert_eq!(retried.id, first.id);
    assert_eq!(retried.sequence, first.sequence);
    let inputs = db
        .list_message_events_limited(session.id, None)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.event_type == "input.message")
        .count();
    assert_eq!(inputs, 2);
}

#[tokio::test]
async fn client_metadata_cannot_claim_a_client_message_id() {
    let db = Arc::new(StorageBackend::test_database());
    let runner: Arc<dyn TurnBackend> = Arc::new(NoopRunner);
    let delivery = crate::live_updates::event_delivery::EventDelivery::in_memory();
    let svc = MessageService::new(db.clone(), runner, delivery);
    let session = create_test_session(&db, 1).await;

    let forged = Uuid::now_v7().to_string();
    let mut req = CreateMessageRequest::user("hello");
    req.metadata = Some(std::collections::HashMap::from([(
        everruns_core::message::CLIENT_MESSAGE_ID_METADATA_KEY.to_string(),
        serde_json::Value::String(forged.clone()),
    )]));
    let message = svc.create(send_ctx(&session), req).await.unwrap();
    assert!(message.metadata.as_ref().is_none_or(|metadata| {
        !metadata.contains_key(everruns_core::message::CLIENT_MESSAGE_ID_METADATA_KEY)
    }));
    assert!(
        db.find_input_message_by_client_id(session.id, &forged)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn active_turn_cap_reserves_started_session_before_persisting() {
    let db = Arc::new(StorageBackend::test_database());
    let runner: Arc<dyn TurnBackend> = Arc::new(NoopRunner);
    let delivery = crate::live_updates::event_delivery::EventDelivery::in_memory();

    let svc = MessageService::new(db.clone(), runner, delivery).with_caps(OrgCaps {
        max_concurrent_sessions: 10_000,
        max_active_turns: 1,
    });

    let first = create_test_session(&db, 1).await;
    let second = create_test_session(&db, 1).await;

    let first_message = svc
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: 1,
                user_id: None,
                harness_id: first.id.uuid(),
                agent_id: None,
                session_id: first.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("first"),
        )
        .await
        .unwrap();
    assert_eq!(first_message.session_id, first.id);
    assert_eq!(db.count_active_turns_for_org(1).await.unwrap(), 1);

    let err = svc
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: 1,
                user_id: None,
                harness_id: second.id.uuid(),
                agent_id: None,
                session_id: second.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("second"),
        )
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<BadRequestError>().is_some(),
        "expected BadRequestError, got: {err}"
    );
    assert!(
        err.to_string().contains("Too many active turns"),
        "got: {err}"
    );
    assert_eq!(db.count_active_turns_for_org(1).await.unwrap(), 1);
    assert!(
        db.list_message_events_limited(second.id, None)
            .await
            .unwrap()
            .is_empty(),
        "rejected turn must not persist a queued message"
    );
}
