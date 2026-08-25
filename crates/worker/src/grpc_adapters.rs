// gRPC-backed adapters for core traits
//
// Decision: Workers communicate with control plane via gRPC for all operations
// Decision: This replaces direct database access in worker crates
// Decision: 15 per-trait wrapper structs consolidated into 2 adapters (EVE-102):
//   - GrpcAdapter      (session-scoped, no org_id)
//   - GrpcOrgAdapter   (org-scoped, carries org_id)
//
// These implementations use the internal-protocol gRPC client to communicate
// with the control-plane service (the API server's gRPC endpoint).

use async_trait::async_trait;
use everruns_core::connection_services::ProviderCredentials;
use everruns_core::events::{Event, EventRequest};
use everruns_core::leased_resource::{LeasedResource, LeasedResourceStatus, UpsertLeasedResource};
use everruns_core::message_retriever::{InputMessage, MessageHistory, MessageRetriever};
use everruns_core::session_file::{
    FileInfo, FileStat, GrepContextBlock, GrepContextLine, GrepMatch, GrepOptions,
    GrepSearchResult, SessionFile,
};
use everruns_core::{
    AgentDefinition, ExecutionSession, HarnessDefinition, Message, MessageFilter, MessageRole,
};
use everruns_core::{
    connection_services::ProviderCredentialStore, event_emitter::EventEmitter,
    execution_loading::AgentStore, execution_loading::HarnessStore,
    execution_loading::SessionStore, image_services::CreateStoredImage,
    image_services::ImageArtifactStore, image_services::ResolvedImage, image_services::StoredImage,
    image_services::StoredImageInfo, provider_resolution::ProviderStore,
    session_files::SessionFileSystem, session_services::LeasedResourceStore,
};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::typed_id::{AgentId, LeasedResourceId, MessageId, ModelId, SessionId};
// EVE-882: the stored Session record and its lifecycle enums moved to
// `everruns-platform`; the worker's PlatformStore surface still transports
// them, while execution paths carry only the portable `ExecutionSession`.
use everruns_platform::{Session, SessionParticipant, SessionStatus};
// EVE-877: the stored Agent record moved to `everruns-platform`; the gRPC wire
// still carries it between server and worker (proto shape unchanged).
use everruns_internal_protocol::proto;
use everruns_internal_protocol::{
    WorkerServiceClient, json_to_proto_list, json_to_proto_struct, proto_list_to_json,
    proto_struct_to_json,
};
// EVE-881: the stored Harness record likewise lives in `everruns-platform`;
// the gRPC wire carries the pre-merged record between server and worker.
use everruns_platform::{Agent, Harness, HarnessStatus};
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::grpc_durable_store::GrpcClientAuth;

const COMMAND_API_VERSION_V1: &str = "v1";

#[derive(Debug, serde::Deserialize)]
struct CommandPage<T> {
    data: Vec<T>,
    total: u32,
}

/// Map a tonic gRPC status to the appropriate AgentLoopError variant.
///
/// Preserves the semantic meaning of gRPC status codes so that callers
/// (e.g. retry logic in the durable engine) can distinguish transient
/// transport errors from permanent domain errors.
fn grpc_status_to_error(status: tonic::Status) -> AgentLoopError {
    let msg = status.message().to_string();
    match status.code() {
        tonic::Code::NotFound => {
            // Map to specific "not found" variants when possible
            if msg.contains("Session") {
                AgentLoopError::store(format!("Session not found: {msg}"))
            } else if msg.contains("Agent") {
                AgentLoopError::store(format!("Agent not found: {msg}"))
            } else if msg.contains("Harness") {
                AgentLoopError::store(format!("Harness not found: {msg}"))
            } else {
                AgentLoopError::store(format!("Not found: {msg}"))
            }
        }
        tonic::Code::InvalidArgument => AgentLoopError::config(format!("Invalid argument: {msg}")),
        tonic::Code::Unavailable => AgentLoopError::store(format!("Service unavailable: {msg}")),
        tonic::Code::ResourceExhausted => {
            let msg_lower = msg.to_ascii_lowercase();
            if msg_lower.contains("message")
                || msg_lower.contains("payload")
                || msg_lower.contains("size")
                || msg_lower.contains("too large")
                || msg_lower.contains("context length")
            {
                AgentLoopError::request_too_large(msg)
            } else {
                AgentLoopError::store(format!("Resource exhausted: {msg}"))
            }
        }
        tonic::Code::Unauthenticated | tonic::Code::PermissionDenied => {
            AgentLoopError::config(format!("Auth error: {msg}"))
        }
        _ => AgentLoopError::store(format!("gRPC error ({}): {msg}", status.code())),
    }
}

/// Create a store error for issues in gRPC responses (e.g., missing fields).
fn grpc_missing_field(field: &str) -> AgentLoopError {
    AgentLoopError::store(format!("gRPC response error: {field}"))
}

fn grpc_command_error_to_error(error: proto::CommandError) -> AgentLoopError {
    match error.kind {
        1 => AgentLoopError::config(error.message),
        2 => AgentLoopError::config(format!("Permission denied: {}", error.message)),
        3 => AgentLoopError::store(format!("Not found: {}", error.message)),
        4 => AgentLoopError::store(format!("Conflict: {}", error.message)),
        _ => AgentLoopError::store(error.message),
    }
}

fn capability_refs_to_configs(capabilities: &[String]) -> Vec<serde_json::Value> {
    capabilities
        .iter()
        .map(|capability| serde_json::json!({ "ref": capability, "config": {} }))
        .collect()
}

