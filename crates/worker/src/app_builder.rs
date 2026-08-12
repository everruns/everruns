// Worker app builder for composable worker configurations
//
// Decision: Builder pattern mirrors ServerAppBuilder for symmetry
// Decision: Encapsulates telemetry, shutdown, and worker lifecycle
// Decision: Error reporter is vendor-neutral (see knowledge/foundations/embedding.md). Fatal
//   `run()` failures flow through the installed reporter when one is present;
//   OSS never imports vendor SDKs directly. Panic interception and per-task /
//   per-workflow scoping are wrapper responsibilities today — see the spec's
//   "Not goals" section.

use anyhow::{Context, Result};
use everruns_core::{ErrorReport, ErrorReporter, ErrorScope};
use everruns_host::HostComposition;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// Maximum time we wait for the embedder reporter on the fatal-exit path
/// before giving up, so a stalled vendor endpoint cannot block worker
/// termination.
const REPORTER_TIMEOUT: Duration = Duration::from_secs(5);

use crate::{GrpcDurableStore, GrpcWorkerAdapters, TaskWorker, TaskWorkerConfig};

/// Builder for composing worker applications.
///
/// Encapsulates the worker lifecycle: configuration, connection, execution,
/// and graceful shutdown on Ctrl-C.
///
/// # Example
///
/// ```rust,ignore
/// use everruns_worker::WorkerAppBuilder;
/// use everruns_worker::TaskWorkerConfig;
///
/// WorkerAppBuilder::new(TaskWorkerConfig::from_env())
///     .run()
///     .await?;
/// ```
pub struct WorkerAppBuilder {
    config: TaskWorkerConfig,
    host_composition: Option<HostComposition>,
    error_reporter: Option<Arc<dyn ErrorReporter>>,
}

impl WorkerAppBuilder {
    /// Create a new worker app builder with the given configuration.
    pub fn new(config: TaskWorkerConfig) -> Self {
        Self {
            config,
            host_composition: None,
            error_reporter: None,
        }
    }

    /// Replace the default worker runtime surface with an explicit platform definition.
    pub fn host_composition(mut self, host_composition: HostComposition) -> Self {
        self.host_composition = Some(host_composition);
        self
    }

    /// Install a vendor-neutral error reporter.
    ///
    /// Fatal `run()` failures are forwarded to the reporter with the worker
    /// component name in the scope. Panic interception and per-task /
    /// per-workflow scoping are wrapper responsibilities (e.g., by wrapping
    /// task execution themselves). See `knowledge/foundations/embedding.md` for the contract.
    pub fn error_reporter(mut self, reporter: Arc<dyn ErrorReporter>) -> Self {
        self.error_reporter = Some(reporter);
        self
    }

    /// Build and run the worker. Blocks until shutdown signal.
    pub async fn run(self) -> Result<()> {
        let Self {
            config,
            host_composition,
            error_reporter,
        } = self;
        info!(
            grpc_address = %config.grpc_address,
            worker_id = %config.worker_id,
            max_concurrent = config.max_concurrent_tasks,
            "Starting task worker"
        );
        if error_reporter.is_some() {
            info!("Embedder error reporter installed");
        }

        let host_composition =
            host_composition.unwrap_or_else(crate::platform::default_host_composition);
        let store = Arc::new(
            GrpcDurableStore::connect_with_timeout(&config.grpc_address, config.connect_timeout)
                .await
                .context("Failed to connect durable task store")?,
        );
        let adapters = GrpcWorkerAdapters::connect_with_host_composition(
            &config.grpc_address,
            host_composition,
        )
        .await
        .context("Failed to connect worker adapters")?;
        let mut worker = TaskWorker::new(config, store, adapters);

        let shutdown_handle = worker.shutdown_handle();

        tokio::select! {
            result = worker.run() => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "Worker error");
                    if let Some(reporter) = &error_reporter {
                        // Bounded timeout so a stalled vendor endpoint cannot
                        // block worker termination / supervisor restart.
                        let report_fut = reporter.report(
                            ErrorReport::fatal("worker.run", e.to_string())
                                .with_scope(ErrorScope::new().with_component("task_worker")),
                        );
                        if tokio::time::timeout(REPORTER_TIMEOUT, report_fut).await.is_err() {
                            tracing::warn!(
                                timeout_secs = REPORTER_TIMEOUT.as_secs(),
                                "Embedder error reporter timed out on worker fatal path"
                            );
                        }
                    }
                    return Err(e);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received shutdown signal");
                shutdown_handle.shutdown();
            }
        }

        info!("Worker shutdown complete");
        Ok(())
    }
}
