// Capability HTTP routes and internal registry
//
// Design Decision: Capabilities are external to the Agent Loop.
// The registry here defines what each capability provides (system prompt additions, tools).
// When building AgentConfig, we resolve the agent's enabled capabilities and merge their
// contributions into the final config.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use everruns_contracts::{
    AgentCapability, Capability, CapabilityId, CapabilityStatus, ListResponse,
    UpdateAgentCapabilitiesRequest,
};
use everruns_storage::Database;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::CapabilityService;

/// App state for capability routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<CapabilityService>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            service: Arc::new(CapabilityService::new(db)),
        }
    }
}

/// Create capability routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/capabilities", get(list_capabilities))
        .route("/v1/capabilities/:capability_id", get(get_capability))
        .route(
            "/v1/agents/:agent_id/capabilities",
            get(get_agent_capabilities).put(set_agent_capabilities),
        )
        .with_state(state)
}

/// GET /v1/capabilities - List all available capabilities
#[utoipa::path(
    get,
    path = "/v1/capabilities",
    responses(
        (status = 200, description = "List of available capabilities", body = ListResponse<Capability>),
    ),
    tag = "capabilities"
)]
pub async fn list_capabilities(State(state): State<AppState>) -> Json<ListResponse<Capability>> {
    let capabilities = state.service.list_all();
    Json(ListResponse::new(capabilities))
}

/// GET /v1/capabilities/{capability_id} - Get a specific capability
#[utoipa::path(
    get,
    path = "/v1/capabilities/{capability_id}",
    params(
        ("capability_id" = String, Path, description = "Capability ID")
    ),
    responses(
        (status = 200, description = "Capability found", body = Capability),
        (status = 404, description = "Capability not found"),
    ),
    tag = "capabilities"
)]
pub async fn get_capability(
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
) -> Result<Json<Capability>, StatusCode> {
    let cap_id: CapabilityId = capability_id.parse().map_err(|_| StatusCode::NOT_FOUND)?;

    let capability = state.service.get(cap_id).ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(capability))
}