/// Fetch image binary from a presigned URL and return as base64-encoded ResolvedImage.
///
/// Used when the control plane returns presigned URLs instead of inline base64 data,
/// keeping gRPC messages small while still providing base64 data to LLM providers.
async fn fetch_image_from_url(url: &str, media_type: &str) -> Result<ResolvedImage> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AgentLoopError::store(format!("Failed to build HTTP client: {e}")))?;

    let response = client.get(url).send().await.map_err(|e| {
        AgentLoopError::store(format!("Failed to fetch image from presigned URL: {e}"))
    })?;

    if !response.status().is_success() {
        return Err(AgentLoopError::store(format!(
            "Presigned image fetch returned status {}",
            response.status()
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| AgentLoopError::store(format!("Failed to read image response body: {e}")))?;

    use base64::Engine;
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(ResolvedImage::new(base64_data, media_type))
}

/// gRPC client wrapper for worker operations
#[derive(Clone)]
pub struct GrpcClient {
    inner: Arc<Mutex<WorkerServiceClient<InterceptedService<Channel, GrpcClientAuth>>>>,
}

/// Max gRPC message size (16MB)
///
/// Image data no longer flows through gRPC — workers fetch images via presigned
/// HTTP URLs returned by resolve_image/resolve_images RPCs. The limit only needs
/// to accommodate metadata, proto messages, and session file content.
const MAX_GRPC_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

impl GrpcClient {
    /// Connect to the control plane gRPC server
    pub async fn connect(addr: &str) -> Result<Self> {
        let endpoint = format!("http://{}", addr);
        let channel = tonic::transport::Endpoint::from_shared(endpoint)
            .map_err(|e| AgentLoopError::store(format!("Invalid gRPC endpoint: {}", e)))?
            .connect()
            .await
            .map_err(|e| AgentLoopError::store(format!("gRPC connection failed: {e}")))?;

        // THREAT[TM-DURABLE-002]: gRPC unauthenticated access
        // Mitigation: Attach bearer token from WORKER_GRPC_AUTH_TOKEN env
        let auth = GrpcClientAuth::from_env();
        let client = WorkerServiceClient::with_interceptor(channel, auth)
            .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE);

        Ok(Self {
            inner: Arc::new(Mutex::new(client)),
        })
    }

    /// Create from an existing channel
    pub fn from_channel(channel: Channel) -> Self {
        let auth = GrpcClientAuth::from_env();
        let client = WorkerServiceClient::with_interceptor(channel, auth)
            .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE);
        Self {
            inner: Arc::new(Mutex::new(client)),
        }
    }

    /// Set session status (started, active, idle).
    ///
    /// Acknowledgement only (EVE-882): the wire response still carries the
    /// stored record for older workers, but status mutation exposes no
    /// session record to the caller.
    pub async fn set_session_status(
        &self,
        org_id: i64,
        session_id: SessionId,
        status: &str,
    ) -> Result<()> {
        let request = proto::SetSessionStatusRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            status: status.to_string(),
            org_id,
        };

        let mut client = self.inner.lock().await;
        client
            .set_session_status(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }

    /// Set session title, acknowledging with the refreshed portable
    /// execution view (EVE-882).
    pub async fn set_session_title(
        &self,
        org_id: i64,
        session_id: SessionId,
        title: &str,
    ) -> Result<ExecutionSession> {
        let request = proto::SetSessionTitleRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            title: title.to_string(),
            org_id,
        };

        let mut client = self.inner.lock().await;
        let response = client
            .set_session_title(request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_session = response
            .into_inner()
            .session
            .ok_or_else(|| grpc_missing_field("No session in response"))?;

        proto_session_to_session(proto_session)
    }

    pub async fn create_image_artifact(
        &self,
        org_id: i64,
        input: CreateStoredImage,
    ) -> Result<StoredImageInfo> {
        let request = proto::CreateImageArtifactRequest {
            org_id,
            filename: input.filename,
            content_type: input.content_type,
            data: input.data,
            metadata: Some(json_to_proto_struct(&input.metadata)),
        };

        let mut client = self.inner.lock().await;
        let response = client
            .create_image_artifact(request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_image = response
            .into_inner()
            .image
            .ok_or_else(|| grpc_missing_field("No image in response"))?;

        proto_stored_image_info_to_schema(proto_image)
    }

    pub async fn get_image_artifact(
        &self,
        org_id: i64,
        image_id: everruns_provider::typed_id::ImageId,
    ) -> Result<Option<StoredImage>> {
        let request = proto::GetImageArtifactRequest {
            org_id,
            image_id: Some(uuid_to_proto(image_id.uuid())),
        };

        let mut client = self.inner.lock().await;
        let response = client
            .get_image_artifact(request)
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .image
            .map(proto_stored_image_to_schema)
            .transpose()
    }

    pub async fn get_image_artifact_info(
        &self,
        org_id: i64,
        image_id: everruns_provider::typed_id::ImageId,
    ) -> Result<Option<StoredImageInfo>> {
        let request = proto::GetImageArtifactInfoRequest {
            org_id,
            image_id: Some(uuid_to_proto(image_id.uuid())),
        };

        let mut client = self.inner.lock().await;
        let response = client
            .get_image_artifact_info(request)
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .image
            .map(proto_stored_image_info_to_schema)
            .transpose()
    }

    pub async fn get_default_provider_credentials(
        &self,
        org_id: i64,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        let request = proto::GetDefaultProviderCredentialsRequest {
            org_id,
            provider_type: provider_type.to_string(),
            provider_id: String::new(),
        };

        let mut client = self.inner.lock().await;
        let response = client
            .get_default_provider_credentials(request)
            .await
            .map_err(grpc_status_to_error)?;

        let response = response.into_inner();
        if !response.found {
            return Ok(None);
        }

        Ok(Some(ProviderCredentials {
            api_key: response.api_key,
            base_url: non_empty_string(response.base_url),
        }))
    }

    pub async fn get_provider_config(
        &self,
        org_id: i64,
        provider_id: &str,
    ) -> Result<Option<everruns_provider::driver_registry::ProviderConfig>> {
        let request = proto::GetDefaultProviderCredentialsRequest {
            org_id,
            provider_type: String::new(),
            provider_id: provider_id.to_string(),
        };
        let mut client = self.inner.lock().await;
        let response = client
            .get_default_provider_credentials(request)
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        if !response.found {
            return Ok(None);
        }
        let provider_type = response
            .provider_type
            .parse()
            .unwrap_or_else(|_| unreachable!());
        // A connection with no options (or an older server that does not send
        // the field) yields the empty options, which changes nothing.
        let request_options = non_empty_string(response.request_options_json)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Ok(Some(everruns_provider::driver_registry::ProviderConfig {
            provider: everruns_provider::runtime_provider::ProviderKey::new(provider_id),
            provider_type,
            api_key: non_empty_string(response.api_key),
            base_url: non_empty_string(response.base_url),
            metadata: everruns_provider::driver_registry::ProviderMetadata::default(),
            request_options,
        }))
    }

    /// Get MCP server info by name prefix (for MCP tool execution)
    pub async fn get_mcp_server_by_prefix(
        &self,
        org_id: i64,
        session_id: Option<uuid::Uuid>,
        server_prefix: &str,
    ) -> Result<crate::mcp_executor::McpServerInfo> {
        let request = proto::GetMcpServerByPrefixRequest {
            server_prefix: server_prefix.to_string(),
            org_id,
            session_id: session_id.map(uuid_to_proto),
        };

        let mut client = self.inner.lock().await;
        let response = client
            .get_mcp_server_by_prefix(request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_server = response.into_inner().server.ok_or_else(|| {
            AgentLoopError::store(format!("MCP server not found for prefix: {server_prefix}"))
        })?;

        let auth_mode = if proto_server.auth_mode.is_empty() && proto_server.api_key.is_some() {
            everruns_core::McpServerAuthMode::ApiKey
        } else {
            everruns_core::McpServerAuthMode::from(proto_server.auth_mode.as_str())
        };

        Ok(crate::mcp_executor::McpServerInfo {
            id: proto_uuid_to_uuid(proto_server.id.as_ref())?,
            name: proto_server.name,
            url: proto_server.url,
            api_key: proto_server.api_key,
            headers: proto_server.headers,
            auth_mode,
            protocol_mode: everruns_core::McpProtocolMode::from(
                proto_server.protocol_mode.as_str(),
            ),
            oauth_provider_id: proto_server.oauth_provider_id,
            secret_bindings: proto_server.secret_bindings.into_iter().fold(
                std::collections::HashMap::new(),
                |mut bindings, binding| {
                    bindings
                        .entry(binding.tool_name)
                        .or_insert_with(Vec::new)
                        .push(everruns_mcp::McpSecretBinding {
                            parameter_name: binding.parameter_name,
                            value: binding.value,
                            setup_url: binding.setup_url,
                            label: binding.label,
                        });
                    bindings
                },
            ),
        })
    }

    /// Claim due leased resources for cleanup.
    pub async fn claim_due_leased_resources(
        &self,
        limit: u32,
        stale_after_seconds: u32,
    ) -> Result<Vec<LeasedResource>> {
        let mut client = self.inner.lock().await;
        let response = client
            .claim_due_leased_resources(proto::ClaimDueLeasedResourcesRequest {
                limit,
                stale_after_seconds,
            })
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .resources
            .into_iter()
            .map(proto_leased_resource_to_schema)
            .collect()
    }

    /// Mark a leased resource cleanup as released using compare-and-set semantics.
    pub async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        let mut client = self.inner.lock().await;
        let response = client
            .mark_leased_resource_released(proto::MarkLeasedResourceReleasedRequest {
                resource_id: Some(uuid_to_proto(resource_id.uuid())),
                expected_cleanup_started_at: Some(
                    everruns_internal_protocol::datetime_to_proto_timestamp(
                        expected_cleanup_started_at,
                    ),
                ),
            })
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response.into_inner().updated)
    }

    /// Mark a leased resource cleanup as failed using compare-and-set semantics.
    pub async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
        retry_after_seconds: u32,
        error: &str,
    ) -> Result<bool> {
        let mut client = self.inner.lock().await;
        let response = client
            .mark_leased_resource_cleanup_failed(proto::MarkLeasedResourceCleanupFailedRequest {
                resource_id: Some(uuid_to_proto(resource_id.uuid())),
                expected_cleanup_started_at: Some(
                    everruns_internal_protocol::datetime_to_proto_timestamp(
                        expected_cleanup_started_at,
                    ),
                ),
                retry_after_seconds,
                error: error.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response.into_inner().updated)
    }

    /// List (session_id, task_id) pairs for tasks with stale heartbeats.
    /// Used by the session_task_reaper durable activity on gRPC workers.
    pub async fn list_orphaned_session_tasks(
        &self,
        stale_after_seconds: i64,
        limit: i64,
    ) -> Result<Vec<(everruns_provider::typed_id::SessionId, String)>> {
        let mut client = self.inner.lock().await;
        let response = client
            .list_orphaned_session_tasks(proto::ListOrphanedSessionTasksRequest {
                stale_after_seconds,
                limit,
            })
            .await
            .map_err(grpc_status_to_error)?;
        response
            .into_inner()
            .entries
            .into_iter()
            .map(|e| {
                let uuid = uuid::Uuid::parse_str(&e.session_id).map_err(|err| {
                    AgentLoopError::store(format!("Invalid session_id in orphan entry: {err}"))
                })?;
                Ok((
                    everruns_provider::typed_id::SessionId::from_uuid(uuid),
                    e.task_id,
                ))
            })
            .collect()
    }

    /// Prune a bounded batch of terminal session tasks older than the TTL,
    /// removing rows, messages, and artifacts server-side. Used by the
    /// retention pass of the session_task_reaper durable activity on gRPC
    /// workers (EVE-580).
    pub async fn prune_terminal_session_tasks(
        &self,
        ttl_seconds: i64,
        limit: i64,
    ) -> Result<usize> {
        let mut client = self.inner.lock().await;
        let response = client
            .prune_terminal_session_tasks(proto::PruneTerminalSessionTasksRequest {
                ttl_seconds,
                limit,
            })
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().pruned.max(0) as usize)
    }

    pub async fn invoke_scheduled_app_channel(
        &self,
        org_id: i64,
        app_id: &str,
        channel_id: &str,
    ) -> Result<serde_json::Value> {
        let mut client = self.inner.lock().await;
        let response = client
            .invoke_scheduled_app_channel(proto::InvokeScheduledAppChannelRequest {
                org_id,
                app_id: app_id.to_string(),
                channel_id: channel_id.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let response = response.into_inner();
        Ok(serde_json::json!({
            "session_id": response.session_id,
            "created_session": response.created_session,
        }))
    }

    pub async fn invoke_agent_trigger(
        &self,
        org_id: i64,
        agent_id: &str,
        trigger_id: &str,
    ) -> Result<serde_json::Value> {
        let mut client = self.inner.lock().await;
        let response = client
            .invoke_agent_trigger(proto::InvokeAgentTriggerRequest {
                org_id,
                agent_id: agent_id.to_string(),
                trigger_id: trigger_id.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let response = response.into_inner();
        Ok(serde_json::json!({
            "session_id": response.session_id,
            "created_session": response.created_session,
        }))
    }
}

// ============================================================================
// Consolidated adapter structs
// ============================================================================

/// Session-scoped gRPC adapter (no org_id needed).
///
/// Implements: MessageRetriever, SessionFileSystem, EventEmitter,
/// SessionStorageStore, UserConnectionResolver, LeasedResourceStore,
/// SessionSqlDbStore.
#[derive(Clone)]
pub struct GrpcAdapter {
    client: GrpcClient,
    proactive_compaction_attempts: Arc<everruns_core::ProactiveCompactionAttemptTracker>,
}

impl GrpcAdapter {
    pub fn new(client: GrpcClient) -> Self {
        Self {
            client,
            proactive_compaction_attempts: Arc::new(
                everruns_core::ProactiveCompactionAttemptTracker::default(),
            ),
        }
    }
}

/// Org-scoped gRPC adapter (carries org_id for authorization).
///
/// Implements: AgentStore, HarnessStore, SessionStore, ProviderStore,
/// ImageResolver, SessionMutator, SessionScheduleStore, PlatformStore.
#[derive(Clone)]
pub struct GrpcOrgAdapter {
    client: GrpcClient,
    org_id: i64,
    platform_session_id: Option<SessionId>,
    platform_user_id: Arc<OnceCell<Uuid>>,
}

impl GrpcOrgAdapter {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self {
            client,
            org_id,
            platform_session_id: None,
            platform_user_id: Arc::new(OnceCell::new()),
        }
    }

    pub fn new_for_platform_session(
        client: GrpcClient,
        org_id: i64,
        session_id: Option<SessionId>,
    ) -> Self {
        Self {
            client,
            org_id,
            platform_session_id: session_id,
            platform_user_id: Arc::new(OnceCell::new()),
        }
    }

    async fn platform_user_id(&self) -> Result<Uuid> {
        let session_id = self.platform_session_id.ok_or_else(|| {
            AgentLoopError::store("PlatformStore requires a platform session context")
        })?;

        let user_id = self
            .platform_user_id
            .get_or_try_init(|| async {
                let mut client = self.client.inner.lock().await;
                let response = client
                    .get_session(proto::GetSessionRequest {
                        session_id: Some(uuid_to_proto(session_id.uuid())),
                        org_id: self.org_id,
                    })
                    .await
                    .map_err(grpc_status_to_error)?;

                let session = response
                    .into_inner()
                    .session
                    .ok_or_else(|| grpc_missing_field("No session in response"))?;
                let user_id = session.resolved_owner_user_id.as_ref().ok_or_else(|| {
                    AgentLoopError::config(
                        "Platform tool authorization requires a user-owned session with a resolved owner"
                            .to_string(),
                    )
                })?;
                proto_uuid_to_uuid(Some(user_id))
            })
            .await?;

        Ok(*user_id)
    }

    async fn execute_platform_command_raw(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> Result<std::result::Result<serde_json::Value, proto::CommandError>> {
        let user_id = self.platform_user_id().await?;
        let mut client = self.client.inner.lock().await;
        let response = client
            .execute_command(proto::ExecuteCommandRequest {
                name: name.to_string(),
                api_version: COMMAND_API_VERSION_V1.to_string(),
                params_json: serde_json::to_vec(&params).map_err(|e| {
                    AgentLoopError::store(format!("JSON serialization failed: {}", e))
                })?,
                org_id: self.org_id,
                user_id: Some(user_id.to_string()),
                idempotency_key: None,
                metadata: Default::default(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();

        let result = response
            .result
            .ok_or_else(|| grpc_missing_field("No command result in response"))?;

        match result {
            proto::execute_command_response::Result::OkJson(ok_json) => {
                let value = serde_json::from_slice(&ok_json).map_err(|e| {
                    AgentLoopError::store(format!("Failed to decode command response: {}", e))
                })?;
                Ok(Ok(value))
            }
            proto::execute_command_response::Result::Error(error) => Ok(Err(error)),
        }
    }

    async fn invoke_platform_command_surface(
        &self,
        operation: proto::PlatformCommandSurfaceOperation,
        arguments: serde_json::Value,
    ) -> Result<String> {
        let session_id = self.platform_session_id.ok_or_else(|| {
            AgentLoopError::store("Platform command surface requires a platform session context")
        })?;
        let mut client = self.client.inner.lock().await;
        let response = client
            .invoke_platform_command_surface(proto::InvokePlatformCommandSurfaceRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                org_id: self.org_id,
                operation: operation as i32,
                arguments_json: serde_json::to_vec(&arguments).map_err(|error| {
                    AgentLoopError::store(format!("JSON serialization failed: {error}"))
                })?,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        match response
            .result
            .ok_or_else(|| grpc_missing_field("No platform command surface result"))?
        {
            proto::invoke_platform_command_surface_response::Result::Output(output) => Ok(output),
            proto::invoke_platform_command_surface_response::Result::Error(error) => {
                Err(AgentLoopError::tool(error))
            }
        }
    }

    async fn execute_platform_command<T>(&self, name: &str, params: serde_json::Value) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        match self.execute_platform_command_raw(name, params).await? {
            Ok(value) => serde_json::from_value(value).map_err(|e| {
                AgentLoopError::store(format!("Failed to parse command response: {}", e))
            }),
            Err(error) => Err(grpc_command_error_to_error(error)),
        }
    }

    async fn execute_platform_lookup<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        match self.execute_platform_command_raw(name, params).await? {
            Ok(value) => serde_json::from_value(value).map(Some).map_err(|e| {
                AgentLoopError::store(format!("Failed to parse command response: {}", e))
            }),
            Err(error) if error.kind == 3 => Ok(None),
            Err(error) => Err(grpc_command_error_to_error(error)),
        }
    }

    async fn latest_terminal_turn_status(&self, session_id: SessionId) -> Result<Option<String>> {
        const TURN_COMPLETED: &str = "turn.completed";
        const TURN_FAILED: &str = "turn.failed";
        const TURN_CANCELLED: &str = "turn.cancelled";
        const TURN_SEALED: &str = "turn.sealed";

        let response: serde_json::Value = self
            .execute_platform_command(
                "list_events",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "types": [TURN_COMPLETED, TURN_FAILED, TURN_CANCELLED, TURN_SEALED],
                    "limit": 1,
                    "order_desc": true,
                }),
            )
            .await?;

        let Some(event_type) = response
            .get("data")
            .and_then(|data| data.as_array())
            .and_then(|events| events.first())
            .and_then(|event| event.get("type"))
            .and_then(|event_type| event_type.as_str())
        else {
            return Ok(None);
        };

        let status = match event_type {
            TURN_COMPLETED => Some("completed"),
            TURN_FAILED => Some("failed"),
            TURN_CANCELLED => Some("cancelled"),
            // A sealed turn is terminal but distinct from a failure: surface it
            // as "sealed" so the parent agent can decide what to do next.
            TURN_SEALED => Some("sealed"),
            _ => None,
        };
        Ok(status.map(str::to_string))
    }
}

/// Budget checker that carries org_id and optional agent_id (captured at
/// construction) for gRPC calls.
pub struct GrpcBudgetChecker {
    client: GrpcClient,
    org_id: i64,
    agent_id: Option<String>,
}

/// Payment authority that forwards paid capability requests to the control plane.
pub struct GrpcPaymentAuthority {
    client: GrpcClient,
    org_id: i64,
    agent_id: Option<String>,
}

/// Session-creation authority backed by the control-plane permission resolver.
pub struct GrpcSessionCreationAuthority {
    client: GrpcClient,
    org_id: i64,
    session_id: SessionId,
}

impl GrpcBudgetChecker {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self {
            client,
            org_id,
            agent_id: None,
        }
    }

    pub fn with_agent_id(mut self, agent_id: Option<String>) -> Self {
        self.agent_id = agent_id;
        self
    }
}

impl GrpcPaymentAuthority {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self {
            client,
            org_id,
            agent_id: None,
        }
    }

    pub fn with_agent_id(mut self, agent_id: Option<String>) -> Self {
        self.agent_id = agent_id;
        self
    }
}

impl GrpcSessionCreationAuthority {
    pub fn new(client: GrpcClient, org_id: i64, session_id: SessionId) -> Self {
        Self {
            client,
            org_id,
            session_id,
        }
    }
}

// ============================================================================
// Helper functions for proto conversion
// ============================================================================

fn uuid_to_proto(id: Uuid) -> proto::Uuid {
    proto::Uuid {
        value: id.to_string(),
    }
}

fn proto_uuid_to_uuid(proto_uuid: Option<&proto::Uuid>) -> Result<Uuid> {
    let uuid_str = proto_uuid
        .map(|u| &u.value)
        .ok_or_else(|| grpc_missing_field("Missing UUID in response"))?;
    Uuid::parse_str(uuid_str).map_err(|e| AgentLoopError::store(format!("Invalid UUID: {}", e)))
}

fn proto_timestamp_to_datetime(ts: &proto::Timestamp) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(ts.seconds, ts.nanos as u32)
        .single()
        .unwrap_or_else(chrono::Utc::now)
}

/// Helper to convert optional proto timestamp to datetime (or now if missing).
/// Reduces the repeated `.as_ref().map().unwrap_or_else()` pattern.
fn proto_timestamp_or_now(ts: Option<&proto::Timestamp>) -> chrono::DateTime<chrono::Utc> {
    ts.map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now)
}

