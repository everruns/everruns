//! Bringing up storage, the durable event store, and the turn backend.
//!
// Split out of `app_builder.rs` (EVE-1069): that file is on the size
// ratchet's debt list, and this is the most self-contained unit in it —
// everything here is decided before any router or service exists.

use anyhow::{Context, Result};
use std::sync::Arc;

use crate::storage::StorageBackend;
use everruns_core::host::TurnBackend;
use everruns_durable::PostgresWorkflowEventStore;
use everruns_worker::{DurableRunner, DurableTaskNotifier};

use crate::app_builder::MigrationFn;
use crate::server::ServerConfig;

pub(crate) struct StorageInit {
    pub(crate) db: Arc<StorageBackend>,
    pub(crate) runner: Arc<dyn TurnBackend>,
    pub(crate) background_runner: Arc<dyn TurnBackend>,
    pub(crate) shared_durable_store: Option<Arc<PostgresWorkflowEventStore>>,
    pub(crate) database_url: Option<String>,
    pub(crate) database_unpooled_url: Option<String>,
    pub(crate) task_broadcaster:
        Option<Arc<crate::live_updates::task_notifications::TaskBroadcaster>>,
}

pub(crate) async fn init_storage(
    config: &ServerConfig,
    migrations: Vec<MigrationFn>,
) -> Result<StorageInit> {
    let database_unpooled_url = std::env::var("DATABASE_UNPOOLED_URL").ok();

    if config.dev_mode {
        // Dev mode needs no database of its own: records live in a throwaway
        // PostgreSQL owned by this process, so they run through the same
        // repositories as production. DATABASE_URL is ignored on purpose, so a
        // dev run never writes into a real database. Durable state shares that
        // database (records reference durable rows by foreign key, trigger
        // schedules for one) and the in-process worker executes it.
        tracing::info!("Starting in DEV MODE (embedded PostgreSQL, deleted on exit)");
        let pg = everruns_pg_embedded::EmbeddedPostgres::shared()
            .await
            .context("Failed to start embedded PostgreSQL for DEV_MODE")?;
        let database = format!("dev_{}", uuid::Uuid::now_v7().simple());
        pg.create_database(&database, None).await?;
        let backend = StorageBackend::postgres(&pg.url(&database))
            .await
            .context("Failed to connect to embedded PostgreSQL")?;
        run_migrations(&backend, migrations).await?;

        let pool = backend.pool().clone();
        let shared_store = Arc::new(PostgresWorkflowEventStore::new(pool.clone()));
        tracing::info!("Creating Durable execution engine runner (PostgreSQL mode)");
        let runner: Arc<dyn TurnBackend> = Arc::new(DurableRunner::new_with_pool(pool));
        return Ok(StorageInit {
            db: Arc::new(backend),
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
        run_migrations(&backend, migrations).await?;
    } else {
        tracing::info!("Skipping database migrations (--no-migrations)");
    }

    let request_pool = backend.pool().clone();
    let background_pool = backend.background_pool().clone();
    let task_broadcaster = crate::live_updates::task_notifications::TaskBroadcaster::from_env(
        Some(database_url.as_str()),
        database_unpooled_url.as_deref(),
    )
    .await
    .map(Arc::new);
    let task_notifier = task_broadcaster.clone().map(|broadcaster| {
        Arc::new(ServerTaskNotifier { broadcaster }) as Arc<dyn DurableTaskNotifier>
    });
    let runner = durable_runner(request_pool, task_notifier.clone());
    let background_runner = durable_runner(background_pool, task_notifier);

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

/// The durable turn backend over `pool`, publishing task availability through
/// `task_notifier` when there is one.
fn durable_runner(
    pool: everruns_durable::PostgresPool,
    task_notifier: Option<Arc<dyn DurableTaskNotifier>>,
) -> Arc<dyn TurnBackend> {
    match task_notifier {
        Some(task_notifier) => {
            tracing::info!(
                "Creating Durable execution engine runner (PostgreSQL mode with task notifier)"
            );
            Arc::new(DurableRunner::new_with_pool_and_task_notifier(
                pool,
                task_notifier,
            ))
        }
        None => {
            tracing::info!("Creating Durable execution engine runner (PostgreSQL mode)");
            Arc::new(DurableRunner::new_with_pool(pool))
        }
    }
}

async fn run_migrations(backend: &StorageBackend, migrations: Vec<MigrationFn>) -> Result<()> {
    tracing::info!("Running database migrations...");
    let pool = backend.pool();
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
    Ok(())
}

/// Forwards the durable runner's "task enqueued" hint to the worker broadcaster.
pub(crate) struct ServerTaskNotifier {
    pub(crate) broadcaster: Arc<crate::live_updates::task_notifications::TaskBroadcaster>,
}

#[async_trait::async_trait]
impl DurableTaskNotifier for ServerTaskNotifier {
    async fn notify_task_available(&self, activity_type: &str) {
        self.broadcaster.notify_task_available(activity_type).await;
    }
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
            .expect("initialize dev storage");

        assert!(Arc::ptr_eq(&storage.runner, &storage.background_runner));
        assert!(
            !storage.db.pool().is_closed(),
            "dev mode keeps records in embedded PostgreSQL"
        );
    }
}
