mod status_error;
pub(crate) use status_error::grpc_status_to_error;

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

use crate::core::connection_services::ProviderCredentials;
use crate::core::events::{Event, EventRequest};
use crate::core::leased_resource::{LeasedResource, LeasedResourceStatus};
use crate::core::message_retriever::{InputMessage, MessageHistory, MessageRetriever};
use crate::core::{
    AgentDefinition, ExecutionSession, HarnessDefinition, MessageFilter, RuntimeMessage,
    RuntimeMessageRole, connection_services::ProviderCredentialStore, event_emitter::EventEmitter,
    execution_loading::AgentStore, execution_loading::HarnessStore,
    execution_loading::SessionStore, file_services::ResolvedFile,
    image_services::CreateStoredImage, image_services::ImageArtifactStore,
    image_services::ResolvedImage, image_services::StoredImage, image_services::StoredImageInfo,
    provider_resolution::ProviderStore,
};
use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::typed_id::{AgentId, LeasedResourceId, MessageId, ModelId, SessionId};
// Workers project internal wire DTOs into portable execution views. Persisted
// agent, harness and session records belong to the server; the transport shape
// stays compatible with workers from the preceding release.
use everruns_internal_protocol::proto;
use everruns_internal_protocol::{
    WorkerServiceClient, json_to_proto_list, json_to_proto_struct, proto_list_to_json,
    proto_struct_to_json,
};
use std::sync::Arc;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::grpc_durable_store::GrpcClientAuth;
mod connection_resolver;
mod event_batch;
mod shared_client;
pub use shared_client::SharedClient;
mod definition_reads;
mod session_storage;
pub(crate) const COMMAND_API_VERSION_V1: &str = "v1";

