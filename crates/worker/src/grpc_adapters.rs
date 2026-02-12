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
use tonic::transport::Channel;
use uuid::Uuid;

// Helper to create store errors for gRPC operations
fn grpc_error(msg: impl Into<String>) -> AgentLoopError {
    AgentLoopError::store(msg)
}

/// gRPC client wrapper for worker operations
#[derive(Clone)]
pub struct GrpcClient {
    inner: Arc<Mutex<WorkerServiceClient<Channel>>>,
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

        // Configure client with larger message size for image resolution
        let client = WorkerServiceClient::new(channel)
            .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE);

        Ok(Self {
            inner: Arc::new(Mutex::new(client)),
        })
    }

    /// Create from an existing channel
    pub fn from_channel(channel: Channel) -> Self {
        let client = WorkerServiceClient::new(channel)
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

        let request = proto::GetDefaultModelRequest {};

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
    use everruns_core::tool_types::{BuiltinTool, ToolDefinition, ToolPolicy};

    // Convert proto Struct to serde_json::Value
    let parameters = proto_tool
        .parameters
        .map(|s| proto_struct_to_json(&s))
        .unwrap_or_else(|| serde_json::json!({"type": "object"}));

    ToolDefinition::Builtin(BuiltinTool {
        name: proto_tool.name,
        description: proto_tool.description,
        parameters,
        policy: ToolPolicy::Auto, // MCP tools are auto-executed
    })
}

// ============================================================================
// ImageResolver implementation
// ============================================================================

use everruns_core::traits::{ImageResolver, ResolvedImage};
use std::collections::HashMap;

/// gRPC-backed image resolver for resolving image_file content parts
///
/// This is used by ReasonAtom to resolve image_file references to actual
/// image data before sending messages to LLM providers.
pub struct GrpcImageResolver {
    client: GrpcClient,
}

impl GrpcImageResolver {
    /// Create a new GrpcImageResolver
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
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