/// Helper to convert empty string to None.
/// Reduces the repeated `if s.is_empty() { None } else { Some(s) }` pattern.
fn non_empty_string(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

fn proto_stored_image_info_to_schema(
    proto_info: proto::StoredImageInfo,
) -> Result<StoredImageInfo> {
    Ok(StoredImageInfo {
        id: proto_uuid_to_uuid(proto_info.id.as_ref())?.into(),
        filename: proto_info.filename,
        content_type: proto_info.content_type,
        size_bytes: proto_info.size_bytes,
        metadata: proto_info
            .metadata
            .as_ref()
            .map(proto_struct_to_json)
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: proto_timestamp_or_now(proto_info.created_at.as_ref()),
    })
}

fn proto_stored_image_to_schema(proto_image: proto::StoredImage) -> Result<StoredImage> {
    let info = proto_image
        .info
        .ok_or_else(|| grpc_missing_field("No image info in response"))?;
    Ok(StoredImage {
        info: proto_stored_image_info_to_schema(info)?,
        data: proto_image.data,
    })
}

// ============================================================================
// MessageRetriever implementation
// ============================================================================

impl GrpcAdapter {
    /// Add a new message via gRPC
    ///
    /// Note: This is provided for API layer convenience.
    /// Messages are stored via gRPC call to control-plane.
    pub async fn add_message(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        let mut client = self.client.inner.lock().await;

        // Convert content to prost ListValue
        let content_json = serde_json::to_value(&input.content)
            .map_err(|e| AgentLoopError::store(format!("JSON serialization failed: {}", e)))?;
        let content = Some(json_to_proto_list(&content_json));

        // Convert controls to prost Struct
        let controls = input.controls.as_ref().map(|c| {
            let json = serde_json::to_value(c).unwrap_or_default();
            json_to_proto_struct(&json)
        });

        // Convert metadata to prost Struct
        let metadata = input.metadata.as_ref().map(|m| {
            let json = serde_json::to_value(m).unwrap_or_default();
            json_to_proto_struct(&json)
        });

        let request = proto::AddMessageRequest {
            session_id: Some(uuid_to_proto(session_id)),
            role: input.role.to_string(),
            content,
            controls,
            metadata,
            tags: input.tags,
        };

        let response = client
            .add_message(request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_msg = response
            .into_inner()
            .message
            .ok_or_else(|| grpc_missing_field("No message in response"))?;

        proto_message_to_message(proto_msg)
    }
}

#[async_trait]
impl MessageRetriever for GrpcAdapter {
    async fn get(&self, session_id: SessionId, message_id: MessageId) -> Result<Option<Message>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetMessageRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            message_id: Some(uuid_to_proto(message_id.uuid())),
        };

        let response = client
            .get_message(request)
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .message
            .map(proto_message_to_message)
            .transpose()
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<Message>> {
        let (messages, _, _) = self.load_with_message_limit(session_id, None, None).await?;
        Ok(messages)
    }

    async fn load_filtered(
        &self,
        query: everruns_core::message_filter::MessageQuery,
    ) -> Result<Vec<Message>> {
        Ok(self.load_filtered_history(query).await?.messages)
    }

    async fn load_filtered_history(
        &self,
        query: everruns_core::message_filter::MessageQuery,
    ) -> Result<MessageHistory> {
        let simple_window_query =
            query.filters.is_empty() && query.offset.is_none() && query.limit.is_some();
        let message_limit = if simple_window_query {
            query
                .limit
                .map(|limit| limit.clamp(0, i32::MAX as i64) as i32)
        } else {
            None
        };
        let (mut messages, total_count, source_sequence) = self
            .load_with_message_limit(query.session_id, message_limit, query.after_sequence)
            .await?;

        for filter in &query.filters {
            match filter {
                MessageFilter::TimeRange { from, to } => {
                    messages.retain(|m| {
                        let after_from = from.is_none_or(|t| m.created_at >= t);
                        let before_to = to.is_none_or(|t| m.created_at <= t);
                        after_from && before_to
                    });
                }
                MessageFilter::EventTypes(types) => {
                    messages.retain(|m| {
                        types.iter().any(|event_type| match event_type.as_str() {
                            "input.message" => m.role == MessageRole::User,
                            "output.message.completed" => m.role == MessageRole::Agent,
                            "tool.completed" => m.role == MessageRole::ToolResult,
                            _ => false,
                        })
                    });
                }
                MessageFilter::Search(q) => {
                    let q_lower = q.to_lowercase();
                    messages
                        .retain(|m| m.content_to_llm_string().to_lowercase().contains(&q_lower));
                }
                MessageFilter::Custom(predicate) => {
                    messages.retain(|m| predicate(m));
                }
                MessageFilter::ToolName(_)
                | MessageFilter::ExcludeIds(_)
                | MessageFilter::IncludeIds(_) => {
                    return Err(AgentLoopError::store(format!(
                        "gRPC MessageRetriever does not support filter: {filter:?}"
                    )));
                }
            }
        }

        if simple_window_query {
            query.apply_window_bounds(&mut messages);
            query.prepend_excluded_notice(&mut messages, total_count);
        } else {
            query.apply_windowing(&mut messages);
        }

        if query.has_injections() {
            query.apply_injections(&mut messages);
        }

        Ok(MessageHistory {
            messages,
            source_sequence,
        })
    }
}

impl GrpcAdapter {
    async fn load_with_message_limit(
        &self,
        session_id: SessionId,
        message_limit: Option<i32>,
        after_sequence: Option<i64>,
    ) -> Result<(Vec<Message>, usize, Option<i64>)> {
        let mut client = self.client.inner.lock().await;

        let request = proto::LoadMessagesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            message_limit,
            after_sequence,
        };

        let response = client
            .load_messages(request)
            .await
            .map_err(grpc_status_to_error)?;
        let response = response.into_inner();
        let total_count = if response.total_count > 0 {
            response.total_count as usize
        } else {
            response.messages.len()
        };

        let messages = response
            .messages
            .into_iter()
            .map(proto_message_to_message)
            .collect::<Result<Vec<_>>>()?;

        Ok((messages, total_count, response.source_sequence))
    }
}

