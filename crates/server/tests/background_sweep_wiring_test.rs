//! Real-server regression for background scheduler and timeout sweep wiring.
//!
//! This is a dedicated single-test binary because the production builder reads
//! process environment. The synchronous test installs that environment before
//! constructing Tokio, so no test or runtime thread can observe partial config.

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
use serde_json::json;
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

fn configure_server_environment() {
    // SAFETY: this integration binary contains one synchronous test. It sets
    // every variable before constructing Tokio, and no sibling test can run in
    // this process.
    unsafe {
        std::env::set_var("DATABASE_URL", database_url());
        std::env::set_var("DATABASE_POOL_MAX", "4");
        std::env::set_var("DATABASE_POOL_MIN", "1");
        std::env::set_var("DATABASE_ACQUIRE_TIMEOUT_SECS", "1");
        std::env::set_var("DATABASE_BACKGROUND_POOL_MAX", "8");
        std::env::set_var("DATABASE_BACKGROUND_ACQUIRE_TIMEOUT_SECS", "10");
        std::env::set_var("DEPLOYMENT_GRADE", "dev");
        std::env::set_var("AUTH_MODE", "none");
        std::env::set_var("WORKER_GRPC_AUTH_TOKEN", "background-sweep-wiring-test");
        std::env::set_var("TOOL_RESULT_TIMEOUT_SECS", "0");
        // Production waits 15s/30s for the first scheduler poll and timeout
        // sweep; the wiring under test is the same at 1s.
        std::env::set_var("SESSION_SCHEDULER_POLL_INTERVAL_SECS", "1");
        std::env::set_var("TOOL_RESULT_TIMEOUT_SWEEP_INTERVAL_SECS", "1");
    }
}

async fn saturate(
    pool: &sqlx::PgPool,
    count: usize,
) -> Vec<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    let mut held = Vec::new();
    for _ in 0..count {
        held.push(
            pool.acquire()
                .await
                .expect("request burst should fill an idle pool"),
        );
    }
    held
}

async fn start_real_server() -> (JoinHandle<anyhow::Result<()>>, ServerContext) {
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

async fn create_test_session(
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
        metadata: json!({ "source": "background_sweep_wiring_test" }),
    })
    .await
    .expect("create test principal");
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: format!("{label}-{}", Uuid::now_v7()),
                display_name: Some("Background sweep wiring".to_string()),
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

async fn run_background_sweep_regression() {
    let (server, context) = start_real_server().await;
    let observer = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("connect observer pool");

    let scheduled_session = create_test_session(&context.db, "scheduled-background").await;
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

    let timeout_session = create_test_session(&context.db, "timeout-background").await;
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

#[test]
fn built_in_background_sweeps_survive_a_saturated_request_pool() {
    configure_server_environment();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build Tokio runtime")
        .block_on(run_background_sweep_regression());
}
