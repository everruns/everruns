// OpenAPI specification generation
//
// This module defines the OpenAPI spec for the Everruns API.
// It can be used by both the main API server (for Swagger UI)
// and the export-openapi binary (for static spec generation).

use crate::api;
use crate::api::ListResponse;
use everruns_core::llm_models::LlmProvider;
use everruns_core::{
    events::{
        ActCompletedData, ActStartedData, InputReceivedData, LlmGenerationData,
        LlmGenerationMetadata, LlmGenerationOutput, MessageAgentData, MessageUserData,
        ModelMetadata, ReasonCompletedData, ReasonStartedData, SessionStartedData, TokenUsage,
        ToolCallCompletedData, ToolCallStartedData, ToolCallSummary, TurnCompletedData,
        TurnFailedData, TurnStartedData,
    },
    Agent, AgentStatus, CapabilityInfo, Event, EventContext, EventData, FileInfo, FileStat,
    GrepMatch, GrepResult, LlmModel, LlmModelStatus, LlmModelWithProvider, LlmProviderStatus,
    LlmProviderType, Session, SessionFile, SessionStatus, ToolCall,
};
use utoipa::OpenApi;

/// OpenAPI documentation for the Everruns API
#[derive(OpenApi)]
#[openapi(
    servers(
        (url = "https://app.everruns.com/api", description = "Production API"),
    ),
    paths(
        api::agents::create_agent,
        api::agents::list_agents,
        api::agents::get_agent,
        api::agents::update_agent,
        api::agents::delete_agent,
        api::sessions::create_session,
        api::sessions::list_sessions,
        api::sessions::get_session,
        api::sessions::update_session,
        api::sessions::delete_session,
        api::messages::create_message,
        api::messages::list_messages,
        api::events::stream_sse,
        api::events::list_events,
        api::llm_providers::create_provider,
        api::llm_providers::list_providers,
        api::llm_providers::get_provider,
        api::llm_providers::update_provider,
        api::llm_providers::delete_provider,
        api::llm_models::create_model,
        api::llm_models::list_provider_models,
        api::llm_models::list_all_models,
        api::llm_models::get_model,
        api::llm_models::update_model,
        api::llm_models::delete_model,
        api::capabilities::list_capabilities,
        api::capabilities::get_capability,
        api::users::list_users,
        api::session_files::get_root,
        api::session_files::get_path,
        api::session_files::create_path,
        api::session_files::update_path,
        api::session_files::delete_path,
        api::session_files::move_file,
        api::session_files::copy_file,
        api::session_files::grep_files,
        api::session_files::stat_file,
    ),
    components(
        schemas(
            Agent, AgentStatus,
            Session, SessionStatus, Event, EventContext, EventData,
            // Event data types
            MessageUserData, MessageAgentData, ModelMetadata, TokenUsage,
            TurnStartedData, TurnCompletedData, TurnFailedData,
            InputReceivedData, ReasonStartedData, ReasonCompletedData,
            ActStartedData, ActCompletedData, ToolCallSummary,
            ToolCallStartedData, ToolCallCompletedData,
            LlmGenerationData, LlmGenerationOutput, LlmGenerationMetadata,
            SessionStartedData,
            // Agent/Session types
            api::agents::CreateAgentRequest, api::agents::UpdateAgentRequest,
            api::sessions::CreateSessionRequest, api::sessions::UpdateSessionRequest,
            api::messages::Message, api::messages::MessageRole, api::messages::ContentPart, api::messages::InputContentPart,
            api::messages::CreateMessageRequest, api::messages::InputMessage,
            api::messages::Controls, api::messages::ReasoningConfig,
            ListResponse<Agent>,
            ListResponse<Session>,
            ListResponse<api::messages::Message>,
            ListResponse<Event>,
            LlmProvider, LlmProviderType, LlmProviderStatus,
            LlmModel, LlmModelWithProvider, LlmModelStatus,
            api::llm_providers::CreateLlmProviderRequest,
            api::llm_providers::UpdateLlmProviderRequest,
            api::llm_models::CreateLlmModelRequest,
            api::llm_models::UpdateLlmModelRequest,
            CapabilityInfo,
            ListResponse<CapabilityInfo>,
            api::users::User,
            api::users::ListUsersQuery,
            ListResponse<api::users::User>,
            SessionFile, FileInfo, FileStat, GrepMatch, GrepResult,
            api::session_files::CreateFileRequest, api::session_files::UpdateFileRequest,
            api::session_files::MoveFileRequest, api::session_files::CopyFileRequest,
            api::session_files::GrepRequest, api::session_files::DeleteResponse,
            api::session_files::GetQuery, api::session_files::DeleteQuery, api::session_files::GetResponse,
            ListResponse<FileInfo>,
            ListResponse<GrepResult>,
            // Tool types
            ToolCall,
        )
    ),
    tags(
        (name = "agents", description = "Agent management endpoints"),
        (name = "sessions", description = "Session management endpoints"),
        (name = "messages", description = "Message management endpoints"),
        (name = "events", description = "Event streaming endpoints (SSE)"),
        (name = "llm-providers", description = "LLM Provider management endpoints"),
        (name = "llm-models", description = "LLM Model management endpoints"),
        (name = "capabilities", description = "Capability management endpoints"),
        (name = "users", description = "User management endpoints"),
        (name = "filesystem", description = "Session virtual filesystem endpoints")
    ),
    info(
        title = "Everruns API",
        version = "0.2.0",
        description = "API for managing AI agents, sessions, messages, and events",
        license(name = "MIT", url = "https://opensource.org/licenses/MIT")
    )
)]
pub struct ApiDoc;

impl ApiDoc {
    /// Generate the OpenAPI spec as a pretty-printed JSON string
    pub fn to_json() -> String {
        Self::openapi()
            .to_pretty_json()
            .expect("Failed to serialize OpenAPI spec")
    }
}
