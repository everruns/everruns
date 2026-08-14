//! PostgreSQL conformance for blank-schema migration and process restart.
//!
//! Run with `cargo test -p everruns-scale --features postgres-tests --test postgres_restart`.

#![cfg(feature = "postgres-tests")]

use std::sync::Arc;

use everruns::{Engine, SessionId};
use everruns_provider::runtime_provider::Provider;
use everruns_provider::typed_id::{HarnessId, MessageId};
use everruns_scale::{
    DurableStoreBackend, PortableAgent, PostgresDurableStore, RegistrationPath, Registry,
    RunController, ScaleEngine,
};
use everruns_test_support::{LlmSimConfig, LlmSimDriver};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn path(value: &str) -> RegistrationPath {
    RegistrationPath::new(value).unwrap()
}

fn registry(response: &str, workspace_root: Arc<std::path::PathBuf>) -> Registry {
    let mut registry = Registry::new();
    registry
        .register_provider(
            path("providers/llmsim/default"),
            Provider::new(
                "llmsim",
                LlmSimDriver::new(LlmSimConfig::fixed(response.to_string())),
            ),
        )
        .unwrap();
    registry
        .register_workspace(path("workspaces/shared/test"), move |builder| {
            builder.workspace(workspace_root.as_ref())
        })
        .unwrap();
    registry
}

fn definition() -> PortableAgent {
    PortableAgent::builder(
        "Be concise.",
        "llmsim-model",
        path("providers/llmsim/default"),
        path("workspaces/shared/test"),
    )
    .build()
    .unwrap()
}

async fn blank_schema_pool() -> (PgPool, PgPool, String) {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://postgres:postgres@localhost:{port}/everruns_test")
    });
    let admin = PgPool::connect(&database_url)
        .await
        .expect("PostgreSQL is required for postgres-tests");
    let schema = format!("scale_test_{}", Uuid::new_v4().simple());
    let create_schema = format!("CREATE SCHEMA {schema}");
    // The only dynamic token is a code-generated lowercase UUID identifier.
    sqlx::query(sqlx::AssertSqlSafe(create_schema))
        .execute(&admin)
        .await
        .unwrap();
    let search_path = schema.clone();
    let pool = PgPoolOptions::new()
        .after_connect(move |connection, _| {
            let statement = format!("SET search_path TO {search_path}");
            Box::pin(async move {
                sqlx::query(sqlx::AssertSqlSafe(statement))
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .unwrap();
    (admin, pool, schema)
}

#[tokio::test]
async fn blank_database_restart_preserves_history_environment_and_event_authority() {
    let (admin, pool, schema) = blank_schema_pool().await;
    let workspace_root =
        Arc::new(std::env::temp_dir().join(format!("everruns-scale-workspace-{}", Uuid::new_v4())));

    let engine = ScaleEngine::new(pool.clone(), registry("same", workspace_root.clone()))
        .await
        .unwrap();
    let session = engine.submit(definition()).await.unwrap();
    let session_id = session.session_id();
    assert_eq!(session.send_and_wait("one").await.unwrap().response, "same");
    let harness_id = HarnessId::new();
    engine
        .start_run(
            1,
            session_id,
            harness_id,
            None,
            MessageId::new(),
            Some("before-restart".into()),
        )
        .await
        .unwrap();
    drop(session);
    drop(engine);

    let restarted = ScaleEngine::new(pool.clone(), registry("same", workspace_root.clone()))
        .await
        .unwrap();
    let resumed = restarted.resume(session_id).await.unwrap();
    assert_eq!(resumed.history().page().await.unwrap().messages.len(), 2);
    assert_eq!(resumed.send_and_wait("two").await.unwrap().response, "same");
    assert_eq!(resumed.history().page().await.unwrap().messages.len(), 4);
    restarted
        .start_run(
            1,
            session_id,
            harness_id,
            None,
            MessageId::new(),
            Some("after-restart".into()),
        )
        .await
        .unwrap();

    assert_one_canonical_event_per_identity(&pool, session_id).await;
    assert_one_durable_start_and_one_steering_signal(&pool, session_id).await;
    let mut durable_store = PostgresDurableStore::new(pool.clone());
    assert_eq!(
        durable_store
            .get_and_consume_signals(session_id.uuid())
            .await
            .unwrap()
            .len(),
        1,
        "blank Scale schema supports the durable substrate signal contract"
    );
    restarted.cancel_run(session_id).await.unwrap();
    assert!(!restarted.is_running(session_id).await);

    drop(resumed);
    drop(restarted);
    pool.close().await;
    let drop_schema = format!("DROP SCHEMA {schema} CASCADE");
    sqlx::query(sqlx::AssertSqlSafe(drop_schema))
        .execute(&admin)
        .await
        .unwrap();
    let _ = std::fs::remove_dir_all(workspace_root.as_ref());
}

async fn assert_one_durable_start_and_one_steering_signal(pool: &PgPool, session_id: SessionId) {
    let workflow_id = session_id.uuid();
    let starts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM durable_workflow_events \
         WHERE workflow_id = $1 AND event_type = 'workflow_started'",
    )
    .bind(workflow_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let tasks: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM durable_task_queue WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_one(pool)
            .await
            .unwrap();
    let signals: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM durable_signals WHERE workflow_id = $1 AND processed_at IS NULL",
    )
    .bind(workflow_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(starts, 1);
    assert_eq!(tasks, 1);
    assert_eq!(signals, 1);
}

async fn assert_one_canonical_event_per_identity(pool: &PgPool, session_id: SessionId) {
    let (total, event_ids, sequences): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(DISTINCT event_id), COUNT(DISTINCT sequence) \
         FROM everruns_scale_events WHERE session_id = $1",
    )
    .bind(session_id.uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(total > 0);
    assert_eq!(total, event_ids);
    assert_eq!(total, sequences);
}
