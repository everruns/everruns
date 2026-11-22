use super::*;

#[tokio::test]
async fn scheduled_invocation_replays_committed_result() {
    use crate::domains::agent_triggers::invoke_scheduled_agent_trigger;
    use everruns_durable::{TaskDefinition, TaskQueue};
    for mode in [SessionBinding::Ephemeral, SessionBinding::Shared] {
        let db = Arc::new(StorageBackend::test_database());
        let store = workflow_store(&db);
        let ctx = test_ctx(db.clone(), store.clone());
        let (agent_id, _) = seed_agent(&db).await;
        let mut req = create_req("0 9 * * *", "retry reproduction", true);
        req.session_mode = mode;
        let trigger = CreateAgentTrigger {
            agent_id: agent_id.clone(),
            req,
        }
        .execute(&ctx)
        .await
        .unwrap();
        let task = store
            .enqueue_task(TaskDefinition {
                workflow_id: None,
                activity_id: "schedule-retry".into(),
                activity_type: "invoke_agent_trigger".into(),
                input: serde_json::json!({"org_id": DEFAULT_ORG_ID, "agent_id": agent_id, "trigger_id": trigger.id.to_string()}),
                options: Default::default(),
            })
            .await
            .unwrap();
        let sessions = SessionService::new(db.clone());
        let runner = Arc::new(RecordingRunner::default());
        let messages = MessageService::new(db.clone(), runner.clone(), EventDelivery::in_memory());
        let first = invoke_scheduled_agent_trigger(
            &db,
            &sessions,
            &messages,
            None,
            None,
            DEFAULT_ORG_ID,
            &agent_id,
            &trigger.id.to_string(),
            task,
        )
        .await
        .unwrap();
        // The effect committed but the caller lost its acknowledgement.
        let second = invoke_scheduled_agent_trigger(
            &db,
            &sessions,
            &messages,
            None,
            None,
            DEFAULT_ORG_ID,
            &agent_id,
            &trigger.id.to_string(),
            task,
        )
        .await
        .unwrap();
        let deliveries = db
            .list_agent_trigger_deliveries(DEFAULT_ORG_ID, trigger.id, 10)
            .await
            .unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(runner.active_count().await, 1);
        assert_eq!(first.session_id, second.session_id);
        assert_eq!(first.created_session, second.created_session);
        let wrong_agent = invoke_scheduled_agent_trigger(
            &db,
            &sessions,
            &messages,
            None,
            None,
            DEFAULT_ORG_ID,
            "agent_wrong",
            &trigger.id.to_string(),
            task,
        )
        .await;
        assert!(wrong_agent.is_err(), "a receipt cannot cross task identity");

        let next_task = store.enqueue_task(TaskDefinition {
            workflow_id: None, activity_id: "schedule-next".into(),
            activity_type: "invoke_agent_trigger".into(),
            input: serde_json::json!({"org_id": DEFAULT_ORG_ID, "agent_id": agent_id, "trigger_id": trigger.id.to_string()}),
            options: Default::default(),
        }).await.unwrap();
        let next = invoke_scheduled_agent_trigger(
            &db,
            &sessions,
            &messages,
            None,
            None,
            DEFAULT_ORG_ID,
            &agent_id,
            &trigger.id.to_string(),
            next_task,
        )
        .await
        .unwrap();
        if mode == SessionBinding::Ephemeral {
            assert_ne!(first.session_id, next.session_id);
        } else {
            assert_eq!(first.session_id, next.session_id);
        }
        assert_eq!(
            runner.active_count().await,
            2,
            "a different occurrence still runs"
        );
    }
}

