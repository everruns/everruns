// Activity implementations for workflow execution
//
// Activities are the units of work scheduled by the workflow.
// Each activity runs outside the workflow and returns a result.
//
// These implementations use Atoms from everruns-core for the actual work:
// - CallModelAtom for LLM calls
// - ExecuteToolAtom for tool execution
//
// Atoms handle message loading/storage internally via MessageStore trait.

use anyhow::{Context, Result};
use everruns_core::atoms::{
    Atom, CallModelAtom, CallModelInput as AtomCallModelInput, ExecuteToolAtom,
    ExecuteToolInput as AtomExecuteToolInput,
};
use everruns_core::config::AgentConfigBuilder;
use everruns_core::provider_factory::{create_provider, ProviderConfig, ProviderType};
use everruns_core::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy, ToolRegistry};
use everruns_storage::{repositories::Database, EncryptionService};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::adapters::DbMessageStore;
use crate::agent_workflow::{AgentConfigData, ToolCallData, ToolDefinitionData, ToolResultData};

// ============================================================================
// Activity Input/Output Types
// ============================================================================

/// Input for load-agent activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadAgentInput {
    /// Agent ID (UUID string)
    pub agent_id: String,
}

/// Input for call-model activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallModelInput {
    /// Session ID (UUID string)
    pub session_id: String,
    /// Agent configuration (model, tools, system_prompt)
    pub agent_config: AgentConfigData,
}

/// Output from call-model activity
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallModelOutput {
    /// Text response from the model
    pub text: String,
    /// Tool calls requested by the model (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallData>>,
    /// Whether tool execution is needed
    pub needs_tool_execution: bool,
}

/// Input for execute-tool activity (single tool)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolInput {
    /// Session ID (UUID string)
    pub session_id: String,
    /// Tool call to execute
    pub tool_call: ToolCallData,
    /// Available tool definitions
    pub tool_definitions: Vec<ToolDefinitionData>,
}

/// Output from execute-tool activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolOutput {
    /// Result of the tool execution
    pub result: ToolResultData,
}

// ============================================================================
// Activity Implementations
// ============================================================================

/// Load agent configuration from database
///
/// This activity loads the agent from the database and builds the AgentConfigData
/// including the model, system_prompt, tools from capabilities, and max_iterations.
/// API keys are decrypted from the database using the provided encryption service.
pub async fn load_agent_activity(
    db: Database,
    encryption: EncryptionService,
    input: LoadAgentInput,
) -> Result<AgentConfigData> {
    use everruns_core::capabilities::CapabilityRegistry;

    let agent_id: Uuid = input.agent_id.parse().context("Invalid agent_id UUID")?;

    tracing::info!(agent_id = %agent_id, "Loading agent configuration");

    // Load agent from database
    let agent = db
        .get_agent(agent_id)
        .await
        .context("Database error loading agent")?
        .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", agent_id))?;

    // Load capabilities for this agent
    let capabilities = db
        .get_agent_capabilities(agent_id)
        .await
        .context("Database error loading agent capabilities")?;

    // Collect capability IDs as strings (no parsing needed now)
    let capability_ids: Vec<String> = capabilities.into_iter().map(|c| c.capability_id).collect();

    // Apply capabilities to get tools
    let registry = CapabilityRegistry::with_builtins();

    // Look up the LLM model configuration if default_model_id is set
    // Also get the decrypted API key from the provider
    let (model_id, provider_type, api_key, base_url) = if let Some(llm_model_uuid) =
        agent.default_model_id
    {
        // Look up the LLM model to get the actual model_id and provider_type
        match db.get_llm_model(llm_model_uuid).await {
            Ok(Some(llm_model)) => {
                // Get provider info to determine provider_type and get API key
                match db.get_llm_provider(llm_model.provider_id).await {
                    Ok(Some(provider)) => {
                        // Decrypt the API key from the provider
                        let provider_with_key = db
                            .get_provider_with_api_key(&provider, &encryption)
                            .context("Failed to decrypt provider API key")?;

                        tracing::info!(
                            agent_id = %agent_id,
                            model_id = %llm_model.model_id,
                            provider_type = %provider.provider_type,
                            api_key_set = provider_with_key.api_key.is_some(),
                            "Resolved LLM model and provider from database"
                        );
                        (
                            llm_model.model_id,
                            provider.provider_type,
                            provider_with_key.api_key,
                            provider_with_key.base_url,
                        )
                    }
                    _ => {
                        // Fallback to model detection if provider lookup fails
                        let model_id = llm_model.model_id.clone();
                        let provider_type = detect_provider_type(&model_id);
                        (model_id, provider_type, None, None)
                    }
                }
            }
            _ => {
                // Fallback to default model if agent's model lookup fails
                match db.get_default_llm_model().await {
                    Ok(Some(default_model_row)) => {
                        // Get provider info to get API key
                        match db.get_llm_provider(default_model_row.provider_id).await {
                            Ok(Some(provider)) => {
                                let provider_with_key = db
                                    .get_provider_with_api_key(&provider, &encryption)
                                    .context("Failed to decrypt default model provider API key")?;
                                (
                                    default_model_row.model_id,
                                    provider.provider_type,
                                    provider_with_key.api_key,
                                    provider_with_key.base_url,
                                )
                            }
                            _ => {
                                let model_id = default_model_row.model_id;
                                let provider_type = detect_provider_type(&model_id);
                                (model_id, provider_type, None, None)
                            }
                        }
                    }
                    _ => {
                        let default_model = "gpt-4o".to_string();
                        let provider_type = detect_provider_type(&default_model);
                        (default_model, provider_type, None, None)
                    }
                }
            }
        }
    } else {
        // No default_model_id set, try to use the default model
        match db.get_default_llm_model().await {
            Ok(Some(default_model_row)) => {
                // Get provider info to get API key
                match db.get_llm_provider(default_model_row.provider_id).await {
                    Ok(Some(provider)) => {
                        let provider_with_key = db
                            .get_provider_with_api_key(&provider, &encryption)
                            .context("Failed to decrypt default model provider API key")?;
                        (
                            default_model_row.model_id,
                            provider.provider_type,
                            provider_with_key.api_key,
                            provider_with_key.base_url,
                        )
                    }
                    _ => {
                        let model_id = default_model_row.model_id;
                        let provider_type = detect_provider_type(&model_id);
                        (model_id, provider_type, None, None)
                    }
                }
            }
            _ => {
                let default_model = "gpt-4o".to_string();
                let provider_type = detect_provider_type(&default_model);
                (default_model, provider_type, None, None)
            }
        }
    };

    // Build base config and apply capabilities
    let base_config = everruns_core::AgentConfig::new(&agent.system_prompt, &model_id);
    let applied =
        everruns_core::capabilities::apply_capabilities(base_config, &capability_ids, &registry);

    // Convert tools to ToolDefinitionData
    let tools: Vec<ToolDefinitionData> = applied
        .config
        .tools
        .iter()
        .map(|tool| match tool {
            ToolDefinition::Builtin(b) => ToolDefinitionData {
                name: b.name.clone(),
                description: b.description.clone(),
                parameters: b.parameters.clone(),
            },
        })
        .collect();

    tracing::info!(
        agent_id = %agent_id,
        model = %model_id,
        provider_type = %provider_type,
        api_key_configured = api_key.is_some(),
        capability_count = capability_ids.len(),
        tool_count = tools.len(),
        "Loaded agent with capabilities"
    );

    Ok(AgentConfigData {
        model: model_id,
        provider_type,
        api_key, // Decrypted from database
        base_url,
        system_prompt: Some(applied.config.system_prompt),
        tools,
        max_iterations: 10,
    })
}