#[async_trait]
impl everruns_core::CompactionCheckpointStore for GrpcAdapter {
    async fn get_latest(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<everruns_core::CompactionCheckpoint>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_compaction_checkpoint(proto::GetCompactionCheckpointRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                provider_type: provider_type.to_string(),
                model: model.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        response
            .checkpoint
            .map(|checkpoint| {
                Ok(everruns_core::CompactionCheckpoint {
                    id: proto_uuid_to_uuid(checkpoint.id.as_ref())?,
                    session_id: proto_uuid_to_uuid(checkpoint.session_id.as_ref())?.into(),
                    source_sequence: checkpoint.source_sequence,
                    provider_type: checkpoint.provider_type,
                    model: checkpoint.model,
                    format_version: checkpoint.format_version,
                    payload: serde_json::from_slice(&checkpoint.payload_json).map_err(|error| {
                        AgentLoopError::store(format!("invalid checkpoint payload: {error}"))
                    })?,
                })
            })
            .transpose()
    }

    async fn install(&self, checkpoint: everruns_core::CompactionCheckpoint) -> Result<bool> {
        let mut client = self.client.inner.lock().await;
        let payload_json = serde_json::to_vec(&checkpoint.payload)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        Ok(client
            .install_compaction_checkpoint(proto::InstallCompactionCheckpointRequest {
                checkpoint: Some(proto::CompactionCheckpoint {
                    id: Some(uuid_to_proto(checkpoint.id)),
                    session_id: Some(uuid_to_proto(checkpoint.session_id.uuid())),
                    source_sequence: checkpoint.source_sequence,
                    provider_type: checkpoint.provider_type,
                    model: checkpoint.model,
                    format_version: checkpoint.format_version,
                    payload_json,
                }),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner()
            .installed)
    }

    async fn get_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<everruns_core::ProactiveCompactionAttempt>> {
        Ok(self
            .proactive_compaction_attempts
            .get(session_id, provider_type, model)
            .await)
    }

    async fn record_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        attempt: everruns_core::ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.proactive_compaction_attempts
            .record(session_id, provider_type, model, attempt)
            .await;
        Ok(())
    }
}

fn proto_message_to_message(proto_msg: proto::Message) -> Result<Message> {
    let id = proto_uuid_to_uuid(proto_msg.id.as_ref())?;

    // Convert prost ListValue to Vec<ContentPart>
    let content_json = proto_msg
        .content
        .as_ref()
        .map(proto_list_to_json)
        .unwrap_or_else(|| serde_json::Value::Array(vec![]));
    let content: Vec<everruns_core::ContentPart> = serde_json::from_value(content_json)
        .map_err(|e| AgentLoopError::store(format!("Failed to parse message content: {}", e)))?;

    // Convert prost Struct to Controls
    let controls: Option<everruns_core::Controls> = proto_msg
        .controls
        .as_ref()
        .map(|s| serde_json::from_value(proto_struct_to_json(s)))
        .transpose()
        .map_err(|e| AgentLoopError::store(format!("Failed to parse message controls: {}", e)))?;

    // Convert prost Struct to metadata
    let metadata: Option<std::collections::HashMap<String, serde_json::Value>> = proto_msg
        .metadata
        .as_ref()
        .map(|s| serde_json::from_value(proto_struct_to_json(s)))
        .transpose()
        .map_err(|e| AgentLoopError::store(format!("Failed to parse message metadata: {}", e)))?;

    let role = match proto_msg.role.to_lowercase().as_str() {
        "system" => everruns_core::MessageRole::System,
        "user" => everruns_core::MessageRole::User,
        // Map both "assistant" (legacy) and "agent" to Agent role
        "assistant" | "agent" => everruns_core::MessageRole::Agent,
        "tool_result" => everruns_core::MessageRole::ToolResult,
        _ => everruns_core::MessageRole::User,
    };

    Ok(Message {
        id: id.into(),
        role,
        content,
        // Reasoning rides along inside `content` as ordered reasoning parts.
        phase: proto_msg
            .phase
            .as_deref()
            .and_then(everruns_provider::ExecutionPhase::from_provider_str),
        controls,
        metadata,
        external_actor: {
            use everruns_internal_protocol::proto_struct_to_json;
            proto_msg
                .external_actor
                .as_ref()
                .map(|s| serde_json::from_value(proto_struct_to_json(s)))
                .transpose()
                .unwrap_or(None)
        },
        created_at: proto_timestamp_or_now(proto_msg.created_at.as_ref()),
    })
}

// ============================================================================
// AgentStore implementation
// ============================================================================

#[async_trait]
impl AgentStore for GrpcOrgAdapter {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<AgentDefinition>> {
        // Loading seam (EVE-877): project the transported record into the
        // portable execution definition; archived/deleted agents fail here.
        self.fetch_agent_record(agent_id)
            .await?
            .map(|agent| agent.execution_definition())
            .transpose()
    }

    async fn get_agent_blocker(
        &self,
        agent_id: AgentId,
    ) -> Result<Option<everruns_core::DependencyBlocker>> {
        Ok(match self.fetch_agent_record(agent_id).await? {
            Some(agent) => agent.dependency_blocker(),
            None => Some(everruns_core::DependencyBlocker::AgentDeleted),
        })
    }
}

impl GrpcOrgAdapter {
    /// Fetch the stored agent record off the wire (platform-side transport;
    /// projected to `AgentDefinition` before it reaches host execution).
    pub(crate) async fn fetch_agent_record(&self, agent_id: AgentId) -> Result<Option<Agent>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetAgentRequest {
            agent_id: Some(uuid_to_proto(agent_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_agent(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().agent {
            Some(proto_agent) => {
                let agent = proto_agent_to_agent(proto_agent)?;
                Ok(Some(agent))
            }
            None => Ok(None),
        }
    }
}

fn proto_agent_to_agent(proto_agent: proto::Agent) -> Result<Agent> {
    let id = proto_uuid_to_uuid(proto_agent.id.as_ref())?;
    let default_model_id = proto_agent
        .default_model_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;
    let harness_id = proto_agent
        .harness_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("proto Agent missing harness_id"))?;

    let status = match proto_agent.status.to_lowercase().as_str() {
        "active" => everruns_platform::AgentStatus::Active,
        "archived" => everruns_platform::AgentStatus::Archived,
        "deleted" => everruns_platform::AgentStatus::Deleted,
        _ => everruns_platform::AgentStatus::Active,
    };

    let capabilities = if proto_agent.capabilities.is_empty() {
        proto_agent
            .capability_ids
            .into_iter()
            .map(everruns_capability::CapabilityRef::new)
            .collect()
    } else {
        proto_agent
            .capabilities
            .into_iter()
            .map(|config| {
                serde_json::from_str(&config).map_err(|error| {
                    AgentLoopError::store(format!(
                        "Invalid agent capability config in gRPC response: {error}"
                    ))
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?
    };

    Ok(Agent {
        public_id: everruns_provider::typed_id::AgentId::from_uuid(id),
        internal_id: id,
        name: proto_agent.name.clone(),
        display_name: proto_agent.display_name,
        description: non_empty_string(proto_agent.description),
        system_prompt: proto_agent.system_prompt,
        default_model_id: default_model_id.map(|u| u.into()),
        harness_id: harness_id.into(),
        default_version_id: None,
        forked_from_agent_id: None,
        forked_from_version_id: None,
        root_agent_id: None,
        tags: vec![],
        capabilities,
        mcp_servers: Default::default(),
        initial_files: vec![],
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: proto_agent.parallel_tool_calls,
        tools: vec![],
        status,
        created_at: proto_timestamp_or_now(proto_agent.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(proto_agent.updated_at.as_ref()),
        archived_at: None,
        deleted_at: None,
        usage: None, // Usage not tracked in worker context
    })
}

// ============================================================================
// HarnessStore implementation
// ============================================================================

#[async_trait]
impl HarnessStore for GrpcOrgAdapter {
    async fn get_harness(
        &self,
        harness_id: everruns_provider::typed_id::HarnessId,
    ) -> Result<Option<HarnessDefinition>> {
        // Loading seam (EVE-881): the server pre-merges the inheritance chain;
        // project the transported record into the portable execution
        // definition, failing archived/deleted records here.
        self.fetch_harness_record(harness_id)
            .await?
            .map(|harness| harness.execution_definition())
            .transpose()
    }

    async fn get_harness_blocker(
        &self,
        harness_id: everruns_provider::typed_id::HarnessId,
    ) -> Result<Option<everruns_core::DependencyBlocker>> {
        Ok(match self.fetch_harness_record(harness_id).await? {
            Some(harness) => harness.dependency_blocker(),
            None => Some(everruns_core::DependencyBlocker::HarnessDeleted),
        })
    }
}

impl GrpcOrgAdapter {
    /// Fetch the stored (pre-merged) harness record off the wire
    /// (platform-side transport; projected to `HarnessDefinition` before it
    /// reaches host execution).
    pub(crate) async fn fetch_harness_record(
        &self,
        harness_id: everruns_provider::typed_id::HarnessId,
    ) -> Result<Option<Harness>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetHarnessRequest {
            harness_id: Some(uuid_to_proto(harness_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_harness(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().harness {
            Some(proto_harness) => Ok(Some(proto_harness_to_harness(proto_harness)?)),
            None => Ok(None),
        }
    }
}

fn proto_harness_to_harness(proto_harness: proto::Harness) -> Result<Harness> {
    let id = proto_uuid_to_uuid(proto_harness.id.as_ref())?;
    let default_model_id = proto_harness
        .default_model_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;
    let parent_harness_id = proto_harness
        .parent_harness_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;

    let status = match proto_harness.status.to_lowercase().as_str() {
        "active" => HarnessStatus::Active,
        "archived" => HarnessStatus::Archived,
        "deleted" => HarnessStatus::Deleted,
        _ => HarnessStatus::Active,
    };

    let capabilities = if proto_harness.capabilities.is_empty() {
        proto_harness
            .capability_ids
            .into_iter()
            .map(everruns_capability::CapabilityRef::new)
            .collect()
    } else {
        proto_harness
            .capabilities
            .into_iter()
            .map(|config| {
                serde_json::from_str(&config).map_err(|error| {
                    AgentLoopError::store(format!(
                        "Invalid harness capability config in gRPC response: {error}"
                    ))
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?
    };

    Ok(Harness {
        id: id.into(),
        name: proto_harness.name,
        display_name: proto_harness.display_name,
        description: non_empty_string(proto_harness.description),
        // proto carries a plain string; empty/whitespace means no base prompt.
        system_prompt: Some(proto_harness.system_prompt).filter(|s| !s.trim().is_empty()),
        parent_harness_id: parent_harness_id.map(|u| u.into()),
        default_model_id: default_model_id.map(|u| u.into()),
        tags: proto_harness.tags,
        capabilities,
        mcp_servers: Default::default(),
        initial_files: vec![],
        network_access: None,
        // Re-resolved from durable config, not carried on the proto.
        parallel_tool_calls: None,
        embedder_metadata: Default::default(),
        is_built_in: proto_harness.is_built_in,
        status,
        created_at: proto_timestamp_or_now(proto_harness.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(proto_harness.updated_at.as_ref()),
        archived_at: None,
        deleted_at: None,
    })
}

// ============================================================================
// SessionStore implementation
// ============================================================================

#[async_trait]
impl SessionStore for GrpcOrgAdapter {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetSessionRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_session(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().session {
            Some(proto_session) => {
                let session = proto_session_to_session(proto_session)?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }
}

/// Project the proto session payload straight into the portable execution
/// view (EVE-882): the worker never materializes the stored Session record.
fn proto_session_to_session(proto_session: proto::Session) -> Result<ExecutionSession> {
    let id = proto_uuid_to_uuid(proto_session.id.as_ref())?;
    let agent_id = proto_session
        .agent_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;
    let harness_id = proto_session
        .harness_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?
        .unwrap_or(uuid::Uuid::nil());
    let model_id = proto_session
        .default_model_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;
    let parent_session_id = proto_session
        .parent_session_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)).map(SessionId::from_uuid))
        .transpose()?;
    let blueprint_config = proto_session
        .blueprint_config_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok());

    let status =
        everruns_core::SessionExecutionState::from(proto_session.status.to_lowercase().as_str());

    // Parse capabilities from proto if present
    let capabilities = proto_session
        .capabilities
        .iter()
        .filter_map(|c| serde_json::from_str::<everruns_capability::CapabilityRef>(c).ok())
        .collect();

    Ok(ExecutionSession {
        id: id.into(),
        workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid(id),
        organization_id: proto_session.organization_id,
        agent_id: agent_id.map(|u| u.into()),
        harness_id: harness_id.into(),
        title: non_empty_string(proto_session.title),
        goal: proto_session.goal.clone(),
        locale: non_empty_string(proto_session.locale),
        tags: proto_session.tags,
        model_id: model_id.map(|u| u.into()),
        capabilities,
        tools: vec![],
        mcp_servers: Default::default(),
        system_prompt: proto_session.system_prompt.clone(),
        initial_files: serde_json::from_str(&proto_session.initial_files_json).unwrap_or_default(),
        hints: proto_session.hints.as_ref().and_then(|s| {
            let json = everruns_internal_protocol::proto_struct_to_json(s);
            match serde_json::from_value(json) {
                Ok(hints) => Some(hints),
                Err(err) => {
                    tracing::warn!("Failed to deserialize session hints: {err}");
                    None
                }
            }
        }),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: proto_session.parallel_tool_calls,
        status,
        usage: None, // Usage not tracked in worker context
        parent_session_id,
        // Fork lineage is API-read-time metadata, not carried over gRPC.
        forked_from_session_id: None,
        blueprint_id: proto_session.blueprint_id,
        blueprint_config,
    })
}

// ============================================================================
// ProviderStore implementation
// ============================================================================

#[async_trait]
impl ProviderStore for GrpcOrgAdapter {
    async fn get_model_spec(&self, model_id: ModelId) -> Result<Option<ModelSpec>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetResolvedModelRequest {
            model_id: Some(uuid_to_proto(model_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_resolved_model(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().model {
            Some(proto_model) => {
                let model = proto_model_to_model_spec(proto_model)?;
                Ok(Some(model))
            }
            None => Ok(None),
        }
    }

    async fn get_default_model_spec(&self) -> Result<Option<ModelSpec>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetDefaultModelRequest {
            org_id: self.org_id,
        };

        let response = client
            .get_default_model(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().model {
            Some(proto_model) => {
                let model = proto_model_to_model_spec(proto_model)?;
                Ok(Some(model))
            }
            None => Ok(None),
        }
    }

    async fn get_provider_config(
        &self,
        provider: &everruns_provider::runtime_provider::ProviderKey,
    ) -> Result<Option<everruns_provider::driver_registry::ProviderConfig>> {
        self.client
            .get_provider_config(self.org_id, provider.as_str())
            .await
    }
}

#[async_trait]
impl ImageArtifactStore for GrpcOrgAdapter {
    async fn create_image(&self, input: CreateStoredImage) -> Result<StoredImageInfo> {
        self.client.create_image_artifact(self.org_id, input).await
    }

    async fn get_image(
        &self,
        image_id: everruns_provider::typed_id::ImageId,
    ) -> Result<Option<StoredImage>> {
        self.client.get_image_artifact(self.org_id, image_id).await
    }

    async fn get_image_info(
        &self,
        image_id: everruns_provider::typed_id::ImageId,
    ) -> Result<Option<StoredImageInfo>> {
        self.client
            .get_image_artifact_info(self.org_id, image_id)
            .await
    }
}

#[async_trait]
impl ProviderCredentialStore for GrpcOrgAdapter {
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        self.client
            .get_default_provider_credentials(self.org_id, provider_type)
            .await
    }
}

fn proto_model_to_model_spec(proto: proto::ResolvedModel) -> Result<ModelSpec> {
    // An empty provider_type is a corrupt/missing proto field; fail fast with
    // a clear store error rather than parsing it into an unusable External("").
    if proto.provider_type.trim().is_empty() {
        return Err(AgentLoopError::store(
            "empty provider_type in ResolvedModel proto",
        ));
    }
    if proto.provider_id.trim().is_empty() {
        return Err(AgentLoopError::store(
            "empty provider_id in ResolvedModel proto",
        ));
    }
    Ok(ModelSpec::on(proto.provider_id, proto.model))
}

// ============================================================================
// SessionFileSystem implementation
// ============================================================================

#[async_trait]
impl SessionFileSystem for GrpcAdapter {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionReadFileRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            path: path.to_string(),
        };

        let response = client
            .session_read_file(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().file {
            Some(proto_file) => {
                let file = proto_session_file_to_file(proto_file)?;
                Ok(Some(file))
            }
            None => Ok(None),
        }
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionWriteFileRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            path: path.to_string(),
            content: content.to_string(),
            encoding: encoding.to_string(),
        };

        let response = client
            .session_write_file(request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_file = response
            .into_inner()
            .file
            .ok_or_else(|| grpc_missing_field("No file in response"))?;

        proto_session_file_to_file(proto_file)
    }

    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionWriteFileIfContentMatchesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            path: path.to_string(),
            expected_content: expected_content.to_string(),
            expected_encoding: expected_encoding.to_string(),
            content: content.to_string(),
            encoding: encoding.to_string(),
        };

        let response = client
            .session_write_file_if_content_matches(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().file {
            Some(proto_file) => proto_session_file_to_file(proto_file).map(Some),
            None => Ok(None),
        }
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionDeleteFileRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            path: path.to_string(),
            recursive,
        };

        let response = client
            .session_delete_file(request)
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response.into_inner().deleted)
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionListDirectoryRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            path: path.to_string(),
        };

        let response = client
            .session_list_directory(request)
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .files
            .into_iter()
            .map(proto_file_info_to_file_info)
            .collect()
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionStatFileRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            path: path.to_string(),
        };

        let response = client
            .session_stat_file(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().stat {
            Some(proto_stat) => {
                let stat = proto_file_stat_to_stat(proto_stat)?;
                Ok(Some(stat))
            }
            None => Ok(None),
        }
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionGrepFilesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            pattern: pattern.to_string(),
            path_pattern: path_pattern.map(|s| s.to_string()),
            before_context: 0,
            after_context: 0,
            offset: 0,
            limit: u64::MAX,
            max_bytes: u64::MAX,
        };

        let response = client
            .session_grep_files(request)
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response
            .into_inner()
            .matches
            .into_iter()
            .map(|m| GrepMatch {
                path: m.path,
                line_number: m.line_number as usize,
                line: m.line,
            })
            .collect())
    }

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .session_grep_files(proto::SessionGrepFilesRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                pattern: pattern.to_string(),
                path_pattern: options.path_pattern.clone(),
                before_context: options.before_context as u64,
                after_context: options.after_context as u64,
                offset: options.offset as u64,
                limit: options.limit as u64,
                max_bytes: options.max_bytes as u64,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(GrepSearchResult {
            matches: response
                .matches
                .into_iter()
                .map(|item| GrepMatch {
                    path: item.path,
                    line_number: item.line_number as usize,
                    line: item.line,
                })
                .collect(),
            blocks: response
                .blocks
                .into_iter()
                .map(|block| GrepContextBlock {
                    path: block.path,
                    start_line: block.start_line as usize,
                    end_line: block.end_line as usize,
                    match_line_numbers: block
                        .match_line_numbers
                        .into_iter()
                        .map(|line| line as usize)
                        .collect(),
                    lines: block
                        .lines
                        .into_iter()
                        .map(|line| GrepContextLine {
                            line_number: line.line_number as usize,
                            line: line.line,
                            is_match: line.is_match,
                        })
                        .collect(),
                })
                .collect(),
            total_matches: response.total_matches as usize,
            returned_matches: response.returned_matches as usize,
            bytes_returned: response.bytes_returned as usize,
            bytes_total: response.bytes_total as usize,
            next_offset: response.next_offset.map(|offset| offset as usize),
            byte_truncated: response.byte_truncated,
        })
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionCreateDirectoryRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            path: path.to_string(),
        };

        let response = client
            .session_create_directory(request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_info = response
            .into_inner()
            .directory
            .ok_or_else(|| grpc_missing_field("No directory info in response"))?;

        proto_file_info_to_file_info(proto_info)
    }
}

fn proto_session_file_to_file(proto: proto::SessionFile) -> Result<SessionFile> {
    Ok(SessionFile {
        id: proto_uuid_to_uuid(proto.id.as_ref())?,
        session_id: proto_uuid_to_uuid(proto.session_id.as_ref())?,
        path: proto.path,
        name: proto.name,
        content: proto.content,
        encoding: proto.encoding,
        is_directory: proto.is_directory,
        is_readonly: proto.is_readonly,
        size_bytes: proto.size_bytes,
        created_at: proto_timestamp_or_now(proto.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(proto.updated_at.as_ref()),
    })
}

fn proto_file_info_to_file_info(proto: proto::FileInfo) -> Result<FileInfo> {
    Ok(FileInfo {
        id: proto_uuid_to_uuid(proto.id.as_ref())?,
        session_id: proto_uuid_to_uuid(proto.session_id.as_ref())?,
        path: proto.path,
        name: proto.name,
        is_directory: proto.is_directory,
        is_readonly: proto.is_readonly,
        size_bytes: proto.size_bytes,
        created_at: proto_timestamp_or_now(proto.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(proto.updated_at.as_ref()),
    })
}

fn proto_file_stat_to_stat(proto: proto::FileStat) -> Result<FileStat> {
    Ok(FileStat {
        path: proto.path,
        name: proto.name,
        is_directory: proto.is_directory,
        is_readonly: proto.is_readonly,
        size_bytes: proto.size_bytes,
        created_at: proto_timestamp_or_now(proto.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(proto.updated_at.as_ref()),
    })
}

// ============================================================================
// EventEmitter implementation
// ============================================================================

/// Whether the control plane has NATS-backed event delivery, meaning ephemeral
/// events skip PG. When true, the worker can fire-and-forget for deltas.
/// When false, all events go to PG — must use blocking gRPC for correct id/sequence.
fn server_supports_ephemeral_skip() -> bool {
    // Matches the server-side check: EventDelivery::Nats is active only when NATS_URL is set.
    // Worker and server share the same environment in typical deployments.
    std::env::var("NATS_URL").is_ok()
}

#[async_trait]
impl EventEmitter for GrpcAdapter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        // Fire-and-forget for ephemeral events only when the server has NATS
        // (ephemeral events skip PG). Without NATS, all events persist to PG
        // and we need the server-assigned id/sequence — use blocking path.
        if request.is_ephemeral() && server_supports_ephemeral_skip() {
            return self.emit_ephemeral(request).await;
        }

        // Blocking gRPC round-trip (needs server-assigned id + sequence)
        let mut client = self.client.inner.lock().await;

        let proto_event_request = core_event_request_to_proto(&request)?;

        let grpc_request = proto::EmitEventRequest {
            event: Some(proto_event_request),
        };

        let response = client
            .emit_event(grpc_request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_event = response
            .into_inner()
            .event
            .ok_or_else(|| grpc_missing_field("No event in response"))?;

        proto_event_to_core(proto_event)
    }
}

impl GrpcAdapter {
    /// Fire-and-forget emit for ephemeral events.
    /// Returns a synthetic Event immediately; the gRPC call runs in background.
    /// Only used when the server has NATS (ephemeral events skip PG).
    async fn emit_ephemeral(&self, request: EventRequest) -> Result<Event> {
        use everruns_provider::typed_id::EventId;

        // Convert to proto while we still have &request
        let proto_event_request = core_event_request_to_proto(&request)?;
        let grpc_request = proto::EmitEventRequest {
            event: Some(proto_event_request),
        };

        // Destructure to avoid clones
        let session_id = request.session_id;
        let event_type = request.event_type;
        let event = Event {
            id: EventId::new(),
            event_type: event_type.clone(),
            ts: request.ts,
            session_id,
            context: request.context,
            data: request.data,
            metadata: request.metadata,
            tags: request.tags,
            sequence: None,
        };

        // Fire gRPC call in background with backpressure: if the client mutex
        // is already held (previous emit still in flight), drop this event
        // rather than accumulating unbounded background tasks.
        let client = self.client.clone();
        tokio::spawn(async move {
            match client.inner.try_lock() {
                Ok(mut inner) => {
                    if let Err(e) = inner.emit_event(grpc_request).await {
                        tracing::debug!(
                            error = %e,
                            %session_id,
                            event_type,
                            "Background ephemeral event emit failed (non-fatal)"
                        );
                    }
                }
                Err(_) => {
                    tracing::debug!(
                        %session_id,
                        event_type,
                        "Dropping ephemeral event emit — client busy (backpressure)"
                    );
                }
            }
        });

        Ok(event)
    }
}

/// Convert everruns_core::EventRequest to proto::EventRequest
fn core_event_request_to_proto(request: &EventRequest) -> Result<proto::EventRequest> {
    // Use the typed event conversion from internal-protocol
    Ok(everruns_internal_protocol::schema_event_request_to_proto(
        request,
    ))
}

/// Convert proto::Event to everruns_core::Event
fn proto_event_to_core(proto_event: proto::Event) -> Result<Event> {
    everruns_internal_protocol::proto_event_to_schema(proto_event)
        .map_err(|e| AgentLoopError::store(format!("Failed to convert proto event: {}", e)))
}

// ============================================================================
// Batch context loader
// ============================================================================

/// Turn context loaded in one batched gRPC call
pub struct TurnContext {
    pub agent: Option<Agent>,
    pub session: ExecutionSession,
    pub messages: Vec<Message>,
    pub model: Option<ModelSpec>,
    /// MCP tool definitions pre-resolved from agent's MCP capabilities
    pub mcp_tool_definitions: Vec<everruns_provider::tool_types::ToolDefinition>,
}

/// Load turn context in one batched call (optimization)
///
/// This is more efficient than making separate calls for agent, session, messages.
pub async fn load_turn_context(
    client: &GrpcClient,
    org_id: i64,
    session_id: SessionId,
) -> Result<TurnContext> {
    let mut grpc_client = client.inner.lock().await;

    let request = proto::GetTurnContextRequest {
        session_id: Some(uuid_to_proto(session_id.uuid())),
        org_id,
        message_limit: None, // use server default
    };

    let response = grpc_client
        .get_turn_context(request)
        .await
        .map_err(grpc_status_to_error)?;

    let inner = response.into_inner();

    let agent = inner.agent.map(proto_agent_to_agent).transpose()?;
    let proto_session = inner
        .session
        .ok_or_else(|| grpc_missing_field("No session in turn context"))?;

    let session = proto_session_to_session(proto_session)?;

    let messages: Vec<Message> = inner
        .messages
        .into_iter()
        .map(proto_message_to_message)
        .collect::<Result<Vec<_>>>()?;

    let model = inner.model.map(proto_model_to_model_spec).transpose()?;

    // Convert MCP tool definitions from proto to core types
    let mcp_tool_definitions = inner
        .mcp_tool_definitions
        .into_iter()
        .map(proto_mcp_tool_def_to_tool_definition)
        .collect();

    Ok(TurnContext {
        agent,
        session,
        messages,
        model,
        mcp_tool_definitions,
    })
}

/// Convert proto McpToolDef to core ToolDefinition
fn proto_mcp_tool_def_to_tool_definition(
    proto_tool: proto::McpToolDef,
) -> everruns_provider::tool_types::ToolDefinition {
    use everruns_provider::tool_types::{
        BuiltinTool, DeferrablePolicy, ToolDefinition, ToolPolicy,
    };

    // Convert proto Struct to serde_json::Value
    let parameters = proto_tool
        .parameters
        .map(|s| proto_struct_to_json(&s))
        .unwrap_or_else(|| serde_json::json!({"type": "object"}));

    let mut hints = everruns_provider::tool_types::ToolHints::default().with_open_world(true);
    if !proto_tool.capability_id.is_empty() {
        hints = hints.with_capability_attribution(
            proto_tool.capability_id.clone(),
            (!proto_tool.capability_name.is_empty()).then_some(proto_tool.capability_name.clone()),
        );
    }

    ToolDefinition::Builtin(BuiltinTool {
        name: proto_tool.name,
        display_name: None,
        description: proto_tool.description,
        parameters,
        policy: ToolPolicy::Auto, // MCP tools are auto-executed
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints,
        full_parameters: None,
    })
}

// ============================================================================
// ImageResolver implementation
// ============================================================================

use everruns_core::{
    image_services::ImageResolver, session_services::KeyInfo, session_services::SecretInfo,
    session_services::SessionStorageStore,
};
use std::collections::HashMap;

impl GrpcOrgAdapter {
    /// Resolve multiple images in a batch (more efficient)
    ///
    /// Returns a HashMap mapping image_id to ResolvedImage for all found images.
    /// Missing images are silently skipped.
    /// When the server returns presigned URLs, images are fetched via HTTP.
    pub async fn resolve_images_batch(
        &self,
        image_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedImage>> {
        if image_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut client = self.client.inner.lock().await;

        let request = proto::ResolveImagesRequest {
            image_ids: image_ids.iter().map(|id| uuid_to_proto(*id)).collect(),
            org_id: self.org_id,
        };

        let response = client
            .resolve_images(request)
            .await
            .map_err(grpc_status_to_error)?;

        let mut result = HashMap::new();
        for (id_str, data) in response.into_inner().images {
            if let Ok(id) = Uuid::parse_str(&id_str) {
                let resolved = if !data.url.is_empty() {
                    fetch_image_from_url(&data.url, &data.media_type).await?
                } else {
                    ResolvedImage::new(data.base64, data.media_type)
                };
                result.insert(id, resolved);
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl ImageResolver for GrpcOrgAdapter {
    /// Resolve a single image by ID
    ///
    /// Returns the base64-encoded image data and media type, or None if not found.
    /// When the server returns a presigned URL, the image is fetched via HTTP.
    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::ResolveImageRequest {
            image_id: Some(uuid_to_proto(image_id)),
            org_id: self.org_id,
        };

        let response = client
            .resolve_image(request)
            .await
            .map_err(grpc_status_to_error)?;

        let inner = response.into_inner();

        if !inner.found {
            return Ok(None);
        }

        if !inner.url.is_empty() {
            let resolved = fetch_image_from_url(&inner.url, &inner.media_type).await?;
            Ok(Some(resolved))
        } else {
            Ok(Some(ResolvedImage::new(inner.base64, inner.media_type)))
        }
    }
}

// ============================================================================
// SessionStorageStore implementation
// ============================================================================

#[async_trait]
impl SessionStorageStore for GrpcAdapter {
    async fn set_value(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionStorageSetValueRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            key: key.to_string(),
            value: value.to_string(),
        };
        client
            .session_storage_set_value(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }

    async fn get_value(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        key: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionStorageGetValueRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            key: key.to_string(),
        };
        let response = client
            .session_storage_get_value(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().value)
    }

    async fn delete_value(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        key: &str,
    ) -> Result<bool> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionStorageDeleteValueRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            key: key.to_string(),
        };
        let response = client
            .session_storage_delete_value(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().deleted)
    }

    async fn list_keys(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
    ) -> Result<Vec<KeyInfo>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionStorageListKeysRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
        };
        let response = client
            .session_storage_list_keys(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response
            .into_inner()
            .keys
            .into_iter()
            .map(|k| KeyInfo {
                key: k.key,
                created_at: proto_timestamp_or_now(k.created_at.as_ref()),
                updated_at: proto_timestamp_or_now(k.updated_at.as_ref()),
            })
            .collect())
    }

    async fn set_secret(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        name: &str,
        value: &str,
    ) -> Result<()> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionStorageSetSecretRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
            value: value.to_string(),
        };
        client
            .session_storage_set_secret(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }

    async fn get_secret(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        name: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionStorageGetSecretRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
        };
        let response = client
            .session_storage_get_secret(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().value)
    }

    async fn delete_secret(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        name: &str,
    ) -> Result<bool> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionStorageDeleteSecretRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
        };
        let response = client
            .session_storage_delete_secret(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().deleted)
    }

    async fn list_secrets(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
    ) -> Result<Vec<SecretInfo>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionStorageListSecretsRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
        };
        let response = client
            .session_storage_list_secrets(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response
            .into_inner()
            .secrets
            .into_iter()
            .map(|s| SecretInfo {
                name: s.name,
                created_at: proto_timestamp_or_now(s.created_at.as_ref()),
                updated_at: proto_timestamp_or_now(s.updated_at.as_ref()),
            })
            .collect())
    }
}

// ============================================================================
// GrpcAdapter - UserConnectionResolver over gRPC
// ============================================================================

#[async_trait]
impl everruns_core::connection_services::UserConnectionResolver for GrpcAdapter {
    async fn get_connection_token(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::GetConnectionTokenRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            provider: provider.to_string(),
        };
        let response = client
            .get_connection_token(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().token)
    }

    async fn get_connection_user(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_connection_user(proto::GetConnectionUserRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                provider: provider.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().user_id {
            Some(user_id) => Ok(Some(proto_uuid_to_uuid(Some(&user_id))?)),
            None => Ok(None),
        }
    }

    async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<String>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_connection_token_for_user(proto::GetConnectionTokenForUserRequest {
                user_id: Some(uuid_to_proto(user_id)),
                provider: provider.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response.into_inner().token)
    }
}

// ============================================================================
// GrpcOrgAdapter - SessionMutator over gRPC
// ============================================================================

#[async_trait]
impl everruns_platform::SessionMutator for GrpcOrgAdapter {
    async fn update_session_title(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.client
            .set_session_title(self.org_id, session_id, &title)
            .await
    }
}

// ============================================================================
// GrpcAdapter - LeasedResourceStore over gRPC
// ============================================================================

#[async_trait]
impl LeasedResourceStore for GrpcAdapter {
    async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .upsert_leased_resource(proto::UpsertLeasedResourceRequest {
                session_id: Some(uuid_to_proto(input.session_id.uuid())),
                provider: input.provider,
                resource_type: input.resource_type,
                external_id: input.external_id,
                display_name: input.display_name,
                owner_user_id: input.owner_user_id.map(uuid_to_proto),
                lease_duration_seconds: input.lease_duration_seconds,
                metadata: Some(json_to_proto_struct(&input.metadata)),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let resource = response
            .into_inner()
            .resource
            .ok_or_else(|| grpc_missing_field("No leased resource in upsert response"))?;
        proto_leased_resource_to_schema(resource)
    }

    async fn release_resource(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) -> Result<Option<LeasedResource>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .release_leased_resource(proto::ReleaseLeasedResourceRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                provider: provider.to_string(),
                resource_type: resource_type.to_string(),
                external_id: external_id.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .resource
            .map(proto_leased_resource_to_schema)
            .transpose()
    }

    async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .list_session_leased_resources(proto::ListSessionLeasedResourcesRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
            })
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .resources
            .into_iter()
            .map(proto_leased_resource_to_schema)
            .collect()
    }
}

// ============================================================================
// GrpcAdapter - SessionResourceRegistry over gRPC
// ============================================================================

fn proto_session_resource_to_schema(
    e: proto::SessionResourceEntryProto,
) -> Result<everruns_core::SessionResourceEntry> {
    let session_id = proto_uuid_to_uuid(e.session_id.as_ref())?;
    Ok(everruns_core::SessionResourceEntry {
        resource_id: e.resource_id,
        session_id: SessionId::from_uuid(session_id),
        kind: e.kind,
        display_name: e.display_name,
        status: everruns_core::SessionResourceStatus::from(e.status.as_str()),
        metadata: e
            .metadata
            .as_ref()
            .map(proto_struct_to_json)
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: proto_timestamp_or_now(e.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(e.updated_at.as_ref()),
    })
}

#[async_trait]
impl everruns_core::session_services::SessionResourceRegistry for GrpcAdapter {
    async fn register(
        &self,
        entry: everruns_core::RegisterSessionResource,
    ) -> Result<everruns_core::SessionResourceEntry> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .register_session_resource(proto::RegisterSessionResourceRequest {
                session_id: Some(uuid_to_proto(entry.session_id.uuid())),
                resource_id: entry.resource_id,
                kind: entry.kind,
                display_name: entry.display_name,
                status: entry.status.to_string(),
                metadata: Some(json_to_proto_struct(&entry.metadata)),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let entry = response
            .into_inner()
            .entry
            .ok_or_else(|| grpc_missing_field("No entry in register response"))?;
        proto_session_resource_to_schema(entry)
    }

    async fn update_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: everruns_core::SessionResourceStatus,
    ) -> Result<Option<everruns_core::SessionResourceEntry>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .update_session_resource_status(proto::UpdateSessionResourceStatusRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                resource_id: resource_id.to_string(),
                status: status.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .entry
            .map(proto_session_resource_to_schema)
            .transpose()
    }

    async fn get(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<everruns_core::SessionResourceEntry>> {
        // Emulate via list — no dedicated GetSessionResource RPC yet.
        let entries = self.list(session_id, None).await?;
        Ok(entries.into_iter().find(|e| e.resource_id == resource_id))
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&everruns_core::SessionResourceFilter>,
    ) -> Result<Vec<everruns_core::SessionResourceEntry>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .list_session_resources(proto::ListSessionResourcesRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                kind: filter.and_then(|f| f.kind.clone()),
                status: filter.and_then(|f| f.status.map(|s| s.to_string())),
            })
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .entries
            .into_iter()
            .map(proto_session_resource_to_schema)
            .collect()
    }

    async fn deregister(&self, session_id: SessionId, resource_id: &str) -> Result<bool> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .deregister_session_resource(proto::DeregisterSessionResourceRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                resource_id: resource_id.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response.into_inner().removed)
    }
}

// ============================================================================
// GrpcOrgAdapter - SessionScheduleStore over gRPC
// ============================================================================

fn proto_leased_resource_to_schema(s: proto::LeasedResourceProto) -> Result<LeasedResource> {
    let id_uuid = proto_uuid_to_uuid(s.id.as_ref())?;
    let status = match s.status.as_str() {
        "active" => LeasedResourceStatus::Active,
        "cleaning" => LeasedResourceStatus::Cleaning,
        "released" => LeasedResourceStatus::Released,
        "cleanup_failed" => LeasedResourceStatus::CleanupFailed,
        other => {
            return Err(AgentLoopError::store(format!(
                "Unknown leased resource status from gRPC: {other}"
            )));
        }
    };

    Ok(LeasedResource {
        id: LeasedResourceId::from_uuid(id_uuid),
        session_id: s
            .session_id
            .as_ref()
            .map(|id| proto_uuid_to_uuid(Some(id)).map(SessionId::from_uuid))
            .transpose()?,
        provider: s.provider,
        resource_type: s.resource_type,
        external_id: s.external_id,
        display_name: s.display_name,
        status,
        owner_user_id: s
            .owner_user_id
            .as_ref()
            .map(|id| proto_uuid_to_uuid(Some(id)))
            .transpose()?,
        lease_duration_seconds: s.lease_duration_seconds,
        last_touched_at: proto_timestamp_or_now(s.last_touched_at.as_ref()),
        lease_expires_at: proto_timestamp_or_now(s.lease_expires_at.as_ref()),
        cleanup_started_at: s
            .cleanup_started_at
            .as_ref()
            .map(proto_timestamp_to_datetime),
        cleanup_completed_at: s
            .cleanup_completed_at
            .as_ref()
            .map(proto_timestamp_to_datetime),
        cleanup_attempts: s.cleanup_attempts,
        last_cleanup_error: s.last_cleanup_error,
        metadata: s
            .metadata
            .as_ref()
            .map(proto_struct_to_json)
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: proto_timestamp_or_now(s.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(s.updated_at.as_ref()),
    })
}

fn proto_schedule_to_schema(
    s: proto::SessionScheduleProto,
) -> Result<everruns_core::session_schedule::SessionSchedule> {
    use everruns_core::session_schedule::{ScheduleType, SessionSchedule};
    use everruns_provider::typed_id::{ScheduleId, SessionId};

    let id_uuid = proto_uuid_to_uuid(s.id.as_ref())?;
    let session_uuid = proto_uuid_to_uuid(s.session_id.as_ref())?;
    let owner_principal_uuid = proto_uuid_to_uuid(s.owner_principal_id.as_ref())?;
    let resolved_owner_user_id = s
        .resolved_owner_user_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;

    let schedule_type = match s.schedule_type.as_str() {
        "recurring" => ScheduleType::Recurring,
        _ => ScheduleType::OneShot,
    };

    Ok(SessionSchedule {
        id: ScheduleId::from_uuid(id_uuid),
        session_id: SessionId::from_uuid(session_uuid),
        owner_principal_id: everruns_provider::typed_id::PrincipalId::from_uuid(
            owner_principal_uuid,
        ),
        resolved_owner_user_id,
        owner: None,
        effective_owner: None,
        description: s.description,
        cron_expression: s.cron_expression,
        scheduled_at: s.scheduled_at.as_ref().map(proto_timestamp_to_datetime),
        timezone: s.timezone,
        enabled: s.enabled,
        schedule_type,
        next_trigger_at: s.next_trigger_at.as_ref().map(proto_timestamp_to_datetime),
        last_triggered_at: s
            .last_triggered_at
            .as_ref()
            .map(proto_timestamp_to_datetime),
        trigger_count: s.trigger_count as u32,
        created_at: proto_timestamp_or_now(s.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(s.updated_at.as_ref()),
    })
}

#[async_trait]
impl everruns_core::session_services::SessionScheduleStore for GrpcOrgAdapter {
    async fn create_schedule(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: String,
    ) -> Result<everruns_core::session_schedule::SessionSchedule> {
        let mut client = self.client.inner.lock().await;
        let request = proto::CreateSessionScheduleRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            description,
            cron_expression,
            scheduled_at: scheduled_at.map(everruns_internal_protocol::datetime_to_proto_timestamp),
            timezone,
            org_id: self.org_id,
        };
        let response = client
            .create_session_schedule(request)
            .await
            .map_err(grpc_status_to_error)?;
        let proto_schedule = response
            .into_inner()
            .schedule
            .ok_or_else(|| grpc_missing_field("No schedule in response"))?;
        proto_schedule_to_schema(proto_schedule)
    }

    async fn create_schedule_enforcing_limits(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: String,
    ) -> std::result::Result<
        everruns_core::session_schedule::SessionSchedule,
        everruns_core::session_schedule::ScheduleLimitError,
    > {
        let mut client = self.client.inner.lock().await;
        let request = proto::CreateSessionScheduleRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            description,
            cron_expression,
            scheduled_at: scheduled_at.map(everruns_internal_protocol::datetime_to_proto_timestamp),
            timezone,
            org_id: self.org_id,
        };
        let response = client
            .create_session_schedule(request)
            .await
            .map_err(|status| {
                if matches!(
                    status.code(),
                    tonic::Code::ResourceExhausted | tonic::Code::InvalidArgument
                ) {
                    everruns_core::session_schedule::ScheduleLimitError::Rejected(
                        status.message().to_string(),
                    )
                } else {
                    everruns_core::session_schedule::ScheduleLimitError::Store(
                        grpc_status_to_error(status),
                    )
                }
            })?;
        let proto_schedule = response.into_inner().schedule.ok_or_else(|| {
            everruns_core::session_schedule::ScheduleLimitError::Store(grpc_missing_field(
                "No schedule in response",
            ))
        })?;
        proto_schedule_to_schema(proto_schedule)
            .map_err(everruns_core::session_schedule::ScheduleLimitError::Store)
    }

    async fn cancel_schedule(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        schedule_id: everruns_provider::typed_id::ScheduleId,
    ) -> Result<everruns_core::session_schedule::SessionSchedule> {
        let mut client = self.client.inner.lock().await;
        let request = proto::CancelSessionScheduleRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            schedule_id: Some(uuid_to_proto(schedule_id.uuid())),
            org_id: self.org_id,
        };
        let response = client
            .cancel_session_schedule(request)
            .await
            .map_err(grpc_status_to_error)?;
        let proto_schedule = response
            .into_inner()
            .schedule
            .ok_or_else(|| grpc_missing_field("No schedule in response"))?;
        proto_schedule_to_schema(proto_schedule)
    }

    async fn list_schedules(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
    ) -> Result<Vec<everruns_core::session_schedule::SessionSchedule>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::ListSessionSchedulesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            org_id: self.org_id,
        };
        let response = client
            .list_session_schedules(request)
            .await
            .map_err(grpc_status_to_error)?;
        response
            .into_inner()
            .schedules
            .into_iter()
            .map(proto_schedule_to_schema)
            .collect()
    }

