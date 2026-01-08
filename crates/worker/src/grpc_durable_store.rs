// gRPC-based durable store adapter
// Decision: Workers communicate with control-plane via gRPC for durable execution
// Decision: No direct database access from workers - all operations go through gRPC

use anyhow::Result;
use everruns_internal_protocol::proto::{
    self, ClaimDurableTasksRequest, CompleteDurableTaskRequest, CountActiveDurableWorkflowsRequest,
    CreateDurableWorkflowRequest, DurableActivityOptions, DurableTaskDefinition,
    EnqueueDurableTaskRequest, FailDurableTaskRequest, GetDurableWorkflowStatusRequest,
    HeartbeatDurableTaskRequest, UpdateDurableWorkflowStatusRequest,
};
use everruns_internal_protocol::{json_to_proto_struct, uuid_to_proto_uuid, WorkerServiceClient};
use tonic::transport::Channel;
use uuid::Uuid;

/// gRPC-based durable store client for workers
///
/// This adapter provides durable execution operations via gRPC,
/// eliminating the need for workers to have direct database access.
#[derive(Clone)]
pub struct GrpcDurableStore {
    client: WorkerServiceClient<Channel>,
}

impl GrpcDurableStore {
    /// Connect to the control-plane gRPC service
    pub async fn connect(address: &str) -> Result<Self> {
        let endpoint = format!("http://{}", address);
        let client = WorkerServiceClient::connect(endpoint).await?;
        Ok(Self { client })
    }

    /// Create a new durable workflow
    pub async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        let request = CreateDurableWorkflowRequest {
            workflow_type: workflow_type.to_string(),
            input: Some(json_to_proto_struct(&input)),
            workflow_id: Some(uuid_to_proto_uuid(workflow_id)),
        };

        let response = self.client.create_durable_workflow(request).await?;
        let workflow_id = response
            .into_inner()
            .workflow_id
            .ok_or_else(|| anyhow::anyhow!("Missing workflow_id in response"))?;

