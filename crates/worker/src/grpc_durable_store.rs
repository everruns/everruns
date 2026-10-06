// gRPC-based durable store adapter
// Decision: Workers communicate with control-plane via gRPC for durable execution
// Decision: No direct database access from workers - all operations go through gRPC
// Decision: Supports push-based task notifications with polling fallback

use std::time::Duration;

use anyhow::Result;
use everruns_internal_protocol::proto::{
    self, ClaimDurableTasksRequest, CompleteDurableTaskRequest, CountActiveDurableWorkflowsRequest,
    CreateDurableWorkflowRequest, DeregisterDurableWorkerRequest, DurableActivityOptions,
    DurableTaskDefinition, EnqueueDurableTaskRequest, FailDurableTaskRequest,
    GetDurableWorkflowStatusRequest, HeartbeatDurableTaskRequest, HeartbeatDurableWorkerRequest,
    RegisterDurableWorkerRequest, SubscribeTaskNotificationsRequest, TaskNotification,
    TaskNotificationType, UpdateDurableWorkflowStatusRequest,
};
use everruns_internal_protocol::{WorkerServiceClient, json_to_proto_struct, uuid_to_proto_uuid};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::{Request, Status, Streaming};
use uuid::Uuid;

/// Client-side interceptor that injects `authorization: Bearer <token>` metadata.
/// When `token` is `None` the interceptor is a no-op (dev mode).
#[derive(Clone)]
pub struct GrpcClientAuth {
    token: Option<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>,
}

impl GrpcClientAuth {
    /// Build from env. Reads `WORKER_GRPC_AUTH_TOKEN`; returns no-op when unset.
    pub fn from_env() -> Self {
        let token = std::env::var("WORKER_GRPC_AUTH_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
            .and_then(|t| format!("Bearer {}", t).parse().ok());
        Self { token }
    }
}

impl tonic::service::Interceptor for GrpcClientAuth {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        if let Some(ref token) = self.token {
            request
                .metadata_mut()
                .insert("authorization", token.clone());
        }
        Ok(request)
    }
}

/// Build client-side TLS config from environment variables for mutual TLS (mTLS).
///
/// When `WORKER_GRPC_TLS_CA_CERT` is set, the worker verifies the server's certificate
/// against this CA. When `WORKER_GRPC_TLS_CERT` and `WORKER_GRPC_TLS_KEY` are also set,
/// the worker presents its own certificate to the server (mutual TLS).
///
/// Returns `None` when TLS env vars are not configured (plain HTTP/2).
pub fn grpc_client_tls_from_env() -> Option<tonic::transport::ClientTlsConfig> {
    use tonic::transport::{Certificate, ClientTlsConfig, Identity};

    let ca_path = std::env::var("WORKER_GRPC_TLS_CA_CERT")
        .ok()
        .filter(|s| !s.is_empty())?;

    let ca_pem = std::fs::read_to_string(&ca_path)
        .unwrap_or_else(|e| panic!("Failed to read WORKER_GRPC_TLS_CA_CERT at {ca_path}: {e}"));
    let mut tls = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(ca_pem));

    // Optional: override the expected server domain name (for certs with non-matching CN/SAN)
    if let Some(domain) = std::env::var("WORKER_GRPC_TLS_DOMAIN")
        .ok()
        .filter(|s| !s.is_empty())
    {
        tls = tls.domain_name(domain);
    }

    // When client cert+key are provided, present them to the server (mTLS)
    if let (Some(cert_path), Some(key_path)) = (
        std::env::var("WORKER_GRPC_TLS_CERT")
            .ok()
            .filter(|s| !s.is_empty()),
        std::env::var("WORKER_GRPC_TLS_KEY")
            .ok()
            .filter(|s| !s.is_empty()),
    ) {
        let cert_pem = std::fs::read_to_string(&cert_path)
            .unwrap_or_else(|e| panic!("Failed to read WORKER_GRPC_TLS_CERT at {cert_path}: {e}"));
        let key_pem = std::fs::read_to_string(&key_path)
            .unwrap_or_else(|e| panic!("Failed to read WORKER_GRPC_TLS_KEY at {key_path}: {e}"));
        tls = tls.identity(Identity::from_pem(cert_pem, key_pem));
        tracing::info!("gRPC mTLS enabled: CA verification + client certificate");
    } else {
        tracing::info!("gRPC TLS enabled: CA verification only (no client certificate)");
    }

    Some(tls)
}