/// Create a store error for issues in gRPC responses (e.g., missing fields).
pub(crate) fn grpc_missing_field(field: &str) -> AgentLoopError {
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
    pub(crate) inner: SharedClient,
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
            inner: SharedClient::new(client),
        })
    }

    /// Create from an existing channel
    pub fn from_channel(channel: Channel) -> Self {
        let auth = GrpcClientAuth::from_env();
        let client = WorkerServiceClient::with_interceptor(channel, auth)
            .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE);
        Self {
            inner: SharedClient::new(client),
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

        let mut client = self.inner.client();
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

        let mut client = self.inner.client();
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

        let mut client = self.inner.client();
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
        image_id: everruns_contracts::typed_id::ImageId,
    ) -> Result<Option<StoredImage>> {
        let request = proto::GetImageArtifactRequest {
            org_id,
            image_id: Some(uuid_to_proto(image_id.uuid())),
        };

        let mut client = self.inner.client();
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
        image_id: everruns_contracts::typed_id::ImageId,
    ) -> Result<Option<StoredImageInfo>> {
        let request = proto::GetImageArtifactInfoRequest {
            org_id,
            image_id: Some(uuid_to_proto(image_id.uuid())),
        };

        let mut client = self.inner.client();
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
            session_id: None,
            ..Default::default()
        };

        let mut client = self.inner.client();
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
    ) -> Result<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        self.get_provider_config_for_session(org_id, provider_id, None)
            .await
    }
    pub async fn get_provider_config_for_session(
        &self,
        org_id: i64,
        provider_id: &str,
        session_id: Option<SessionId>,
    ) -> Result<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        let request = proto::GetDefaultProviderCredentialsRequest {
            org_id,
            provider_type: String::new(),
            provider_id: provider_id.to_string(),
            session_id: session_id.map(|id| uuid_to_proto(id.uuid())),
            ..Default::default()
        };
        let mut client = self.inner.client();
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
        let mut config = everruns_contracts::driver_registry::ProviderConfig::for_provider(
            everruns_contracts::runtime_provider::ProviderKey::new(provider_id),
            provider_type,
        );
        config.api_key = non_empty_string(response.api_key);
        config.base_url = non_empty_string(response.base_url);
        config.request_options = request_options;
        Ok(Some(config))
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
            input_message_id: None,
            org_id,
            session_id: session_id.map(uuid_to_proto),
        };

        let mut client = self.inner.client();
        let response = client
            .get_mcp_server_by_prefix(request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_server = response.into_inner().server.ok_or_else(|| {
            AgentLoopError::store(format!("MCP server not found for prefix: {server_prefix}"))
        })?;

        proto_mcp_server_to_info(proto_server)
    }

    pub async fn get_mcp_server_for_execution(
        &self,
        org_id: i64,
        session_id: Uuid,
        server_prefix: &str,
        input_message_id: Uuid,
    ) -> Result<crate::mcp_executor::McpServerInfo> {
        let request = proto::GetMcpServerByPrefixRequest {
            server_prefix: server_prefix.into(),
            org_id,
            session_id: Some(uuid_to_proto(session_id)),
            input_message_id: Some(uuid_to_proto(input_message_id)),
        };
        let response = self
            .inner
            .client()
            .get_mcp_server_by_prefix(request)
            .await
            .map_err(grpc_status_to_error)?;
        proto_mcp_server_to_info(
            response
                .into_inner()
                .server
                .ok_or_else(|| grpc_missing_field("MCP server missing"))?,
        )
    }

    /// Claim due leased resources for cleanup.
    pub async fn claim_due_leased_resources(
        &self,
        limit: u32,
        stale_after_seconds: u32,
    ) -> Result<Vec<(i64, LeasedResource)>> {
        let mut client = self.inner.client();
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
            .map(|s| (s.org_id, s))
            .map(|(org_id, s)| Ok((org_id, proto_leased_resource_to_schema(s)?)))
            .collect()
    }

    /// Mark a leased resource cleanup as released using compare-and-set semantics.
    pub async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        let mut client = self.inner.client();
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
        let mut client = self.inner.client();
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
    ) -> Result<Vec<(i64, everruns_contracts::typed_id::SessionId, String)>> {
        let mut client = self.inner.client();
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
                let session_id = everruns_contracts::typed_id::SessionId::from_uuid(uuid);
                Ok((e.org_id, session_id, e.task_id))
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
        let mut client = self.inner.client();
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
        let mut client = self.inner.client();
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
        let mut client = self.inner.client();
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
/// SessionSecretStorage, UserConnectionResolver, SessionSqlDbStore.
#[derive(Clone)]
pub struct GrpcAdapter {
    input_message_id: Option<Uuid>,
    mcp_server_prefix: Option<String>,
    pub(crate) client: GrpcClient,
    /// The org this adapter speaks for, when it has one.
    ///
    /// `None` is the cross-org background sweeper context (leased-resource
    /// cleanup, the task reaper): those claim work spanning organizations, so
    /// there is no org to carry. Surfaces that reach the org-scoped command
    /// transport require `Some`, and say so rather than inventing a default.
    pub(crate) org_id: Option<i64>,
    proactive_compaction_attempts: Arc<crate::core::ProactiveCompactionAttemptTracker>,
}

impl GrpcAdapter {
    pub fn new(client: GrpcClient) -> Self {
        Self {
            client,
            org_id: None,
            input_message_id: None,
            mcp_server_prefix: None,
            proactive_compaction_attempts: Arc::new(
                crate::core::ProactiveCompactionAttemptTracker::default(),
            ),
        }
    }

    pub fn new_org_scoped(client: GrpcClient, org_id: i64) -> Self {
        Self {
            org_id: Some(org_id),
            ..Self::new(client)
        }
    }
}

/// Org-scoped gRPC adapter (carries org_id for authorization).
///
/// Implements: AgentStore, HarnessStore, SessionStore, ProviderStore,
/// ImageResolver, SessionMutator, PlatformStore.
#[derive(Clone)]
pub struct GrpcOrgAdapter {
    input_message_id: Option<Uuid>,
    client: GrpcClient,
    org_id: i64,
    platform_session_id: Option<SessionId>,
}

impl GrpcOrgAdapter {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self {
            client,
            org_id,
            platform_session_id: None,
            input_message_id: None,
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
            input_message_id: None,
        }
    }

    async fn execute_platform_command_raw(
        &self,
        name: &str,
        params: serde_json::Value,
        runtime_view: bool,
    ) -> Result<std::result::Result<serde_json::Value, proto::CommandError>> {
        if self.input_message_id.is_none() {
            return Err(AgentLoopError::config(
                "Platform command requires a management-authorized invocation",
            ));
        }
        let mut client = self.client.inner.client();
        let response = client
            .execute_command(proto::ExecuteCommandRequest {
                runtime_view,
                name: name.to_string(),
                api_version: COMMAND_API_VERSION_V1.to_string(),
                params_json: serde_json::to_vec(&params).map_err(|e| {
                    AgentLoopError::store(format!("JSON serialization failed: {}", e))
                })?,
                org_id: self.org_id,
                user_id: None,
                platform_session_id: self.platform_session_id.map(|id| uuid_to_proto(id.uuid())),
                // Acts AS the invocation's management user, not as a session
                // runtime claiming its own mount.
                acting_for_session_id: None,
                input_message_id: self.input_message_id.map(uuid_to_proto),
                ..Default::default()
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
        let mut client = self.client.inner.client();
        let response = client
            .invoke_platform_command_surface(proto::InvokePlatformCommandSurfaceRequest {
                input_message_id: self.input_message_id.map(uuid_to_proto),
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
        match self
            .execute_platform_command_raw(name, params, false)
            .await?
        {
            Ok(value) => serde_json::from_value(value).map_err(|e| {
                AgentLoopError::store(format!("Failed to parse command response: {}", e))
            }),
            Err(error) => Err(grpc_command_error_to_error(error)),
        }
    }

    async fn execute_runtime_command<T>(&self, name: &str, params: serde_json::Value) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        match self
            .execute_platform_command_raw(name, params, true)
            .await?
        {
            Ok(value) => serde_json::from_value(value).map_err(|error| {
                AgentLoopError::store(format!("Failed to parse runtime response: {error}"))
            }),
            Err(error) => Err(grpc_command_error_to_error(error)),
        }
    }

    async fn execute_runtime_lookup<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        match self
            .execute_platform_command_raw(name, params, true)
            .await?
        {
            Ok(value) => serde_json::from_value(value).map(Some).map_err(|error| {
                AgentLoopError::store(format!("Failed to parse runtime response: {error}"))
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
#[derive(Clone)]
pub struct GrpcPaymentAuthority {
    client: GrpcClient,
    org_id: i64,
    agent_id: Option<String>,
    input_message_id: Option<Uuid>,
}

/// Session-creation authority backed by the control-plane permission resolver.
#[derive(Clone)]
pub struct GrpcSessionCreationAuthority {
    input_message_id: Option<Uuid>,
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
            input_message_id: None,
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
            input_message_id: None,
            client,
            org_id,
            session_id,
        }
    }
}

// ============================================================================
// Helper functions for proto conversion
// ============================================================================

pub(crate) fn uuid_to_proto(id: Uuid) -> proto::Uuid {
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

fn optional_proto_uuid(proto_uuid: Option<&proto::Uuid>) -> Result<Option<Uuid>> {
    proto_uuid
        .map(|id| proto_uuid_to_uuid(Some(id)))
        .transpose()
}

fn proto_mcp_server_to_info(
    proto_server: proto::McpServerInfo,
) -> Result<crate::mcp_executor::McpServerInfo> {
    let id = proto_uuid_to_uuid(proto_server.id.as_ref())?;
    Ok(crate::mcp_executor::McpServerInfo::from_proto(
        id,
        proto_server,
    ))
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
pub(crate) fn proto_timestamp_or_now(
    ts: Option<&proto::Timestamp>,
) -> chrono::DateTime<chrono::Utc> {
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
    pub async fn add_message(
        &self,
        session_id: Uuid,
        input: InputMessage,
    ) -> Result<RuntimeMessage> {
        let mut client = self.client.inner.client();

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
    async fn get(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<Option<RuntimeMessage>> {
        let mut client = self.client.inner.client();

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

    async fn load(&self, session_id: SessionId) -> Result<Vec<RuntimeMessage>> {
        let (messages, _, _) = self.load_with_message_limit(session_id, None, None).await?;
        Ok(messages)
    }

    async fn load_filtered(
        &self,
        query: crate::core::message_filter::MessageQuery,
    ) -> Result<Vec<RuntimeMessage>> {
        Ok(self.load_filtered_history(query).await?.messages)
    }

    async fn load_filtered_history(
        &self,
        query: crate::core::message_filter::MessageQuery,
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
                            "input.message" => m.role == RuntimeMessageRole::User,
                            "output.message.completed" => m.role == RuntimeMessageRole::Agent,
                            "tool.completed" => m.role == RuntimeMessageRole::ToolResult,
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
    ) -> Result<(Vec<RuntimeMessage>, usize, Option<i64>)> {
        let mut client = self.client.inner.client();

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
impl crate::core::CompactionCheckpointStore for GrpcAdapter {
    async fn get_latest(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<crate::core::CompactionCheckpoint>> {
        let mut client = self.client.inner.client();
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
                Ok(crate::core::CompactionCheckpoint {
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

    async fn install(&self, checkpoint: crate::core::CompactionCheckpoint) -> Result<bool> {
        let mut client = self.client.inner.client();
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
    ) -> Result<Option<crate::core::ProactiveCompactionAttempt>> {
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
        attempt: crate::core::ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.proactive_compaction_attempts
            .record(session_id, provider_type, model, attempt)
            .await;
        Ok(())
    }
}

fn proto_message_to_message(proto_msg: proto::Message) -> Result<RuntimeMessage> {
    let id = proto_uuid_to_uuid(proto_msg.id.as_ref())?;

    // Convert prost ListValue to Vec<ContentPart>
    let content_json = proto_msg
        .content
        .as_ref()
        .map(proto_list_to_json)
        .unwrap_or_else(|| serde_json::Value::Array(vec![]));
    let content: Vec<crate::core::ContentPart> = serde_json::from_value(content_json)
        .map_err(|e| AgentLoopError::store(format!("Failed to parse message content: {}", e)))?;

    // Convert prost Struct to Controls
    let controls: Option<crate::core::Controls> = proto_msg
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
        "system" => crate::core::RuntimeMessageRole::System,
        "user" => crate::core::RuntimeMessageRole::User,
        // Map both "assistant" (legacy) and "agent" to Agent role
        "assistant" | "agent" => crate::core::RuntimeMessageRole::Agent,
        "tool_result" => crate::core::RuntimeMessageRole::ToolResult,
        _ => crate::core::RuntimeMessageRole::User,
    };

    Ok(RuntimeMessage {
        id: id.into(),
        role,
        content,
        // Reasoning rides along inside `content` as ordered reasoning parts.
        phase: proto_msg
            .phase
            .as_deref()
            .and_then(everruns_contracts::ExecutionPhase::from_provider_str),
        phase_source: proto_msg
            .phase_source
            .as_deref()
            .and_then(everruns_contracts::PhaseSource::from_str_opt),
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
            .map(proto_agent_to_definition)
            .transpose()
    }

    async fn get_agent_blocker(
        &self,
        agent_id: AgentId,
    ) -> Result<Option<crate::core::DependencyBlocker>> {
        Ok(match self.fetch_agent_record(agent_id).await? {
            Some(agent) => match agent.status.to_lowercase().as_str() {
                "archived" => Some(crate::core::DependencyBlocker::AgentArchived),
                "deleted" => Some(crate::core::DependencyBlocker::AgentDeleted),
                _ => None,
            },
            None => Some(crate::core::DependencyBlocker::AgentDeleted),
        })
    }
}

impl GrpcOrgAdapter {
    /// Fetch the stored agent record off the wire (platform-side transport;
    /// projected to `AgentDefinition` before it reaches host execution).
    pub(crate) async fn fetch_agent_record(
        &self,
        agent_id: AgentId,
    ) -> Result<Option<proto::Agent>> {
        let mut client = self.client.inner.client();

        let request = proto::GetAgentRequest {
            agent_id: Some(uuid_to_proto(agent_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_agent(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().agent {
            Some(proto_agent) => Ok(Some(proto_agent)),
            None => Ok(None),
        }
    }
}

fn proto_agent_to_definition(proto_agent: proto::Agent) -> Result<AgentDefinition> {
    let id = proto_uuid_to_uuid(proto_agent.id.as_ref())?;
    let default_model_id = proto_agent
        .default_model_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;
    if matches!(
        proto_agent.status.to_lowercase().as_str(),
        "archived" | "deleted"
    ) {
        return Err(AgentLoopError::config(format!(
            "agent {} is {} and cannot execute turns",
            AgentId::from_uuid(id),
            proto_agent.status
        )));
    }

    let capabilities = if proto_agent.capabilities.is_empty() {
        proto_agent
            .capability_ids
            .into_iter()
            .map(everruns_contracts::CapabilityRef::new)
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

    Ok(AgentDefinition {
        id: AgentId::from_uuid(id),
        name: proto_agent.name,
        display_name: proto_agent.display_name,
        description: non_empty_string(proto_agent.description),
        system_prompt: proto_agent.system_prompt,
        default_model_id: default_model_id.map(Into::into),
        capabilities,
        initial_files: vec![],
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: proto_agent.parallel_tool_calls,
        tools: vec![],
        mcp_servers: Default::default(),
    })
}

// ============================================================================
// HarnessStore implementation
// ============================================================================

#[async_trait]
impl HarnessStore for GrpcOrgAdapter {
    async fn get_harness(
        &self,
        harness_id: everruns_contracts::typed_id::HarnessId,
    ) -> Result<Option<HarnessDefinition>> {
        // Loading seam (EVE-881): the server pre-merges the inheritance chain;
        // project the transported record into the portable execution
        // definition, failing archived/deleted records here.
        self.fetch_harness_record(harness_id)
            .await?
            .map(proto_harness_to_definition)
            .transpose()
    }

    async fn get_harness_blocker(
        &self,
        harness_id: everruns_contracts::typed_id::HarnessId,
    ) -> Result<Option<crate::core::DependencyBlocker>> {
        Ok(match self.fetch_harness_record(harness_id).await? {
            Some(harness) => match harness.status.to_lowercase().as_str() {
                "archived" => Some(crate::core::DependencyBlocker::HarnessArchived),
                "deleted" => Some(crate::core::DependencyBlocker::HarnessDeleted),
                _ => None,
            },
            None => Some(crate::core::DependencyBlocker::HarnessDeleted),
        })
    }
}

impl GrpcOrgAdapter {
    /// Fetch the harness transport snapshot off the wire
    /// (platform-side transport; projected to `HarnessDefinition` before it
    /// reaches host execution).
    pub(crate) async fn fetch_harness_record(
        &self,
        harness_id: everruns_contracts::typed_id::HarnessId,
    ) -> Result<Option<proto::Harness>> {
        let mut client = self.client.inner.client();

        let request = proto::GetHarnessRequest {
            harness_id: Some(uuid_to_proto(harness_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_harness(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().harness {
            Some(proto_harness) => Ok(Some(proto_harness)),
            None => Ok(None),
        }
    }
}

fn proto_harness_to_definition(proto_harness: proto::Harness) -> Result<HarnessDefinition> {
    let id = proto_uuid_to_uuid(proto_harness.id.as_ref())?;
    let default_model_id = proto_harness
        .default_model_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;
    if matches!(
        proto_harness.status.to_lowercase().as_str(),
        "archived" | "deleted"
    ) {
        return Err(AgentLoopError::config(format!(
            "harness {} is {} and cannot execute turns",
            everruns_contracts::typed_id::HarnessId::from_uuid(id),
            proto_harness.status
        )));
    }

    let capabilities = if proto_harness.capabilities.is_empty() {
        proto_harness
            .capability_ids
            .into_iter()
            .map(everruns_contracts::CapabilityRef::new)
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

    Ok(HarnessDefinition {
        name: proto_harness.name,
        system_prompt: Some(proto_harness.system_prompt).filter(|s| !s.trim().is_empty()),
        default_model_id: default_model_id.map(Into::into),
        capabilities,
        mcp_servers: Default::default(),
        initial_files: vec![],
        network_access: None,
        parallel_tool_calls: None,
        embedder_metadata: Default::default(),
    })
}

// ============================================================================
// SessionStore implementation
// ============================================================================

#[async_trait]
impl SessionStore for GrpcOrgAdapter {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        let mut client = self.client.inner.client();

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
        crate::core::SessionExecutionState::from(proto_session.status.to_lowercase().as_str());

    // Parse capabilities from proto if present
    let capabilities = proto_session
        .capabilities
        .iter()
        .filter_map(|c| serde_json::from_str::<everruns_contracts::CapabilityRef>(c).ok())
        .collect();

    Ok(ExecutionSession {
        id: id.into(),
        workspace_id: everruns_contracts::typed_id::WorkspaceId::from_uuid(id),
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
        let mut client = self.client.inner.client();

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
        let mut client = self.client.inner.client();

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

    async fn get_provider_config_for_session(
        &self,
        provider: &everruns_contracts::ProviderKey,
        session: SessionId,
    ) -> Result<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        self.client
            .get_provider_config_for_session(self.org_id, provider.as_str(), Some(session))
            .await
    }

    async fn get_provider_config(
        &self,
        provider: &everruns_contracts::runtime_provider::ProviderKey,
    ) -> Result<Option<everruns_contracts::driver_registry::ProviderConfig>> {
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
        image_id: everruns_contracts::typed_id::ImageId,
    ) -> Result<Option<StoredImage>> {
        self.client.get_image_artifact(self.org_id, image_id).await
    }

    async fn get_image_info(
        &self,
        image_id: everruns_contracts::typed_id::ImageId,
    ) -> Result<Option<StoredImageInfo>> {
        self.client
            .get_image_artifact_info(self.org_id, image_id)
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
        let mut client = self.client.inner.client();

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
        use everruns_contracts::typed_id::EventId;

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

        // Fire gRPC call in background with backpressure: while the previous
        // ephemeral emit is still in flight, drop this event rather than
        // accumulating unbounded background tasks.
        let client = self.client.clone();
        tokio::spawn(async move {
            match client.inner.try_ephemeral() {
                Some((mut inner, _permit)) => {
                    if let Err(e) = inner.emit_event(grpc_request).await {
                        tracing::debug!(
                            error = %e,
                            %session_id,
                            event_type,
                            "Background ephemeral event emit failed (non-fatal)"
                        );
                    }
                }
                None => {
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

/// Convert crate::core::EventRequest to proto::EventRequest
pub(crate) fn core_event_request_to_proto(request: &EventRequest) -> Result<proto::EventRequest> {
    // Use the typed event conversion from internal-protocol
    Ok(everruns_internal_protocol::schema_event_request_to_proto(
        request,
    ))
}

/// Convert proto::Event to crate::core::Event
pub(super) fn proto_event_to_core(proto_event: proto::Event) -> Result<Event> {
    everruns_internal_protocol::proto_event_to_schema(proto_event)
        .map_err(|e| AgentLoopError::store(format!("Failed to convert proto event: {}", e)))
}

// ============================================================================
// Batch context loader
// ============================================================================

/// Turn context loaded in one batched gRPC call
pub struct TurnContext {
    pub agent: Option<AgentDefinition>,
    pub session: ExecutionSession,
    pub messages: Vec<RuntimeMessage>,
    pub model: Option<ModelSpec>,
    /// MCP tool definitions pre-resolved from agent's MCP capabilities
    pub mcp_tool_definitions: Vec<everruns_contracts::tool_types::ToolDefinition>,
}

/// Load turn context in one batched call, cheaper than separate agent, session,
/// and message calls.
pub async fn load_turn_context(
    client: &GrpcClient,
    org_id: i64,
    session_id: SessionId,
) -> Result<TurnContext> {
    load_turn_context_for_execution(client, org_id, session_id, None).await
}
pub async fn load_turn_context_for_execution(
    client: &GrpcClient,
    org_id: i64,
    session_id: SessionId,
    input_message_id: Option<Uuid>,
) -> Result<TurnContext> {
    let mut grpc_client = client.inner.client();

    let request = proto::GetTurnContextRequest {
        session_id: Some(uuid_to_proto(session_id.uuid())),
        org_id,
        message_limit: None, // use server default; execution assembles its own:
        omit_messages_and_model: input_message_id.map(|_| true),
        input_message_id: input_message_id.map(uuid_to_proto),
    };

    let response = grpc_client
        .get_turn_context(request)
        .await
        .map_err(grpc_status_to_error)?;

    let inner = response.into_inner();

    let agent = inner.agent.map(proto_agent_to_definition).transpose()?;
    let proto_session = inner
        .session
        .ok_or_else(|| grpc_missing_field("No session in turn context"))?;

    let session = proto_session_to_session(proto_session)?;

    let messages: Vec<RuntimeMessage> = inner
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
) -> everruns_contracts::tool_types::ToolDefinition {
    use everruns_contracts::tool_types::{
        BuiltinTool, DeferrablePolicy, ToolDefinition, ToolPolicy,
    };

    // Convert proto Struct to serde_json::Value
    let parameters = proto_tool
        .parameters
        .map(|s| proto_struct_to_json(&s))
        .unwrap_or_else(|| serde_json::json!({"type": "object"}));

    let mut hints = everruns_contracts::tool_types::ToolHints::default().with_open_world(true);
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

use crate::core::{file_services::FileResolver, image_services::ImageResolver};
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

        let mut client = self.client.inner.client();

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

    /// Resolve multiple files in a batch (more efficient)
    ///
    /// Returns a HashMap mapping file_id to ResolvedFile for all found files.
    /// Missing files are silently skipped. Files are always returned inline
    /// as base64 (no presigned-URL variant, unlike images).
    pub async fn resolve_files_batch(
        &self,
        file_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedFile>> {
        if file_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut client = self.client.inner.client();

        let request = proto::ResolveFilesRequest {
            file_ids: file_ids.iter().map(|id| uuid_to_proto(*id)).collect(),
            org_id: self.org_id,
        };

        let response = client
            .resolve_files(request)
            .await
            .map_err(grpc_status_to_error)?;

        let mut result = HashMap::new();
        for (id_str, data) in response.into_inner().files {
            if let Ok(id) = Uuid::parse_str(&id_str) {
                result.insert(
                    id,
                    ResolvedFile {
                        base64: data.base64,
                        media_type: data.media_type,
                        filename: if data.filename.is_empty() {
                            None
                        } else {
                            Some(data.filename)
                        },
                    },
                );
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
        let mut client = self.client.inner.client();

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

#[async_trait]
impl FileResolver for GrpcOrgAdapter {
    /// Resolve file attachments by ID via the ResolveFiles batch RPC.
    async fn resolve_files(&self, file_ids: &[Uuid]) -> Result<HashMap<Uuid, ResolvedFile>> {
        self.resolve_files_batch(file_ids).await
    }
}

// ============================================================================
// GrpcOrgAdapter - SessionMutator over gRPC
// ============================================================================

#[async_trait]
impl everruns_capabilities::SessionMutator for GrpcOrgAdapter {
    async fn update_session_title(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.client
            .set_session_title(self.org_id, session_id, &title)
            .await
    }
}

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
        session_id: optional_proto_uuid(s.session_id.as_ref())?.map(SessionId::from_uuid),
        provider: s.provider,
        resource_type: s.resource_type,
        external_id: s.external_id,
        display_name: s.display_name,
        status,
        owner_user_id: optional_proto_uuid(s.owner_user_id.as_ref())?,
        connection_id: optional_proto_uuid(s.connection_id.as_ref())?,
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

// ============================================================================
// GrpcOrgAdapter - PlatformStore implementation over gRPC
// ============================================================================

#[path = "grpc_adapters/platform_store.rs"]
mod platform_store;

// ============================================================================
// GrpcAdapter - SessionSqlDbStore implementation over gRPC
// ============================================================================
/// Convert a proto Value to serde_json::Value for SQL query results.
pub(crate) fn proto_value_to_json(value: prost_types::Value) -> serde_json::Value {
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
impl crate::core::tool_execution::OutboundToolRateLimiter for GrpcOutboundToolRateLimiter {
    async fn check_org(&self, org_id: &everruns_contracts::typed_id::OrgId) -> bool {
        let mut client = self.client.inner.client();
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
impl crate::core::tool_execution::BudgetChecker for GrpcBudgetChecker {
    async fn check_budgets(
        &self,
        session_id: &str,
    ) -> everruns_contracts::error::Result<crate::core::budget::BudgetToolResponse> {
        let mut client = self.client.inner.client();
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
        Ok(crate::core::budget::BudgetToolResponse {
            status: resp.status,
            budgets: resp
                .budgets
                .into_iter()
                .map(|b| crate::core::budget::BudgetSummary {
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

#[path = "grpc_adapters/payment_authority.rs"]
mod payment_authority;

#[async_trait]
impl crate::core::delegation_services::SessionCreationAuthority for GrpcSessionCreationAuthority {
    fn for_execution(
        &self,
        id: Uuid,
    ) -> Option<Arc<dyn crate::core::delegation_services::SessionCreationAuthority>> {
        let mut bound = self.clone();
        bound.input_message_id = Some(id);
        Some(Arc::new(bound))
    }

    async fn authorize_session_creation(
        &self,
        session_id: SessionId,
    ) -> everruns_contracts::error::Result<SessionId> {
        if session_id != self.session_id {
            return Err(AgentLoopError::tool(
                "session-creation authority is scoped to the current session",
            ));
        }
        let mut client = self.client.inner.client();
        let response = client
            .authorize_session_creation(proto::AuthorizeSessionCreationRequest {
                input_message_id: self.input_message_id.map(uuid_to_proto),
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

#[path = "grpc_adapters/journals.rs"]
mod journals;

#[cfg(test)]
#[path = "grpc_adapters/tests.rs"]
mod tests;

mod decision_models;
