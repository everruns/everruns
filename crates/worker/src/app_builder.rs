// Worker app builder for composable worker configurations
//
// Decision: Builder pattern mirrors ServerAppBuilder for symmetry
// Decision: Encapsulates telemetry, shutdown, and worker lifecycle
// Decision: Error reporter is vendor-neutral (see specs/embedding.md). Worker
//   task failures and panics flow through the installed reporter when one is
//   present; OSS never imports vendor SDKs directly.

use anyhow::{Context, Result};
use everruns_core::{ErrorReport, ErrorReporter, ErrorScope, PlatformDefinition};
use std::sync::Arc;
use tracing::info;

use crate::durable_worker::{DurableWorker, DurableWorkerConfig};

/// Builder for composing worker applications.
///
/// Encapsulates the worker lifecycle: configuration, connection, execution,
/// and graceful shutdown on Ctrl-C.
///
/// # Example
///
/// ```rust,ignore
/// use everruns_worker::WorkerAppBuilder;
/// use everruns_worker::DurableWorkerConfig;
///
/// WorkerAppBuilder::new(DurableWorkerConfig::from_env())
///     .run()
///     .await?;
/// ```
pub struct WorkerAppBuilder {
    config: DurableWorkerConfig,
    platform_definition: Option<PlatformDefinition>,
    error_reporter: Option<Arc<dyn ErrorReporter>>,
}

impl WorkerAppBuilder {
    /// Create a new worker app builder with the given configuration.
    pub fn new(config: DurableWorkerConfig) -> Self {
        Self {
            config,
            platform_definition: None,
            error_reporter: None,
        }
    }

    /// Replace the default worker runtime surface with an explicit platform definition.
    pub fn platform_definition(mut self, platform_definition: PlatformDefinition) -> Self {
        self.platform_definition = Some(platform_definition);
        self
    }

    /// Install a vendor-neutral error reporter.
    ///
    /// Worker-side panics and unrecoverable task failures are forwarded to
    /// the reporter with task / workflow ids in the scope. See
    /// `specs/embedding.md` for the contract.
    pub fn error_reporter(mut self, reporter: Arc<dyn ErrorReporter>) -> Self {
        self.error_reporter = Some(reporter);
        self
    }

    /// Build and run the worker. Blocks until shutdown signal.
    pub async fn run(self) -> Result<()> {
        let Self {
            config,
            platform_definition,
            error_reporter,
        } = self;
        info!(
            grpc_address = %config.grpc_address,
            worker_id = %config.worker_id,
            max_concurrent = config.max_concurrent_tasks,
            "Starting Durable worker"
        );
        if error_reporter.is_some() {
            info!("Embedder error reporter installed");
        }

        let worker = match platform_definition {
            Some(platform_definition) => {
                DurableWorker::with_platform_definition(config, platform_definition)
                    .await
                    .context("Failed to create Durable worker")?
            }
            None => DurableWorker::new(config)
                .await
                .context("Failed to create Durable worker")?,
        };

        let shutdown_handle = worker.shutdown_handle();

        tokio::select! {
            result = worker.run() => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "Worker error");
                    if let Some(reporter) = &error_reporter {
                        reporter
                            .report(
                                ErrorReport::fatal("worker.run", e.to_string())
                                    .with_scope(ErrorScope::new().with_component("durable_worker")),
                            )
                            .await;
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
