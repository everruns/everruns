use anyhow::Result;
use everruns_storage::repositories::Database;
use everruns_worker::WorkflowExecutor;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "everruns_worker=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("everrun-worker starting...");

    // Get database URL from environment
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable required");

    // Initialize database connection
    let db = Database::from_url(&database_url).await?;
    tracing::info!("Database connection established");

    // Create workflow executor
    let _executor = Arc::new(WorkflowExecutor::new(db));
    tracing::info!("Workflow executor initialized");

    // M4: Worker is passive - workflows are triggered by API
    // M5+: Will add active components (queue polling, scheduled runs, etc.)

    // Keep the worker running
    tracing::info!("Worker ready to execute workflows");
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutdown signal received");

    Ok(())
}
