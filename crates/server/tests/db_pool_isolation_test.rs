//! Background sweeps must not be starved by a request burst (EVE-1081).
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set (or the `just start-infra` default)
//!
//! Production evidence (Sentry EVERRUNS-13/15/16/B, release 364f27cf): four
//! independent background loops — the durable scheduler, the observer scoring
//! worker and the sweeps around them — all reported
//! `pool timed out while waiting for an open connection` within two seconds of
//! each other, with zero users impacted. Every one of them was drawing from the
//! single process-wide `sqlx` pool, so one request burst that held every
//! connection for longer than the 5s acquire timeout failed all of them at the
//! same instant.
//!
//! These tests pin both halves of that: the shared pool really does fail every
//! background acquire together, and the dedicated background pool really does
//! keep them running through the same burst.

use chrono::Utc;
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::events::{EventContext, EventRequest, ToolCompletedData};
use everruns_durable::{PostgresWorkflowEventStore, WorkflowEventStore, WorkflowStatus};
use everruns_platform::SessionSource;
use everruns_provider::typed_id::{MessageId, PrincipalId, SessionId, TurnId};
use everruns_server::EventDelivery;
use everruns_server::domains::session_schedules::SessionScheduleService;
use everruns_server::services::EventService;
use everruns_server::services::waiting_turn_resolution::execute_waiting_turn_resolution;
use everruns_server::storage::StorageBackend;
use everruns_server::storage::models::{
    ClaimWaitingTurnResult, CreateHarnessRow, CreatePrincipalRow, CreateSessionRow,
    CreateSessionScheduleRow, UpdateSession, WaitingTurnResolutionPlan,
};
use everruns_server::storage::repositories::{Database, DatabasePoolConfig};
use everruns_worker::{RunnerBackend, create_runner_with_backend};
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://everruns:everruns@localhost:{port}/everruns_test")
    })
}

/// Prod's shape, scaled down so the test runs in seconds: a small request pool
/// with a short acquire timeout, mirroring `DATABASE_POOL_MAX` / the 5s
/// `acquire_timeout` default.
fn scaled_config(background_max: u32) -> DatabasePoolConfig {
    DatabasePoolConfig {
        max_connections: 4,
        min_connections: 1,
        acquire_timeout: Duration::from_millis(500),
        idle_timeout: Duration::from_secs(300),
        background_max_connections: background_max,
        background_acquire_timeout: Duration::from_secs(10),
    }
}

/// Hold every connection in the request pool, the way a burst of HTTP handlers
/// does. The guards are returned so the caller controls when the burst ends.
async fn saturate(
    pool: &sqlx::PgPool,
    count: usize,
) -> Vec<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    let mut held = Vec::new();
    for _ in 0..count {
        held.push(
            pool.acquire()
                .await
                .expect("request burst should be able to fill an idle pool"),
        );
    }
    held
}

/// The three background loops Sentry caught failing together.
const SWEEPS: [&str; 3] = ["durable_scheduler", "observer_worker", "tool_result_sweep"];

/// Run all three concurrently, each doing one trivial query. Prints how long
/// each waited and how it ended, so the run itself is the evidence: under one
/// shared pool they all end at the same instant.
async fn run_sweeps(db: &Database, label: &str) -> Vec<Result<(), String>> {
    let mut set = tokio::task::JoinSet::new();
    for name in SWEEPS {
        let db = db.clone();
        set.spawn(async move {
            let started = std::time::Instant::now();
            let outcome = sqlx::query("SELECT 1 AS one")
                .fetch_one(db.pool())
                .await
                .map(|row| {
                    let _: i32 = row.get("one");
                })
                .map_err(|e| e.to_string());
            (name, started.elapsed(), outcome)
        });
    }
    let mut out = Vec::new();
    while let Some(joined) = set.join_next().await {
        let (name, elapsed, outcome) = joined.expect("sweep task panicked");
        match &outcome {
            Ok(()) => println!("[{label}] {name}: ok after {elapsed:?}"),
            Err(error) => println!("[{label}] {name}: after {elapsed:?} -> {error}"),
        }
        out.push(outcome);
    }
    out
}

async fn background_runtime() -> (
    Database,
    Arc<StorageBackend>,
    Arc<StorageBackend>,
    Arc<dyn everruns_worker::AgentRunner>,
) {
    let database = Database::connect_with_config(&database_url(), scaled_config(2))
        .await
        .expect("connect");
    let request_db = Arc::new(StorageBackend::Postgres(database.clone()));
    let background_db = Arc::new(request_db.for_background());
    let runner =
        create_runner_with_backend(RunnerBackend::Postgres(database.background_pool().clone()))
            .await
            .expect("create background runner");
    (database, request_db, background_db, runner)
}

