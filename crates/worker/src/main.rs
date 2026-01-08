use anyhow::{Context, Result};
use everruns_core::telemetry::{init_telemetry, TelemetryConfig};
use everruns_worker::{DurableWorker, DurableWorkerConfig};

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

    // Create durable worker config (includes grpc_address)
    let config = DurableWorkerConfig::from_env();

    tracing::info!(
        grpc_address = %config.grpc_address,
        worker_id = %config.worker_id,
        max_concurrent = config.max_concurrent_tasks,
        "Starting Durable worker"
    );

    // Create and run the Durable worker (connects to control-plane via gRPC)
    let mut worker = DurableWorker::new(config)
        .await
        .context("Failed to create Durable worker")?;

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