/// Call the LLM model using CallModelAtom
///
/// This activity:
/// 1. Loads messages from the database via MessageStore
/// 2. Calls the LLM with the agent configuration
/// 3. Stores the assistant response and any tool call messages
/// 4. Returns the text and tool calls
pub async fn call_model_activity(db: Database, input: CallModelInput) -> Result<CallModelOutput> {
    let session_id: Uuid = input
        .session_id
        .parse()
        .context("Invalid session_id UUID")?;

    // Create LLM provider based on agent config
    let provider_type: ProviderType = input
        .agent_config
        .provider_type
        .parse()
        .unwrap_or(ProviderType::OpenAI);

    tracing::info!(
        session_id = %session_id,
        model = %input.agent_config.model,
        provider_type = %provider_type,
        "Creating LLM provider for call_model_activity"
    );

    let mut provider_config = ProviderConfig::new(provider_type);
    if let Some(ref api_key) = input.agent_config.api_key {
        provider_config = provider_config.with_api_key(api_key);
    }
    if let Some(ref base_url) = input.agent_config.base_url {
        provider_config = provider_config.with_base_url(base_url);
    }

    let llm_provider =
        create_provider(&provider_config).context("Failed to create LLM provider")?;

    // Create atom dependencies
    let message_store = DbMessageStore::new(db);

    // Build AgentConfig from the workflow's AgentConfigData
    let agent_config = build_agent_config(&input.agent_config);

    // Create and execute CallModelAtom
    let atom = CallModelAtom::new(message_store, llm_provider);
    let result = atom
        .execute(AtomCallModelInput {
            session_id,
            config: agent_config,
        })
        .await
        .context("CallModelAtom execution failed")?;

    // Convert to activity output
    let tool_calls = if result.tool_calls.is_empty() {
        None
    } else {
        Some(
            result
                .tool_calls
                .iter()
                .map(|tc| ToolCallData {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .collect(),
        )
    };

    Ok(CallModelOutput {
        text: result.text,
        tool_calls,
        needs_tool_execution: result.needs_tool_execution,
    })
}

/// Execute a single tool using ExecuteToolAtom
///
/// This activity:
/// 1. Executes the tool call via ToolExecutor
/// 2. Stores the tool result message
/// 3. Returns the result
pub async fn execute_tool_activity(
    db: Database,
    input: ExecuteToolInput,
) -> Result<ExecuteToolOutput> {
    let session_id: Uuid = input
        .session_id
        .parse()
        .context("Invalid session_id UUID")?;

    // Create atom dependencies
    let message_store = DbMessageStore::new(db);
    let tool_executor = ToolRegistry::with_defaults();

    // Convert tool call data
    let tool_call = ToolCall {
        id: input.tool_call.id.clone(),
        name: input.tool_call.name.clone(),
        arguments: input.tool_call.arguments.clone(),
    };

    // Convert tool definitions
    let tool_definitions: Vec<ToolDefinition> = input
        .tool_definitions
        .iter()
        .map(convert_tool_definition)
        .collect();

    // Create and execute ExecuteToolAtom
    let atom = ExecuteToolAtom::new(message_store, tool_executor);
    let result = atom
        .execute(AtomExecuteToolInput {
            session_id,
            tool_call: tool_call.clone(),
            tool_definitions,
        })
        .await
        .context("ExecuteToolAtom execution failed")?;

    Ok(ExecuteToolOutput {
        result: ToolResultData {
            tool_call_id: tool_call.id,
            result: result.result.result,
            error: result.result.error,
        },
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Build AgentConfig from workflow's AgentConfigData
fn build_agent_config(data: &AgentConfigData) -> everruns_core::AgentConfig {
    let tools: Vec<ToolDefinition> = data.tools.iter().map(convert_tool_definition).collect();

    AgentConfigBuilder::new()
        .model(&data.model)
        .system_prompt(data.system_prompt.as_deref().unwrap_or(""))
        .tools(tools)
        .max_iterations(data.max_iterations as usize)
        .build()
}

/// Convert workflow's ToolDefinitionData to core's ToolDefinition
fn convert_tool_definition(tool: &ToolDefinitionData) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters: tool.parameters.clone(),
        policy: ToolPolicy::Auto,
    })
}

/// Detect provider type from model name pattern
///
/// This is a helper function that infers the provider type from the model name.
/// It supports common patterns for OpenAI and Anthropic providers.
fn detect_provider_type(model_id: &str) -> String {
    let model_lower = model_id.to_lowercase();

    if model_lower.starts_with("claude")
        || model_lower.contains("claude")
        || model_lower.starts_with("anthropic")
    {
        "anthropic".to_string()
    } else {
        // Default to OpenAI for all other models (including GPT, O1, O3, etc.)
        "openai".to_string()
    }
}

// ============================================================================
// Activity Type Constants
// ============================================================================

/// Activity type constants for workflow scheduling
pub mod activity_types {
    pub const CALL_MODEL: &str = "call-model";
    pub const EXECUTE_TOOL: &str = "execute-tool";
    pub const LOAD_AGENT: &str = "load-agent";
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_call_model_input_serialization() {
        let input = CallModelInput {
            session_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            agent_config: AgentConfigData {
                model: "gpt-4o".into(),
                provider_type: "openai".into(),
                api_key: None,
                base_url: None,
                system_prompt: Some("You are a helpful assistant.".into()),
                tools: vec![],
                max_iterations: 5,
            },
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: CallModelInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_id, input.session_id);
        assert_eq!(parsed.agent_config.model, "gpt-4o");
    }

    #[test]
    fn test_execute_tool_input_serialization() {
        let input = ExecuteToolInput {
            session_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            tool_call: ToolCallData {
                id: "call_1".into(),
                name: "get_time".into(),
                arguments: json!({}),
            },
            tool_definitions: vec![ToolDefinitionData {
                name: "get_time".into(),
                description: "Get current time".into(),
                parameters: json!({"type": "object", "properties": {}}),
            }],
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: ExecuteToolInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_call.name, "get_time");
    }

    #[test]
    fn test_build_agent_config() {
        let data = AgentConfigData {
            model: "gpt-4o".into(),
            provider_type: "openai".into(),
            api_key: None,
            base_url: None,
            system_prompt: Some("Test prompt".into()),
            tools: vec![ToolDefinitionData {
                name: "test_tool".into(),
                description: "A test tool".into(),
                parameters: json!({}),
            }],
            max_iterations: 10,
        };

        let config = build_agent_config(&data);
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.system_prompt, "Test prompt");
        assert_eq!(config.tools.len(), 1);
        assert_eq!(config.max_iterations, 10);
    }

    #[test]
    fn test_detect_provider_type() {
        assert_eq!(detect_provider_type("gpt-4o"), "openai");
        assert_eq!(detect_provider_type("gpt-4-turbo"), "openai");
        assert_eq!(detect_provider_type("claude-3-opus"), "anthropic");
        assert_eq!(
            detect_provider_type("claude-3-5-sonnet-20241022"),
            "anthropic"
        );
        assert_eq!(detect_provider_type("o1-preview"), "openai");
        assert_eq!(detect_provider_type("unknown-model"), "openai"); // Default to OpenAI
                                                                     // Previously Ollama models now default to OpenAI
        assert_eq!(detect_provider_type("llama-3.1-70b"), "openai");
        assert_eq!(detect_provider_type("mistral-7b"), "openai");
    }
}