/// gRPC-based durable store client for workers
///
/// This adapter provides durable execution operations via gRPC,
/// eliminating the need for workers to have direct database access.
#[derive(Clone)]
pub struct GrpcDurableStore {
    client: WorkerServiceClient<InterceptedService<Channel, GrpcClientAuth>>,
}

impl GrpcDurableStore {
    /// Connect to the control-plane gRPC service with retry logic.
    /// Retries with exponential backoff for up to `connect_timeout` (default 30s).
    pub async fn connect(address: &str) -> Result<Self> {
        Self::connect_with_timeout(address, Duration::from_secs(30)).await
    }

    /// Connect with explicit timeout. Used by tests to avoid long waits.
    pub async fn connect_with_timeout(address: &str, max_duration: Duration) -> Result<Self> {
        use tokio::time::sleep;

        let tls_config = grpc_client_tls_from_env();
        let scheme = if tls_config.is_some() {
            "https"
        } else {
            "http"
        };
        let endpoint = format!("{}://{}", scheme, address);
        let initial_backoff = Duration::from_millis(100);
        let max_backoff = Duration::from_secs(2);

        let start = std::time::Instant::now();
        let mut backoff = initial_backoff;
        let mut attempt = 0;

        // Parsed once: the endpoint does not change between retries.
        let base_endpoint = Channel::from_shared(endpoint.clone())
            .map_err(|e| anyhow::anyhow!("Invalid gRPC endpoint '{endpoint}': {e}"))?;

        loop {
            attempt += 1;
            let mut ep = base_endpoint.clone();
            if let Some(ref tls) = tls_config {
                ep = ep
                    .tls_config(tls.clone())
                    .map_err(|e| anyhow::anyhow!("Invalid gRPC client TLS configuration: {e}"))?;
            }
            match ep.connect().await {
                Ok(channel) => {
                    // THREAT[TM-DURABLE-002]: gRPC unauthenticated access
                    // Mitigation: Attach bearer token from WORKER_GRPC_AUTH_TOKEN env
                    let auth = GrpcClientAuth::from_env();
                    let client = WorkerServiceClient::with_interceptor(channel, auth);
                    if attempt > 1 {
                        tracing::info!(
                            address = %address,
                            tls = tls_config.is_some(),
                            attempts = attempt,
                            elapsed_ms = start.elapsed().as_millis(),
                            "Connected to control-plane gRPC"
                        );
                    }
                    return Ok(Self { client });
                }
                Err(e) => {
                    let elapsed = start.elapsed();
                    if elapsed >= max_duration {
                        return Err(anyhow::anyhow!(
                            "Failed to connect to control-plane at {} after {:.1}s ({} attempts): {}",
                            address,
                            elapsed.as_secs_f32(),
                            attempt,
                            e
                        ));
                    }

                    tracing::debug!(
                        address = %address,
                        attempt = attempt,
                        backoff_ms = backoff.as_millis(),
                        error = %e,
                        "Control-plane not available, retrying..."
                    );

                    sleep(backoff).await;
                    backoff = std::cmp::min(backoff * 2, max_backoff);
                }
            }
        }
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

    /// Enqueue a task, claimed for `claim_for` when set. Returns the task's id
    /// and, when the control plane enqueued it claimed, the claimed task.
    pub async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        claim_for: Option<&str>,
    ) -> Result<(Uuid, Option<crate::durable::ClaimedTask>)> {
        let task = DurableTaskDefinition {
            workflow_id: Some(uuid_to_proto_uuid(workflow_id)),
            activity_id,
            activity_type,
            input: Some(json_to_proto_struct(&input)),
            options: Some(DurableActivityOptions::default()),
        };

        let request = EnqueueDurableTaskRequest {
            task: Some(task),
            claim_for_worker_id: claim_for.map(str::to_string),
        };

        let response = self
            .client
            .enqueue_durable_task(request)
            .await?
            .into_inner();
        let task_id = response
            .task_id
            .ok_or_else(|| anyhow::anyhow!("Missing task_id in response"))?;
        let claimed = response.claimed.map(claimed_task_from_proto).transpose()?;

        Ok((parse_proto_uuid(&task_id)?, claimed))
    }

