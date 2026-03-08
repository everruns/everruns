// gRPC-backed adapters for core traits
//
// Decision: Workers communicate with control plane via gRPC for all operations
// Decision: This replaces direct database access in worker crates
//
// These implementations use the internal-protocol gRPC client to communicate
// with the control-plane service (the API server's gRPC endpoint).

use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{Event, EventRequest};
use everruns_core::message_retriever::{InputMessage, MessageRetriever};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, LlmProviderStore, ModelWithProvider, SessionFileStore,
    SessionStore,
};
use everruns_core::typed_id::{AgentId, MessageId, ModelId, SessionId};
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

// Helper to create store errors for gRPC operations
fn grpc_error(msg: impl Into<String>) -> AgentLoopError {
    AgentLoopError::store(msg)
}

/// gRPC client wrapper for worker operations
#[derive(Clone)]
pub struct GrpcClient {
    inner: Arc<Mutex<WorkerServiceClient<InterceptedService<Channel, GrpcClientAuth>>>>,
}

/// Max gRPC message size (150MB for base64-encoded images + overhead)
///
/// TODO: Sending large images over gRPC is inefficient. Future improvements:
/// - Use presigned URLs to fetch images directly from storage
/// - Stream images in chunks instead of single large messages
/// - Move to S3/blob storage with direct worker access
const MAX_GRPC_MESSAGE_SIZE: usize = 150 * 1024 * 1024;

