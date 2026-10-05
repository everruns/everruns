#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Everruns worker
// Decision: Uses WorkerAppBuilder for composable worker setup

use anyhow::Result;
use everruns_durable_engine::host::observability::{TelemetryConfig, init_telemetry};
use everruns_worker::{TaskWorkerConfig, WorkerAppBuilder};

// Use mimalloc as the global allocator. The worker runs heavy concurrent
// agent/tool execution; a sharded allocator reduces fragmentation and improves
// tail latency vs. system malloc. Defined here (the bin crate root) so it
// applies only to the everruns-worker binary, not library crates.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize telemetry with OpenTelemetry support
    let mut telemetry_config = TelemetryConfig::from_env();
    if telemetry_config.service_name == "everruns" {
        telemetry_config.service_name = "everruns-worker".to_string();
    }
    if telemetry_config.log_filter.is_none() {
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "debug".to_string());
        // durable-engine runs every claimed task (`TurnTaskDriver`), so its
        // task lifecycle lines belong in the worker's default filter.
        telemetry_config.log_filter = Some(format!(
            "everruns_worker={log_level},everruns_durable_engine={log_level},everruns_core={log_level}"
        ));
    }
    let _telemetry_guard = init_telemetry(telemetry_config);

    tracing::info!("everrun-worker starting...");

    WorkerAppBuilder::new(TaskWorkerConfig::from_env())
        .run()
        .await
}
