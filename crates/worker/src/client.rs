// Temporal client wrapper for workflow management (M2)
// Decision: Wrap the temporal-sdk-core APIs behind a simple interface for our use case
//
// This module provides:
// - Connection management to Temporal server
// - Workflow start/cancel operations (used by API)
// - Activity and workflow polling (used by Worker)

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use temporal_sdk_core::{
    protos::temporal::api::{
        common::v1::{Payloads, WorkflowType},
        taskqueue::v1::TaskQueue,
        workflowservice::v1::{StartWorkflowExecutionRequest, StartWorkflowExecutionResponse},
    },
    Core, CoreInitOptions, ServerGateway, ServerGatewayApis, ServerGatewayOptions, Url,
};

use crate::runner::RunnerConfig;
use crate::turn_workflow::TurnWorkflowInput;
use crate::types::workflow_names;

/// Client for interacting with Temporal server
/// Used by the API to start workflows
pub struct TemporalClient {
    gateway: Arc<ServerGateway>,
    config: RunnerConfig,
}

impl TemporalClient {
    /// Create a new Temporal client connected to the server
    pub async fn new(config: RunnerConfig) -> Result<Self> {
        let target_url = Url::parse(&format!("http://{}", config.temporal_address()))
            .context("Invalid Temporal address")?;

        let gateway_opts = ServerGatewayOptions {
            target_url,
            namespace: config.temporal_namespace(),
            task_queue: config.temporal_task_queue(),
            identity: format!("everruns-control-plane-{}", uuid::Uuid::now_v7()),
            worker_binary_id: env!("CARGO_PKG_VERSION").to_string(),
            long_poll_timeout: Duration::from_secs(60),
        };

        info!(
            address = %config.temporal_address(),
            namespace = %config.temporal_namespace(),
            task_queue = %config.temporal_task_queue(),
            "Connecting to Temporal server"
        );

        let gateway = gateway_opts
            .connect()
            .await
            .context("Failed to connect to Temporal server")?;

        info!("Connected to Temporal server");

        Ok(Self {
            gateway: Arc::new(gateway),
            config,
        })
    }

    /// Start a new turn workflow
    pub async fn start_turn_workflow(
        &self,
        input: &TurnWorkflowInput,
    ) -> Result<StartWorkflowExecutionResponse> {
        let workflow_id = Self::workflow_id_for_session(input.session_id);

        info!(
            workflow_id = %workflow_id,
            session_id = %input.session_id,
            agent_id = %input.agent_id,
            input_message_id = %input.input_message_id,
            "Starting turn workflow"
        );

        // Serialize workflow input
        let input_bytes =
            serde_json::to_vec(input).context("Failed to serialize workflow input")?;
        let input_payload = temporal_sdk_core::protos::temporal::api::common::v1::Payload {
            metadata: Default::default(),
            data: input_bytes,
        };

        // Build request with input
        let request = StartWorkflowExecutionRequest {
            namespace: self.config.temporal_namespace(),
            workflow_id: workflow_id.clone(),
            workflow_type: Some(WorkflowType {
                name: workflow_names::TURN_WORKFLOW.to_string(),
            }),
            task_queue: Some(TaskQueue {
                name: self.config.temporal_task_queue(),
                kind: 0,
            }),
            input: Some(Payloads {
                payloads: vec![input_payload],
            }),
            request_id: uuid::Uuid::now_v7().to_string(),
            ..Default::default()
        };

        // Call service directly to pass input
        let response = self
            .gateway
            .service
            .clone()
            .start_workflow_execution(request)
            .await
            .context("Failed to start workflow")?
            .into_inner();

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

    /// Get the underlying gateway for advanced operations
    pub fn gateway(&self) -> Arc<dyn ServerGatewayApis> {
        self.gateway.clone() as Arc<dyn ServerGatewayApis>
    }
}

/// Worker-side Temporal core for polling and processing tasks
pub struct TemporalWorkerCore {
    core: Box<dyn Core>,
    #[allow(dead_code)]
    config: RunnerConfig,
}

impl TemporalWorkerCore {
    /// Create a new Temporal worker core
    pub async fn new(config: RunnerConfig) -> Result<Self> {
        let target_url = Url::parse(&format!("http://{}", config.temporal_address()))
            .context("Invalid Temporal address")?;

        let gateway_opts = ServerGatewayOptions {
            target_url,
            namespace: config.temporal_namespace(),
            task_queue: config.temporal_task_queue(),
            identity: format!("everruns-worker-{}", uuid::Uuid::now_v7()),
            worker_binary_id: env!("CARGO_PKG_VERSION").to_string(),
            long_poll_timeout: Duration::from_secs(60),
        };

        info!(
            address = %config.temporal_address(),
            namespace = %config.temporal_namespace(),
            task_queue = %config.temporal_task_queue(),
            "Initializing Temporal worker core"
        );

        let init_opts = CoreInitOptions {
            gateway_opts,
            evict_after_pending_cleared: true,
            max_outstanding_workflow_tasks: 100,
            max_outstanding_activities: 100,
        };

        let core = temporal_sdk_core::init(init_opts)
            .await
            .context("Failed to initialize Temporal core")?;

        info!("Temporal worker core initialized");

        Ok(Self {
            core: Box::new(core),
            config,
        })
    }

    /// Get a reference to the core for polling
    pub fn core(&self) -> &dyn Core {
        self.core.as_ref()
    }

    /// Shutdown the worker gracefully
    pub async fn shutdown(&self) {
        info!("Shutting down Temporal worker core");
        self.core.shutdown().await;
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