    /// Claim tasks for execution
    pub async fn claim_tasks(
        &mut self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<crate::durable::ClaimedTask>> {
        let request = ClaimDurableTasksRequest {
            worker_id: worker_id.to_string(),
            activity_types: activity_types.to_vec(),
            max_tasks: max_tasks as i32,
        };

        let response = self.client.claim_durable_tasks(request).await?;
        response
            .into_inner()
            .tasks
            .into_iter()
            .map(claimed_task_from_proto)
            .collect()
    }

    /// Complete a task, handing its workflow off to `hand_off` in the same
    /// write when set.
    ///
    /// The worker_id must match the worker that claimed the task. Returns an
    /// error if the task was reclaimed by another worker; otherwise whether
    /// the control plane committed the hand-off (one that predates hand-offs
    /// does not) and the next step it enqueued claimed by this worker.
    pub async fn complete_task(
        &mut self,
        task_id: Uuid,
        worker_id: &str,
        output: serde_json::Value,
        hand_off: Option<proto::DurableHandOff>,
    ) -> Result<(bool, Option<crate::durable::ClaimedTask>)> {
        let request = CompleteDurableTaskRequest {
            task_id: Some(uuid_to_proto_uuid(task_id)),
            worker_id: worker_id.to_string(),
            output: Some(json_to_proto_struct(&output)),
            drain_signal_type: None,
            hand_off,
        };

        let response = self.client.complete_durable_task(request).await?;
        let inner = response.into_inner();

        if !inner.success {
            anyhow::bail!("Task not owned by worker (was reclaimed or already completed)")
        }
        let claimed = inner.claimed.map(claimed_task_from_proto).transpose()?;
        Ok((inner.handed_off, claimed))
    }

    /// Fail a task
    pub async fn fail_task(
        &mut self,
        task_id: Uuid,
        error: &str,
        retryable: bool,
    ) -> Result<(bool, bool)> {
        let request = FailDurableTaskRequest {
            task_id: Some(uuid_to_proto_uuid(task_id)),
            error: error.to_string(),
            retryable: Some(retryable),
        };

        let response = self.client.fail_durable_task(request).await?;
        let response = response.into_inner();
        Ok((response.will_retry, response.terminal_failure_owner))
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

    /// Send a signal to a running workflow
    pub async fn send_signal(
        &mut self,
        workflow_id: Uuid,
        signal: crate::durable::WorkflowSignal,
    ) -> Result<()> {
        use everruns_internal_protocol::proto::{
            DurableWorkflowSignal, SendDurableWorkflowSignalRequest,
        };
        let proto_signal = DurableWorkflowSignal {
            signal_type: signal.signal_type,
            payload: Some(everruns_internal_protocol::json_to_proto_value(
                &signal.payload,
            )),
            sent_at: signal.sent_at.to_rfc3339(),
        };
        let request = SendDurableWorkflowSignalRequest {
            workflow_id: workflow_id.to_string(),
            signal: Some(proto_signal),
        };
        self.client.send_durable_workflow_signal(request).await?;
        Ok(())
    }

    /// Get and consume pending signals for a workflow
    pub async fn get_and_consume_signals(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<Vec<crate::durable::WorkflowSignal>> {
        use everruns_internal_protocol::proto::GetAndConsumeDurableWorkflowSignalsRequest;
        let request = GetAndConsumeDurableWorkflowSignalsRequest {
            workflow_id: workflow_id.to_string(),
            signal_type: String::new(),
            peek: None,
        };
        let response = self
            .client
            .get_and_consume_durable_workflow_signals(request)
            .await?;
        let signals = response
            .into_inner()
            .signals
            .into_iter()
            .filter_map(|s| {
                let payload = s
                    .payload
                    .as_ref()
                    .map(everruns_internal_protocol::proto_value_to_json)
                    .unwrap_or(serde_json::json!({}));
                match chrono::DateTime::parse_from_rfc3339(&s.sent_at) {
                    Ok(dt) => Some(crate::durable::WorkflowSignal {
                        signal_type: s.signal_type,
                        payload,
                        sent_at: dt.with_timezone(&chrono::Utc),
                    }),
                    Err(err) => {
                        tracing::warn!(
                            sent_at = %s.sent_at,
                            signal_type = %s.signal_type,
                            error = %err,
                            "Skipping workflow signal with malformed sent_at"
                        );
                        None
                    }
                }
            })
            .collect();
        Ok(signals)
    }

    /// How many signals of `signal_type` are pending, without consuming them.
    ///
    /// A control plane that predates `peek` consumes them; the count is then
    /// what a destructive drain would have returned, as before.
    pub async fn count_pending_signals(
        &mut self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<usize> {
        use everruns_internal_protocol::proto::GetAndConsumeDurableWorkflowSignalsRequest;
        let request = GetAndConsumeDurableWorkflowSignalsRequest {
            workflow_id: workflow_id.to_string(),
            signal_type: signal_type.to_string(),
            peek: Some(true),
        };
        let response = self
            .client
            .get_and_consume_durable_workflow_signals(request)
            .await?;
        Ok(response.into_inner().signals.len())
    }

    /// Get and consume pending signals of a specific type for a workflow
    pub async fn get_and_consume_signals_by_type(
        &mut self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<crate::durable::WorkflowSignal>> {
        use everruns_internal_protocol::proto::GetAndConsumeDurableWorkflowSignalsRequest;
        let request = GetAndConsumeDurableWorkflowSignalsRequest {
            workflow_id: workflow_id.to_string(),
            signal_type: signal_type.to_string(),
            peek: None,
        };
        let response = self
            .client
            .get_and_consume_durable_workflow_signals(request)
            .await?;
        let signals = response
            .into_inner()
            .signals
            .into_iter()
            .filter_map(|s| {
                let payload = s
                    .payload
                    .as_ref()
                    .map(everruns_internal_protocol::proto_value_to_json)
                    .unwrap_or(serde_json::json!({}));
                match chrono::DateTime::parse_from_rfc3339(&s.sent_at) {
                    Ok(dt) => Some(crate::durable::WorkflowSignal {
                        signal_type: s.signal_type,
                        payload,
                        sent_at: dt.with_timezone(&chrono::Utc),
                    }),
                    Err(err) => {
                        tracing::warn!(
                            sent_at = %s.sent_at,
                            signal_type = %s.signal_type,
                            error = %err,
                            "Skipping workflow signal with malformed sent_at"
                        );
                        None
                    }
                }
            })
            .collect();
        Ok(signals)
    }

    /// Register this worker with the control-plane
    pub async fn register_worker(
        &mut self,
        worker_id: &str,
        worker_group: Option<String>,
        activity_types: Vec<String>,
        max_concurrency: u32,
    ) -> Result<()> {
        let request = RegisterDurableWorkerRequest {
            worker_id: worker_id.to_string(),
            worker_group,
            activity_types,
            max_concurrency: max_concurrency as i32,
        };

        self.client.register_durable_worker(request).await?;
        Ok(())
    }

    /// Send worker heartbeat
    pub async fn heartbeat_worker(
        &mut self,
        worker_id: &str,
        current_load: u32,
        accepting_tasks: bool,
    ) -> Result<()> {
        let request = HeartbeatDurableWorkerRequest {
            worker_id: worker_id.to_string(),
            current_load: current_load as i32,
            accepting_tasks,
        };

        self.client.heartbeat_durable_worker(request).await?;
        Ok(())
    }

    /// Deregister this worker and reclaim all its tasks
    /// Returns the number of tasks that were reclaimed (set back to pending)
    pub async fn deregister_worker(&mut self, worker_id: &str) -> Result<usize> {
        let request = DeregisterDurableWorkerRequest {
            worker_id: worker_id.to_string(),
        };

        let response = self.client.deregister_durable_worker(request).await?;
        Ok(response.into_inner().tasks_reclaimed as usize)
    }

    // ========================================================================
    // Push-based task notifications
    // ========================================================================

    /// Subscribe to task notifications for push-based task pickup
    ///
    /// Returns a stream of task notifications. The stream will emit:
    /// - TASK_AVAILABLE notifications when tasks matching the activity types are enqueued
    /// - HEARTBEAT notifications periodically to keep the connection alive
    ///
    /// If the stream disconnects, the caller should fall back to polling.
    pub async fn subscribe_task_notifications(
        &mut self,
        worker_id: &str,
        activity_types: Vec<String>,
    ) -> Result<TaskNotificationStream> {
        let request = SubscribeTaskNotificationsRequest {
            worker_id: worker_id.to_string(),
            activity_types,
        };

        let response = self.client.subscribe_task_notifications(request).await?;

        Ok(TaskNotificationStream {
            inner: response.into_inner(),
        })
    }
}

/// Stream of task notifications from the control-plane
pub struct TaskNotificationStream {
    inner: Streaming<TaskNotification>,
}

impl TaskNotificationStream {
    /// Receive the next notification from the stream
    ///
    /// Returns None if the stream has ended.
    pub async fn recv(&mut self) -> Option<TaskNotificationEvent> {
        match self.inner.message().await {
            Ok(Some(notification)) => {
                let notification_type =
                    TaskNotificationType::try_from(notification.notification_type)
                        .unwrap_or(TaskNotificationType::Unspecified);

                match notification_type {
                    TaskNotificationType::TaskAvailable => {
                        Some(TaskNotificationEvent::TaskAvailable {
                            activity_type: notification.activity_type,
                            pending_count: notification.pending_count,
                        })
                    }
                    TaskNotificationType::Heartbeat => Some(TaskNotificationEvent::Heartbeat),
                    TaskNotificationType::Unspecified => Some(TaskNotificationEvent::Heartbeat),
                }
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(error = %e, "Task notification stream error");
                None
            }
        }
    }
}

// ============================================================================
// Helper types
// ============================================================================

/// Task notification event from the control-plane
#[derive(Debug, Clone)]
pub enum TaskNotificationEvent {
    /// A task is available for claiming
    TaskAvailable {
        /// The activity type of the available task
        activity_type: String,
        /// Approximate number of pending tasks (hint for batch claiming)
        pending_count: i32,
    },
    /// Heartbeat to keep the connection alive
    Heartbeat,
}

/// Response from heartbeat operation
#[derive(Debug, Clone)]
pub struct HeartbeatResponse {
    pub acknowledged: bool,
    pub should_cancel: bool,
}

/// Workflow status (mirrors crate::durable::WorkflowStatus)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    ContinuedAsNew,
}

impl WorkflowStatus {
    /// Check if this status is terminal
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::ContinuedAsNew
        )
    }
}

// ============================================================================
// Helper functions
// ============================================================================

fn parse_proto_uuid(proto_uuid: &proto::Uuid) -> Result<Uuid> {
    Uuid::parse_str(&proto_uuid.value).map_err(|e| anyhow::anyhow!("Invalid UUID: {}", e))
}

/// A claimed task from the wire. The claim reports the workflow status, so
/// the driver's pre-execution check reads it from the task.
fn claimed_task_from_proto(t: proto::DurableClaimedTask) -> Result<crate::durable::ClaimedTask> {
    let status = t
        .workflow_status
        .map(|_| proto_status_to_workflow(t.workflow_status()));
    let id =
        t.id.as_ref()
            .map(parse_proto_uuid)
            .transpose()?
            .unwrap_or_else(Uuid::nil);
    let workflow_id = t.workflow_id.as_ref().map(parse_proto_uuid).transpose()?;
    let input = t
        .input
        .map(|s| everruns_internal_protocol::proto_struct_to_json(&s))
        .unwrap_or_else(|| serde_json::json!({}));

    Ok(crate::durable::ClaimedTask {
        id,
        workflow_id,
        activity_id: t.activity_id,
        activity_type: t.activity_type,
        input,
        options: crate::durable::ActivityOptions::default(),
        attempt: t.attempt as u32,
        max_attempts: t.max_attempts as u32,
        workflow_status: status.map(crate::grpc_task_store::grpc_status_to_workflow_status),
    })
}

fn workflow_status_to_proto(status: WorkflowStatus) -> proto::DurableWorkflowStatus {
    match status {
        WorkflowStatus::Pending => proto::DurableWorkflowStatus::Pending,
        WorkflowStatus::Running => proto::DurableWorkflowStatus::Running,
        WorkflowStatus::Completed => proto::DurableWorkflowStatus::Completed,
        WorkflowStatus::Failed => proto::DurableWorkflowStatus::Failed,
        WorkflowStatus::Cancelled => proto::DurableWorkflowStatus::Cancelled,
        WorkflowStatus::ContinuedAsNew => proto::DurableWorkflowStatus::ContinuedAsNew,
    }
}

fn proto_status_to_workflow(status: proto::DurableWorkflowStatus) -> WorkflowStatus {
    match status {
        proto::DurableWorkflowStatus::Pending => WorkflowStatus::Pending,
        proto::DurableWorkflowStatus::Running => WorkflowStatus::Running,
        proto::DurableWorkflowStatus::Completed => WorkflowStatus::Completed,
        proto::DurableWorkflowStatus::Failed => WorkflowStatus::Failed,
        proto::DurableWorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        proto::DurableWorkflowStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
        proto::DurableWorkflowStatus::Unspecified => WorkflowStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex, MutexGuard};
    use tonic::service::Interceptor;

    static TLS_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn a_claimed_task_carries_its_workflow_status() {
        let workflow_id = Uuid::now_v7();
        let task = claimed_task_from_proto(proto::DurableClaimedTask {
            id: Some(uuid_to_proto_uuid(Uuid::now_v7())),
            workflow_id: Some(uuid_to_proto_uuid(workflow_id)),
            activity_id: "reason_1".into(),
            activity_type: "reason".into(),
            input: None,
            attempt: 1,
            max_attempts: 3,
            workflow_status: Some(proto::DurableWorkflowStatus::Cancelled.into()),
        })
        .unwrap();
        assert_eq!(task.workflow_id, Some(workflow_id));
        assert_eq!(
            task.workflow_status,
            Some(crate::durable::WorkflowStatus::Cancelled)
        );
        assert_eq!(task.input, serde_json::json!({}));
    }

    struct TlsEnvGuard {
        _lock: MutexGuard<'static, ()>,
        ca_cert: Option<String>,
        cert: Option<String>,
        key: Option<String>,
        domain: Option<String>,
    }

    impl TlsEnvGuard {
        fn new() -> Self {
            let lock = TLS_ENV_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            Self {
                _lock: lock,
                ca_cert: std::env::var("WORKER_GRPC_TLS_CA_CERT").ok(),
                cert: std::env::var("WORKER_GRPC_TLS_CERT").ok(),
                key: std::env::var("WORKER_GRPC_TLS_KEY").ok(),
                domain: std::env::var("WORKER_GRPC_TLS_DOMAIN").ok(),
            }
        }

        fn set_var(&self, key: &str, value: &str) {
            unsafe {
                std::env::set_var(key, value);
            }
        }

        fn remove_var(&self, key: &str) {
            unsafe {
                std::env::remove_var(key);
            }
        }
    }

    impl Drop for TlsEnvGuard {
        fn drop(&mut self) {
            restore_env_var("WORKER_GRPC_TLS_CA_CERT", self.ca_cert.as_deref());
            restore_env_var("WORKER_GRPC_TLS_CERT", self.cert.as_deref());
            restore_env_var("WORKER_GRPC_TLS_KEY", self.key.as_deref());
            restore_env_var("WORKER_GRPC_TLS_DOMAIN", self.domain.as_deref());
        }
    }

    fn restore_env_var(key: &str, value: Option<&str>) {
        unsafe {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }

    #[test]
    fn test_workflow_status_is_terminal() {
        assert!(!WorkflowStatus::Pending.is_terminal());
        assert!(!WorkflowStatus::Running.is_terminal());
        assert!(WorkflowStatus::Completed.is_terminal());
        assert!(WorkflowStatus::Failed.is_terminal());
        assert!(WorkflowStatus::Cancelled.is_terminal());
    }

    #[tokio::test]
    async fn test_connect_fails_after_timeout_on_unavailable_server() {
        let env = TlsEnvGuard::new();
        env.remove_var("WORKER_GRPC_TLS_CA_CERT");
        env.remove_var("WORKER_GRPC_TLS_CERT");
        env.remove_var("WORKER_GRPC_TLS_KEY");
        env.remove_var("WORKER_GRPC_TLS_DOMAIN");

        // Verify connection fails with clear error after retry timeout
        // when control-plane is unavailable
        let timeout = Duration::from_secs(5);
        let start = std::time::Instant::now();
        let result = GrpcDurableStore::connect_with_timeout("127.0.0.1:19999", timeout).await;

        assert!(result.is_err());
        let elapsed = start.elapsed();

        // Should take ~5 seconds (the retry timeout)
        assert!(
            elapsed.as_secs() >= 4,
            "Should retry for at least 4 seconds, got {:?}",
            elapsed
        );
        assert!(
            elapsed.as_secs() <= 8,
            "Should not take more than 8 seconds, got {:?}",
            elapsed
        );

        // Error message should be helpful
        let err_msg = result.err().expect("Expected error").to_string();
        assert!(
            err_msg.contains("127.0.0.1:19999"),
            "Error should contain address: {}",
            err_msg
        );
        assert!(
            err_msg.contains("5") || err_msg.contains("attempts"),
            "Error should mention timeout or attempts: {}",
            err_msg
        );
    }

    #[test]
    fn test_client_auth_injects_bearer_token() {
        let mut auth = GrpcClientAuth {
            token: Some("Bearer my-secret".parse().unwrap()),
        };
        let request = Request::new(());
        let result = auth.call(request).unwrap();
        let authz = result
            .metadata()
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(authz, "Bearer my-secret");
    }

    #[test]
    fn test_client_auth_noop_when_no_token() {
        let mut auth = GrpcClientAuth { token: None };
        let request = Request::new(());
        let result = auth.call(request).unwrap();
        assert!(result.metadata().get("authorization").is_none());
    }

    #[test]
    fn test_grpc_client_tls_returns_none_when_no_env_vars() {
        let env = TlsEnvGuard::new();
        env.remove_var("WORKER_GRPC_TLS_CA_CERT");
        env.remove_var("WORKER_GRPC_TLS_CERT");
        env.remove_var("WORKER_GRPC_TLS_KEY");
        env.remove_var("WORKER_GRPC_TLS_DOMAIN");

        let config = grpc_client_tls_from_env();
        assert!(
            config.is_none(),
            "Should return None when TLS not configured"
        );
    }

    #[test]
    fn test_grpc_client_tls_returns_none_when_ca_cert_empty() {
        let env = TlsEnvGuard::new();
        env.set_var("WORKER_GRPC_TLS_CA_CERT", "");
        let config = grpc_client_tls_from_env();
        assert!(config.is_none());
    }

    #[test]
    fn test_grpc_client_tls_returns_config_with_valid_ca() {
        let env = TlsEnvGuard::new();
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let ca_path = format!("{}/tests/fixtures/test-ca.pem", manifest);

        env.set_var("WORKER_GRPC_TLS_CA_CERT", &ca_path);
        env.remove_var("WORKER_GRPC_TLS_CERT");
        env.remove_var("WORKER_GRPC_TLS_KEY");
        env.remove_var("WORKER_GRPC_TLS_DOMAIN");

        let config = grpc_client_tls_from_env();
        assert!(
            config.is_some(),
            "Should return Some when CA cert is configured"
        );
    }

    #[test]
    fn test_grpc_client_tls_with_client_cert() {
        let env = TlsEnvGuard::new();
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let ca_path = format!("{}/tests/fixtures/test-ca.pem", manifest);
        let cert_path = format!("{}/tests/fixtures/test-client-cert.pem", manifest);
        let key_path = format!("{}/tests/fixtures/test-client-key.pem", manifest);

        env.set_var("WORKER_GRPC_TLS_CA_CERT", &ca_path);
        env.set_var("WORKER_GRPC_TLS_CERT", &cert_path);
        env.set_var("WORKER_GRPC_TLS_KEY", &key_path);
        env.remove_var("WORKER_GRPC_TLS_DOMAIN");

        let config = grpc_client_tls_from_env();
        assert!(
            config.is_some(),
            "Should return Some when CA+cert+key are configured"
        );
    }

    #[test]
    #[should_panic(expected = "Failed to read WORKER_GRPC_TLS_CA_CERT")]
    fn test_grpc_client_tls_panics_on_missing_ca_file() {
        let env = TlsEnvGuard::new();
        env.set_var("WORKER_GRPC_TLS_CA_CERT", "/nonexistent/ca.pem");
        env.remove_var("WORKER_GRPC_TLS_CERT");
        env.remove_var("WORKER_GRPC_TLS_KEY");
        env.remove_var("WORKER_GRPC_TLS_DOMAIN");
        let _config = grpc_client_tls_from_env();
    }
}
