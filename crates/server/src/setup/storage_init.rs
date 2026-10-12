//! Bringing up storage, the durable event store, and the turn backend.
//!
// Split out of `app_builder.rs` (EVE-1069): that file is on the size
// ratchet's debt list, and this is the most self-contained unit in it —
// everything here is decided before any router or service exists.

use anyhow::{Context, Result};
use std::sync::Arc;

use crate::storage::StorageBackend;
use crate::turns::ActorTurnBackend;
use everruns_core::host::TurnBackend;
use everruns_durable::PostgresWorkflowEventStore;

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
        // schedules for one) and the in-process worker executes it, turns
        // included: it leases session actors exactly as a remote worker does.
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
        let shared_store = Arc::new(PostgresWorkflowEventStore::new(pool));
        let db = Arc::new(backend);
        let runner: Arc<dyn TurnBackend> = Arc::new(ActorTurnBackend::new(db.clone(), None));
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
        run_migrations(&backend, migrations).await?;
    } else {
        tracing::info!("Skipping database migrations (--no-migrations)");
    }

    let task_broadcaster = crate::live_updates::task_notifications::TaskBroadcaster::from_env(
        Some(database_url.as_str()),
        database_unpooled_url.as_deref(),
    )
    .await
    .map(Arc::new);
    // Starting a turn is a single upsert that joins a caller's command
    // transaction when one is open. Request callers write through the request
    // pool; background loops (scheduler, tool-result timeouts) get a backend on
    // the reserved pool, so a saturated request pool cannot strand a schedule
    // that already fired or a park that already timed out (EVE-1081).
    let db = Arc::new(backend);
    let runner: Arc<dyn TurnBackend> =
        Arc::new(ActorTurnBackend::new(db.clone(), task_broadcaster.clone()));
    let background_runner: Arc<dyn TurnBackend> = Arc::new(ActorTurnBackend::new(
        Arc::new(db.for_background()),
        task_broadcaster.clone(),
    ));

    tracing::info!("Using the session actor turn backend");
    Ok(StorageInit {
        db,
        background_runner,
        runner,
        shared_durable_store: None,
        database_url: Some(database_url),
        database_unpooled_url,
        task_broadcaster,
    })
}

async fn run_migrations(backend: &StorageBackend, migrations: Vec<MigrationFn>) -> Result<()> {
    tracing::info!("Running database migrations...");
    let pool = backend.pool();
    if let Err(e) = crate::setup::migrations::run(pool).await {
        tracing::error!(error = %e, "Database migration failed");
        return Err(e);
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
