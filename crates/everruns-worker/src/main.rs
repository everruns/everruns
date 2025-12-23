use anyhow::{Context, Result};
use everruns_storage::{repositories::Database, EncryptionService};
use everruns_worker::{RunnerConfig, TemporalWorker};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with LOG_LEVEL env var (default: debug)
    // Supports: trace, debug, info, warn, error
    // Can also use RUST_LOG for more fine-grained control
    let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "debug".to_string());
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| format!("everruns_worker={}", log_level).into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("everrun-worker starting...");

    // Load runner configuration
    let config = RunnerConfig::from_env();

    // Start the Temporal worker to poll for tasks
    tracing::info!(
        task_queue = %config.temporal_task_queue(),
        "Starting Temporal worker"
    );

    // Connect to database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://everruns:everruns@localhost:5432/everruns".into());
    let db = Database::from_url(&database_url).await?;

    // Initialize encryption service for decrypting API keys
    // SECRETS_ENCRYPTION_KEY is required for API key decryption
    let encryption = EncryptionService::from_env()
        .context("Failed to initialize encryption service. Ensure SECRETS_ENCRYPTION_KEY is set.")?;

    // Create and run the Temporal worker
    let worker = TemporalWorker::new(config, db, encryption).await?;

    // Run the worker (blocks until shutdown)
    tokio::select! {
        result = worker.run() => {
            if let Err(e) = result {
                tracing::error!(error = %e, "Worker error");
                return Err(e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received shutdown signal");
            worker.shutdown();
        }
    }

    tracing::info!("Worker shutdown complete");
    Ok(())
}
