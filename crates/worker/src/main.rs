use anyhow::{Context, Result};
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

    // Get gRPC address for control-plane communication
    let grpc_address = std::env::var("GRPC_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:9001".into());

    tracing::info!(
        task_queue = %config.temporal_task_queue(),
        grpc_address = %grpc_address,
        "Starting Temporal worker"
    );

    // Create and run the Temporal worker (connects to control-plane via gRPC)
    let worker = TemporalWorker::new(config, &grpc_address)
        .await
        .context("Failed to create Temporal worker")?;

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
