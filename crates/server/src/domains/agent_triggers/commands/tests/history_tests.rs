use super::*;

#[tokio::test]
#[allow(clippy::await_holding_lock)] // exclude the env-mutating cap test
async fn investigation_trigger_history_pages_stably_and_scopes_cursors() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
    let ctx = test_ctx(db.clone(), store.clone());
    let (agent_id, _) = seed_agent(&db).await;
    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 9 * * *", "hello", true),
    }
    .run(&ctx)
    .await
    .unwrap();
    let schedule = db
        .get_agent_trigger(DEFAULT_ORG_ID, trigger.id)
        .await
        .unwrap()
        .unwrap()
        .durable_schedule_id
        .unwrap();
    let at = Utc::now();
    let mut executions = Vec::new();
    for _ in 0..3 {
        executions.push(store.create_schedule_execution(schedule, at).await.unwrap());
    }
    // Legacy manual fires stored a session in task_id without recording source.
    store
        .complete_schedule_execution(executions[0], SessionId::new().uuid(), false)
        .await
        .unwrap();
    let page = |before| ListAgentTriggerRuns {
        agent_id: agent_id.clone(),
        trigger_id: trigger.id.to_string(),
        from: Some(at),
        to: Some(at + chrono::Duration::seconds(1)),
        before,
        limit: Some(2),
    };
    let first = page(None).run(&ctx).await.unwrap();
    assert_eq!(first.len(), 2);
    let last: Uuid = first[1].id.parse().unwrap();
    let second = page(Some(last)).run(&ctx).await.unwrap();
    assert_eq!(second.len(), 1);
    assert!(first.iter().all(|a| second.iter().all(|b| a.id != b.id)));
    assert!(
        first
            .iter()
            .all(|r| r.source.is_none() && r.requested_by.is_none()),
        "old unknown attribution stays unknown"
    );
    let other = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 10 * * *", "other", true),
    }
    .run(&ctx)
    .await
    .unwrap();
    let wrong_parent = ListAgentTriggerRuns {
        agent_id: agent_id.clone(),
        trigger_id: other.id.to_string(),
        before: Some(last),
        ..Default::default()
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert!(matches!(wrong_parent.kind, CommandErrorKind::NotFound(_)));
    let mut invalid = page(None);
    invalid.to = Some(at);
    assert!(matches!(
        invalid.run(&ctx).await.unwrap_err().kind,
        CommandErrorKind::BadRequest(_)
    ));
    for id in executions {
        assert!(
            first
                .iter()
                .chain(second.iter())
                .any(|r| r.id == id.to_string())
        );
    }
}

#[tokio::test]
async fn investigation_delivery_retention_keeps_busy_days_and_pages_the_full_history() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db.clone(), OrgRole::Owner);
    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: webhook_req(false),
    }
    .run(&ctx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO agent_trigger_deliveries (org_id,trigger_id,source,status,created_at) SELECT $1,$2,'webhook','dispatched',CASE WHEN n=0 THEN NOW()-INTERVAL '91 days' ELSE NOW() END FROM generate_series(0,205) n")
        .bind(DEFAULT_ORG_ID).bind(trigger.id).execute(db.database().pool()).await.unwrap();
    assert_eq!(
        db.database()
            .prune_agent_trigger_delivery_history(trigger.id, 90)
            .await
            .unwrap(),
        1
    );
    let page = |before| ListAgentTriggerDeliveries {
        agent_id: agent_id.clone(),
        trigger_id: trigger.id.to_string(),
        from: None,
        to: None,
        before,
        limit: Some(200),
    };
    let first = page(None).run(&ctx).await.unwrap();
    let second = page(Some(first.last().unwrap().id))
        .run(&ctx)
        .await
        .unwrap();
    assert_eq!(first.len(), 200);
    assert_eq!(
        second.len(),
        5,
        "a busy day is not erased by a page-size cap"
    );
    assert!(first.iter().all(|a| second.iter().all(|b| a.id != b.id)));
}

#[tokio::test]
async fn investigation_legacy_schedule_labels_do_not_claim_automatic_attribution() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db.clone(), OrgRole::Owner);
    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: webhook_req(false),
    }
    .run(&ctx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO agent_trigger_deliveries (org_id,trigger_id,source,source_version,status) SELECT $1,$2,src,ver,'failed' FROM (VALUES ('schedule',0),('schedule',1),('manual',0)) fixture(src,ver)")
        .bind(DEFAULT_ORG_ID).bind(trigger.id).execute(db.database().pool()).await.unwrap();
    let deliveries = ListAgentTriggerDeliveries {
        agent_id,
        trigger_id: trigger.id.to_string(),
        from: None,
        to: None,
        before: None,
        limit: None,
    }
    .run(&ctx)
    .await
    .unwrap();
    let mut sources = deliveries.into_iter().map(|d| d.source).collect::<Vec<_>>();
    sources.sort();
    assert_eq!(sources, vec!["manual", "schedule", "unknown"]);
}