async fn create_background_test_session(
    db: &Arc<StorageBackend>,
    label: &str,
) -> everruns_server::storage::SessionRow {
    let principal_id = PrincipalId::new();
    db.create_principal(CreatePrincipalRow {
        id: principal_id,
        org_id: DEFAULT_ORG_ID,
        kind: "system".to_string(),
        subject_id: Some(Uuid::now_v7()),
        parent_principal_id: None,
        resolved_user_id: None,
        metadata: json!({ "source": "db_pool_isolation_test" }),
    })
    .await
    .expect("create test principal");
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: format!("{label}-{}", Uuid::now_v7()),
                display_name: Some("Background pool isolation".to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: Some("Test background execution.".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                embedder_metadata: json!({}),
                is_built_in: false,
            },
        )
        .await
        .expect("create test harness");
    db.create_session(CreateSessionRow {
        source: SessionSource::Api,
        workspace_id: None,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        endpoint_id: None,
        harness_id: Some(harness.id),
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: principal_id,
        resolved_owner_user_id: None,
        title: Some(label.to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: json!([]),
        tools: json!([]),
        mcp_servers: json!({}),
        system_prompt: None,
        initial_files: json!([]),
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
    .expect("create test session")
}

async fn wait_for_task(pool: &sqlx::PgPool, session_id: SessionId, activity_type: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM durable_task_queue \
             WHERE workflow_id = $1 AND activity_type = $2 AND status = 'pending')",
        )
        .bind(session_id.uuid())
        .bind(activity_type)
        .fetch_one(pool)
        .await
        .expect("query durable task");
        if exists {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{activity_type} task was not enqueued for {session_id}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The bug: with one pool for everything, a request burst fails every
/// background sweep at once.
#[tokio::test(flavor = "multi_thread")]
async fn shared_pool_fails_every_background_sweep_at_once() {
    // `background_max_connections: 0` is the documented escape hatch that puts
    // background work back on the request pool — i.e. the pre-EVE-1081 shape.
    let db = Database::connect_with_config(&database_url(), scaled_config(0))
        .await
        .expect("connect");

    let _burst = saturate(db.pool(), 4).await;
    let results = run_sweeps(&db, "shared pool").await;

    assert!(
        results.iter().all(|r| r.is_err()),
        "expected the shared pool to starve every sweep, got {results:?}"
    );
    for result in &results {
        let error = result.as_ref().unwrap_err();
        assert!(
            error.contains("pool timed out"),
            "expected the production error, got {error}"
        );
    }
}

/// The fix: the same burst leaves the background pool untouched.
#[tokio::test(flavor = "multi_thread")]
async fn dedicated_background_pool_survives_a_request_burst() {
    let db = Database::connect_with_config(&database_url(), scaled_config(2))
        .await
        .expect("connect");
    let background = db.for_background();

    let _burst = saturate(db.pool(), 4).await;
    let results = run_sweeps(&background, "background pool").await;

    assert!(
        results.iter().all(|r| r.is_ok()),
        "background sweeps must survive a request burst, got {results:?}"
    );
}

/// The request pool still fails fast while the burst is in flight — the fix
/// must not paper over saturation on the path where a caller is waiting.
#[tokio::test(flavor = "multi_thread")]
async fn request_pool_still_fails_fast_under_the_same_burst() {
    let db = Database::connect_with_config(&database_url(), scaled_config(2))
        .await
        .expect("connect");

    let _burst = saturate(db.pool(), 4).await;
    let results = run_sweeps(&db, "request pool").await;

    assert!(
        results.iter().all(|r| r.is_err()),
        "request-path acquires must still time out, got {results:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_enqueue_survives_a_saturated_request_pool() {
    let (database, request_db, background_db, runner) = background_runtime().await;
    let session = create_background_test_session(&request_db, "scheduled-background").await;
    let schedule = background_db
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: DEFAULT_ORG_ID,
            session_id: session.id,
            owner_principal_id: session.owner_principal_id,
            resolved_owner_user_id: session.resolved_owner_user_id,
            description: "Run from the reserved pool".to_string(),
            cron_expression: None,
            scheduled_at: Some(Utc::now() - chrono::Duration::minutes(1)),
            timezone: "UTC".to_string(),
            next_trigger_at: Some(Utc::now() - chrono::Duration::days(3650)),
        })
        .await
        .expect("create overdue schedule");
    let event_service = Arc::new(EventService::with_listeners(
        background_db.clone(),
        EventDelivery::in_memory(),
        vec![],
    ));
    let schedule_service = Arc::new(SessionScheduleService::new(background_db.clone()));

    let _burst = saturate(database.pool(), 4).await;
    let scheduler = everruns_server::session_scheduler::spawn_session_scheduler(
        background_db.clone(),
        schedule_service,
        event_service,
        runner,
        None,
        Duration::from_millis(10),
    );

    wait_for_task(database.background_pool(), session.id, "process_input").await;
    scheduler.abort();

    let updated = background_db
        .get_session_schedule(DEFAULT_ORG_ID, schedule.id)
        .await
        .expect("load fired schedule")
        .expect("schedule exists");
    assert_eq!(updated.trigger_count, 1);
    assert!(!updated.enabled);
    let events = background_db
        .list_events(
            session.id,
            None,
            None,
            &["input.message".to_string()],
            &[],
            None,
            Some(10),
        )
        .await
        .expect("load scheduled input event");
    assert_eq!(events.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_resolution_survives_a_saturated_request_pool() {
    let (database, request_db, background_db, runner) = background_runtime().await;
    let session = create_background_test_session(&request_db, "timeout-background").await;
    background_db
        .update_session(
            DEFAULT_ORG_ID,
            session.id,
            UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("park session")
        .expect("session exists");

    let saved_turn = json!({
        "org_id": DEFAULT_ORG_ID,
        "session_id": session.id,
        "harness_id": session.harness_id.expect("test session harness"),
        "agent_id": null,
        "input_message_id": MessageId::new(),
        "turn_id": null,
        "previous_response_id": null,
        "iteration": 1,
        "request_id": null,
        "started_at": null,
        "cumulative_usage": null,
        "tool_call_count": 0,
        "llm_call_count": 0,
        "time_to_first_token_ms": null,
        "final_message_id": null,
        "final_answer_preview": null
    });
    let durable_store = PostgresWorkflowEventStore::new(database.background_pool().clone());
    durable_store
        .create_workflow(session.id.uuid(), "turn_workflow", saved_turn.clone(), None)
        .await
        .expect("create parked workflow");
    durable_store
        .update_workflow_status(
            session.id.uuid(),
            WorkflowStatus::Completed,
            Some(saved_turn),
            None,
        )
        .await
        .expect("save parked turn input");

    let tool_call_id = format!("call_{}", Uuid::now_v7());
    let event = EventRequest::new(
        session.id,
        EventContext::turn(
            TurnId::from_uuid(session.id.uuid()),
            MessageId::from_uuid(session.id.uuid()),
        ),
        ToolCompletedData::failure(
            tool_call_id.clone(),
            String::new(),
            "timeout".to_string(),
            "Timed out waiting for client tool results".to_string(),
            None,
        ),
    );
    let claim = match background_db
        .recover_waiting_turn(
            DEFAULT_ORG_ID,
            session.id,
            WaitingTurnResolutionPlan {
                kind: "timeout".to_string(),
                events: vec![event],
                session_values: vec![],
                response: serde_json::Value::Null,
            },
        )
        .await
        .expect("claim waiting turn")
    {
        ClaimWaitingTurnResult::Claimed(claim) => claim,
        other => panic!("expected waiting-turn claim, got {other:?}"),
    };
    let event_service = EventService::new(background_db.clone(), EventDelivery::in_memory());

    let _burst = saturate(database.pool(), 4).await;
    execute_waiting_turn_resolution(
        &background_db,
        &event_service,
        &runner,
        DEFAULT_ORG_ID,
        session.id,
        &claim,
    )
    .await
    .expect("resolve timed-out turn through background pool");

    wait_for_task(database.background_pool(), session.id, "reason").await;
    let events = background_db
        .list_events(
            session.id,
            None,
            None,
            &["tool.completed".to_string()],
            &[],
            None,
            Some(10),
        )
        .await
        .expect("load timeout event");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["tool_call_id"], tool_call_id);
    assert_eq!(
        background_db
            .get_session(DEFAULT_ORG_ID, session.id)
            .await
            .expect("load resumed session")
            .expect("session exists")
            .status,
        "active"
    );
}