        parse_proto_uuid(&workflow_id)
    }

    /// Get workflow status
    pub async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let request = GetDurableWorkflowStatusRequest {
            workflow_id: Some(uuid_to_proto_uuid(workflow_id)),
        };

        let response = self.client.get_durable_workflow_status(request).await?;
        let inner = response.into_inner();

        let status = proto_status_to_workflow(inner.status());
        let output = inner
            .output
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s));
        let error = inner.error;

        Ok((status, output, error))
    }

    /// Update workflow status
    pub async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()> {
        let request = UpdateDurableWorkflowStatusRequest {
            workflow_id: Some(uuid_to_proto_uuid(workflow_id)),
            status: workflow_status_to_proto(status).into(),
            output: output.map(|o| json_to_proto_struct(&o)),
            error,
        };

        self.client.update_durable_workflow_status(request).await?;
        Ok(())
    }

    /// Enqueue a task
    pub async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        let task = DurableTaskDefinition {
            workflow_id: Some(uuid_to_proto_uuid(workflow_id)),
            activity_id,
            activity_type,
            input: Some(json_to_proto_struct(&input)),
            options: Some(DurableActivityOptions::default()),
        };

        let request = EnqueueDurableTaskRequest { task: Some(task) };

        let response = self.client.enqueue_durable_task(request).await?;
        let task_id = response
            .into_inner()
            .task_id
            .ok_or_else(|| anyhow::anyhow!("Missing task_id in response"))?;

        parse_proto_uuid(&task_id)
    }

    /// Claim tasks for execution
    pub async fn claim_tasks(
        &mut self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>> {
        let request = ClaimDurableTasksRequest {
            worker_id: worker_id.to_string(),
            activity_types: activity_types.to_vec(),
            max_tasks: max_tasks as i32,
        };

        let response = self.client.claim_durable_tasks(request).await?;
        let tasks = response
            .into_inner()
            .tasks
            .into_iter()
            .map(|t| {
                let id =
                    t.id.as_ref()
                        .map(parse_proto_uuid)
                        .transpose()?
                        .unwrap_or_else(Uuid::nil);
                let workflow_id = t
                    .workflow_id
                    .as_ref()
                    .map(parse_proto_uuid)
                    .transpose()?
                    .unwrap_or_else(Uuid::nil);
                let input = t
                    .input
                    .map(|s| everruns_internal_protocol::proto_struct_to_json(&s))
                    .unwrap_or_else(|| serde_json::json!({}));

                Ok(ClaimedTask {
                    id,
                    workflow_id,
                    activity_id: t.activity_id,
                    activity_type: t.activity_type,
                    input,
                    attempt: t.attempt as u32,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(tasks)
    }

    /// Complete a task
    pub async fn complete_task(&mut self, task_id: Uuid, output: serde_json::Value) -> Result<()> {
        let request = CompleteDurableTaskRequest {
            task_id: Some(uuid_to_proto_uuid(task_id)),
            output: Some(json_to_proto_struct(&output)),
        };

        self.client.complete_durable_task(request).await?;
        Ok(())
    }

    /// Fail a task
    pub async fn fail_task(&mut self, task_id: Uuid, error: &str) -> Result<bool> {
        let request = FailDurableTaskRequest {
            task_id: Some(uuid_to_proto_uuid(task_id)),
            error: error.to_string(),
        };

        let response = self.client.fail_durable_task(request).await?;
        Ok(response.into_inner().will_retry)
    }

    /// Send heartbeat for a task
    pub async fn heartbeat_task(
        &mut self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse> {
        let request = HeartbeatDurableTaskRequest {
            task_id: Some(uuid_to_proto_uuid(task_id)),
            worker_id: worker_id.to_string(),
            details: details.map(|d| json_to_proto_struct(&d)),
        };

        let response = self.client.heartbeat_durable_task(request).await?;
        let inner = response.into_inner();

        Ok(HeartbeatResponse {
            acknowledged: inner.acknowledged,
            should_cancel: inner.should_cancel,
        })
    }

    /// Count active (non-terminal) workflows
    pub async fn count_active_workflows(&mut self) -> Result<usize> {
        let request = CountActiveDurableWorkflowsRequest {};
        let response = self.client.count_active_durable_workflows(request).await?;
        Ok(response.into_inner().count as usize)
    }
}

// ============================================================================
// Helper types
// ============================================================================

/// Claimed task from the queue
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_id: String,
    pub activity_type: String,
    pub input: serde_json::Value,
    pub attempt: u32,
}

/// Response from heartbeat operation
#[derive(Debug, Clone)]
pub struct HeartbeatResponse {
    pub acknowledged: bool,
    pub should_cancel: bool,
}

/// Workflow status (mirrors everruns_durable::WorkflowStatus)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl WorkflowStatus {
    /// Check if this status is terminal
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

// ============================================================================
// Helper functions
// ============================================================================

fn parse_proto_uuid(proto_uuid: &proto::Uuid) -> Result<Uuid> {
    Uuid::parse_str(&proto_uuid.value).map_err(|e| anyhow::anyhow!("Invalid UUID: {}", e))
}

fn workflow_status_to_proto(status: WorkflowStatus) -> proto::DurableWorkflowStatus {
    match status {
        WorkflowStatus::Pending => proto::DurableWorkflowStatus::Pending,
        WorkflowStatus::Running => proto::DurableWorkflowStatus::Running,
        WorkflowStatus::Completed => proto::DurableWorkflowStatus::Completed,
        WorkflowStatus::Failed => proto::DurableWorkflowStatus::Failed,
        WorkflowStatus::Cancelled => proto::DurableWorkflowStatus::Cancelled,
    }
}

fn proto_status_to_workflow(status: proto::DurableWorkflowStatus) -> WorkflowStatus {
    match status {
        proto::DurableWorkflowStatus::Pending => WorkflowStatus::Pending,
        proto::DurableWorkflowStatus::Running => WorkflowStatus::Running,
        proto::DurableWorkflowStatus::Completed => WorkflowStatus::Completed,
        proto::DurableWorkflowStatus::Failed => WorkflowStatus::Failed,
        proto::DurableWorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        proto::DurableWorkflowStatus::Unspecified => WorkflowStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_status_is_terminal() {
        assert!(!WorkflowStatus::Pending.is_terminal());
        assert!(!WorkflowStatus::Running.is_terminal());
        assert!(WorkflowStatus::Completed.is_terminal());
        assert!(WorkflowStatus::Failed.is_terminal());
        assert!(WorkflowStatus::Cancelled.is_terminal());
    }
}
