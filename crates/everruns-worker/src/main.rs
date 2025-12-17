use anyhow::Result;
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
            // Temporal mode disabled during M2 migration to Harness/Session model
            // Will be re-enabled when Temporal integration is updated
            tracing::warn!("Temporal mode requested but disabled during M2 migration");
            tracing::info!("Falling back to passive mode, waiting for shutdown signal...");
            tokio::signal::ctrl_c().await?;
        }
    }

    tracing::info!("Worker shutdown complete");
    Ok(())
}