impl GrpcClient {
    /// Connect to the control plane gRPC server
    pub async fn connect(addr: &str) -> Result<Self> {
        let endpoint = format!("http://{}", addr);
        let channel = tonic::transport::Endpoint::from_shared(endpoint)
            .map_err(|e| grpc_error(format!("Invalid gRPC endpoint: {}", e)))?
            .connect()
            .await
            .map_err(|e| grpc_error(format!("gRPC connection failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("Failed to set session status: {}", e)))?;

        let proto_session = response
            .into_inner()
            .session
            .ok_or_else(|| grpc_error("No session in response"))?;

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
            .map_err(|e| grpc_error(format!("Failed to set session title: {}", e)))?;

        let proto_session = response
            .into_inner()
            .session
            .ok_or_else(|| grpc_error("No session in response"))?;

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
            .map_err(|e| grpc_error(format!("Failed to get MCP server: {}", e)))?;

        let proto_server = response.into_inner().server.ok_or_else(|| {
            grpc_error(format!(
                "MCP server not found for prefix: {}",
                server_prefix
            ))
        })?;

        Ok(crate::mcp_executor::McpServerInfo {
            id: proto_uuid_to_uuid(proto_server.id.as_ref())?,
            name: proto_server.name,
            url: proto_server.url,
            api_key: proto_server.api_key,
            headers: proto_server.headers,
        })
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
        .ok_or_else(|| grpc_error("Missing UUID in response"))?;
    Uuid::parse_str(uuid_str).map_err(|e| grpc_error(format!("Invalid UUID: {}", e)))
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

/// gRPC-backed message retriever
///
/// Retrieves conversation messages via gRPC from the control-plane.
/// Message storage is handled via EventEmitter (messages are stored as events).
pub struct GrpcMessageRetriever {
    client: GrpcClient,
}

impl GrpcMessageRetriever {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }

    /// Add a new message via gRPC
    ///
    /// Note: This is provided for API layer convenience.
    /// Messages are stored via gRPC call to control-plane.
    pub async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        let mut client = self.client.inner.lock().await;

        // Convert content to prost ListValue
        let content_json = serde_json::to_value(&input.content)
            .map_err(|e| grpc_error(format!("JSON serialization failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC add_message failed: {}", e)))?;

        let proto_msg = response
            .into_inner()
            .message
            .ok_or_else(|| grpc_error("No message in response"))?;

        proto_message_to_message(proto_msg)
    }
}

#[async_trait]
impl MessageRetriever for GrpcMessageRetriever {
    async fn get(&self, session_id: SessionId, message_id: MessageId) -> Result<Option<Message>> {
        // Load all messages and find the one we want
        // TODO: Add a specific get_message RPC
        let messages = self.load(session_id).await?;
        Ok(messages.into_iter().find(|m| m.id == message_id))
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<Message>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::LoadMessagesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
        };

        let response = client
            .load_messages(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC load_messages failed: {}", e)))?;

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
        .map_err(|e| grpc_error(format!("Failed to parse message content: {}", e)))?;

    // Convert prost Struct to Controls
    let controls: Option<everruns_core::Controls> = proto_msg
        .controls
        .as_ref()
        .map(|s| serde_json::from_value(proto_struct_to_json(s)))
        .transpose()
        .map_err(|e| grpc_error(format!("Failed to parse message controls: {}", e)))?;

    // Convert prost Struct to metadata
    let metadata: Option<std::collections::HashMap<String, serde_json::Value>> = proto_msg
        .metadata
        .as_ref()
        .map(|s| serde_json::from_value(proto_struct_to_json(s)))
        .transpose()
        .map_err(|e| grpc_error(format!("Failed to parse message metadata: {}", e)))?;

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

/// gRPC-backed agent store
pub struct GrpcAgentStore {
    client: GrpcClient,
    org_id: i64,
}

impl GrpcAgentStore {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self { client, org_id }
    }
}

#[async_trait]
impl AgentStore for GrpcAgentStore {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetAgentRequest {
            agent_id: Some(uuid_to_proto(agent_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_agent(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC get_agent failed: {}", e)))?;

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
        _ => everruns_core::AgentStatus::Active,
    };

    Ok(Agent {
        public_id: everruns_core::AgentId::from_uuid(id),
        internal_id: id,
        name: proto_agent.name,
        description: non_empty_string(proto_agent.description),
        system_prompt: proto_agent.system_prompt,
        default_model_id: default_model_id.map(|u| u.into()),
        tags: vec![],
        capabilities: proto_agent
            .capability_ids
            .into_iter()
            .map(everruns_core::AgentCapabilityConfig::new)
            .collect(),
        tools: vec![],
        status,
        created_at: proto_timestamp_or_now(proto_agent.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(proto_agent.updated_at.as_ref()),
        usage: None, // Usage not tracked in worker context
    })
}

// ============================================================================
// HarnessStore implementation
// ============================================================================

/// gRPC-backed harness store
pub struct GrpcHarnessStore {
    client: GrpcClient,
    org_id: i64,
}

impl GrpcHarnessStore {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self { client, org_id }
    }
}

#[async_trait]
impl HarnessStore for GrpcHarnessStore {
    async fn get_harness(&self, harness_id: everruns_core::HarnessId) -> Result<Option<Harness>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetHarnessRequest {
            harness_id: Some(uuid_to_proto(harness_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_harness(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC get_harness failed: {}", e)))?;

        match response.into_inner().harness {
            Some(proto_harness) => {
                let harness = proto_harness_to_harness(proto_harness)?;
                Ok(Some(harness))
            }
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

    let status = match proto_harness.status.to_lowercase().as_str() {
        "active" => HarnessStatus::Active,
        "archived" => HarnessStatus::Archived,
        _ => HarnessStatus::Active,
    };

    Ok(Harness {
        id: id.into(),
        name: proto_harness.name,
        description: non_empty_string(proto_harness.description),
        system_prompt: proto_harness.system_prompt,
        default_model_id: default_model_id.map(|u| u.into()),
        tags: vec![],
        capabilities: proto_harness
            .capability_ids
            .into_iter()
            .map(everruns_core::AgentCapabilityConfig::new)
            .collect(),
        status,
        created_at: proto_timestamp_or_now(proto_harness.created_at.as_ref()),
        updated_at: proto_timestamp_or_now(proto_harness.updated_at.as_ref()),
    })
}

// ============================================================================
// SessionStore implementation
// ============================================================================

/// gRPC-backed session store
pub struct GrpcSessionStore {
    client: GrpcClient,
    org_id: i64,
}

impl GrpcSessionStore {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self { client, org_id }
    }
}

#[async_trait]
impl SessionStore for GrpcSessionStore {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetSessionRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            org_id: self.org_id,
        };

        let response = client
            .get_session(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC get_session failed: {}", e)))?;

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
        harness_id: harness_id.into(),
        title: non_empty_string(proto_session.title),
        preview: None,
        output_preview: None,
        tags: vec![],
        model_id: model_id.map(|u| u.into()),
        capabilities,
        tools: vec![],
        status,
        created_at,
        updated_at,
        started_at: None,
        finished_at: None,
        usage: None, // Usage not tracked in worker context
        is_pinned: None,
        active_schedule_count: None,
        features: vec![], // Computed at API read time, not in worker
    })
}

// ============================================================================
// LlmProviderStore implementation
// ============================================================================

/// gRPC-backed LLM provider store
pub struct GrpcLlmProviderStore {
    client: GrpcClient,
    org_id: i64,
}

impl GrpcLlmProviderStore {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self { client, org_id }
    }
}

#[async_trait]
impl LlmProviderStore for GrpcLlmProviderStore {
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
            .map_err(|e| grpc_error(format!("gRPC get_model_with_provider failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("gRPC get_default_model failed: {}", e)))?;

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
            return Err(grpc_error(format!(
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

/// gRPC-backed session file store
pub struct GrpcSessionFileStore {
    client: GrpcClient,
}

impl GrpcSessionFileStore {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl SessionFileStore for GrpcSessionFileStore {
    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::SessionReadFileRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            path: path.to_string(),
        };

        let response = client
            .session_read_file(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC session_read_file failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("gRPC session_write_file failed: {}", e)))?;

        let proto_file = response
            .into_inner()
            .file
            .ok_or_else(|| grpc_error("No file in response"))?;

        proto_session_file_to_file(proto_file)
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
            .map_err(|e| grpc_error(format!("gRPC session_delete_file failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("gRPC session_list_directory failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("gRPC session_stat_file failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("gRPC session_grep_files failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("gRPC session_create_directory failed: {}", e)))?;

        let proto_info = response
            .into_inner()
            .directory
            .ok_or_else(|| grpc_error("No directory info in response"))?;

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

/// gRPC-backed event emitter
pub struct GrpcEventEmitter {
    client: GrpcClient,
}

impl GrpcEventEmitter {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl EventEmitter for GrpcEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let mut client = self.client.inner.lock().await;

        // Convert core EventRequest to proto EventRequest
        let proto_event_request = core_event_request_to_proto(&request)?;

        let grpc_request = proto::EmitEventRequest {
            event: Some(proto_event_request),
        };

        let response = client
            .emit_event(grpc_request)
            .await
            .map_err(|e| grpc_error(format!("gRPC emit_event failed: {}", e)))?;

        // Convert proto Event response back to core Event
        let proto_event = response
            .into_inner()
            .event
            .ok_or_else(|| grpc_error("No event in response"))?;

        proto_event_to_core(proto_event)
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
        .map_err(|e| grpc_error(format!("Failed to convert proto event: {}", e)))
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
        .map_err(|e| grpc_error(format!("gRPC get_turn_context failed: {}", e)))?;

    let inner = response.into_inner();

    let agent = inner.agent.map(proto_agent_to_agent).transpose()?;
    let proto_session = inner
        .session
        .ok_or_else(|| grpc_error("No session in turn context"))?;

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
    })
}

// ============================================================================
// ImageResolver implementation
// ============================================================================

use everruns_core::traits::{
    ImageResolver, KeyInfo, ResolvedImage, SecretInfo, SessionStorageStore,
};
use std::collections::HashMap;

/// gRPC-backed image resolver for resolving image_file content parts
///
/// This is used by ReasonAtom to resolve image_file references to actual
/// image data before sending messages to LLM providers.
pub struct GrpcImageResolver {
    client: GrpcClient,
    org_id: i64,
}

impl GrpcImageResolver {
    /// Create a new GrpcImageResolver scoped to an organization
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self { client, org_id }
    }

    /// Resolve multiple images in a batch (more efficient)
    ///
    /// Returns a HashMap mapping image_id to ResolvedImage for all found images.
    /// Missing images are silently skipped.
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
            .map_err(|e| grpc_error(format!("gRPC resolve_images failed: {}", e)))?;

        let mut result = HashMap::new();
        for (id_str, data) in response.into_inner().images {
            if let Ok(id) = Uuid::parse_str(&id_str) {
                result.insert(id, ResolvedImage::new(data.base64, data.media_type));
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl ImageResolver for GrpcImageResolver {
    /// Resolve a single image by ID
    ///
    /// Returns the base64-encoded image data and media type, or None if not found.
    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::ResolveImageRequest {
            image_id: Some(uuid_to_proto(image_id)),
            org_id: self.org_id,
        };

        let response = client
            .resolve_image(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC resolve_image failed: {}", e)))?;

        let inner = response.into_inner();

        if inner.found {
            Ok(Some(ResolvedImage::new(inner.base64, inner.media_type)))
        } else {
            Ok(None)
        }
    }
}

// ============================================================================
// SessionStorageStore implementation
// ============================================================================

/// gRPC-backed session storage store for key/value and secret operations
pub struct GrpcSessionStorageStore {
    client: GrpcClient,
}

impl GrpcSessionStorageStore {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl SessionStorageStore for GrpcSessionStorageStore {
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
            .map_err(|e| grpc_error(format!("gRPC session_storage_set_value failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC session_storage_get_value failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC session_storage_delete_value failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC session_storage_list_keys failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC session_storage_set_secret failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC session_storage_get_secret failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC session_storage_delete_secret failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC session_storage_list_secrets failed: {}", e)))?;
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

/// gRPC-backed user connection resolver.
///
/// Proxies `get_connection_token` calls to the control-plane which has access
/// to encrypted tokens and GitHub App credentials.
pub struct GrpcConnectionResolver {
    client: GrpcClient,
}

impl GrpcConnectionResolver {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl everruns_core::traits::UserConnectionResolver for GrpcConnectionResolver {
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
            .map_err(|e| grpc_error(format!("gRPC get_connection_token failed: {}", e)))?;
        Ok(response.into_inner().token)
    }
}

// ============================================================================
// GrpcScheduleStore - SessionScheduleStore over gRPC
// ============================================================================

/// gRPC-backed session schedule store.
///
/// Proxies schedule CRUD to the control-plane via gRPC RPCs.
pub struct GrpcScheduleStore {
    client: GrpcClient,
    org_id: i64,
}

impl GrpcScheduleStore {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self { client, org_id }
    }
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
impl everruns_core::traits::SessionScheduleStore for GrpcScheduleStore {
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
            .map_err(|e| grpc_error(format!("gRPC create_session_schedule failed: {}", e)))?;
        let proto_schedule = response
            .into_inner()
            .schedule
            .ok_or_else(|| grpc_error("No schedule in response"))?;
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
            .map_err(|e| grpc_error(format!("gRPC cancel_session_schedule failed: {}", e)))?;
        let proto_schedule = response
            .into_inner()
            .schedule
            .ok_or_else(|| grpc_error("No schedule in response"))?;
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
            .map_err(|e| grpc_error(format!("gRPC list_session_schedules failed: {}", e)))?;
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
            .map_err(|e| {
                grpc_error(format!("gRPC count_active_session_schedules failed: {}", e))
            })?;
        Ok(response.into_inner().count)
    }
}

// ============================================================================
// GrpcPlatformStore - PlatformStore implementation over gRPC
// ============================================================================

/// gRPC-backed platform store for org-scoped management operations.
///
/// Proxies all PlatformStore trait methods to the control-plane via gRPC RPCs.
/// Used by gRPC workers to support the platform_management capability.
pub struct GrpcPlatformStore {
    client: GrpcClient,
    org_id: i64,
}

impl GrpcPlatformStore {
    pub fn new(client: GrpcClient, org_id: i64) -> Self {
        Self { client, org_id }
    }
}

#[async_trait]
impl everruns_core::platform_store::PlatformStore for GrpcPlatformStore {
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
            .map_err(|e| grpc_error(format!("gRPC platform_list_harnesses failed: {}", e)))?;

        response
            .into_inner()
            .harnesses
            .into_iter()
            .map(|h| {
                everruns_internal_protocol::proto_harness_to_schema(h)
                    .map_err(|e| grpc_error(format!("Harness conversion failed: {}", e)))
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
            .map_err(|e| grpc_error(format!("gRPC get_harness failed: {}", e)))?;

        match response.into_inner().harness {
            Some(h) => Ok(Some(
                everruns_internal_protocol::proto_harness_to_schema(h)
                    .map_err(|e| grpc_error(format!("Harness conversion failed: {}", e)))?,
            )),
            None => Ok(None),
        }
    }

    async fn create_harness(
        &self,
        name: &str,
        description: Option<&str>,
        system_prompt: &str,
        capabilities: &[String],
    ) -> Result<Harness> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_create_harness(proto::PlatformCreateHarnessRequest {
                org_id: self.org_id,
                name: name.to_string(),
                description: description.map(|s| s.to_string()),
                system_prompt: system_prompt.to_string(),
                capabilities: capabilities.to_vec(),
            })
            .await
            .map_err(|e| grpc_error(format!("gRPC platform_create_harness failed: {}", e)))?;

        let harness = response
            .into_inner()
            .harness
            .ok_or_else(|| grpc_error("No harness in create response"))?;

        everruns_internal_protocol::proto_harness_to_schema(harness)
            .map_err(|e| grpc_error(format!("Harness conversion failed: {}", e)))
    }

    async fn update_harness(
        &self,
        id: everruns_core::HarnessId,
        name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<Harness> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_update_harness(proto::PlatformUpdateHarnessRequest {
                org_id: self.org_id,
                harness_id: Some(uuid_to_proto(id.uuid())),
                name: name.map(|s| s.to_string()),
                description: description.map(|s| s.to_string()),
                system_prompt: system_prompt.map(|s| s.to_string()),
            })
            .await
            .map_err(|e| grpc_error(format!("gRPC platform_update_harness failed: {}", e)))?;

        let harness = response
            .into_inner()
            .harness
            .ok_or_else(|| grpc_error("No harness in update response"))?;

        everruns_internal_protocol::proto_harness_to_schema(harness)
            .map_err(|e| grpc_error(format!("Harness conversion failed: {}", e)))
    }

    async fn delete_harness(&self, id: everruns_core::HarnessId) -> Result<()> {
        let mut client = self.client.inner.lock().await;
        client
            .platform_delete_harness(proto::PlatformDeleteHarnessRequest {
                org_id: self.org_id,
                harness_id: Some(uuid_to_proto(id.uuid())),
            })
            .await
            .map_err(|e| grpc_error(format!("gRPC platform_delete_harness failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC platform_copy_harness failed: {}", e)))?;

        let harness = response
            .into_inner()
            .harness
            .ok_or_else(|| grpc_error("No harness in copy response"))?;

        everruns_internal_protocol::proto_harness_to_schema(harness)
            .map_err(|e| grpc_error(format!("Harness conversion failed: {}", e)))
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
            .map_err(|e| grpc_error(format!("gRPC platform_list_agents failed: {}", e)))?;

        response
            .into_inner()
            .agents
            .into_iter()
            .map(|a| {
                everruns_internal_protocol::proto_agent_to_schema(a)
                    .map_err(|e| grpc_error(format!("Agent conversion failed: {}", e)))
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
            .map_err(|e| grpc_error(format!("gRPC get_agent failed: {}", e)))?;

        match response.into_inner().agent {
            Some(a) => Ok(Some(
                everruns_internal_protocol::proto_agent_to_schema(a)
                    .map_err(|e| grpc_error(format!("Agent conversion failed: {}", e)))?,
            )),
            None => Ok(None),
        }
    }

    async fn create_agent(
        &self,
        name: &str,
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
            })
            .await
            .map_err(|e| grpc_error(format!("gRPC platform_create_agent failed: {}", e)))?;

        let agent = response
            .into_inner()
            .agent
            .ok_or_else(|| grpc_error("No agent in create response"))?;

        everruns_internal_protocol::proto_agent_to_schema(agent)
            .map_err(|e| grpc_error(format!("Agent conversion failed: {}", e)))
    }

    async fn update_agent(
        &self,
        id: AgentId,
        name: Option<&str>,
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
            })
            .await
            .map_err(|e| grpc_error(format!("gRPC platform_update_agent failed: {}", e)))?;

        let agent = response
            .into_inner()
            .agent
            .ok_or_else(|| grpc_error("No agent in update response"))?;

        everruns_internal_protocol::proto_agent_to_schema(agent)
            .map_err(|e| grpc_error(format!("Agent conversion failed: {}", e)))
    }

    async fn delete_agent(&self, id: AgentId) -> Result<()> {
        let mut client = self.client.inner.lock().await;
        client
            .platform_delete_agent(proto::PlatformDeleteAgentRequest {
                org_id: self.org_id,
                agent_id: Some(uuid_to_proto(id.uuid())),
            })
            .await
            .map_err(|e| grpc_error(format!("gRPC platform_delete_agent failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC platform_list_sessions failed: {}", e)))?;

        response
            .into_inner()
            .sessions
            .into_iter()
            .map(|s| {
                everruns_internal_protocol::proto_session_to_schema(s)
                    .map_err(|e| grpc_error(format!("Session conversion failed: {}", e)))
            })
            .collect()
    }

    async fn create_session(
        &self,
        harness_id: everruns_core::HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
    ) -> Result<Session> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .platform_create_session(proto::PlatformCreateSessionRequest {
                org_id: self.org_id,
                harness_id: Some(uuid_to_proto(harness_id.uuid())),
                agent_id: agent_id.map(|id| uuid_to_proto(id.uuid())),
                title: title.map(|s| s.to_string()),
            })
            .await
            .map_err(|e| grpc_error(format!("gRPC platform_create_session failed: {}", e)))?;

        let session = response
            .into_inner()
            .session
            .ok_or_else(|| grpc_error("No session in create response"))?;

        everruns_internal_protocol::proto_session_to_schema(session)
            .map_err(|e| grpc_error(format!("Session conversion failed: {}", e)))
    }

    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>> {
        let mut client = self.client.inner.lock().await;
        let response = client
            .get_session(proto::GetSessionRequest {
                session_id: Some(uuid_to_proto(id.uuid())),
                org_id: self.org_id,
            })
            .await
            .map_err(|e| grpc_error(format!("gRPC get_session failed: {}", e)))?;

        match response.into_inner().session {
            Some(s) => Ok(Some(
                everruns_internal_protocol::proto_session_to_schema(s)
                    .map_err(|e| grpc_error(format!("Session conversion failed: {}", e)))?,
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
            .map_err(|e| grpc_error(format!("gRPC platform_delete_session failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC platform_send_message failed: {}", e)))?;
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
            .map_err(|e| grpc_error(format!("gRPC platform_get_messages failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("gRPC platform_wait_for_idle failed: {}", e)))?;

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
            .map_err(|e| grpc_error(format!("gRPC platform_list_capabilities failed: {}", e)))?;

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

/// gRPC-backed session SQL database store.
///
/// Proxies all SessionSqlDbStore trait methods to the control-plane via gRPC RPCs.
pub struct GrpcSessionSqlDbStore {
    client: GrpcClient,
}

impl GrpcSessionSqlDbStore {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

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
impl SessionSqlDbStore for GrpcSessionSqlDbStore {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
