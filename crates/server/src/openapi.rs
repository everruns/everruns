// OpenAPI specification generation
//
// This module defines the OpenAPI spec for the Everruns API.
// It can be used by both the main API server (for Swagger UI)
// and the export-openapi binary (for static spec generation).

use crate::api;
use crate::api::{ListResponse, PaginatedResponse};
use everruns_core::llm_models::LlmProvider;
use everruns_core::{
    Agent, AgentStatus, CapabilityInfo, Event, EventContext, EventData, FileInfo, FileStat,
    GrepMatch, GrepResult, LlmModel, LlmModelStatus, LlmModelWithProvider, LlmProviderStatus,
    LlmProviderType, McpServer, McpServerStatus, McpServerTransportType, Session, SessionFile,
    SessionStatus, Skill, SkillContent, SkillFileEntry, SkillSourceType, SkillStatus,
    SkillValidationResult, ToolCall,
    events::{
        ActCompletedData, ActStartedData, InputMessageData, LlmGenerationData,
        LlmGenerationMetadata, LlmGenerationOutput, ModelMetadata, OutputMessageCompletedData,
        OutputMessageDeltaData, OutputMessageStartedData, ReasonCompletedData, ReasonStartedData,
        SessionStartedData, TokenUsage, ToolCallSummary, ToolCompletedData, ToolStartedData,
        TurnCompletedData, TurnFailedData, TurnStartedData,
    },
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
        api::agents::export_agent,
        api::agents::import_agent,
        api::sessions::create_session,
        api::sessions::list_sessions,
        api::sessions::get_session,
        api::sessions::update_session,
        api::sessions::delete_session,
        api::sessions::cancel_turn,
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
        api::mcp_servers::create_mcp_server,
        api::mcp_servers::list_mcp_servers,
        api::mcp_servers::get_mcp_server,
        api::mcp_servers::update_mcp_server,
        api::mcp_servers::delete_mcp_server,
        // Durable schedules
        api::schedules::create_schedule,
        api::schedules::list_schedules,
        api::schedules::get_schedule,
        api::schedules::update_schedule,
        api::schedules::delete_schedule,
        api::schedules::pause_schedule,
        api::schedules::resume_schedule,
        api::schedules::trigger_schedule,
        api::schedules::list_schedule_executions,
        api::schedules::get_execution,
        api::schedules::get_schedule_stats,
        // Agents - additional
        api::agents::preview_agent,
        api::agents::upsert_agent,
        // LLM Providers - additional
        api::llm_providers::sync_models,
        // Users - additional
        api::users::switch_org,
        // Organizations
        api::organizations::create_organization,
        api::organizations::list_organizations,
        api::organizations::get_organization,
        api::organizations::update_organization,
        // Images
        api::images::upload_image,
        api::images::list_images,
        api::images::get_image,
        api::images::delete_image,
        api::images::get_thumbnail,
        // Session Databases
        api::session_databases::create_database,
        api::session_databases::list_databases,
        api::session_databases::get_database,
        api::session_databases::delete_database,
        api::session_databases::get_schema,
        // Session Storage
        api::session_storage::list_keys,
        api::session_storage::list_secrets,
        // Client-side tool results
        api::tool_results::submit_tool_results,
        // Skills
        api::skills::create_skill,
        api::skills::upload_skill,
        api::skills::list_skills,
        api::skills::get_skill,
        api::skills::get_skill_content,
        api::skills::update_skill,
        api::skills::delete_skill,
        api::skills::validate_skill,
    ),
    components(
        schemas(
            Agent, AgentStatus,
            Session, SessionStatus, Event, EventContext, EventData,
            // Event data types
            InputMessageData, OutputMessageStartedData, OutputMessageDeltaData, OutputMessageCompletedData,
            ModelMetadata, TokenUsage,
            TurnStartedData, TurnCompletedData, TurnFailedData,
            ReasonStartedData, ReasonCompletedData,
            ActStartedData, ActCompletedData, ToolCallSummary,
            ToolStartedData, ToolCompletedData,
            LlmGenerationData, LlmGenerationOutput, LlmGenerationMetadata,
            SessionStartedData,
            // Agent/Session types
            api::agents::CreateAgentRequest, api::agents::UpdateAgentRequest,
            api::sessions::CreateSessionRequest, api::sessions::UpdateSessionRequest,
            api::sessions::CancelTurnResponse, api::sessions::CancelStatus,
            api::messages::Message, api::messages::MessageRole, api::messages::ContentPart, api::messages::InputContentPart,
            api::messages::CreateMessageRequest, api::messages::InputMessage,
            api::messages::Controls, api::messages::ReasoningConfig,
            ListResponse<Agent>,
            PaginatedResponse<Session>,
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
            everruns_core::ToolDefinition, everruns_core::BuiltinTool, everruns_core::ClientSideTool,
            everruns_core::ToolPolicy,
            everruns_core::ToolCallRequestedData,
            api::tool_results::SubmitToolResultsRequest,
            api::tool_results::ClientToolResult,
            api::tool_results::SubmitToolResultsResponse,
            // MCP Server types
            McpServer, McpServerStatus, McpServerTransportType,
            api::mcp_servers::CreateMcpServerRequest,
            api::mcp_servers::UpdateMcpServerRequest,
            ListResponse<McpServer>,
            // Schedule types
            api::schedules::ScheduleTarget,
            api::schedules::CreateScheduleRequest,
            api::schedules::UpdateScheduleRequest,
            api::schedules::ScheduleResponse,
            api::schedules::ScheduleTargetResponse,
            api::schedules::SchedulesListResponse,
            api::schedules::ScheduleExecutionResponse,
            api::schedules::ScheduleExecutionsListResponse,
            api::schedules::ScheduleStatsResponse,
            api::schedules::TriggerResponse,
            api::schedules::ListSchedulesQuery,
            api::schedules::ListExecutionsQuery,
            // Skill types
            Skill, SkillSourceType, SkillStatus, SkillContent, SkillFileEntry,
            SkillValidationResult,
            api::skills::CreateSkillRequest,
            api::skills::UpdateSkillRequest,
            api::skills::ValidateSkillRequest,
            ListResponse<Skill>,
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
        (name = "filesystem", description = "Session virtual filesystem endpoints"),
        (name = "mcp-servers", description = "MCP Server management endpoints"),
        (name = "durable-schedules", description = "Durable scheduled tasks management endpoints"),
        (name = "organizations", description = "Organization management endpoints"),
        (name = "images", description = "Image upload and management endpoints"),
        (name = "session-databases", description = "Session-scoped SQL database endpoints"),
        (name = "session-storage", description = "Session key-value storage endpoints"),
        (name = "skills", description = "Skills registry endpoints"),
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