#[tokio::test]
async fn scheduled_invocation_rolls_back_before_ack_and_serializes_retries() {
    use crate::domains::agent_triggers::invoke_scheduled_agent_trigger;
    use everruns_durable::{TaskDefinition, TaskQueue};
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
    let ctx = test_ctx(db.clone(), store.clone());
    let (agent_id, _) = seed_agent(&db).await;
    let mut req = create_req("0 9 * * *", "crash before commit", true);
    req.session_mode = SessionBinding::Ephemeral;
    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req,
    }
    .execute(&ctx)
    .await
    .unwrap();
    let trigger_id = trigger.id.to_string();
    let task = store
        .enqueue_task(TaskDefinition {
            workflow_id: None,
            activity_id: "schedule-rollback".into(),
            activity_type: "invoke_agent_trigger".into(),
            input: serde_json::json!({"org_id": DEFAULT_ORG_ID, "agent_id": agent_id, "trigger_id": trigger.id.to_string()}),
            options: Default::default(),
        })
        .await
        .unwrap();
    let sessions = SessionService::new(db.clone());
    let runner = Arc::new(RecordingRunner::default());
    let messages = MessageService::new(db.clone(), runner.clone(), EventDelivery::in_memory());
    let rollback: Result<(), CommandError> = crate::storage::transaction::scope(
        &db,
        async {
            invoke_scheduled_agent_trigger(
                &db,
                &sessions,
                &messages,
                None,
                None,
                DEFAULT_ORG_ID,
                &agent_id,
                &trigger_id,
                task,
            )
            .await?;
            Err(CommandError::internal(anyhow::anyhow!(
                "injected failure before commit"
            )))
        },
        CommandError::internal,
    )
    .await;
    assert!(rollback.is_err());
    assert!(
        db.scheduled_trigger_invocation(task, DEFAULT_ORG_ID, trigger.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.list_agent_trigger_deliveries(DEFAULT_ORG_ID, trigger.id, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE trigger_id = $1")
        .bind(trigger.id)
        .fetch_one(db.database().pool())
        .await
        .unwrap();
    assert_eq!(count, 0, "failed dispatch must not orphan a new session");

    let (first, second) = tokio::join!(
        invoke_scheduled_agent_trigger(
            &db,
            &sessions,
            &messages,
            None,
            None,
            DEFAULT_ORG_ID,
            &agent_id,
            &trigger_id,
            task
        ),
        invoke_scheduled_agent_trigger(
            &db,
            &sessions,
            &messages,
            None,
            None,
            DEFAULT_ORG_ID,
            &agent_id,
            &trigger_id,
            task
        ),
    );
    assert_eq!(first.unwrap().session_id, second.unwrap().session_id);
    assert_eq!(
        db.list_agent_trigger_deliveries(DEFAULT_ORG_ID, trigger.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn failed_scheduled_attempt_leaves_a_failed_delivery() {
    use crate::domains::agent_triggers::invoke_scheduled_agent_trigger;
    use everruns_durable::{TaskDefinition, TaskQueue};
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
    let ctx = test_ctx(db.clone(), store.clone());
    let (agent_id, _) = seed_agent(&db).await;
    // Disabled: the attempt fails inside its transaction and rolls back.
    let mut req = create_req("0 9 * * *", "daily run", false);
    req.session_mode = SessionBinding::Ephemeral;
    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req,
    }
    .execute(&ctx)
    .await
    .unwrap();
    let task = store
        .enqueue_task(TaskDefinition {
            workflow_id: None,
            activity_id: "schedule-failed".into(),
            activity_type: "invoke_agent_trigger".into(),
            input: serde_json::json!({"org_id": DEFAULT_ORG_ID, "agent_id": agent_id, "trigger_id": trigger.id.to_string()}),
            options: Default::default(),
        })
        .await
        .unwrap();
    let sessions = SessionService::new(db.clone());
    let runner = Arc::new(RecordingRunner::default());
    let messages = MessageService::new(db.clone(), runner.clone(), EventDelivery::in_memory());
    let error = invoke_scheduled_agent_trigger(
        &db,
        &sessions,
        &messages,
        None,
        None,
        DEFAULT_ORG_ID,
        &agent_id,
        &trigger.id.to_string(),
        task,
    )
    .await
    .expect_err("a disabled trigger does not run");
    let deliveries = db
        .list_agent_trigger_deliveries(DEFAULT_ORG_ID, trigger.id, 10)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1, "the failure survives the rollback");
    assert_eq!(deliveries[0].status, "failed");
    assert_eq!(deliveries[0].source, "schedule");
    assert_eq!(
        deliveries[0].reason.as_deref(),
        Some(error.to_string().as_str())
    );
    assert!(deliveries[0].session_id.is_none());
    assert_eq!(runner.active_count().await, 0);

    // A task of another org records nothing on this org's trigger.
    let _ = invoke_scheduled_agent_trigger(
        &db,
        &sessions,
        &messages,
        None,
        None,
        DEFAULT_ORG_ID + 1,
        &agent_id,
        &trigger.id.to_string(),
        task,
    )
    .await
    .expect_err("wrong org");
    assert_eq!(
        db.list_agent_trigger_deliveries(DEFAULT_ORG_ID, trigger.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn trigger_run_reports_its_task_outcome() {
    use crate::domains::agent_triggers::types::AgentTriggerRun;
    use everruns_durable::{TaskInfo, TaskStatus};
    let queued_at = chrono::Utc::now();
    let run = || AgentTriggerRun {
        id: "run".into(),
        status: "completed".into(),
        scheduled_at: queued_at,
        completed_at: Some(queued_at),
        error: None,
    };
    let task = |status, attempt, last_error: Option<&str>| TaskInfo {
        id: uuid::Uuid::nil(),
        workflow_id: None,
        activity_id: "scheduled".into(),
        activity_type: "invoke_agent_trigger".into(),
        status,
        priority: 0,
        attempt,
        max_attempts: 3,
        claimed_by: None,
        last_error: last_error.map(str::to_string),
        created_at: queued_at,
        claimed_at: None,
        queue: None,
    };

    let mut failed = run();
    apply_task_outcome(&mut failed, &task(TaskStatus::Dead, 3, Some("boom")));
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.error.as_deref(), Some("boom"));

    let mut retrying = run();
    apply_task_outcome(
        &mut retrying,
        &task(TaskStatus::Pending, 1, Some("transient")),
    );
    assert_eq!(retrying.status, "retrying");
    assert_eq!(retrying.completed_at, None);
    assert_eq!(retrying.error.as_deref(), Some("transient"));

    let mut queued = run();
    apply_task_outcome(&mut queued, &task(TaskStatus::Pending, 0, None));
    assert_eq!(queued.status, "queued");

    let mut done = run();
    apply_task_outcome(&mut done, &task(TaskStatus::Completed, 1, None));
    assert_eq!(done.status, "completed");
    assert_eq!(done.completed_at, Some(queued_at));
}
