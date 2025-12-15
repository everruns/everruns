use anyhow::Result;
use everruns_storage::repositories::Database;
use everruns_worker::{RunnerConfig, RunnerMode};
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

    // Load runner configuration
    let config = RunnerConfig::from_env();
    tracing::info!(mode = ?config.mode, "Runner mode configured");

    match config.mode {
        RunnerMode::InProcess => {
            // In-process mode: Worker is passive, workflows triggered by API
            // Database connection not needed - API handles execution
            tracing::info!("Worker running in passive mode (in-process execution handled by API)");
            tracing::info!("Worker ready, waiting for shutdown signal...");
            tokio::signal::ctrl_c().await?;
        }
        RunnerMode::Temporal => {
            // Temporal mode: Worker with step checkpointing for durability
            let database_url =
                std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable required");

            // Initialize database connection for Temporal worker
            let db = Database::from_url(&database_url).await?;
            tracing::info!("Database connection established");

            tracing::info!(
                address = %config.temporal_address(),
                namespace = %config.temporal_namespace(),
                task_queue = %config.temporal_task_queue(),
                "Starting Temporal worker with checkpointing"
            );

            // Run the temporal worker (keeps running until shutdown)
            everruns_worker::runner_temporal::run_temporal_worker(&config, db).await?;
        }
    }

    tracing::info!("Worker shutdown complete");
    Ok(())
}