/// GET /v1/agents/{agent_id}/capabilities - Get capabilities for an agent
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/capabilities",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "Agent capabilities", body = ListResponse<AgentCapability>),
        (status = 500, description = "Internal server error"),
    ),
    tag = "capabilities"
)]
pub async fn get_agent_capabilities(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<ListResponse<AgentCapability>>, StatusCode> {
    let capabilities = state
        .service
        .get_agent_capabilities(agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent capabilities: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(ListResponse::new(capabilities)))
}

/// PUT /v1/agents/{agent_id}/capabilities - Set capabilities for an agent
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/capabilities",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    request_body = UpdateAgentCapabilitiesRequest,
    responses(
        (status = 200, description = "Agent capabilities updated", body = ListResponse<AgentCapability>),
        (status = 400, description = "Invalid capability ID"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "capabilities"
)]
pub async fn set_agent_capabilities(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
    Json(req): Json<UpdateAgentCapabilitiesRequest>,
) -> Result<Json<ListResponse<AgentCapability>>, StatusCode> {
    // Validate all capability IDs exist
    for cap_id in &req.capabilities {
        if state.service.get(*cap_id).is_none() {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let capabilities = state
        .service
        .set_agent_capabilities(agent_id, req.capabilities)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set agent capabilities: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(ListResponse::new(capabilities)))
}

// ============================================
// Internal Capability Registry
// ============================================

/// Internal capability definition with full details
/// This is used internally to build AgentConfig
#[allow(dead_code)] // Fields used when capabilities are fully implemented
pub struct InternalCapability {
    /// Public info
    pub info: Capability,
    /// System prompt addition (prepended to agent's system prompt)
    pub system_prompt_addition: Option<String>,
    /// Tools provided by this capability
    pub tools: Vec<everruns_contracts::tools::ToolDefinition>,
}

/// Get all internal capability definitions
pub fn get_capability_registry() -> Vec<InternalCapability> {
    use everruns_contracts::tools::{BuiltinTool, BuiltinToolKind, ToolDefinition, ToolPolicy};
    use serde_json::json;

    vec![
        // Noop capability - for testing/demo
        InternalCapability {
            info: Capability {
                id: CapabilityId::Noop,
                name: "No-Op".to_string(),
                description: "A no-operation capability for testing and demonstration purposes. Does not add any functionality.".to_string(),
                status: CapabilityStatus::Available,
                icon: Some("circle-off".to_string()),
                category: Some("Testing".to_string()),
            },
            system_prompt_addition: None,
            tools: vec![],
        },
        // CurrentTime capability - adds current time tool
        InternalCapability {
            info: Capability {
                id: CapabilityId::CurrentTime,
                name: "Current Time".to_string(),
                description: "Adds a tool to get the current date and time in various formats and timezones.".to_string(),
                status: CapabilityStatus::Available,
                icon: Some("clock".to_string()),
                category: Some("Utilities".to_string()),
            },
            system_prompt_addition: None, // No system prompt contribution
            tools: vec![
                ToolDefinition::Builtin(BuiltinTool {
                    name: "get_current_time".to_string(),
                    description: "Get the current date and time. Can return time in different formats and timezones.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "timezone": {
                                "type": "string",
                                "description": "Timezone to return the time in (e.g., 'UTC', 'America/New_York', 'Europe/London'). Defaults to UTC."
                            },
                            "format": {
                                "type": "string",
                                "enum": ["iso8601", "unix", "human"],
                                "description": "Output format: 'iso8601' for ISO 8601 format, 'unix' for Unix timestamp, 'human' for human-readable format. Defaults to 'iso8601'."
                            }
                        },
                        "required": []
                    }),
                    kind: BuiltinToolKind::CurrentTime,
                    policy: ToolPolicy::Auto,
                }),
            ],
        },
        // Research capability - coming soon
        InternalCapability {
            info: Capability {
                id: CapabilityId::Research,
                name: "Deep Research".to_string(),
                description: "Enables deep research capabilities with a scratchpad for notes, web search tools, and structured thinking.".to_string(),
                status: CapabilityStatus::ComingSoon,
                icon: Some("search".to_string()),
                category: Some("AI".to_string()),
            },
            system_prompt_addition: Some(
                "You have access to a research scratchpad. Use it to organize your thoughts and findings.".to_string()
            ),
            tools: vec![], // Tools would be added here when implemented
        },
        // Sandbox capability - coming soon
        InternalCapability {
            info: Capability {
                id: CapabilityId::Sandbox,
                name: "Sandboxed Execution".to_string(),
                description: "Enables sandboxed code execution environment for running code safely.".to_string(),
                status: CapabilityStatus::ComingSoon,
                icon: Some("box".to_string()),
                category: Some("Execution".to_string()),
            },
            system_prompt_addition: Some(
                "You can execute code in a sandboxed environment. Use the execute_code tool to run code safely.".to_string()
            ),
            tools: vec![], // Tools would be added here when implemented
        },
        // FileSystem capability - coming soon
        InternalCapability {
            info: Capability {
                id: CapabilityId::FileSystem,
                name: "File System Access".to_string(),
                description: "Adds tools to access and manipulate files - read, write, grep, and more.".to_string(),
                status: CapabilityStatus::ComingSoon,
                icon: Some("folder".to_string()),
                category: Some("File Operations".to_string()),
            },
            system_prompt_addition: Some(
                "You have access to file system tools. You can read, write, and search files.".to_string()
            ),
            tools: vec![], // Tools would be added here when implemented
        },
        // TestMath capability - for testing tool calling
        InternalCapability {
            info: Capability {
                id: CapabilityId::TestMath,
                name: "Test Math".to_string(),
                description: "Testing capability: adds calculator tools (add, subtract, multiply, divide) for tool calling tests.".to_string(),
                status: CapabilityStatus::Available,
                icon: Some("calculator".to_string()),
                category: Some("Testing".to_string()),
            },
            system_prompt_addition: Some(
                "You have access to math tools. Use them for calculations: add, subtract, multiply, divide.".to_string()
            ),
            tools: vec![
                ToolDefinition::Builtin(BuiltinTool {
                    name: "add".to_string(),
                    description: "Add two numbers together.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "a": { "type": "number", "description": "First number" },
                            "b": { "type": "number", "description": "Second number" }
                        },
                        "required": ["a", "b"]
                    }),
                    kind: BuiltinToolKind::TestMathAdd,
                    policy: ToolPolicy::Auto,
                }),
                ToolDefinition::Builtin(BuiltinTool {
                    name: "subtract".to_string(),
                    description: "Subtract the second number from the first.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "a": { "type": "number", "description": "Number to subtract from" },
                            "b": { "type": "number", "description": "Number to subtract" }
                        },
                        "required": ["a", "b"]
                    }),
                    kind: BuiltinToolKind::TestMathSubtract,
                    policy: ToolPolicy::Auto,
                }),
                ToolDefinition::Builtin(BuiltinTool {
                    name: "multiply".to_string(),
                    description: "Multiply two numbers together.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "a": { "type": "number", "description": "First number" },
                            "b": { "type": "number", "description": "Second number" }
                        },
                        "required": ["a", "b"]
                    }),
                    kind: BuiltinToolKind::TestMathMultiply,
                    policy: ToolPolicy::Auto,
                }),
                ToolDefinition::Builtin(BuiltinTool {
                    name: "divide".to_string(),
                    description: "Divide the first number by the second.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "a": { "type": "number", "description": "Dividend (number to divide)" },
                            "b": { "type": "number", "description": "Divisor (number to divide by)" }
                        },
                        "required": ["a", "b"]
                    }),
                    kind: BuiltinToolKind::TestMathDivide,
                    policy: ToolPolicy::Auto,
                }),
            ],
        },
        // TestWeather capability - for testing tool calling
        InternalCapability {
            info: Capability {
                id: CapabilityId::TestWeather,
                name: "Test Weather".to_string(),
                description: "Testing capability: adds mock weather tools (get_weather, get_forecast) for tool calling tests.".to_string(),
                status: CapabilityStatus::Available,
                icon: Some("cloud-sun".to_string()),
                category: Some("Testing".to_string()),
            },
            system_prompt_addition: Some(
                "You have access to weather tools. Use get_weather for current conditions and get_forecast for multi-day forecasts.".to_string()
            ),
            tools: vec![
                ToolDefinition::Builtin(BuiltinTool {
                    name: "get_weather".to_string(),
                    description: "Get current weather for a city.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "city": { "type": "string", "description": "City name (e.g., 'New York', 'London', 'Tokyo')" }
                        },
                        "required": ["city"]
                    }),
                    kind: BuiltinToolKind::TestWeatherGet,
                    policy: ToolPolicy::Auto,
                }),
                ToolDefinition::Builtin(BuiltinTool {
                    name: "get_forecast".to_string(),
                    description: "Get multi-day weather forecast for a city.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "city": { "type": "string", "description": "City name" },
                            "days": { "type": "integer", "description": "Number of days (1-7, default: 5)" }
                        },
                        "required": ["city"]
                    }),
                    kind: BuiltinToolKind::TestWeatherForecast,
                    policy: ToolPolicy::Auto,
                }),
            ],
        },
    ]
}

/// Get a specific capability from the registry
pub fn get_capability_definition(id: CapabilityId) -> Option<InternalCapability> {
    get_capability_registry()
        .into_iter()
        .find(|c| c.info.id == id)
}
