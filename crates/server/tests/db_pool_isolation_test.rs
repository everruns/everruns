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
use everruns_durable::{PostgresWorkflowEventStore, WorkflowEventStore, WorkflowStatus};
use everruns_platform::SessionSource;
use everruns_provider::typed_id::{MessageId, PrincipalId, SessionId};
use everruns_server::app_builder::{ServerAppBuilder, ServerContext};
use everruns_server::server::ServerConfig;
use everruns_server::storage::StorageBackend;
use everruns_server::storage::models::{
    CreateEventRow, CreateHarnessRow, CreatePrincipalRow, CreateSessionRow,
    CreateSessionScheduleRow, UpdateSession,
};
use everruns_server::storage::repositories::{Database, DatabasePoolConfig};
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
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

fn configure_server_environment() {
    // SAFETY: this integration binary is run with `--test-threads=1`; all
    // server configuration is installed before the application task starts.
    unsafe {
        std::env::set_var("DATABASE_URL", database_url());
        std::env::set_var("DATABASE_POOL_MAX", "4");
        std::env::set_var("DATABASE_POOL_MIN", "1");
        std::env::set_var("DATABASE_ACQUIRE_TIMEOUT_SECS", "1");
        std::env::set_var("DATABASE_BACKGROUND_POOL_MAX", "8");
        std::env::set_var("DATABASE_BACKGROUND_ACQUIRE_TIMEOUT_SECS", "10");
        std::env::set_var("DEPLOYMENT_GRADE", "dev");
        std::env::set_var("AUTH_MODE", "none");
        std::env::set_var("WORKER_GRPC_AUTH_TOKEN", "db-pool-isolation-test");
        std::env::set_var("TOOL_RESULT_TIMEOUT_SECS", "0");
    }
}

async fn start_real_server() -> (JoinHandle<anyhow::Result<()>>, ServerContext) {
    configure_server_environment();
    let config = ServerConfig {
        dev_mode: false,
        no_migrations: true,
        api_prefix: String::new(),
        cors_origins: vec![],
        addr: "127.0.0.1:0".to_string(),
        grpc_addr: "127.0.0.1:0".to_string(),
    };
    let (context_tx, context_rx) = tokio::sync::oneshot::channel();
    let mut server = tokio::spawn(
        ServerAppBuilder::new(config)
            .background_task(move |context| async move {
                let _ = context_tx.send(context);
                futures::future::pending::<()>().await;
            })
            .run(),
    );
    let context = tokio::select! {
        context = context_rx => context.expect("server context sender dropped"),
        result = &mut server => panic!("server exited during startup: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(30)) => panic!("server startup timed out"),
    };
    (server, context)
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

async fn wait_for_server_sweeps(
    pool: &sqlx::PgPool,
    schedule_id: Uuid,
    scheduled_session_id: SessionId,
    timeout_session_id: SessionId,
    tool_call_id: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let outcomes: (bool, bool, bool, bool, bool, bool) = sqlx::query_as(
            r#"
            SELECT
                EXISTS(
                    SELECT 1 FROM session_schedules
                    WHERE id = $1 AND trigger_count = 1 AND enabled = false
                ),
                EXISTS(
                    SELECT 1 FROM events
                    WHERE session_id = $2 AND event_type = 'input.message'
                ),
                EXISTS(
                    SELECT 1 FROM durable_task_queue
                    WHERE workflow_id = $2
                      AND activity_type = 'process_input'
                      AND status = 'pending'
                ),
                EXISTS(
                    SELECT 1 FROM events
                    WHERE session_id = $3
                      AND event_type = 'tool.completed'
                      AND data->>'tool_call_id' = $4
                ),
                EXISTS(
                    SELECT 1 FROM durable_task_queue
                    WHERE workflow_id = $3
                      AND activity_type = 'reason'
                      AND status = 'pending'
                ),
                EXISTS(
                    SELECT 1 FROM sessions
                    WHERE id = $3 AND status = 'active'
                )
            "#,
        )
        .bind(schedule_id)
        .bind(scheduled_session_id.uuid())
        .bind(timeout_session_id.uuid())
        .bind(tool_call_id)
        .fetch_one(pool)
        .await
        .expect("query background sweep outcomes");
        if outcomes == (true, true, true, true, true, true) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "built-in sweep outcomes incomplete: schedule_triggered={}, input_event={}, \
             process_input_task={}, timeout_event={}, reason_task={}, timeout_session_active={}",
            outcomes.0,
            outcomes.1,
            outcomes.2,
            outcomes.3,
            outcomes.4,
            outcomes.5,
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
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

#[tokio::test]
async fn built_in_background_sweeps_survive_a_saturated_request_pool() {
    let (server, context) = start_real_server().await;
    let observer = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("connect observer pool");

    let scheduled_session =
        create_background_test_session(&context.db, "scheduled-background").await;
    let schedule = context
        .db
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: DEFAULT_ORG_ID,
            session_id: scheduled_session.id,
            owner_principal_id: scheduled_session.owner_principal_id,
            resolved_owner_user_id: scheduled_session.resolved_owner_user_id,
            description: "Run from the reserved pool".to_string(),
            cron_expression: None,
            scheduled_at: Some(Utc::now() - chrono::Duration::minutes(1)),
            timezone: "UTC".to_string(),
            next_trigger_at: Some(Utc::now() - chrono::Duration::days(3650)),
        })
        .await
        .expect("create overdue schedule");

    let timeout_session = create_background_test_session(&context.db, "timeout-background").await;
    context
        .db
        .update_session(
            DEFAULT_ORG_ID,
            timeout_session.id,
            UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("park session")
        .expect("session exists");
    let tool_call_id = format!("call_{}", Uuid::now_v7());
    context
        .db
        .create_event(CreateEventRow {
            session_id: timeout_session.id,
            event_type: "tool.call_requested".to_string(),
            ts: Utc::now() - chrono::Duration::minutes(10),
            context: json!({}),
            data: json!({
                "tool_calls": [{
                    "id": tool_call_id,
                    "name": "client_tool",
                    "arguments": {}
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("record pending client tool call");

    let saved_turn = json!({
        "org_id": DEFAULT_ORG_ID,
        "session_id": timeout_session.id,
        "harness_id": timeout_session.harness_id.expect("test session harness"),
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
    let durable_store = PostgresWorkflowEventStore::new(observer.clone());
    durable_store
        .create_workflow(
            timeout_session.id.uuid(),
            "turn_workflow",
            saved_turn.clone(),
            None,
        )
        .await
        .expect("create parked workflow");
    durable_store
        .update_workflow_status(
            timeout_session.id.uuid(),
            WorkflowStatus::Completed,
            Some(saved_turn),
            None,
        )
        .await
        .expect("save parked turn input");
    let _burst = saturate(context.db.pool().expect("request pool"), 4).await;
    let request_error = context
        .db
        .get_session(DEFAULT_ORG_ID, scheduled_session.id)
        .await
        .expect_err("request-backed reads must remain subject to saturation");
    assert!(
        request_error.to_string().contains("pool timed out"),
        "expected request-pool timeout, got {request_error}"
    );

    wait_for_server_sweeps(
        &observer,
        schedule.id.uuid(),
        scheduled_session.id,
        timeout_session.id,
        &tool_call_id,
    )
    .await;

    server.abort();
    let _ = server.await;
}
