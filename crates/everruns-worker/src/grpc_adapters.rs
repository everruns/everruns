// gRPC-backed adapters for core traits
//
// Decision: Workers communicate with control plane via gRPC for all operations
// Decision: This replaces direct database access in worker crates
//
// These implementations use the internal-protocol gRPC client to communicate
// with the control-plane service (the API server's gRPC endpoint).

use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::Event;
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::{
    AgentStore, EventEmitter, InputMessage, LlmProviderStore, MessageStore, ModelWithProvider,
    SessionFileStore, SessionStore,
};
use everruns_core::{Agent, Message, Session};
use everruns_internal_protocol::proto;
use everruns_internal_protocol::WorkerServiceClient;
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

impl GrpcClient {
    /// Connect to the control plane gRPC server
    pub async fn connect(addr: &str) -> Result<Self> {
        let endpoint = format!("http://{}", addr);
        let client = WorkerServiceClient::connect(endpoint)
            .await
            .map_err(|e| grpc_error(format!("gRPC connection failed: {}", e)))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(client)),
        })
    }

    /// Create from an existing channel
    pub fn from_channel(channel: Channel) -> Self {
        Self {
            inner: Arc::new(Mutex::new(WorkerServiceClient::new(channel))),
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

// ============================================================================
// MessageStore implementation
// ============================================================================

/// gRPC-backed message store
pub struct GrpcMessageStore {
    client: GrpcClient,
}

impl GrpcMessageStore {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MessageStore for GrpcMessageStore {
    async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        let mut client = self.client.inner.lock().await;

        let content_json = serde_json::to_string(&input.content)
            .map_err(|e| grpc_error(format!("JSON serialization failed: {}", e)))?;

        let controls_json = input
            .controls
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| grpc_error(format!("JSON serialization failed: {}", e)))?;

        let metadata_json = input
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| grpc_error(format!("JSON serialization failed: {}", e)))?;

        let request = proto::AddMessageRequest {
            session_id: Some(uuid_to_proto(session_id)),
            role: input.role.to_string(),
            content_json,
            controls_json,
            metadata_json,
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

    async fn get(&self, session_id: Uuid, message_id: Uuid) -> Result<Option<Message>> {
        // Load all messages and find the one we want
        // TODO: Add a specific get_message RPC
        let messages = self.load(session_id).await?;
        Ok(messages.into_iter().find(|m| m.id == message_id))
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::LoadMessagesRequest {
            session_id: Some(uuid_to_proto(session_id)),
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

    let content: Vec<everruns_core::ContentPart> = serde_json::from_str(&proto_msg.content_json)
        .map_err(|e| grpc_error(format!("Failed to parse message content: {}", e)))?;

    let controls: Option<everruns_core::Controls> = proto_msg
        .controls_json
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|j| serde_json::from_str(j))
        .transpose()
        .map_err(|e| grpc_error(format!("Failed to parse message controls: {}", e)))?;

    let metadata: Option<std::collections::HashMap<String, serde_json::Value>> = proto_msg
        .metadata_json
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|j| serde_json::from_str(j))
        .transpose()
        .map_err(|e| grpc_error(format!("Failed to parse message metadata: {}", e)))?;

    let role = match proto_msg.role.to_lowercase().as_str() {
        "system" => everruns_core::MessageRole::System,
        "user" => everruns_core::MessageRole::User,
        "assistant" => everruns_core::MessageRole::Assistant,
        "tool_result" => everruns_core::MessageRole::ToolResult,
        _ => everruns_core::MessageRole::User,
    };

    let created_at = proto_msg
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    Ok(Message {
        id,
        role,
        content,
        controls,
        metadata,
        created_at,
    })
}

// ============================================================================
// AgentStore implementation
// ============================================================================

/// gRPC-backed agent store
pub struct GrpcAgentStore {
    client: GrpcClient,
}

impl GrpcAgentStore {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl AgentStore for GrpcAgentStore {
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<Agent>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetAgentRequest {
            agent_id: Some(uuid_to_proto(agent_id)),
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

    let created_at = proto_agent
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    let updated_at = proto_agent
        .updated_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    let status = match proto_agent.status.to_lowercase().as_str() {
        "active" => everruns_core::AgentStatus::Active,
        "archived" => everruns_core::AgentStatus::Archived,
        _ => everruns_core::AgentStatus::Active,
    };

    Ok(Agent {
        id,
        name: proto_agent.name,
        description: if proto_agent.description.is_empty() {
            None
        } else {
            Some(proto_agent.description)
        },
        system_prompt: proto_agent.system_prompt,
        default_model_id,
        tags: vec![],
        capabilities: proto_agent
            .capability_ids
            .into_iter()
            .filter_map(|s| s.parse().ok())
            .collect(),
        status,
        created_at,
        updated_at,
    })
}

// ============================================================================
// SessionStore implementation
// ============================================================================

/// gRPC-backed session store
pub struct GrpcSessionStore {
    client: GrpcClient,
}

impl GrpcSessionStore {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl SessionStore for GrpcSessionStore {
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetSessionRequest {
            session_id: Some(uuid_to_proto(session_id)),
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
    let agent_id = proto_uuid_to_uuid(proto_session.agent_id.as_ref())?;
    let model_id = proto_session
        .default_model_id
        .as_ref()
        .map(|u| proto_uuid_to_uuid(Some(u)))
        .transpose()?;

    let created_at = proto_session
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    let status = match proto_session.status.to_lowercase().as_str() {
        "pending" => everruns_core::SessionStatus::Pending,
        "running" => everruns_core::SessionStatus::Running,
        "completed" => everruns_core::SessionStatus::Completed,
        "failed" => everruns_core::SessionStatus::Failed,
        _ => everruns_core::SessionStatus::Pending,
    };

    Ok(Session {
        id,
        agent_id,
        title: if proto_session.title.is_empty() {
            None
        } else {
            Some(proto_session.title)
        },
        tags: vec![],
        model_id,
        status,
        created_at,
        started_at: None,
        finished_at: None,
    })
}

// ============================================================================
// LlmProviderStore implementation
// ============================================================================

/// gRPC-backed LLM provider store
pub struct GrpcLlmProviderStore {
    client: GrpcClient,
}

impl GrpcLlmProviderStore {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl LlmProviderStore for GrpcLlmProviderStore {
    async fn get_model_with_provider(&self, model_id: Uuid) -> Result<Option<ModelWithProvider>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GetModelWithProviderRequest {
            model_id: Some(uuid_to_proto(model_id)),
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
        "anthropic" => everruns_core::LlmProviderType::Anthropic,
        "azure" | "azure_openai" => everruns_core::LlmProviderType::AzureOpenAI,
        _ => {
            return Err(grpc_error(format!(
                "Unknown provider type: {}",
                proto.provider_type
            )))
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
    async fn read_file(&self, session_id: Uuid, path: &str) -> Result<Option<SessionFile>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::ReadFileRequest {
            session_id: Some(uuid_to_proto(session_id)),
            path: path.to_string(),
        };

        let response = client
            .read_file(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC read_file failed: {}", e)))?;

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
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let mut client = self.client.inner.lock().await;

        let request = proto::WriteFileRequest {
            session_id: Some(uuid_to_proto(session_id)),
            path: path.to_string(),
            content: content.to_string(),
            encoding: encoding.to_string(),
        };

        let response = client
            .write_file(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC write_file failed: {}", e)))?;

        let proto_file = response
            .into_inner()
            .file
            .ok_or_else(|| grpc_error("No file in response"))?;

        proto_session_file_to_file(proto_file)
    }

    async fn delete_file(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool> {
        let mut client = self.client.inner.lock().await;

        let request = proto::DeleteFileRequest {
            session_id: Some(uuid_to_proto(session_id)),
            path: path.to_string(),
            recursive,
        };

        let response = client
            .delete_file(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC delete_file failed: {}", e)))?;

        Ok(response.into_inner().deleted)
    }

    async fn list_directory(&self, session_id: Uuid, path: &str) -> Result<Vec<FileInfo>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::ListDirectoryRequest {
            session_id: Some(uuid_to_proto(session_id)),
            path: path.to_string(),
        };

        let response = client
            .list_directory(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC list_directory failed: {}", e)))?;

        response
            .into_inner()
            .files
            .into_iter()
            .map(proto_file_info_to_file_info)
            .collect()
    }

    async fn stat_file(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::StatFileRequest {
            session_id: Some(uuid_to_proto(session_id)),
            path: path.to_string(),
        };

        let response = client
            .stat_file(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC stat_file failed: {}", e)))?;

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
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let mut client = self.client.inner.lock().await;

        let request = proto::GrepFilesRequest {
            session_id: Some(uuid_to_proto(session_id)),
            pattern: pattern.to_string(),
            path_pattern: path_pattern.map(|s| s.to_string()),
        };

        let response = client
            .grep_files(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC grep_files failed: {}", e)))?;

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

    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo> {
        let mut client = self.client.inner.lock().await;

        let request = proto::CreateDirectoryRequest {
            session_id: Some(uuid_to_proto(session_id)),
            path: path.to_string(),
        };

        let response = client
            .create_directory(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC create_directory failed: {}", e)))?;

        let proto_info = response
            .into_inner()
            .directory
            .ok_or_else(|| grpc_error("No directory info in response"))?;

        proto_file_info_to_file_info(proto_info)
    }
}

fn proto_session_file_to_file(proto: proto::SessionFile) -> Result<SessionFile> {
    let id = proto_uuid_to_uuid(proto.id.as_ref())?;
    let session_id = proto_uuid_to_uuid(proto.session_id.as_ref())?;

    let created_at = proto
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    let updated_at = proto
        .updated_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    Ok(SessionFile {
        id,
        session_id,
        path: proto.path,
        name: proto.name,
        content: proto.content,
        encoding: proto.encoding,
        is_directory: proto.is_directory,
        is_readonly: proto.is_readonly,
        size_bytes: proto.size_bytes,
        created_at,
        updated_at,
    })
}

fn proto_file_info_to_file_info(proto: proto::FileInfo) -> Result<FileInfo> {
    let id = proto_uuid_to_uuid(proto.id.as_ref())?;
    let session_id = proto_uuid_to_uuid(proto.session_id.as_ref())?;

    let created_at = proto
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    let updated_at = proto
        .updated_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    Ok(FileInfo {
        id,
        session_id,
        path: proto.path,
        name: proto.name,
        is_directory: proto.is_directory,
        is_readonly: proto.is_readonly,
        size_bytes: proto.size_bytes,
        created_at,
        updated_at,
    })
}

fn proto_file_stat_to_stat(proto: proto::FileStat) -> Result<FileStat> {
    let created_at = proto
        .created_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    let updated_at = proto
        .updated_at
        .as_ref()
        .map(proto_timestamp_to_datetime)
        .unwrap_or_else(chrono::Utc::now);

    Ok(FileStat {
        path: proto.path,
        name: proto.name,
        is_directory: proto.is_directory,
        is_readonly: proto.is_readonly,
        size_bytes: proto.size_bytes,
        created_at,
        updated_at,
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
    async fn emit(&self, event: Event) -> Result<i32> {
        let mut client = self.client.inner.lock().await;

        // Convert core Event to proto Event
        let proto_event = core_event_to_proto(&event)?;

        let request = proto::EmitEventRequest {
            event: Some(proto_event),
        };

        let response = client
            .emit_event(request)
            .await
            .map_err(|e| grpc_error(format!("gRPC emit_event failed: {}", e)))?;

        Ok(response.into_inner().seq)
    }
}

/// Convert everruns_core::Event to proto::Event
fn core_event_to_proto(event: &Event) -> Result<proto::Event> {
    use everruns_internal_protocol::datetime_to_proto_timestamp;

    let data_json = serde_json::to_string(&event.data)
        .map_err(|e| grpc_error(format!("Failed to serialize event data: {}", e)))?;

    Ok(proto::Event {
        id: Some(uuid_to_proto(event.id)),
        event_type: event.event_type.clone(),
        ts: Some(datetime_to_proto_timestamp(event.ts)),
        context: Some(proto::EventContext {
            session_id: Some(uuid_to_proto(event.session_id)),
            turn: event.context.turn_id.map(|u| u.as_u128() as i32),
            exec_id: event.context.exec_id.map(uuid_to_proto),
        }),
        data_json,
    })
}

// ============================================================================
// Batch context loader
// ============================================================================

/// Turn context loaded in one batched gRPC call
pub struct TurnContext {
    pub agent: Agent,
    pub session: Session,
    pub messages: Vec<Message>,
    pub model: Option<ModelWithProvider>,
}

/// Load turn context in one batched call (optimization)
///
/// This is more efficient than making separate calls for agent, session, messages.
pub async fn load_turn_context(client: &GrpcClient, session_id: Uuid) -> Result<TurnContext> {
    let mut grpc_client = client.inner.lock().await;

    let request = proto::GetTurnContextRequest {
        session_id: Some(uuid_to_proto(session_id)),
    };

    let response = grpc_client
        .get_turn_context(request)
        .await
        .map_err(|e| grpc_error(format!("gRPC get_turn_context failed: {}", e)))?;

    let inner = response.into_inner();

    let proto_agent = inner
        .agent
        .ok_or_else(|| grpc_error("No agent in turn context"))?;
    let proto_session = inner
        .session
        .ok_or_else(|| grpc_error("No session in turn context"))?;

    let agent = proto_agent_to_agent(proto_agent)?;
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

    Ok(TurnContext {
        agent,
        session,
        messages,
        model,
    })
}