    async fn count_active_schedules(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
    ) -> Result<u32> {
        let mut client = self.client.inner.lock().await;
        let request = proto::CountActiveSessionSchedulesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            org_id: self.org_id,
        };
        let response = client
            .count_active_session_schedules(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().count)
    }

    async fn count_active_org_schedules(&self) -> Result<u32> {
        let mut client = self.client.inner.lock().await;
        let request = proto::CountActiveOrgSchedulesRequest {
            org_id: self.org_id,
        };
        let response = client
            .count_active_org_schedules(request)
            .await
            .map_err(grpc_status_to_error)?;
        Ok(response.into_inner().count)
    }
}

// ============================================================================
// GrpcOrgAdapter - PlatformStore implementation over gRPC
// ============================================================================

#[async_trait]
impl everruns_platform::PlatformStore for GrpcOrgAdapter {
    async fn platform_discover(&self, arguments: serde_json::Value) -> Result<String> {
        self.invoke_platform_command_surface(
            proto::PlatformCommandSurfaceOperation::Discover,
            arguments,
        )
        .await
    }

    async fn platform_query(&self, arguments: serde_json::Value) -> Result<String> {
        self.invoke_platform_command_surface(
            proto::PlatformCommandSurfaceOperation::Query,
            arguments,
        )
        .await
    }

