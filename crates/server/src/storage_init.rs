//! Bringing up storage, the durable event store, and the agent runner.
//!
//! Split out of `app_builder.rs` (EVE-1069): that file is on the size
//! ratchet's debt list, and this is the most self-contained unit in it —
//! everything here is decided before any router or service exists.

use anyhow::{Context, Result};
use std::sync::Arc;

use crate::storage::StorageBackend;
use everruns_durable::InMemoryWorkflowEventStore;
use everruns_worker::{
    AgentRunner, DurableTaskNotifier, RunnerBackend, create_runner_with_backend,
};

use crate::app_builder::{MigrationFn, ServerTaskNotifier};
use crate::server::ServerConfig;

pub(crate) struct StorageInit {
    pub(crate) db: Arc<StorageBackend>,
    pub(crate) runner: Arc<dyn AgentRunner>,
    pub(crate) background_runner: Arc<dyn AgentRunner>,
    pub(crate) shared_durable_store: Option<Arc<InMemoryWorkflowEventStore>>,
    pub(crate) database_url: Option<String>,
    pub(crate) database_unpooled_url: Option<String>,
    pub(crate) task_broadcaster: Option<Arc<crate::task_notifications::TaskBroadcaster>>,
}

pub(crate) async fn init_storage(
    config: &ServerConfig,
    migrations: Vec<MigrationFn>,
) -> Result<StorageInit> {
    let database_unpooled_url = std::env::var("DATABASE_UNPOOLED_URL").ok();

    if config.dev_mode {
        tracing::info!("Starting in DEV MODE (in-memory storage, no PostgreSQL required)");

        let db = Arc::new(StorageBackend::in_memory());
        let shared_store = Arc::new(InMemoryWorkflowEventStore::new());
        let runner =
            create_runner_with_backend(RunnerBackend::SharedInMemory(shared_store.clone()))
                .await
                .context("Failed to create in-memory agent runner")?;

        tracing::info!(
            "Using in-memory storage and durable execution engine with in-process worker"
        );
        return Ok(StorageInit {
            db,
            background_runner: runner.clone(),
            runner,
            shared_durable_store: Some(shared_store),
            database_url: None,
            database_unpooled_url,
            task_broadcaster: None,
        });
    }

    let database_url =
        std::env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;

    // TM-NEW: Warn when DATABASE_URL lacks TLS in production.
    if !database_url.contains("sslmode=") {
        tracing::warn!(
            "DATABASE_URL does not specify sslmode. \
             For production, use sslmode=require or sslmode=verify-full \
             to encrypt database connections."
        );
    } else if database_url.contains("sslmode=disable") {
        tracing::warn!(
            "DATABASE_URL has sslmode=disable — database connections are unencrypted. \
             For production, use sslmode=require or sslmode=verify-full."
        );
    }

    let backend = StorageBackend::postgres(&database_url)
        .await
        .context("Failed to connect to database")?;
    tracing::info!("Connected to PostgreSQL database");

    // Optional S3-compatible blob backend for file/image content offload
    // (knowledge/runtime-resources/object-storage.md). Defaults to inline PostgreSQL storage.
    let blob_store = crate::storage::blob_store::blob_store_from_env()
        .context("Invalid object-storage configuration (STORAGE_S3_*)")?;
    let backend = backend.with_blob_store(blob_store);

    if !config.no_migrations {
        tracing::info!("Running database migrations...");
        let pool = backend.pool().expect("PostgreSQL backend should have pool");
        if let Err(e) = sqlx::migrate!("./migrations").run(pool).await {
            tracing::error!(
                error = %e,
                "Database migration failed - check migration files and database state"
            );
            return Err(e)
                .context("Database migration failed - check migration files and database state");
        }
        tracing::info!("Database migrations complete");

        for migration_fn in migrations {
            if let Err(e) = migration_fn(pool.clone()).await {
                tracing::error!(error = %e, "Custom database migration failed");
                return Err(e);
            }
        }
    } else {
        tracing::info!("Skipping database migrations (--no-migrations)");
    }

    let request_pool = backend
        .pool()
        .expect("PostgreSQL backend should have pool")
        .clone();
    let background_pool = backend
        .background_pool()
        .expect("PostgreSQL backend should have background pool")
        .clone();
    let task_broadcaster = crate::task_notifications::TaskBroadcaster::from_env(
        Some(database_url.as_str()),
        database_unpooled_url.as_deref(),
    )
    .await
    .map(Arc::new);
    let task_notifier = task_broadcaster.clone().map(|broadcaster| {
        Arc::new(ServerTaskNotifier { broadcaster }) as Arc<dyn DurableTaskNotifier>
    });
    let runner_backend = if let Some(task_notifier) = task_notifier.clone() {
        RunnerBackend::PostgresWithNotifier {
            pool: request_pool,
            task_notifier,
        }
    } else {
        RunnerBackend::Postgres(request_pool)
    };
    let runner = create_runner_with_backend(runner_backend)
        .await
        .context("Failed to create agent runner")?;
    let background_runner_backend = if let Some(task_notifier) = task_notifier {
        RunnerBackend::PostgresWithNotifier {
            pool: background_pool,
            task_notifier,
        }
    } else {
        RunnerBackend::Postgres(background_pool)
    };
    let background_runner = create_runner_with_backend(background_runner_backend)
        .await
        .context("Failed to create background agent runner")?;

    tracing::info!("Using Durable execution engine runner (PostgreSQL-backed)");
    Ok(StorageInit {
        db: Arc::new(backend),
        runner,
        background_runner,
        shared_durable_store: None,
        database_url: Some(database_url),
        database_unpooled_url,
        task_broadcaster,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dev_mode_shares_the_request_runner_with_background_work() {
        let config = ServerConfig {
            dev_mode: true,
            no_migrations: true,
            api_prefix: String::new(),
            cors_origins: vec![],
            addr: "127.0.0.1:0".to_string(),
            grpc_addr: "127.0.0.1:0".to_string(),
        };

        let storage = init_storage(&config, vec![])
            .await
            .expect("initialize in-memory storage");

        assert!(Arc::ptr_eq(&storage.runner, &storage.background_runner));
    }
}
