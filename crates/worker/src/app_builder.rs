// Worker app builder for composable worker configurations
//
// Decision: Builder pattern mirrors ServerAppBuilder for symmetry
// Decision: Encapsulates telemetry, shutdown, and worker lifecycle

use anyhow::{Context, Result};
use everruns_core::PlatformDefinition;
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
}

impl WorkerAppBuilder {
    /// Create a new worker app builder with the given configuration.
    pub fn new(config: DurableWorkerConfig) -> Self {
        Self {
            config,
            platform_definition: None,
        }
    }

    /// Replace the default worker runtime surface with an explicit platform definition.
    pub fn platform_definition(mut self, platform_definition: PlatformDefinition) -> Self {
        self.platform_definition = Some(platform_definition);
        self
    }

    /// Build and run the worker. Blocks until shutdown signal.
    pub async fn run(self) -> Result<()> {
        let Self {
            config,
            platform_definition,
        } = self;
        info!(
            grpc_address = %config.grpc_address,
            worker_id = %config.worker_id,
            max_concurrent = config.max_concurrent_tasks,
            "Starting Durable worker"
        );

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
