// Temporal client wrapper for workflow management (M2)
// Decision: Wrap the temporal-sdk-core APIs behind a simple interface for our use case
//
// This module provides:
// - Connection management to Temporal server
// - Workflow start/cancel operations (used by API)
// - Activity and workflow polling (used by Worker)

use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::info;

use temporal_sdk_core::{CoreRuntime, RuntimeOptions, Worker, WorkerConfig, init_worker};
use temporalio_client::{ClientOptions, RetryClient, Client, WorkflowClientTrait, WorkflowOptions};
use temporalio_common::protos::temporal::api::common::v1::Payload;
use temporalio_common::protos::temporal::api::workflowservice::v1::StartWorkflowExecutionResponse;
use temporalio_common::Worker as WorkerTrait;

use crate::agent_workflow::AgentWorkflowInput;
use crate::runner::RunnerConfig;
use crate::types::workflow_names;

/// Client for interacting with Temporal server
/// Used by the API to start workflows
pub struct TemporalClient {
    client: RetryClient<Client>,
    config: RunnerConfig,
}

impl TemporalClient {
    /// Create a new Temporal client connected to the server
    pub async fn new(config: RunnerConfig) -> Result<Self> {
        let target_url: url::Url = format!("http://{}", config.temporal_address())
            .parse()
            .context("Invalid Temporal address")?;

        info!(
            address = %config.temporal_address(),
            namespace = %config.temporal_namespace(),
            task_queue = %config.temporal_task_queue(),
            "Connecting to Temporal server"
        );

        let client_opts = ClientOptions::builder()
            .target_url(target_url)
            .client_name("everruns-api")
            .client_version(env!("CARGO_PKG_VERSION"))
            .identity(format!("everruns-api-{}", uuid::Uuid::now_v7()))
            .build();

        let client = client_opts
            .connect(config.temporal_namespace(), None)
            .await
            .context("Failed to connect to Temporal server")?;

        info!("Connected to Temporal server");

        Ok(Self { client, config })
    }

    /// Start a new agent workflow
    pub async fn start_agent_workflow(
        &self,
        input: &AgentWorkflowInput,
    ) -> Result<StartWorkflowExecutionResponse> {
        let workflow_id = Self::workflow_id_for_session(input.session_id);

        info!(
            workflow_id = %workflow_id,
            session_id = %input.session_id,
            agent_id = %input.agent_id,
            "Starting agent workflow"
        );

        // Serialize workflow input
        let input_bytes =
            serde_json::to_vec(input).context("Failed to serialize workflow input")?;
        let input_payload = Payload {
            metadata: Default::default(),
            data: input_bytes,
        };

        // Start workflow using the client trait method
        let response = self
            .client
            .start_workflow(
                vec![input_payload],
                self.config.temporal_task_queue(),
                workflow_id.clone(),
                workflow_names::AGENT_WORKFLOW.to_string(),
                None, // request_id
                WorkflowOptions::default(),
            )
            .await
            .context("Failed to start workflow")?;

        info!(
            workflow_id = %workflow_id,
            temporal_run_id = %response.run_id,
            "Workflow started successfully"
        );

        Ok(response)
    }

    /// Get the workflow ID for a session
    pub fn workflow_id_for_session(session_id: uuid::Uuid) -> String {
        format!("session-{}", session_id)
    }

    /// Get the underlying client for advanced operations
    pub fn client(&self) -> &RetryClient<Client> {
        &self.client
    }
}

/// Worker-side Temporal core for polling and processing tasks
pub struct TemporalWorkerCore {
    worker: Worker,
    #[allow(dead_code)]
    runtime: Arc<CoreRuntime>,
    #[allow(dead_code)]
    config: RunnerConfig,
}

impl TemporalWorkerCore {
    /// Create a new Temporal worker core
    pub async fn new(config: RunnerConfig) -> Result<Self> {
        let target_url: url::Url = format!("http://{}", config.temporal_address())
            .parse()
            .context("Invalid Temporal address")?;

        info!(
            address = %config.temporal_address(),
            namespace = %config.temporal_namespace(),
            task_queue = %config.temporal_task_queue(),
            "Initializing Temporal worker core"
        );

        // Create runtime options with defaults
        let runtime_opts = RuntimeOptions::default();

        // Create core runtime
        let runtime = CoreRuntime::new_assume_tokio(runtime_opts)
            .context("Failed to create core runtime")?;
        let runtime = Arc::new(runtime);

        // Build client options
        let client_opts = ClientOptions::builder()
            .target_url(target_url)
            .client_name("everruns-worker")
            .client_version(env!("CARGO_PKG_VERSION"))
            .identity(format!("everruns-worker-{}", uuid::Uuid::now_v7()))
            .build();

        // Connect client
        let client = client_opts
            .connect(config.temporal_namespace(), None)
            .await
            .context("Failed to connect to Temporal server")?;

        // Build worker config with required fields
        use temporalio_common::worker::{WorkerTaskTypes, WorkerVersioningStrategy};
        let worker_config = WorkerConfig::builder()
            .namespace(config.temporal_namespace())
            .task_queue(config.temporal_task_queue())
            .max_cached_workflows(100_usize)
            .max_outstanding_workflow_tasks(100_usize)
            .max_outstanding_activities(100_usize)
            .task_types(WorkerTaskTypes {
                enable_workflows: true,
                enable_local_activities: false,
                enable_remote_activities: true,
                enable_nexus: false,
            })
            .versioning_strategy(WorkerVersioningStrategy::None { build_id: String::new() })
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build worker config: {}", e))?;

        // Initialize worker
        let worker = init_worker(&runtime, worker_config, client)
            .context("Failed to initialize Temporal worker")?;

        info!("Temporal worker core initialized");

        Ok(Self {
            worker,
            runtime,
            config,
        })
    }

    /// Get a reference to the worker for polling
    pub fn worker(&self) -> &Worker {
        &self.worker
    }

    /// Shutdown the worker gracefully
    pub async fn shutdown(&self) {
        info!("Shutting down Temporal worker core");
        WorkerTrait::initiate_shutdown(&self.worker);
        WorkerTrait::shutdown(&self.worker).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_id_generation() {
        let session_id = uuid::Uuid::now_v7();
        let workflow_id = TemporalClient::workflow_id_for_session(session_id);
        assert!(workflow_id.starts_with("session-"));
        assert!(workflow_id.contains(&session_id.to_string()));
    }
}
