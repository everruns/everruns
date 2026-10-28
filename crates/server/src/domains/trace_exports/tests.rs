use super::test_support::*;
use super::transport::DestinationTransport;
use super::*;
use crate::domains::common::{Command, dispatch};
use crate::storage::{CreateSessionRow, StorageBackend};
use everruns_contracts::typed_id::{AgentId, MessageId, PrincipalId, TurnId};
use everruns_core::{
    EgressRequest, EgressRequestKind, EgressScope, EgressService, Event, EventContext, OrgRole,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn customer_tracing_storage_is_tenant_scoped_and_revision_guarded() {
    let db = StorageBackend::test_database();
    let id = destination(&db, 1).await;
    assert!(db.tracing_destination(2, id).await.unwrap().is_none());
    assert!(
        !db.update_tracing_destination(2, id, 1, &config(), None)
            .await
            .unwrap()
    );
    let policy = RoutingPolicy {
        mode: RoutingMode::Replace,
        destination_ids: vec![id],
    };
    assert!(db.save_tracing_policy(2, None, 0, &policy).await.is_err());
    assert!(db.save_tracing_policy(1, None, 0, &policy).await.unwrap());
    assert!(
        !db.save_tracing_policy(1, None, 0, &RoutingPolicy::default())
            .await
            .unwrap()
    );
    assert!(db.delete_tracing_destination(1, id, 1).await.is_err());
    assert!(
        db.save_tracing_policy(
            1,
            None,
            1,
            &RoutingPolicy {
                mode: RoutingMode::Replace,
                destination_ids: vec![]
            }
        )
        .await
        .unwrap()
    );
    assert!(db.delete_tracing_destination(1, id, 1).await.unwrap());
}
#[tokio::test]
async fn customer_tracing_history_undo_preserves_credentials_and_rejects_stale_review() {
    let db = Arc::new(StorageBackend::test_database());
    let id = destination(&db, 1).await;
    let owner = ctx(db.clone(), OrgRole::Owner);
    let org = owner.caller.org_public_id.clone();
    commands::UpdateTracingPolicy {
        id: org.clone(),
        mode: RoutingMode::Replace,
        destination_ids: vec![id],
        expected_revision: Some(0),
    }
    .run(&owner)
    .await
    .unwrap();
    commands::UpdateTracingPolicy {
        id: org.clone(),
        mode: RoutingMode::Replace,
        destination_ids: vec![],
        expected_revision: Some(1),
    }
    .run(&owner)
    .await
    .unwrap();
    let output = dispatch(
        "restore_entity_revision",
        json!({"entity_ref":org,"kind":"tracing_policy","revision":2,"expected_revision":3}),
        &owner,
    )
    .await
    .unwrap();
    assert!(!output.contains("CUSTOMER_KEY"));
    assert_eq!(
        db.tracing_policy(1, None)
            .await
            .unwrap()
            .policy
            .destination_ids,
        vec![id]
    );
    assert!(dispatch("restore_entity_revision",json!({"entity_ref":owner.caller.org_public_id,"kind":"tracing_policy","revision":2,"expected_revision":3}),&owner).await.is_err());
    assert!(
        commands::GetTracingPolicy {
            id: owner.caller.org_public_id.clone()
        }
        .run(&ctx(db.clone(), OrgRole::Member))
        .await
        .is_err()
    );
    let mut readonly = owner.clone();
    readonly.caller.read_only = true;
    assert!(
        commands::UpdateTracingPolicy {
            id: org,
            mode: RoutingMode::Replace,
            destination_ids: vec![],
            expected_revision: None
        }
        .run(&readonly)
        .await
        .is_err()
    );
}
#[tokio::test]
async fn customer_tracing_revocation_and_authentication_are_enforced_at_send_time() {
    let db = Arc::new(StorageBackend::test_database());
    let id = destination(&db, 1).await;
    let row = db.tracing_destination(1, id).await.unwrap().unwrap();
    let collector = Arc::new(Collector::default());
    let boundary = DestinationTransport {
        db: db.clone(),
        inner: collector.clone(),
        row,
        scope: EgressScope {
            org_id: Some("org_00000000000000000000000000000001".parse().unwrap()),
            session_id: None,
        },
        secrets: TraceSecrets {
            headers: [("Authorization".into(), "Bearer CUSTOMER_KEY".into())].into(),
        },
    };
    let request = EgressRequest::new(
        "POST",
        "https://collector.example/v1/traces",
        EgressRequestKind::Other("tracing".into()),
    )
    .header("Authorization", "PLATFORM_KEY")
    .header("x-api-key", "PLATFORM_OTHER_KEY")
    .header("Content-Type", "application/x-protobuf");
    boundary.send(request.clone()).await.unwrap();
    {
        let requests = collector.requests.lock().unwrap();
        assert_eq!(
            requests[0].headers.get("Authorization").unwrap(),
            "Bearer CUSTOMER_KEY"
        );
        assert!(!requests[0].headers.contains_key("x-api-key"));
        assert!(requests[0].dns_pinning_required);
    }
    let mut paused = config();
    paused.enabled = false;
    assert!(
        db.update_tracing_destination(1, id, 1, &paused, None)
            .await
            .unwrap()
    );
    assert!(
        db.tracing_destination(1, id)
            .await
            .unwrap()
            .unwrap()
            .last_accepted_at
            .is_none(),
        "a new revision cannot inherit the previous receiver acceptance"
    );
    assert!(boundary.send(request).await.is_err());
    assert_eq!(collector.requests.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn customer_tracing_excludes_private_chat_and_uses_turn_routing() {
    let db = StorageBackend::test_database();
    let id = destination(&db, 1).await;
    db.save_tracing_policy(
        1,
        None,
        0,
        &RoutingPolicy {
            mode: RoutingMode::Replace,
            destination_ids: vec![id],
        },
    )
    .await
    .unwrap();
    let pool = db.database().pool();
    let harness:Uuid=sqlx::query_scalar("INSERT INTO harnesses (org_id,name,system_prompt) VALUES (1,'tracing-test','') RETURNING id").fetch_one(pool).await.unwrap();
    let agent = AgentId::new();
    sqlx::query("INSERT INTO agents (id,org_id,public_id,name,system_prompt,harness_id) VALUES ($1,1,$2,'tracing-test','',$3)").bind(agent.uuid()).bind(agent.to_string()).bind(harness).execute(pool).await.unwrap();
    let session = db
        .database()
        .create_session(CreateSessionRow {
            org_id: 1,
            agent_id: Some(agent),
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();
    let root_input = MessageId::new();
    db.record_continued_runtime_invocation(1, session.id, root_input.uuid(), Some(agent.uuid()))
        .await
        .unwrap();
    let context = EventContext::turn(TurnId::new(), root_input);
    let mut event = Event::new(
        session.id,
        context,
        everruns_core::events::TurnStartedData {
            turn_id: TurnId::new(),
            input_message_id: root_input,
            input_content: None,
            agent_id: Some(agent),
            agent_name: None,
            agent_description: None,
        },
    );
    assert_eq!(
        router::resolve_for_test(&db, &event).await.unwrap(),
        vec![id]
    );
    // A queued start must not acquire policy or content consent saved after it began.
    assert!(
        db.save_tracing_policy(
            1,
            None,
            1,
            &RoutingPolicy {
                mode: RoutingMode::Replace,
                destination_ids: vec![id],
            },
        )
        .await
        .unwrap()
    );
    assert!(
        router::resolve_for_test(&db, &event)
            .await
            .unwrap()
            .is_empty()
    );
    event.ts = chrono::Utc::now();
    assert_eq!(
        router::resolve_for_test(&db, &event).await.unwrap(),
        vec![id]
    );
    let mut changed_config = config();
    changed_config.capture_content = true;
    assert!(
        db.update_tracing_destination(1, id, 1, &changed_config, None)
            .await
            .unwrap()
    );
    assert!(
        router::resolve_for_test(&db, &event)
            .await
            .unwrap()
            .is_empty()
    );
    event.ts = chrono::Utc::now();
    assert_eq!(
        router::resolve_for_test(&db, &event).await.unwrap(),
        vec![id]
    );
    assert!(
        db.save_tracing_policy(1, Some(agent.uuid()), 0, &RoutingPolicy::default())
            .await
            .unwrap()
    );
    assert!(
        router::resolve_for_test(&db, &event)
            .await
            .unwrap()
            .is_empty()
    );
    event.ts = chrono::Utc::now();
    assert_eq!(
        router::resolve_for_test(&db, &event).await.unwrap(),
        vec![id]
    );
    let mut missing = event.clone();
    missing.context.input_message_id = None;
    assert!(
        router::resolve_for_test(&db, &missing)
            .await
            .unwrap()
            .is_empty()
    );
    // Worker event labels cannot choose an agent; the persisted invocation does.
    let guest = AgentId::new();
    sqlx::query("INSERT INTO agents (id,org_id,public_id,name,system_prompt,harness_id) VALUES ($1,1,$2,'guest','',$3)").bind(guest.uuid()).bind(guest.to_string()).bind(harness).execute(pool).await.unwrap();
    db.save_tracing_policy(
        1,
        Some(guest.uuid()),
        0,
        &RoutingPolicy {
            mode: RoutingMode::Off,
            destination_ids: vec![],
        },
    )
    .await
    .unwrap();
    let mut invocation_event = event.clone();
    let input = MessageId::new();
    invocation_event.context.input_message_id = Some(input);
    assert!(
        router::resolve_for_test(&db, &invocation_event)
            .await
            .unwrap()
            .is_empty()
    );
    // A new guest invocation has its own responder; a continued wake inherits the previous one.
    sqlx::query("INSERT INTO runtime_invocations (input_message_id,org_id,session_id,responder_agent_id) VALUES ($1,1,$2,$3)")
        .bind(input.uuid()).bind(session.id.uuid()).bind(guest.uuid()).execute(pool).await.unwrap();
    assert!(
        router::resolve_for_test(&db, &invocation_event)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        router::resolve_for_test(&db, &event).await.unwrap(),
        vec![id]
    );
    sqlx::query("UPDATE sessions SET source='chat' WHERE id=$1")
        .bind(session.id.uuid())
        .execute(pool)
        .await
        .unwrap();
    assert!(
        router::resolve_for_test(&db, &event)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn customer_tracing_fans_out_with_a_fixed_per_turn_snapshot() {
    use everruns_core::{EventListener, events::TurnCompletedData};
    let db = Arc::new(StorageBackend::test_database());
    let first = destination(&db, 1).await;
    let second = destination(&db, 1).await;
    db.save_tracing_policy(
        1,
        None,
        0,
        &RoutingPolicy {
            mode: RoutingMode::Replace,
            destination_ids: vec![first, second],
        },
    )
    .await
    .unwrap();
    let pool = db.database().pool();
    let harness: Uuid = sqlx::query_scalar(
        "INSERT INTO harnesses (org_id,name,system_prompt) VALUES (1,'fanout','') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let agent = AgentId::new();
    sqlx::query("INSERT INTO agents (id,org_id,public_id,name,system_prompt,harness_id) VALUES ($1,1,$2,'fanout','',$3)").bind(agent.uuid()).bind(agent.to_string()).bind(harness).execute(pool).await.unwrap();
    let session = db
        .database()
        .create_session(CreateSessionRow {
            org_id: 1,
            agent_id: Some(agent),
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();
    let turn = TurnId::new();
    let input = MessageId::new();
    db.record_continued_runtime_invocation(1, session.id, input.uuid(), Some(agent.uuid()))
        .await
        .unwrap();
    let context = EventContext::turn(turn, input);
    let collector = Arc::new(Collector::default());
    let listener =
        router::CustomerTracingListener::new(db.clone(), encryption(), collector.clone());
    let started = Event::new(
        session.id,
        context.clone(),
        everruns_core::events::TurnStartedData {
            turn_id: turn,
            input_message_id: input,
            input_content: Some("PRIVATE_PROMPT".into()),
            agent_id: Some(agent),
            agent_name: Some("fanout".into()),
            agent_description: None,
        },
    );
    listener.on_event(&started).await;
    listener.flush().await.unwrap();
    db.save_tracing_policy(
        1,
        None,
        1,
        &RoutingPolicy {
            mode: RoutingMode::Replace,
            destination_ids: vec![],
        },
    )
    .await
    .unwrap();
    let exhausted = listener.exhaust_budget_for_test();
    let completed = Event::new(
        session.id,
        context,
        TurnCompletedData {
            turn_id: turn,
            iterations: 1,
            final_answer_preview: Some("PRIVATE_ANSWER".into()),
            ..Default::default()
        },
    );
    listener.on_event(&completed).await;
    listener.flush().await.unwrap();
    drop(exhausted);
    db.save_tracing_policy(
        1,
        None,
        2,
        &RoutingPolicy {
            mode: RoutingMode::Replace,
            destination_ids: vec![first],
        },
    )
    .await
    .unwrap();
    listener.on_event(&started).await;
    listener.on_event(&completed).await;
    listener.flush().await.unwrap();
    let restarted =
        router::CustomerTracingListener::new(db.clone(), encryption(), collector.clone());
    restarted.on_event(&started).await;
    restarted.on_event(&completed).await;
    restarted.flush().await.unwrap();
    let requests = collector.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "routing edits do not redirect a running turn"
    );
    for request in requests.iter() {
        assert!(request.dns_pinning_required);
        assert_eq!(
            request.headers.get("Authorization").unwrap(),
            "Bearer CUSTOMER_KEY"
        );
        assert!(!String::from_utf8_lossy(&request.body).contains("PRIVATE_"));
    }
}

#[tokio::test]
async fn customer_tracing_changes_roll_back_with_the_shared_command_transaction() {
    let db = StorageBackend::test_database();
    let id = destination(&db, 1).await;
    let result: anyhow::Result<()> = crate::storage::transaction::scope(
        &db,
        async {
            db.save_tracing_policy(
                1,
                None,
                0,
                &RoutingPolicy {
                    mode: RoutingMode::Replace,
                    destination_ids: vec![id],
                },
            )
            .await?;
            let mut paused = config();
            paused.enabled = false;
            db.update_tracing_destination(1, id, 1, &paused, None)
                .await?;
            anyhow::bail!("later journal or command step failed")
        },
        anyhow::Error::from,
    )
    .await;
    assert!(result.is_err());
    assert_eq!(db.tracing_policy(1, None).await.unwrap().revision, 0);
    assert_eq!(
        db.tracing_destination(1, id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn customer_tracing_braintrust_uses_the_project_and_scoped_auth_without_content() {
    let db = Arc::new(StorageBackend::test_database());
    let project = Uuid::new_v4();
    let mut braintrust = config();
    braintrust.provider = TraceProvider::Braintrust;
    braintrust.project_id = Some(project);
    let id = db
        .create_tracing_destination(
            1,
            &braintrust,
            encryption()
                .encrypt_string(
                    &json!({"headers":{"Authorization":"Bearer CUSTOMER_KEY"}}).to_string(),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    let collector = Arc::new(Collector::default());
    let listener = router::make_listener(
        db.clone(),
        encryption(),
        collector.clone(),
        db.tracing_destination(1, id).await.unwrap().unwrap(),
        EgressScope::default(),
    )
    .await
    .unwrap();
    let session = everruns_contracts::typed_id::SessionId::new();
    let turn = TurnId::new();
    let message = MessageId::new();
    let context = EventContext::turn(turn, message);
    listener
        .on_event(&Event::new(
            session,
            context.clone(),
            everruns_core::events::TurnStartedData {
                turn_id: turn,
                input_message_id: message,
                input_content: Some("PRIVATE_PROMPT".into()),
                agent_id: None,
                agent_name: Some("Customer test".into()),
                agent_description: None,
            },
        ))
        .await;
    listener
        .on_event(&Event::new(
            session,
            context,
            everruns_core::events::TurnCompletedData {
                turn_id: turn,
                iterations: 1,
                final_answer_preview: Some("PRIVATE_ANSWER".into()),
                ..Default::default()
            },
        ))
        .await;
    listener.flush().await.unwrap();
    let requests = collector.requests.lock().unwrap();
    assert!(!requests.is_empty());
    for request in requests.iter() {
        assert!(
            request
                .url
                .contains(&format!("/project_logs/{project}/insert"))
        );
        assert_eq!(
            request.headers.get("Authorization").unwrap(),
            "Bearer CUSTOMER_KEY"
        );
        assert!(request.dns_pinning_required);
        assert!(!String::from_utf8_lossy(&request.body).contains("PRIVATE_"));
    }
}
