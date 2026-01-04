use anyhow::{Context, Result};
use everruns_core::telemetry::{init_telemetry, TelemetryConfig};
use everruns_worker::{RunnerConfig, TemporalWorker};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize telemetry with OpenTelemetry support
    // Configure via environment variables:
    // - OTEL_SERVICE_NAME: Service name (default: "everruns-worker")
    // - OTEL_EXPORTER_OTLP_ENDPOINT: OTLP endpoint (e.g., "http://localhost:4317")
    // - RUST_LOG or LOG_LEVEL: Log filter (default: "everruns_worker=debug")
    let mut telemetry_config = TelemetryConfig::from_env();
    if telemetry_config.service_name == "everruns" {
        telemetry_config.service_name = "everruns-worker".to_string();
    }
    if telemetry_config.log_filter.is_none() {
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "debug".to_string());
        telemetry_config.log_filter = Some(format!("everruns_worker={}", log_level));
    }

    // Keep the guard alive for the lifetime of the application
    let _telemetry_guard = init_telemetry(telemetry_config);

    tracing::info!("everrun-worker starting...");

    // Load runner configuration
    let config = RunnerConfig::from_env();

    // Get gRPC address for control-plane communication
    let grpc_address = std::env::var("GRPC_ADDRESS").unwrap_or_else(|_| "127.0.0.1:9001".into());

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