    async fn platform_execute(&self, arguments: serde_json::Value) -> Result<String> {
        self.invoke_platform_command_surface(
            proto::PlatformCommandSurfaceOperation::Execute,
            arguments,
        )
        .await
    }

    // =========================================================================
    // Harness Operations
    // =========================================================================

    async fn list_harnesses(&self) -> Result<Vec<Harness>> {
        self.execute_platform_command("list_harnesses", serde_json::json!({}))
            .await
    }

    async fn get_harness(
        &self,
        id: everruns_provider::typed_id::HarnessId,
    ) -> Result<Option<Harness>> {
        self.execute_platform_lookup("get_harness", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn create_harness(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
        parent_harness_id: Option<everruns_provider::typed_id::HarnessId>,
        capabilities: &[String],
    ) -> Result<Harness> {
        self.execute_platform_command(
            "create_harness",
            serde_json::json!({
                "name": name,
                "display_name": display_name,
                "description": description,
                // Omit when absent so the harness contributes no base prompt.
                "system_prompt": system_prompt,
                "parent_harness_id": parent_harness_id.map(|id| id.to_string()),
                "capabilities": capability_refs_to_configs(capabilities),
            }),
        )
        .await
    }

    async fn update_harness(
        &self,
        id: everruns_provider::typed_id::HarnessId,
        name: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
        parent_harness_id: Option<Option<everruns_provider::typed_id::HarnessId>>,
    ) -> Result<Harness> {
        let mut params = serde_json::Map::from_iter([(
            "id".to_string(),
            serde_json::Value::String(id.to_string()),
        )]);
        if let Some(name) = name {
            params.insert(
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            );
        }
        if let Some(display_name) = display_name {
            params.insert(
                "display_name".to_string(),
                serde_json::Value::String(display_name.to_string()),
            );
        }
        if let Some(description) = description {
            params.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }
        if let Some(system_prompt) = system_prompt {
            params.insert(
                "system_prompt".to_string(),
                serde_json::Value::String(system_prompt.to_string()),
            );
        }
        match parent_harness_id {
            Some(Some(parent_id)) => {
                params.insert(
                    "parent_harness_id".to_string(),
                    serde_json::Value::String(parent_id.to_string()),
                );
            }
            Some(None) => {
                params.insert("parent_harness_id".to_string(), serde_json::Value::Null);
            }
            None => {}
        }

        self.execute_platform_command("update_harness", serde_json::Value::Object(params))
            .await
    }

    async fn delete_harness(&self, id: everruns_provider::typed_id::HarnessId) -> Result<()> {
        let _: serde_json::Value = self
            .execute_platform_command(
                "delete_harness",
                serde_json::json!({ "id": id.to_string() }),
            )
            .await?;
        Ok(())
    }

    async fn copy_harness(
        &self,
        id: everruns_provider::typed_id::HarnessId,
        new_name: Option<&str>,
    ) -> Result<Harness> {
        let harness: Harness = self
            .execute_platform_command("copy_harness", serde_json::json!({ "id": id.to_string() }))
            .await?;

        if let Some(new_name) = new_name {
            self.update_harness(harness.id, Some(new_name), None, None, None, None)
                .await
        } else {
            Ok(harness)
        }
    }

    // =========================================================================
    // Agent Operations
    // =========================================================================

    async fn list_agents(&self) -> Result<Vec<Agent>> {
        let mut agents = Vec::new();
        let mut offset = 0usize;
        let limit = 100usize;

        loop {
            let page: CommandPage<Agent> = self
                .execute_platform_command(
                    "list_agents",
                    serde_json::json!({ "offset": offset, "limit": limit }),
                )
                .await?;
            let page_len = page.data.len();
            let total = page.total as usize;
            agents.extend(page.data);

            if page_len == 0 || agents.len() >= total {
                break;
            }

            offset = agents.len();
        }

        Ok(agents)
    }

    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<Agent>> {
        self.execute_platform_lookup("get_agent", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn create_agent(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: &str,
        capabilities: &[String],
    ) -> Result<Agent> {
        self.execute_platform_command(
            "create_agent",
            serde_json::json!({
                "name": name,
                "display_name": display_name,
                "description": description,
                "system_prompt": system_prompt,
                "capabilities": capability_refs_to_configs(capabilities),
            }),
        )
        .await
    }

    async fn update_agent(
        &self,
        id: AgentId,
        name: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<Agent> {
        let mut params = serde_json::Map::from_iter([(
            "id".to_string(),
            serde_json::Value::String(id.to_string()),
        )]);
        if let Some(name) = name {
            params.insert(
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            );
        }
        if let Some(display_name) = display_name {
            params.insert(
                "display_name".to_string(),
                serde_json::Value::String(display_name.to_string()),
            );
        }
        if let Some(description) = description {
            params.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }
        if let Some(system_prompt) = system_prompt {
            params.insert(
                "system_prompt".to_string(),
                serde_json::Value::String(system_prompt.to_string()),
            );
        }

        self.execute_platform_command("update_agent", serde_json::Value::Object(params))
            .await
    }

    async fn delete_agent(&self, id: AgentId) -> Result<()> {
        let _: serde_json::Value = self
            .execute_platform_command("delete_agent", serde_json::json!({ "id": id.to_string() }))
            .await?;
        Ok(())
    }

    // =========================================================================
    // App Operations
    // =========================================================================

    async fn list_apps(
        &self,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<everruns_platform::App>> {
        let mut params = serde_json::Map::new();
        if let Some(search) = search {
            params.insert(
                "search".to_string(),
                serde_json::Value::String(search.to_string()),
            );
        }
        if include_archived {
            params.insert(
                "include_archived".to_string(),
                serde_json::Value::Bool(true),
            );
        }
        self.execute_platform_command("list_apps", serde_json::Value::Object(params))
            .await
    }

    async fn get_app(
        &self,
        id: everruns_provider::typed_id::AppId,
    ) -> Result<Option<everruns_platform::App>> {
        self.execute_platform_lookup("get_app", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn create_app(
        &self,
        name: &str,
        description: Option<&str>,
        harness_id: everruns_provider::typed_id::HarnessId,
        agent_id: Option<everruns_provider::typed_id::AgentId>,
        agent_identity_id: Option<everruns_provider::typed_id::AgentIdentityId>,
        channel_type: Option<everruns_platform::ChannelType>,
        channel_config: Option<&serde_json::Value>,
    ) -> Result<everruns_platform::App> {
        let mut params = serde_json::Map::from_iter([
            (
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            ),
            (
                "harness_id".to_string(),
                serde_json::Value::String(harness_id.to_string()),
            ),
        ]);
        if let Some(description) = description {
            params.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }
        if let Some(agent_id) = agent_id {
            params.insert(
                "agent_id".to_string(),
                serde_json::Value::String(agent_id.to_string()),
            );
        }
        if let Some(agent_identity_id) = agent_identity_id {
            params.insert(
                "agent_identity_id".to_string(),
                serde_json::Value::String(agent_identity_id.to_string()),
            );
        }
        if let Some(channel_type) = channel_type {
            params.insert(
                "channel_type".to_string(),
                serde_json::Value::String(channel_type.to_string()),
            );
        }
        if let Some(channel_config) = channel_config {
            params.insert("channel_config".to_string(), channel_config.clone());
        }
        self.execute_platform_command("create_app", serde_json::Value::Object(params))
            .await
    }

    async fn update_app(
        &self,
        id: everruns_provider::typed_id::AppId,
        name: Option<&str>,
        description: Option<&str>,
        harness_id: Option<everruns_provider::typed_id::HarnessId>,
        agent_id: Option<everruns_provider::typed_id::AgentId>,
        agent_identity_id: Option<Option<everruns_provider::typed_id::AgentIdentityId>>,
    ) -> Result<everruns_platform::App> {
        let mut params = serde_json::Map::from_iter([(
            "id".to_string(),
            serde_json::Value::String(id.to_string()),
        )]);
        if let Some(name) = name {
            params.insert(
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            );
        }
        if let Some(description) = description {
            params.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }
        if let Some(harness_id) = harness_id {
            params.insert(
                "harness_id".to_string(),
                serde_json::Value::String(harness_id.to_string()),
            );
        }
        if let Some(agent_id) = agent_id {
            params.insert(
                "agent_id".to_string(),
                serde_json::Value::String(agent_id.to_string()),
            );
        }
        match agent_identity_id {
            Some(Some(agent_identity_id)) => {
                params.insert(
                    "agent_identity_id".to_string(),
                    serde_json::Value::String(agent_identity_id.to_string()),
                );
            }
            Some(None) => {
                params.insert("agent_identity_id".to_string(), serde_json::Value::Null);
            }
            None => {}
        }
        self.execute_platform_command("update_app", serde_json::Value::Object(params))
            .await
    }

    async fn delete_app(&self, id: everruns_provider::typed_id::AppId) -> Result<()> {
        let _: serde_json::Value = self
            .execute_platform_command("delete_app", serde_json::json!({ "id": id.to_string() }))
            .await?;
        Ok(())
    }

    async fn destroy_app(&self, id: everruns_provider::typed_id::AppId) -> Result<()> {
        let _: serde_json::Value = self
            .execute_platform_command("destroy_app", serde_json::json!({ "id": id.to_string() }))
            .await?;
        Ok(())
    }

    async fn publish_app(
        &self,
        id: everruns_provider::typed_id::AppId,
    ) -> Result<everruns_platform::App> {
        self.execute_platform_command("publish_app", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn unpublish_app(
        &self,
        id: everruns_provider::typed_id::AppId,
    ) -> Result<everruns_platform::App> {
        self.execute_platform_command("unpublish_app", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn add_app_channel(
        &self,
        app_id: everruns_provider::typed_id::AppId,
        channel_type: everruns_platform::ChannelType,
        channel_config: Option<&serde_json::Value>,
        enabled: Option<bool>,
    ) -> Result<everruns_platform::AppChannel> {
        let mut params = serde_json::Map::from_iter([
            (
                "app_id".to_string(),
                serde_json::Value::String(app_id.to_string()),
            ),
            (
                "channel_type".to_string(),
                serde_json::Value::String(channel_type.to_string()),
            ),
        ]);
        if let Some(channel_config) = channel_config {
            params.insert("channel_config".to_string(), channel_config.clone());
        }
        if let Some(enabled) = enabled {
            params.insert("enabled".to_string(), serde_json::Value::Bool(enabled));
        }
        self.execute_platform_command("add_app_channel", serde_json::Value::Object(params))
            .await
    }

    async fn update_app_channel(
        &self,
        app_id: everruns_provider::typed_id::AppId,
        channel_id: everruns_provider::typed_id::AppChannelId,
        channel_type: Option<everruns_platform::ChannelType>,
        channel_config: Option<&serde_json::Value>,
        enabled: Option<bool>,
    ) -> Result<everruns_platform::AppChannel> {
        let mut params = serde_json::Map::from_iter([
            (
                "app_id".to_string(),
                serde_json::Value::String(app_id.to_string()),
            ),
            (
                "channel_id".to_string(),
                serde_json::Value::String(channel_id.to_string()),
            ),
        ]);
        if let Some(channel_type) = channel_type {
            params.insert(
                "channel_type".to_string(),
                serde_json::Value::String(channel_type.to_string()),
            );
        }
        if let Some(channel_config) = channel_config {
            params.insert("channel_config".to_string(), channel_config.clone());
        }
        if let Some(enabled) = enabled {
            params.insert("enabled".to_string(), serde_json::Value::Bool(enabled));
        }
        self.execute_platform_command("update_app_channel", serde_json::Value::Object(params))
            .await
    }

    async fn delete_app_channel(
        &self,
        app_id: everruns_provider::typed_id::AppId,
        channel_id: everruns_provider::typed_id::AppChannelId,
    ) -> Result<()> {
        let _: serde_json::Value = self
            .execute_platform_command(
                "delete_app_channel",
                serde_json::json!({
                    "app_id": app_id.to_string(),
                    "channel_id": channel_id.to_string(),
                }),
            )
            .await?;
        Ok(())
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn list_sessions(
        &self,
        limit: Option<usize>,
        agent_id: Option<AgentId>,
    ) -> Result<Vec<Session>> {
        let page: CommandPage<Session> = self
            .execute_platform_command(
                "list_sessions",
                serde_json::json!({
                    "limit": limit.unwrap_or(20),
                    "agent_id": agent_id.map(|id| id.to_string()),
                }),
            )
            .await?;
        Ok(page.data)
    }

    async fn create_session(
        &self,
        harness_id: everruns_provider::typed_id::HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        blueprint_id: Option<&str>,
        blueprint_config: Option<&serde_json::Value>,
        parent_session_id: Option<SessionId>,
    ) -> Result<Session> {
        self.execute_platform_command(
            "create_session",
            serde_json::json!({
                "harness_id": harness_id.to_string(),
                "agent_id": agent_id.map(|id| id.to_string()),
                "title": title,
                "locale": locale,
                "tags": ["managed"],
                "capabilities": [],
                "tools": [],
                "mcp_servers": {},
                "initial_files": [],
                "blueprint_id": blueprint_id,
                "blueprint_config": blueprint_config,
                "parent_session_id": parent_session_id.map(|id| id.to_string()),
            }),
        )
        .await
    }

    async fn create_session_with_options(
        &self,
        request: everruns_platform::PlatformCreateSessionRequest,
    ) -> Result<Session> {
        self.execute_platform_command(
            "create_session",
            serde_json::json!({
                "harness_id": request.harness_id.to_string(),
                "agent_id": request.agent_id.map(|id| id.to_string()),
                "title": request.title,
                "goal": request.goal,
                "locale": request.locale,
                "tags": ["managed"],
                "capabilities": [],
                "tools": [],
                "mcp_servers": {},
                "initial_files": [],
                "blueprint_id": request.blueprint_id,
                "blueprint_config": request.blueprint_config,
                "parent_session_id": request.parent_session_id.map(|id| id.to_string()),
                "forked_from_session_id": request.forked_from_session_id.map(|id| id.to_string()),
                "budget_root_session_id": request.budget_root_session_id.map(|id| id.to_string()),
                "seed": request.seed,
            }),
        )
        .await
    }

    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>> {
        self.execute_platform_lookup(
            "get_session",
            serde_json::json!({ "session_id": id.to_string() }),
        )
        .await
    }

    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<SessionParticipant> {
        self.execute_platform_command(
            "add_session_participant",
            serde_json::json!({
                "session_id": session_id.to_string(),
                "kind": "agent",
                "agent_id": agent_id.to_string(),
            }),
        )
        .await
    }

    async fn get_session_context_report(
        &self,
        id: SessionId,
    ) -> Result<everruns_core::SessionContextReport> {
        self.execute_platform_command(
            "get_session_context_report",
            serde_json::json!({ "session_id": id.to_string() }),
        )
        .await
    }

    async fn delete_session(&self, id: SessionId) -> Result<()> {
        let _: serde_json::Value = self
            .execute_platform_command(
                "delete_session",
                serde_json::json!({ "session_id": id.to_string() }),
            )
            .await?;
        Ok(())
    }

    // =========================================================================
    // Messaging
    // =========================================================================

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        let _: serde_json::Value = self
            .execute_platform_command(
                "create_message",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "message": {
                        "content": [{ "type": "text", "text": content }],
                    },
                }),
            )
            .await?;
        Ok(())
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<everruns_platform::PlatformMessage>> {
        let mut messages: Vec<Message> = self
            .execute_platform_command(
                "list_messages",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "limit": limit.unwrap_or(10),
                }),
            )
            .await?;
        messages.retain(|message| {
            matches!(
                message.role,
                everruns_core::MessageRole::User | everruns_core::MessageRole::Agent
            )
        });

        Ok(messages
            .into_iter()
            .filter_map(|message| {
                let content = message
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        everruns_core::ContentPart::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if content.is_empty() {
                    return None;
                }
                Some(everruns_platform::PlatformMessage {
                    role: match message.role {
                        everruns_core::MessageRole::User => "user".to_string(),
                        _ => "agent".to_string(),
                    },
                    content,
                    created_at: message.created_at,
                })
            })
            .collect())
    }

    // =========================================================================
    // Turn Management
    // =========================================================================

    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        timeout_secs: Option<u64>,
    ) -> Result<String> {
        let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(120));
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_millis(500);

        loop {
            let session = self
                .get_session_by_id(session_id)
                .await?
                .ok_or_else(|| AgentLoopError::store("Session not found"))?;

            match session.status {
                SessionStatus::Idle => {
                    if let Some(status) = self.latest_terminal_turn_status(session_id).await? {
                        return Ok(status);
                    }
                    // Session status flips to idle independently from terminal
                    // turn-event persistence. Keep polling until the event lands
                    // so callers can distinguish successful and failed idle turns.
                }
                SessionStatus::WaitingForToolResults => {
                    return Ok("waiting_for_tool_results".to_string());
                }
                SessionStatus::Paused => return Ok("paused".to_string()),
                SessionStatus::Started | SessionStatus::Active => {}
            }

            if start.elapsed() > timeout {
                return Ok(format!("timeout (last status: {:?})", session.status));
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    // =========================================================================
    // Capabilities
    // =========================================================================

    async fn list_capabilities(
        &self,
        search: Option<&str>,
    ) -> Result<Vec<everruns_core::CapabilityInfo>> {
        let mut params = serde_json::Map::new();
        params.insert("limit".to_string(), serde_json::json!(200));
        if let Some(search) = search {
            params.insert(
                "search".to_string(),
                serde_json::Value::String(search.to_string()),
            );
        }

        let page: CommandPage<everruns_core::CapabilityInfo> = self
            .execute_platform_command("list_capabilities", serde_json::Value::Object(params))
            .await?;
        Ok(page.data)
    }

    // =========================================================================
    // UI Links
    // =========================================================================

    fn base_url(&self) -> &str {
        // Cache the base_url as a leaked static to satisfy the &str lifetime
        // Called infrequently, value stable across runtime
        static BASE_URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        BASE_URL.get_or_init(|| {
            everruns_core::config::env_string_any(
                &["PUBLIC_APP_URL", "FRONTEND_URL", "APP_URL"],
                "http://localhost:9300",
            )
        })
    }
}

// ============================================================================
// GrpcAdapter - SessionSqlDbStore implementation over gRPC
// ============================================================================

use everruns_platform::session_sqldb::{
    ColumnSchema, DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult,
    SqlQueryResult, TableSchema,
};
/// Alias std::result::Result to avoid shadowing by everruns_provider::error::Result.
type SqlDbResult<T> = std::result::Result<T, SessionSqlDbError>;

/// Convert a gRPC status to a SessionSqlDbError, preserving error semantics.
fn grpc_status_to_sqldb_error(status: tonic::Status) -> SessionSqlDbError {
    let msg = status.message().to_string();
    match status.code() {
        tonic::Code::NotFound => SessionSqlDbError::DatabaseNotFound(msg),
        tonic::Code::AlreadyExists => SessionSqlDbError::DatabaseAlreadyExists(msg),
        tonic::Code::InvalidArgument => SessionSqlDbError::InvalidDatabaseName(msg),
        tonic::Code::ResourceExhausted => SessionSqlDbError::LimitExceeded(msg),
        tonic::Code::DeadlineExceeded => SessionSqlDbError::QueryTimeout(0),
        tonic::Code::PermissionDenied => SessionSqlDbError::AuthorizerBlocked(msg),
        tonic::Code::FailedPrecondition => SessionSqlDbError::QueryError(msg),
        _ => SessionSqlDbError::Internal(msg),
    }
}

fn proto_db_info_to_core(info: proto::SessionSqlDbDatabaseInfo) -> DatabaseInfo {
    DatabaseInfo {
        name: info.name,
        size_bytes: info.size_bytes,
        page_count: info.page_count,
        created_at: proto_timestamp_or_now(info.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(info.updated_at.as_ref()),
    }
}

#[async_trait]
impl SessionSqlDbStore for GrpcAdapter {
    async fn create_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> SqlDbResult<DatabaseInfo> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbCreateDatabaseRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
        };
        let response = client
            .session_sql_db_create_database(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        let db = response
            .into_inner()
            .database
            .ok_or_else(|| SessionSqlDbError::Internal("Missing database in response".into()))?;
        Ok(proto_db_info_to_core(db))
    }

    async fn list_databases(&self, session_id: SessionId) -> SqlDbResult<Vec<DatabaseInfo>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbListDatabasesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
        };
        let response = client
            .session_sql_db_list_databases(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        Ok(response
            .into_inner()
            .databases
            .into_iter()
            .map(proto_db_info_to_core)
            .collect())
    }

    async fn get_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> SqlDbResult<Option<DatabaseInfo>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbGetDatabaseRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
        };
        let response = client
            .session_sql_db_get_database(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        Ok(response.into_inner().database.map(proto_db_info_to_core))
    }

    async fn delete_database(&self, session_id: SessionId, name: &str) -> SqlDbResult<bool> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbDeleteDatabaseRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            name: name.to_string(),
        };
        let response = client
            .session_sql_db_delete_database(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        Ok(response.into_inner().deleted)
    }

    async fn sql_execute(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> SqlDbResult<SqlExecuteResult> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbExecuteRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            db_name: db_name.to_string(),
            sql: sql.to_string(),
        };
        let response = client
            .session_sql_db_execute(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        Ok(SqlExecuteResult {
            rows_affected: response.into_inner().rows_affected,
        })
    }

    async fn sql_query(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> SqlDbResult<SqlQueryResult> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbQueryRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            db_name: db_name.to_string(),
            sql: sql.to_string(),
        };
        let response = client
            .session_sql_db_query(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        let inner = response.into_inner();
        let rows: Vec<Vec<serde_json::Value>> = inner
            .rows
            .into_iter()
            .map(|list_value| {
                list_value
                    .values
                    .into_iter()
                    .map(proto_value_to_json)
                    .collect()
            })
            .collect();
        Ok(SqlQueryResult {
            columns: inner.columns,
            rows,
            row_count: inner.row_count as usize,
            truncated: inner.truncated,
        })
    }

    async fn sql_schema(
        &self,
        session_id: SessionId,
        db_name: &str,
        table: Option<&str>,
    ) -> SqlDbResult<Vec<TableSchema>> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbSchemaRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            db_name: db_name.to_string(),
            table: table.map(|t| t.to_string()),
        };
        let response = client
            .session_sql_db_schema(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        Ok(response
            .into_inner()
            .tables
            .into_iter()
            .map(|t| TableSchema {
                name: t.name,
                columns: t
                    .columns
                    .into_iter()
                    .map(|c| ColumnSchema {
                        name: c.name,
                        column_type: c.column_type,
                        notnull: c.notnull,
                        pk: c.pk,
                        default_value: c.default_value,
                    })
                    .collect(),
                row_count: t.row_count,
            })
            .collect())
    }
}

/// Convert a proto Value to serde_json::Value for SQL query results.
fn proto_value_to_json(value: prost_types::Value) -> serde_json::Value {
    match value.kind {
        Some(prost_types::value::Kind::NullValue(_)) => serde_json::Value::Null,
        Some(prost_types::value::Kind::NumberValue(n)) => serde_json::Value::Number(
            serde_json::Number::from_f64(n).unwrap_or_else(|| serde_json::Number::from(0)),
        ),
        Some(prost_types::value::Kind::StringValue(s)) => serde_json::Value::String(s),
        Some(prost_types::value::Kind::BoolValue(b)) => serde_json::Value::Bool(b),
        Some(prost_types::value::Kind::ListValue(list)) => {
            serde_json::Value::Array(list.values.into_iter().map(proto_value_to_json).collect())
        }
        Some(prost_types::value::Kind::StructValue(s)) => {
            let map: serde_json::Map<String, serde_json::Value> = s
                .fields
                .into_iter()
                .map(|(k, v)| (k, proto_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        None => serde_json::Value::Null,
    }
}

// ============================================================================
// OutboundToolRateLimiter — gate tool execution via control-plane limiter
// ============================================================================

pub struct GrpcOutboundToolRateLimiter {
    client: GrpcClient,
}

impl GrpcOutboundToolRateLimiter {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl everruns_core::tool_execution::OutboundToolRateLimiter for GrpcOutboundToolRateLimiter {
    async fn check_org(&self, org_id: &everruns_provider::typed_id::OrgId) -> bool {
        let mut client = self.client.inner.lock().await;
        let request = proto::CheckOutboundToolRateLimitRequest {
            org_key: org_id.to_string(),
        };

        match client.check_outbound_tool_rate_limit(request).await {
            Ok(response) => response.into_inner().allowed,
            Err(error) => {
                tracing::error!(
                    %error,
                    org_id = %org_id,
                    "gRPC outbound tool rate-limit check failed; denying tool call"
                );
                false
            }
        }
    }
}

// ============================================================================
// BudgetChecker — check budget status from check_budget tool
// ============================================================================

#[async_trait]
impl everruns_core::tool_execution::BudgetChecker for GrpcBudgetChecker {
    async fn check_budgets(
        &self,
        session_id: &str,
    ) -> everruns_provider::error::Result<everruns_core::budget::BudgetToolResponse> {
        let mut client = self.client.inner.lock().await;
        let request = proto::CheckBudgetsForSessionRequest {
            org_id: self.org_id,
            session_id: session_id.to_string(),
            agent_id: self.agent_id.clone(),
        };
        let response = client
            .check_budgets_for_session(request)
            .await
            .map_err(grpc_status_to_error)?;
        let resp = response.into_inner();
        Ok(everruns_core::budget::BudgetToolResponse {
            status: resp.status,
            budgets: resp
                .budgets
                .into_iter()
                .map(|b| everruns_core::budget::BudgetSummary {
                    currency: b.currency,
                    limit: b.limit,
                    balance: b.balance,
                    soft_limit: b.soft_limit,
                    percent_remaining: b.percent_remaining,
                    status: b.status,
                })
                .collect(),
            hint: resp.hint,
        })
    }
}

#[async_trait]
impl everruns_core::tool_execution::PaymentAuthority for GrpcPaymentAuthority {
    async fn execute_machine_payment(
        &self,
        session_id: SessionId,
        request: everruns_core::payment::MachinePaymentRequest,
    ) -> everruns_provider::error::Result<everruns_core::payment::MachinePaymentResponse> {
        let mut client = self.client.inner.lock().await;
        let proto_request = proto::ExecuteMachinePaymentRequest {
            org_id: self.org_id,
            session_id: session_id.to_string(),
            agent_id: self.agent_id.clone(),
            capability: request.capability,
            operation: request.operation,
            method: request.method.to_string(),
            url: request.url,
            body: request
                .body
                .as_ref()
                .map(everruns_internal_protocol::json_to_proto_value),
            max_amount_usd: request.max_amount_usd,
            rail_preference: request
                .rail_preference
                .iter()
                .map(ToString::to_string)
                .collect(),
            metadata: Some(everruns_internal_protocol::json_to_proto_value(
                &request.metadata,
            )),
        };
        let response = client
            .execute_machine_payment(proto_request)
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();

        let attempt_id = response
            .attempt_id
            .as_deref()
            .map(everruns_provider::typed_id::PaymentAttemptId::parse)
            .transpose()
            .map_err(|error| {
                AgentLoopError::store(format!("Invalid payment attempt id: {error}"))
            })?;
        let rail = response
            .rail
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(AgentLoopError::config)?;
        let body = response
            .response
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json)
            .unwrap_or(serde_json::Value::Null);
        let receipt = response
            .receipt
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json)
            .unwrap_or_else(|| serde_json::json!({}));

        Ok(everruns_core::payment::MachinePaymentResponse {
            attempt_id,
            amount_usd: response.amount_usd,
            rail,
            response: body,
            receipt,
        })
    }
}

#[async_trait]
impl everruns_core::delegation_services::SessionCreationAuthority for GrpcSessionCreationAuthority {
    async fn authorize_session_creation(
        &self,
        session_id: SessionId,
    ) -> everruns_provider::error::Result<SessionId> {
        if session_id != self.session_id {
            return Err(AgentLoopError::tool(
                "session-creation authority is scoped to the current session",
            ));
        }
        let mut client = self.client.inner.lock().await;
        let response = client
            .authorize_session_creation(proto::AuthorizeSessionCreationRequest {
                org_id: self.org_id,
                session_id: session_id.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        SessionId::parse(&response.budget_root_session_id).map_err(|error| {
            AgentLoopError::store(format!("Invalid budget root session id: {error}"))
        })
    }
}

// ============================================================================
// GrpcAdapter - SessionTaskRegistry over gRPC
// ============================================================================
//
// Task and message payloads travel as native protobuf messages (EVE-642), so
// there is no intermediate JSON encode/decode into byte fields. Lifecycle
// invariants stay server-side in DbSessionTaskRegistry and the record shape is
// defined once in everruns-core; the proto↔core conversions live in
// everruns-internal-protocol.

fn decode_task(proto: proto::SessionTaskProto) -> Result<everruns_core::SessionTask> {
    everruns_internal_protocol::proto_to_session_task(proto)
        .map_err(|e| AgentLoopError::store(format!("Invalid session task payload: {e}")))
}

fn decode_task_message(proto: proto::TaskMessageProto) -> Result<everruns_core::TaskMessage> {
    everruns_internal_protocol::proto_to_task_message(proto)
        .map_err(|e| AgentLoopError::store(format!("Invalid task message payload: {e}")))
}

#[async_trait]
impl everruns_core::session_task::SessionTaskRegistry for GrpcAdapter {
    async fn create(
        &self,
        input: everruns_core::CreateSessionTask,
    ) -> Result<everruns_core::SessionTask> {
        let create = everruns_internal_protocol::create_session_task_to_proto(&input);
        let mut client = self.client.inner.lock().await;
        let response = client
            .create_session_task(proto::CreateSessionTaskRequest {
                create: Some(create),
            })
            .await
            .map_err(grpc_status_to_error)?;
        let task = response
            .into_inner()
            .task
            .ok_or_else(|| AgentLoopError::store("Missing task in create response"))?;
        decode_task(task)
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: everruns_core::SessionTaskUpdate,
    ) -> Result<Option<everruns_core::SessionTask>> {
        let update = everruns_internal_protocol::session_task_update_to_proto(&update);
        let mut client = self.client.inner.lock().await;
        let response = client
            .update_session_task(proto::UpdateSessionTaskRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                task_id: task_id.to_string(),
                update: Some(update),
            })
            .await
            .map_err(grpc_status_to_error)?;
        response.into_inner().task.map(decode_task).transpose()
    }

    async fn get(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<everruns_core::SessionTask>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_session_task(proto::GetSessionTaskRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                task_id: task_id.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;
        response.into_inner().task.map(decode_task).transpose()
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&everruns_core::SessionTaskFilter>,
    ) -> Result<Vec<everruns_core::SessionTask>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .list_session_tasks(proto::ListSessionTasksRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                kind: filter.and_then(|f| f.kind.clone()),
                state: filter.and_then(|f| f.state.map(|s| s.to_string())),
            })
            .await
            .map_err(grpc_status_to_error)?;
        response
            .into_inner()
            .tasks
            .into_iter()
            .map(decode_task)
            .collect()
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<everruns_core::SessionTask>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .request_cancel_session_task(proto::RequestCancelSessionTaskRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                task_id: task_id.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;
        response.into_inner().task.map(decode_task).transpose()
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: everruns_core::NewTaskMessage,
    ) -> Result<everruns_core::TaskMessage> {
        let message = everruns_internal_protocol::new_task_message_to_proto(&message);
        let mut client = self.client.inner.lock().await;
        let response = client
            .record_session_task_message(proto::RecordSessionTaskMessageRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                task_id: task_id.to_string(),
                message: Some(message),
            })
            .await
            .map_err(grpc_status_to_error)?;
        let message = response
            .into_inner()
            .message
            .ok_or_else(|| AgentLoopError::store("Missing message in record response"))?;
        decode_task_message(message)
    }

    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        _after_id: Option<&str>,
    ) -> Result<Vec<everruns_core::TaskMessage>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .list_session_task_messages(proto::ListSessionTaskMessagesRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                task_id: task_id.to_string(),
                limit,
            })
            .await
            .map_err(grpc_status_to_error)?;
        response
            .into_inner()
            .messages
            .into_iter()
            .map(decode_task_message)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn worker_parses_the_neutral_capability_reference_shape() {
        // EVE-873: worker resolution consumes the same `{"ref", "config"}`
        // representation the Framework serializes and the control plane
        // persists — no worker-side semantic model.
        let framework_ref = everruns_capability::CapabilityRef::new("web_fetch")
            .config(serde_json::json!({"enable_file_download": true}));
        let wire = serde_json::to_string(&framework_ref).unwrap();

        let parsed = serde_json::from_str::<everruns_capability::CapabilityRef>(&wire).unwrap();
        assert_eq!(parsed, framework_ref);
        assert_eq!(parsed.capability_id(), "web_fetch");

        // Legacy rows without a config payload load as `{}`.
        let bare =
            serde_json::from_str::<everruns_capability::CapabilityRef>(r#"{"ref":"current_time"}"#)
                .unwrap();
        assert_eq!(bare.config_value(), &serde_json::json!({}));
    }

    #[test]
    fn resolved_model_proto_conversion_is_credential_free() {
        let resolved = proto_model_to_model_spec(proto::ResolvedModel {
            model: "custom-model".into(),
            provider_id: "provider-123".into(),
            provider_type: "custom-protocol".into(),
        })
        .unwrap();

        assert_eq!(resolved.model, "custom-model");
        assert_eq!(resolved.provider.as_str(), "provider-123");
    }

    #[test]
    fn test_proto_harness_to_harness_preserves_metadata() {
        let harness_id = Uuid::new_v4();
        let parent_id = Uuid::new_v4();
        let proto = proto::Harness {
            id: Some(uuid_to_proto(harness_id)),
            name: "platform-chat".into(),
            description: "Built-in chat harness".into(),
            system_prompt: "prompt".into(),
            default_model_id: None,
            status: "active".into(),
            created_at: None,
            updated_at: None,
            capability_ids: vec!["platform_management".into()],
            tags: vec!["chat".into(), "built-in".into()],
            parent_harness_id: Some(uuid_to_proto(parent_id)),
            is_built_in: true,
            display_name: Some("Platform Chat".into()),
            capabilities: vec![
                serde_json::json!({
                    "ref": "plugin:plugin_019fda530ed27b4291c67d9f786961d9",
                    "config": {"name": "resend", "description": "Send email"}
                })
                .to_string(),
            ],
        };

        let harness = proto_harness_to_harness(proto).expect("proto harness should convert");

        assert_eq!(harness.id.uuid(), harness_id);
        assert_eq!(
            harness.parent_harness_id.map(|id| id.uuid()),
            Some(parent_id)
        );
        assert_eq!(
            harness.tags,
            vec!["chat".to_string(), "built-in".to_string()]
        );
        assert_eq!(harness.capabilities[0].config_value()["name"], "resend");
        assert!(harness.is_built_in);
    }

    #[test]
    fn test_proto_value_to_json_null() {
        let val = prost_types::Value {
            kind: Some(prost_types::value::Kind::NullValue(0)),
        };
        assert_eq!(proto_value_to_json(val), serde_json::Value::Null);
    }

    #[test]
    fn test_proto_value_to_json_none_kind() {
        let val = prost_types::Value { kind: None };
        assert_eq!(proto_value_to_json(val), serde_json::Value::Null);
    }

    #[test]
    fn test_proto_value_to_json_string() {
        let val = prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue("hello".into())),
        };
        assert_eq!(
            proto_value_to_json(val),
            serde_json::Value::String("hello".into())
        );
    }

    #[test]
    fn test_proto_value_to_json_number() {
        let val = prost_types::Value {
            kind: Some(prost_types::value::Kind::NumberValue(42.5)),
        };
        assert_eq!(proto_value_to_json(val), serde_json::json!(42.5));
    }

    #[test]
    fn test_proto_value_to_json_bool() {
        let val = prost_types::Value {
            kind: Some(prost_types::value::Kind::BoolValue(true)),
        };
        assert_eq!(proto_value_to_json(val), serde_json::Value::Bool(true));
    }

    #[test]
    fn test_proto_value_to_json_list() {
        let val = prost_types::Value {
            kind: Some(prost_types::value::Kind::ListValue(
                prost_types::ListValue {
                    values: vec![
                        prost_types::Value {
                            kind: Some(prost_types::value::Kind::NumberValue(1.0)),
                        },
                        prost_types::Value {
                            kind: Some(prost_types::value::Kind::StringValue("two".into())),
                        },
                    ],
                },
            )),
        };
        assert_eq!(proto_value_to_json(val), serde_json::json!([1.0, "two"]));
    }

    #[test]
    fn test_proto_value_to_json_struct() {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "key".to_string(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::StringValue("value".into())),
            },
        );
        fields.insert(
            "num".to_string(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::NumberValue(42.0)),
            },
        );
        let val = prost_types::Value {
            kind: Some(prost_types::value::Kind::StructValue(prost_types::Struct {
                fields: fields.into_iter().collect(),
            })),
        };
        let json = proto_value_to_json(val);
        assert_eq!(json["key"], "value");
        assert_eq!(json["num"], 42.0);
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_unmapped_code_falls_to_internal() {
        let status = tonic::Status::unimplemented("not implemented");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::Internal(_)));
    }

    #[test]
    fn test_proto_db_info_to_core() {
        let proto = proto::SessionSqlDbDatabaseInfo {
            name: "test".into(),
            size_bytes: 4096,
            page_count: 1,
            created_at: Some(proto::Timestamp {
                seconds: 1700000000,
                nanos: 0,
            }),
            updated_at: Some(proto::Timestamp {
                seconds: 1700000000,
                nanos: 0,
            }),
        };
        let info = proto_db_info_to_core(proto);
        assert_eq!(info.name, "test");
        assert_eq!(info.size_bytes, 4096);
        assert_eq!(info.page_count, 1);
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_not_found() {
        let status = tonic::Status::not_found("db not found");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::DatabaseNotFound(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_already_exists() {
        let status = tonic::Status::already_exists("db exists");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::DatabaseAlreadyExists(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_invalid_argument() {
        let status = tonic::Status::invalid_argument("bad name");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::InvalidDatabaseName(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_resource_exhausted() {
        let status = tonic::Status::resource_exhausted("too many");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::LimitExceeded(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_deadline_exceeded() {
        let status = tonic::Status::deadline_exceeded("timeout");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::QueryTimeout(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_permission_denied() {
        let status = tonic::Status::permission_denied("blocked");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::AuthorizerBlocked(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_failed_precondition() {
        let status = tonic::Status::failed_precondition("syntax error");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::QueryError(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_internal() {
        let status = tonic::Status::internal("unexpected");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::Internal(_)));
    }

    #[test]
    fn test_grpc_status_to_error_not_found() {
        let status = tonic::Status::not_found("Session not found");
        let err = grpc_status_to_error(status);
        assert!(matches!(err, AgentLoopError::MessageStore(_)));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_grpc_status_to_error_invalid_argument() {
        let status = tonic::Status::invalid_argument("bad field");
        let err = grpc_status_to_error(status);
        assert!(matches!(err, AgentLoopError::Configuration(_)));
    }

    #[test]
    fn test_grpc_status_to_error_resource_exhausted_payload() {
        let status = tonic::Status::resource_exhausted("message too large");
        let err = grpc_status_to_error(status);
        assert!(err.is_request_too_large());
    }

    #[test]
    fn test_grpc_status_to_error_resource_exhausted_non_payload() {
        let status = tonic::Status::resource_exhausted("task queue limit exceeded");
        let err = grpc_status_to_error(status);
        assert!(!err.is_request_too_large());
        assert!(matches!(err, AgentLoopError::MessageStore(_)));
    }

    #[test]
    fn test_grpc_status_to_error_unauthenticated() {
        let status = tonic::Status::unauthenticated("bad token");
        let err = grpc_status_to_error(status);
        assert!(matches!(err, AgentLoopError::Configuration(_)));
        assert!(err.to_string().contains("Auth error"));
    }

    #[test]
    fn test_grpc_status_to_error_unavailable() {
        let status = tonic::Status::unavailable("service down");
        let err = grpc_status_to_error(status);
        assert!(matches!(err, AgentLoopError::MessageStore(_)));
        assert!(err.to_string().contains("unavailable"));
    }

    #[test]
    fn test_grpc_status_to_error_internal_fallback() {
        let status = tonic::Status::internal("server error");
        let err = grpc_status_to_error(status);
        assert!(matches!(err, AgentLoopError::MessageStore(_)));
        assert!(err.to_string().contains("Internal"));
    }

    #[test]
    fn test_grpc_missing_field() {
        let err = grpc_missing_field("No session in response");
        assert!(matches!(err, AgentLoopError::MessageStore(_)));
        assert!(err.to_string().contains("No session in response"));
    }

    #[test]
    fn test_capability_refs_to_configs_wraps_ids() {
        let configs =
            capability_refs_to_configs(&["session".to_string(), "platform_management".to_string()]);

        assert_eq!(
            configs,
            vec![
                serde_json::json!({ "ref": "session", "config": {} }),
                serde_json::json!({ "ref": "platform_management", "config": {} }),
            ]
        );
    }

    #[test]
    fn test_proto_stored_image_info_to_schema_roundtrips_image_id_uuid_transport() {
        let image_id = everruns_provider::typed_id::ImageId::new();
        let info = proto_stored_image_info_to_schema(proto::StoredImageInfo {
            id: Some(proto::Uuid {
                value: image_id.uuid().to_string(),
            }),
            filename: "generated-image.png".into(),
            content_type: "image/png".into(),
            size_bytes: 128,
            metadata: None,
            created_at: None,
        })
        .expect("stored image info should convert");

        assert_eq!(info.id, image_id);
    }

    #[test]
    fn test_grpc_command_error_to_error_not_found() {
        let err = grpc_command_error_to_error(proto::CommandError {
            kind: 3,
            message: "Harness not found".into(),
        });

        assert!(matches!(err, AgentLoopError::MessageStore(_)));
        assert!(err.to_string().contains("Harness not found"));
    }
}
