// Everruns worker
// Decision: Uses WorkerAppBuilder for composable worker setup

use anyhow::Result;
use everruns_core::telemetry::{TelemetryConfig, init_telemetry};
use everruns_worker::{DurableWorkerConfig, WorkerAppBuilder};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize telemetry with OpenTelemetry support
    let mut telemetry_config = TelemetryConfig::from_env();
    if telemetry_config.service_name == "everruns" {
        telemetry_config.service_name = "everruns-worker".to_string();
    }
    if telemetry_config.log_filter.is_none() {
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "debug".to_string());
        telemetry_config.log_filter = Some(format!(
            "everruns_worker={},everruns_core={}",
            log_level, log_level
        ));
    }
    let _telemetry_guard = init_telemetry(telemetry_config);

    tracing::info!("everrun-worker starting...");

    WorkerAppBuilder::new(DurableWorkerConfig::from_env())
        .run()
        .await
}
