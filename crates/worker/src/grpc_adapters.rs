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
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{Event, EventRequest};
use everruns_core::leased_resource::{LeasedResource, LeasedResourceStatus, UpsertLeasedResource};
use everruns_core::message_retriever::{InputMessage, MessageRetriever};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, LeasedResourceStore, LlmProviderStore,
    ModelWithProvider, ResolvedImage, SessionFileStore, SessionStore,
};
use everruns_core::typed_id::{AgentId, LeasedResourceId, MessageId, ModelId, SessionId};
use everruns_core::{Agent, Harness, HarnessStatus, Message, Session};
use everruns_internal_protocol::proto;
use everruns_internal_protocol::{
    WorkerServiceClient, json_to_proto_list, json_to_proto_struct, proto_list_to_json,
    proto_struct_to_json,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::grpc_durable_store::GrpcClientAuth;

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

    /// Set session status (started, active, idle)
    pub async fn set_session_status(
        &self,
        org_id: i64,
        session_id: SessionId,
        status: &str,
    ) -> Result<Session> {
        let request = proto::SetSessionStatusRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            status: status.to_string(),
            org_id,
        };

        let mut client = self.inner.lock().await;
        let response = client
            .set_session_status(request)
            .await
            .map_err(grpc_status_to_error)?;

        let proto_session = response
            .into_inner()
            .session
            .ok_or_else(|| grpc_missing_field("No session in response"))?;

        proto_session_to_session(proto_session)
    }

    /// Set session title.
    pub async fn set_session_title(
        &self,
        org_id: i64,
        session_id: SessionId,
        title: &str,
    ) -> Result<Session> {
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

    /// Get MCP server info by name prefix (for MCP tool execution)
    pub async fn get_mcp_server_by_prefix(
        &self,
        org_id: i64,
        server_prefix: &str,
    ) -> Result<crate::mcp_executor::McpServerInfo> {
        let request = proto::GetMcpServerByPrefixRequest {
            server_prefix: server_prefix.to_string(),
            org_id,
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
            oauth_provider_id: proto_server.oauth_provider_id,
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
}

// ============================================================================
// Consolidated adapter structs
// ============================================================================

/// Session-scoped gRPC adapter (no org_id needed).
///
/// Implements: MessageRetriever, SessionFileStore, EventEmitter,
/// SessionStorageStore, UserConnectionResolver, LeasedResourceStore,
/// SessionSqlDbStore.
#[derive(Clone)]
pub struct GrpcAdapter {
    client: GrpcClient,
}

impl GrpcAdapter {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

/// Org-scoped gRPC adapter (carries org_id for authorization).
///
/// Implements: AgentStore, HarnessStore, SessionStore, LlmProviderStore,
/// ImageResolver, SessionMutator, SessionScheduleStore, PlatformStore.
#[derive(Clone)]
pub struct GrpcOrgAdapter {
    client: GrpcClient,
    org_id: i64,
}

impl GrpcOrgAdapter {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self { client, org_id }
    }
}

// Type aliases for backward compatibility at call sites
pub type GrpcMessageRetriever = GrpcAdapter;
pub type GrpcSessionFileStore = GrpcAdapter;
pub type GrpcEventEmitter = GrpcAdapter;
pub type GrpcSessionStorageStore = GrpcAdapter;
pub type GrpcConnectionResolver = GrpcAdapter;
pub type GrpcLeasedResourceStore = GrpcAdapter;
pub type GrpcSessionResourceRegistry = GrpcAdapter;
pub type GrpcSessionSqlDbStore = GrpcAdapter;

pub type GrpcAgentStore = GrpcOrgAdapter;
pub type GrpcHarnessStore = GrpcOrgAdapter;
pub type GrpcSessionStore = GrpcOrgAdapter;
pub type GrpcLlmProviderStore = GrpcOrgAdapter;
pub type GrpcImageResolver = GrpcOrgAdapter;
pub type GrpcSessionMutator = GrpcOrgAdapter;
pub type GrpcScheduleStore = GrpcOrgAdapter;
pub type GrpcPlatformStore = GrpcOrgAdapter;
/// Budget checker that carries org_id and optional agent_id (captured at
/// construction) for gRPC calls.
pub struct GrpcBudgetChecker {
    client: GrpcClient,
    org_id: i64,
    agent_id: Option<String>,
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
        let mut client = self.client.inner.lock().await;

        let request = proto::LoadMessagesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
        };

        let response = client
            .load_messages(request)
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .messages
            .into_iter()
            .map(proto_message_to_message)
            .collect()
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
        phase: None,
        thinking: proto_msg.thinking, // Thinking content from extended thinking models
        thinking_signature: proto_msg.thinking_signature, // Cryptographic signature for thinking
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
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>> {
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

    let status = match proto_agent.status.to_lowercase().as_str() {
        "active" => everruns_core::AgentStatus::Active,
        "archived" => everruns_core::AgentStatus::Archived,
        "deleted" => everruns_core::AgentStatus::Deleted,
        _ => everruns_core::AgentStatus::Active,
    };

    Ok(Agent {
        public_id: everruns_core::AgentId::from_uuid(id),
        internal_id: id,
        name: proto_agent.name.clone(),
        display_name: proto_agent.display_name,
        description: non_empty_string(proto_agent.description),
        system_prompt: proto_agent.system_prompt,
        default_model_id: default_model_id.map(|u| u.into()),
        tags: vec![],
        capabilities: proto_agent
            .capability_ids
            .into_iter()
            .map(everruns_core::AgentCapabilityConfig::new)
            .collect(),
        initial_files: vec![],
        network_access: None,
        max_iterations: None,
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
    async fn get_harness_chain(
        &self,
        harness_id: everruns_core::HarnessId,
    ) -> Result<Vec<Harness>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetHarnessRequest {
            harness_id: Some(uuid_to_proto(harness_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_harness(request)
            .await
            .map_err(grpc_status_to_error)?;

        // gRPC returns a pre-merged harness; wrap as single-element chain
        match response.into_inner().harness {
            Some(proto_harness) => {
                let harness = proto_harness_to_harness(proto_harness)?;
                Ok(vec![harness])
            }
            None => Ok(vec![]),
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

    Ok(Harness {
        id: id.into(),
        name: proto_harness.name,
        display_name: proto_harness.display_name,
        description: non_empty_string(proto_harness.description),
        system_prompt: proto_harness.system_prompt,
        parent_harness_id: parent_harness_id.map(|u| u.into()),
        default_model_id: default_model_id.map(|u| u.into()),
        tags: proto_harness.tags,
        capabilities: proto_harness
            .capability_ids
            .into_iter()
            .map(everruns_core::AgentCapabilityConfig::new)
            .collect(),
        initial_files: vec![],
        network_access: None,
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
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
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

fn proto_session_to_session(proto_session: proto::Session) -> Result<Session> {
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

    let status = match proto_session.status.to_lowercase().as_str() {
        "started" => everruns_core::SessionStatus::Started,
        "active" => everruns_core::SessionStatus::Active,
        "idle" => everruns_core::SessionStatus::Idle,
        // Handle legacy values during migration
        "running" => everruns_core::SessionStatus::Active,
        "pending" | "completed" | "failed" => everruns_core::SessionStatus::Idle,
        _ => everruns_core::SessionStatus::Started,
    };

    let created_at = proto_timestamp_or_now(proto_session.created_at.as_ref());
    let updated_at = proto_timestamp_or_now(proto_session.updated_at.as_ref());

    // Parse capabilities from proto if present
    let capabilities = proto_session
        .capabilities
        .iter()
        .filter_map(|c| serde_json::from_str::<everruns_core::AgentCapabilityConfig>(c).ok())
        .collect();

    Ok(Session {
        id: id.into(),
        organization_id: proto_session.organization_id,
        agent_id: agent_id.map(|u| u.into()),
        agent_identity_id: None,
        harness_id: harness_id.into(),
        title: non_empty_string(proto_session.title),
        locale: non_empty_string(proto_session.locale),
        preview: None,
        output_preview: None,
        tags: proto_session.tags,
        model_id: model_id.map(|u| u.into()),
        capabilities,
        tools: vec![],
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
        status,
        created_at,
        updated_at,
        started_at: None,
        finished_at: None,
        usage: None, // Usage not tracked in worker context
        is_pinned: None,
        active_schedule_count: None,
        features: vec![], // Computed at API read time, not in worker
        parent_session_id: None,
        subagent_name: None,
        subagent_task: None,
        subagent_status: None,
        blueprint_id: None,
        blueprint_config: None,
    })
}

// ============================================================================
// LlmProviderStore implementation
// ============================================================================

#[async_trait]
impl LlmProviderStore for GrpcOrgAdapter {
    async fn get_model_with_provider(
        &self,
        model_id: ModelId,
    ) -> Result<Option<ModelWithProvider>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetModelWithProviderRequest {
            model_id: Some(uuid_to_proto(model_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_model_with_provider(request)
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().model {
            Some(proto_model) => {
                let model = proto_model_with_provider_to_model(proto_model)?;
                Ok(Some(model))
            }
            None => Ok(None),
        }
    }

    async fn get_default_model(&self) -> Result<Option<ModelWithProvider>> {
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
                let model = proto_model_with_provider_to_model(proto_model)?;
                Ok(Some(model))
            }
            None => Ok(None),
        }
    }
}

fn proto_model_with_provider_to_model(
    proto: proto::ModelWithProvider,
) -> Result<ModelWithProvider> {
    let provider_type = match proto.provider_type.to_lowercase().as_str() {
        "openai" => everruns_core::LlmProviderType::Openai,
        "openai_completions" => everruns_core::LlmProviderType::OpenaiCompletions,
        "anthropic" => everruns_core::LlmProviderType::Anthropic,
        "gemini" => everruns_core::LlmProviderType::Gemini,
        "llmsim" => everruns_core::LlmProviderType::LlmSim,
        _ => {
            return Err(AgentLoopError::store(format!(
                "Unknown provider type: {}",
                proto.provider_type
            )));
        }
    };

    Ok(ModelWithProvider {
        model: proto.model,
        provider_type,
        api_key: proto.api_key.filter(|s| !s.is_empty()),
        base_url: proto.base_url.filter(|s| !s.is_empty()),
    })
}

// ============================================================================
// SessionFileStore implementation
// ============================================================================

#[async_trait]
impl SessionFileStore for GrpcAdapter {
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
        use everruns_core::typed_id::EventId;

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
    pub session: Session,
    pub messages: Vec<Message>,
    pub model: Option<ModelWithProvider>,
    /// MCP tool definitions pre-resolved from agent's MCP capabilities
    pub mcp_tool_definitions: Vec<everruns_core::ToolDefinition>,
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

    let model = inner
        .model
        .map(proto_model_with_provider_to_model)
        .transpose()?;

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
) -> everruns_core::ToolDefinition {
    use everruns_core::tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolPolicy};

    // Convert proto Struct to serde_json::Value
    let parameters = proto_tool
        .parameters
        .map(|s| proto_struct_to_json(&s))
        .unwrap_or_else(|| serde_json::json!({"type": "object"}));

    ToolDefinition::Builtin(BuiltinTool {
        name: proto_tool.name,
        display_name: None,
        description: proto_tool.description,
        parameters,
        policy: ToolPolicy::Auto, // MCP tools are auto-executed
        category: None,
        deferrable: DeferrablePolicy::default(),
        // MCP tools are remote/networked; default open_world to true.
        // Full annotation propagation requires extending the internal proto.
        hints: everruns_core::tool_types::ToolHints::default().with_open_world(true),
    })
}

// ============================================================================
// ImageResolver implementation
// ============================================================================

use everruns_core::traits::{ImageResolver, KeyInfo, SecretInfo, SessionStorageStore};
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
        session_id: everruns_core::SessionId,
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
        session_id: everruns_core::SessionId,
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

    async fn delete_value(&self, session_id: everruns_core::SessionId, key: &str) -> Result<bool> {
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

    async fn list_keys(&self, session_id: everruns_core::SessionId) -> Result<Vec<KeyInfo>> {
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
        session_id: everruns_core::SessionId,
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
        session_id: everruns_core::SessionId,
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
        session_id: everruns_core::SessionId,
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

    async fn list_secrets(&self, session_id: everruns_core::SessionId) -> Result<Vec<SecretInfo>> {
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
// GrpcConnectionResolver - UserConnectionResolver over gRPC
// ============================================================================

#[async_trait]
impl everruns_core::traits::UserConnectionResolver for GrpcAdapter {
    async fn get_connection_token(
        &self,
        session_id: everruns_core::SessionId,
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
        session_id: everruns_core::SessionId,
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
// GrpcSessionMutator - SessionMutator over gRPC
// ============================================================================

#[async_trait]
impl everruns_core::traits::SessionMutator for GrpcOrgAdapter {
    async fn update_session_title(
        &self,
        session_id: everruns_core::SessionId,
        title: String,
    ) -> Result<everruns_core::session::Session> {
        self.client
            .set_session_title(self.org_id, session_id, &title)
            .await
    }
}

// ============================================================================
// GrpcLeasedResourceStore - LeasedResourceStore over gRPC
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
// GrpcSessionResourceRegistry - SessionResourceRegistry over gRPC
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
impl everruns_core::traits::SessionResourceRegistry for GrpcAdapter {
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
        _session_id: SessionId,
        _resource_id: &str,
    ) -> Result<Option<everruns_core::SessionResourceEntry>> {
        // get is not used by worker tools; only register/update_status/list matter.
        Ok(None)
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
// GrpcScheduleStore - SessionScheduleStore over gRPC
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
    use everruns_core::typed_id::{ScheduleId, SessionId};

    let id_uuid = proto_uuid_to_uuid(s.id.as_ref())?;
    let session_uuid = proto_uuid_to_uuid(s.session_id.as_ref())?;

    let schedule_type = match s.schedule_type.as_str() {
        "recurring" => ScheduleType::Recurring,
        _ => ScheduleType::OneShot,
    };

    Ok(SessionSchedule {
        id: ScheduleId::from_uuid(id_uuid),
        session_id: SessionId::from_uuid(session_uuid),
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
impl everruns_core::traits::SessionScheduleStore for GrpcOrgAdapter {
    async fn create_schedule(
        &self,
        session_id: everruns_core::SessionId,
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

    async fn cancel_schedule(
        &self,
        session_id: everruns_core::SessionId,
        schedule_id: everruns_core::typed_id::ScheduleId,
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
        session_id: everruns_core::SessionId,
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

    async fn count_active_schedules(&self, session_id: everruns_core::SessionId) -> Result<u32> {
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
}

// ============================================================================
// GrpcPlatformStore - PlatformStore implementation over gRPC
// ============================================================================

#[async_trait]
impl everruns_core::platform_store::PlatformStore for GrpcOrgAdapter {
    // =========================================================================
    // Harness Operations
    // =========================================================================

    async fn list_harnesses(&self) -> Result<Vec<Harness>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_list_harnesses(proto::PlatformListHarnessesRequest {
                org_id: self.org_id,
            })
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .harnesses
            .into_iter()
            .map(|h| {
                everruns_internal_protocol::proto_harness_to_schema(h)
                    .map_err(|e| AgentLoopError::store(format!("Harness conversion failed: {}", e)))
            })
            .collect()
    }

    async fn get_harness(&self, id: everruns_core::HarnessId) -> Result<Option<Harness>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_harness(proto::GetHarnessRequest {
                harness_id: Some(uuid_to_proto(id.uuid())),
                org_id: self.org_id,
            })
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().harness {
            Some(h) => Ok(Some(
                everruns_internal_protocol::proto_harness_to_schema(h).map_err(|e| {
                    AgentLoopError::store(format!("Harness conversion failed: {}", e))
                })?,
            )),
            None => Ok(None),
        }
    }

    async fn create_harness(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: &str,
        parent_harness_id: Option<everruns_core::HarnessId>,
        capabilities: &[String],
    ) -> Result<Harness> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_create_harness(proto::PlatformCreateHarnessRequest {
                org_id: self.org_id,
                name: name.to_string(),
                display_name: display_name.map(|s| s.to_string()),
                description: description.map(|s| s.to_string()),
                system_prompt: system_prompt.to_string(),
                capabilities: capabilities.to_vec(),
                parent_harness_id: parent_harness_id.map(|id| uuid_to_proto(id.uuid())),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let harness = response
            .into_inner()
            .harness
            .ok_or_else(|| grpc_missing_field("No harness in create response"))?;

        everruns_internal_protocol::proto_harness_to_schema(harness)
            .map_err(|e| AgentLoopError::store(format!("Harness conversion failed: {}", e)))
    }

    async fn update_harness(
        &self,
        id: everruns_core::HarnessId,
        name: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
        parent_harness_id: Option<Option<everruns_core::HarnessId>>,
    ) -> Result<Harness> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_update_harness(proto::PlatformUpdateHarnessRequest {
                org_id: self.org_id,
                harness_id: Some(uuid_to_proto(id.uuid())),
                name: name.map(|s| s.to_string()),
                display_name: display_name.map(|s| s.to_string()),
                description: description.map(|s| s.to_string()),
                system_prompt: system_prompt.map(|s| s.to_string()),
                parent_harness_id: parent_harness_id
                    .flatten()
                    .map(|parent_id| uuid_to_proto(parent_id.uuid())),
                clear_parent_harness_id: parent_harness_id
                    .and_then(|value| value.is_none().then_some(true)),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let harness = response
            .into_inner()
            .harness
            .ok_or_else(|| grpc_missing_field("No harness in update response"))?;

        everruns_internal_protocol::proto_harness_to_schema(harness)
            .map_err(|e| AgentLoopError::store(format!("Harness conversion failed: {}", e)))
    }

    async fn delete_harness(&self, id: everruns_core::HarnessId) -> Result<()> {
        let mut client = self.client.inner.lock().await;
        client
            .platform_delete_harness(proto::PlatformDeleteHarnessRequest {
                org_id: self.org_id,
                harness_id: Some(uuid_to_proto(id.uuid())),
            })
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }

    async fn copy_harness(
        &self,
        id: everruns_core::HarnessId,
        new_name: Option<&str>,
    ) -> Result<Harness> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_copy_harness(proto::PlatformCopyHarnessRequest {
                org_id: self.org_id,
                harness_id: Some(uuid_to_proto(id.uuid())),
                new_name: new_name.map(|s| s.to_string()),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let harness = response
            .into_inner()
            .harness
            .ok_or_else(|| grpc_missing_field("No harness in copy response"))?;

        everruns_internal_protocol::proto_harness_to_schema(harness)
            .map_err(|e| AgentLoopError::store(format!("Harness conversion failed: {}", e)))
    }

    // =========================================================================
    // Agent Operations
    // =========================================================================

    async fn list_agents(&self) -> Result<Vec<Agent>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_list_agents(proto::PlatformListAgentsRequest {
                org_id: self.org_id,
            })
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .agents
            .into_iter()
            .map(|a| {
                everruns_internal_protocol::proto_agent_to_schema(a)
                    .map_err(|e| AgentLoopError::store(format!("Agent conversion failed: {}", e)))
            })
            .collect()
    }

    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<Agent>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_agent(proto::GetAgentRequest {
                agent_id: Some(uuid_to_proto(id.uuid())),
                org_id: self.org_id,
            })
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().agent {
            Some(a) => Ok(Some(
                everruns_internal_protocol::proto_agent_to_schema(a).map_err(|e| {
                    AgentLoopError::store(format!("Agent conversion failed: {}", e))
                })?,
            )),
            None => Ok(None),
        }
    }

    async fn create_agent(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: &str,
        capabilities: &[String],
    ) -> Result<Agent> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_create_agent(proto::PlatformCreateAgentRequest {
                org_id: self.org_id,
                name: name.to_string(),
                description: description.map(|s| s.to_string()),
                system_prompt: system_prompt.to_string(),
                capabilities: capabilities.to_vec(),
                display_name: display_name.map(|s| s.to_string()),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let agent = response
            .into_inner()
            .agent
            .ok_or_else(|| grpc_missing_field("No agent in create response"))?;

        everruns_internal_protocol::proto_agent_to_schema(agent)
            .map_err(|e| AgentLoopError::store(format!("Agent conversion failed: {}", e)))
    }

    async fn update_agent(
        &self,
        id: AgentId,
        name: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<Agent> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_update_agent(proto::PlatformUpdateAgentRequest {
                org_id: self.org_id,
                agent_id: Some(uuid_to_proto(id.uuid())),
                name: name.map(|s| s.to_string()),
                description: description.map(|s| s.to_string()),
                system_prompt: system_prompt.map(|s| s.to_string()),
                display_name: display_name.map(|s| s.to_string()),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let agent = response
            .into_inner()
            .agent
            .ok_or_else(|| grpc_missing_field("No agent in update response"))?;

        everruns_internal_protocol::proto_agent_to_schema(agent)
            .map_err(|e| AgentLoopError::store(format!("Agent conversion failed: {}", e)))
    }

    async fn delete_agent(&self, id: AgentId) -> Result<()> {
        let mut client = self.client.inner.lock().await;
        client
            .platform_delete_agent(proto::PlatformDeleteAgentRequest {
                org_id: self.org_id,
                agent_id: Some(uuid_to_proto(id.uuid())),
            })
            .await
            .map_err(grpc_status_to_error)?;
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
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_list_sessions(proto::PlatformListSessionsRequest {
                org_id: self.org_id,
                limit: limit.map(|l| l as u32),
                agent_id: agent_id.map(|id| uuid_to_proto(id.uuid())),
            })
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .sessions
            .into_iter()
            .map(|s| {
                everruns_internal_protocol::proto_session_to_schema(s)
                    .map_err(|e| AgentLoopError::store(format!("Session conversion failed: {}", e)))
            })
            .collect()
    }

    async fn create_session(
        &self,
        harness_id: everruns_core::HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        blueprint_id: Option<&str>,
        blueprint_config: Option<&serde_json::Value>,
    ) -> Result<Session> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_create_session(proto::PlatformCreateSessionRequest {
                org_id: self.org_id,
                harness_id: Some(uuid_to_proto(harness_id.uuid())),
                agent_id: agent_id.map(|id| uuid_to_proto(id.uuid())),
                title: title.map(|s| s.to_string()),
                locale: locale.map(|value| value.to_string()),
                blueprint_id: blueprint_id.map(|s| s.to_string()),
                blueprint_config_json: blueprint_config
                    .map(|v| serde_json::to_string(v).unwrap_or_default()),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let session = response
            .into_inner()
            .session
            .ok_or_else(|| grpc_missing_field("No session in create response"))?;

        everruns_internal_protocol::proto_session_to_schema(session)
            .map_err(|e| AgentLoopError::store(format!("Session conversion failed: {}", e)))
    }

    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_session(proto::GetSessionRequest {
                session_id: Some(uuid_to_proto(id.uuid())),
                org_id: self.org_id,
            })
            .await
            .map_err(grpc_status_to_error)?;

        match response.into_inner().session {
            Some(s) => Ok(Some(
                everruns_internal_protocol::proto_session_to_schema(s).map_err(|e| {
                    AgentLoopError::store(format!("Session conversion failed: {}", e))
                })?,
            )),
            None => Ok(None),
        }
    }

    async fn delete_session(&self, id: SessionId) -> Result<()> {
        let mut client = self.client.inner.lock().await;
        client
            .platform_delete_session(proto::PlatformDeleteSessionRequest {
                org_id: self.org_id,
                session_id: Some(uuid_to_proto(id.uuid())),
            })
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }

    // =========================================================================
    // Messaging
    // =========================================================================

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        let mut client = self.client.inner.lock().await;
        client
            .platform_send_message(proto::PlatformSendMessageRequest {
                org_id: self.org_id,
                session_id: Some(uuid_to_proto(session_id.uuid())),
                content: content.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<everruns_core::platform_store::PlatformMessage>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_get_messages(proto::PlatformGetMessagesRequest {
                org_id: self.org_id,
                session_id: Some(uuid_to_proto(session_id.uuid())),
                limit: limit.map(|l| l as u32),
            })
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response
            .into_inner()
            .messages
            .into_iter()
            .map(|m| everruns_core::platform_store::PlatformMessage {
                role: m.role,
                content: m.content,
                created_at: proto_timestamp_or_now(m.created_at.as_ref()),
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
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_wait_for_idle(proto::PlatformWaitForIdleRequest {
                org_id: self.org_id,
                session_id: Some(uuid_to_proto(session_id.uuid())),
                timeout_secs,
            })
            .await
            .map_err(grpc_status_to_error)?;

        Ok(response.into_inner().status)
    }

    // =========================================================================
    // Capabilities
    // =========================================================================

    async fn list_capabilities(
        &self,
        search: Option<&str>,
    ) -> Result<Vec<everruns_core::CapabilityInfo>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_list_capabilities(proto::PlatformListCapabilitiesRequest {
                org_id: self.org_id,
                search: search.map(|s| s.to_string()),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let caps = response
            .into_inner()
            .capabilities
            .into_iter()
            .map(|c| everruns_core::CapabilityInfo {
                id: everruns_core::CapabilityId::new(&c.id),
                name: c.name,
                description: c.description,
                status: if c.status == "available" {
                    everruns_core::CapabilityStatus::Available
                } else if c.status == "coming_soon" {
                    everruns_core::CapabilityStatus::ComingSoon
                } else {
                    everruns_core::CapabilityStatus::Deprecated
                },
                icon: c.icon,
                category: c.category,
                system_prompt: None,
                tool_definitions: vec![], // Tool definitions not sent over gRPC (tool names suffice for listing)
                is_mcp: c.is_mcp,
                is_skill: c.is_skill,
                dependencies: c.dependencies,
                features: vec![],
                risk_level: everruns_core::RiskLevel::Low,
            })
            .collect();

        Ok(caps)
    }

    // =========================================================================
    // UI Links
    // =========================================================================

    fn base_url(&self) -> &str {
        // Cache the base_url as a leaked static to satisfy the &str lifetime
        // Called infrequently, value stable across runtime
        static BASE_URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        BASE_URL.get_or_init(|| {
            std::env::var("PUBLIC_APP_URL")
                .or_else(|_| std::env::var("APP_URL"))
                .unwrap_or_else(|_| "http://localhost:9300".to_string())
        })
    }
}

// ============================================================================
// GrpcSessionSqlDbStore - SessionSqlDbStore implementation over gRPC
// ============================================================================

use everruns_core::session_sqldb::{
    ColumnSchema, DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult,
    SqlQueryResult, TableSchema,
};
/// Alias std::result::Result to avoid shadowing by everruns_core::error::Result.
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
// BudgetChecker — check budget status from check_budget tool
// ============================================================================

#[async_trait]
impl everruns_core::traits::BudgetChecker for GrpcBudgetChecker {
    async fn check_budgets(
        &self,
        session_id: &str,
    ) -> everruns_core::error::Result<everruns_core::budget::BudgetToolResponse> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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
}
