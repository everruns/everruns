//! Feature-gated prototype boundary for OpenAI's Agents API.

use std::collections::{HashMap, HashSet};

use everruns_core::events::{
    EventContext, EventRequest, LlmGenerationData, ModelMetadata, OutputMessageCompletedData,
    OutputMessageDeltaData, OutputMessageStartedData, SessionIdledData, TokenUsage,
    ToolCallRequestedData, ToolCompletedData, ToolStartedData, TurnCancelledData,
    TurnCompletedData, TurnFailedData, TurnStartedData,
};
use everruns_core::{
    ContentPart, McpServerTransportType, RuntimeAgent, RuntimeMessage, ScopedMcpServer,
    ScopedMcpServers, mcp_tool_name,
};
use everruns_provider::execution_phase::ExecutionPhase;
use everruns_provider::tool_types::{ToolCall, ToolDefinition};
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

/// Cargo feature that must be enabled before the prototype is compiled.
pub const PROTOTYPE_FEATURE: &str = "openai-agents-api-prototype";
/// Product feature flag that must be enabled before a deployment selects this backend.
pub const PRODUCT_FEATURE_FLAG: &str = "openai_agents_api";

#[derive(Debug, Error)]
pub enum AgentsApiPrototypeError {
    #[error("Agents API configuration is missing agent.model")]
    MissingModel,
    #[error("Agents API configuration is missing agent.instructions")]
    MissingInstructions,
    #[error("MCP server '{0}' is not an HTTP server with a URL")]
    UnsupportedMcpServer(String),
    #[error("Agents API event is missing type")]
    MissingEventType,
    #[error("Agents API event '{event_type}' is missing {field}")]
    MissingEventField {
        event_type: String,
        field: &'static str,
    },
}

/// Build the documented continuation item for one client function result.
pub fn build_tool_result_input(call_id: impl Into<String>, output: Value) -> Value {
    json!({
        "type": "agent.session.input.tool_result",
        "call_id": call_id.into(),
        "output": output,
    })
}

/// Prototype session-create configuration for `POST /v1/agents/sessions`.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentsApiSessionConfig {
    pub agent: AgentsApiAgentConfig,
    pub environment: AgentsApiEnvironment,
    pub input: Value,
    pub stream: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentsApiAgentConfig {
    pub model: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AgentsApiTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_agent: Option<AgentsApiMultiAgent>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentsApiMultiAgent {
    pub enabled: bool,
    pub max_concurrent_subagents: u32,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentsApiEnvironment {
    None,
    OpenaiHosted,
    SelfHosted {
        workspace_directory: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capability_directories: Vec<String>,
    },
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentsApiTool {
    Function {
        name: String,
        description: String,
        parameters: Value,
        #[serde(default, skip_serializing_if = "is_false")]
        defer_loading: bool,
    },
    Mcp {
        server_label: String,
        transport: AgentsApiMcpTransport,
        connection_origin: String,
        required: bool,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentsApiMcpTransport {
    #[serde(rename = "type")]
    pub transport_type: String,
    pub server_url: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// Convert the resolved Everruns runtime configuration to an Agents API session request.
pub fn build_session_config(
    runtime_agent: &RuntimeAgent,
    mcp_servers: &ScopedMcpServers,
    input: impl Into<String>,
    max_concurrent_subagents: Option<u32>,
) -> Result<AgentsApiSessionConfig, AgentsApiPrototypeError> {
    let mut tools = runtime_agent
        .tools
        .iter()
        .map(|tool| AgentsApiTool::Function {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: tool.full_parameters().clone(),
            defer_loading: false,
        })
        .collect::<Vec<_>>();

    for (name, server) in mcp_servers {
        if server.transport_type != McpServerTransportType::Http || server.url.trim().is_empty() {
            return Err(AgentsApiPrototypeError::UnsupportedMcpServer(
                name.clone(),
            ));
        }
        tools.push(AgentsApiTool::Mcp {
            server_label: name.clone(),
            transport: AgentsApiMcpTransport {
                transport_type: "http".to_string(),
                server_url: server.url.clone(),
                headers: server.headers.clone(),
            },
            connection_origin: "service".to_string(),
            required: true,
        });
    }

    Ok(AgentsApiSessionConfig {
        agent: AgentsApiAgentConfig {
            model: runtime_agent.model.clone(),
            instructions: runtime_agent.system_prompt.clone(),
            tools,
            multi_agent: max_concurrent_subagents.map(|limit| AgentsApiMultiAgent {
                enabled: true,
                max_concurrent_subagents: limit,
            }),
        },
        environment: AgentsApiEnvironment::None,
        input: Value::String(input.into()),
        stream: true,
    })
}

/// Result of importing the portable subset of an Agents API session configuration.
#[derive(Clone)]
pub struct ImportedAgentsApiConfig {
    pub runtime_agent: RuntimeAgent,
    pub mcp_servers: ScopedMcpServers,
    pub warnings: Vec<String>,
}

/// Import function tools and HTTP MCP servers into native Everruns configuration.
pub fn import_session_config(
    config: AgentsApiSessionConfig,
) -> Result<ImportedAgentsApiConfig, AgentsApiPrototypeError> {
    if config.agent.model.trim().is_empty() {
        return Err(AgentsApiPrototypeError::MissingModel);
    }
    if config.agent.instructions.trim().is_empty() {
        return Err(AgentsApiPrototypeError::MissingInstructions);
    }

    let mut runtime_agent =
        RuntimeAgent::new(config.agent.instructions.clone(), config.agent.model.clone());
    let mut mcp_servers = ScopedMcpServers::default();
    let mut warnings = Vec::new();

    for tool in config.agent.tools {
        match tool {
            AgentsApiTool::Function {
                name,
                description,
                parameters,
                ..
            } => runtime_agent.tools.push(ToolDefinition::function(
                name,
                description,
                parameters,
            )),
            AgentsApiTool::Mcp {
                server_label,
                transport,
                ..
            } if transport.transport_type == "http" => {
                mcp_servers.insert(
                    server_label,
                    ScopedMcpServer {
                        transport_type: McpServerTransportType::Http,
                        url: transport.server_url,
                        headers: transport.headers,
                        ..ScopedMcpServer::default()
                    },
                );
            }
            AgentsApiTool::Mcp { server_label, .. } => warnings.push(format!(
                "MCP server '{server_label}' uses an unsupported transport"
            )),
            AgentsApiTool::Unsupported => warnings.push(
                "OpenAI built-in tool has no native Everruns import mapping".to_string(),
            ),
        }
    }
    if config.agent.multi_agent.is_some() {
        warnings.push(
            "Agents API multi-agent policy requires an explicit Everruns delegation mapping"
                .to_string(),
        );
    }
    if !matches!(config.environment, AgentsApiEnvironment::None) {
        warnings.push(
            "Agents API environment state and files are not imported into the agent definition"
                .to_string(),
        );
    }

    Ok(ImportedAgentsApiConfig {
        runtime_agent,
        mcp_servers,
        warnings,
    })
}

/// Stateful projection from Agents API stream events to canonical Everruns events.
pub struct AgentsApiEventMapper {
    session_id: SessionId,
    turn_id: TurnId,
    input_message_id: MessageId,
    output_message_id: MessageId,
    model: String,
    accumulated_output: String,
    output_started: bool,
    tool_call_count: u32,
    last_usage: Option<TokenUsage>,
    requested_tool_calls: HashSet<String>,
    started_tool_calls: HashSet<String>,
    completed_tool_calls: HashSet<String>,
}

impl AgentsApiEventMapper {
    pub fn new(
        session_id: SessionId,
        turn_id: TurnId,
        input_message_id: MessageId,
        model: impl Into<String>,
    ) -> Self {
        Self {
            session_id,
            turn_id,
            input_message_id,
            output_message_id: MessageId::new(),
            model: model.into(),
            accumulated_output: String::new(),
            output_started: false,
            tool_call_count: 0,
            last_usage: None,
            requested_tool_calls: HashSet::new(),
            started_tool_calls: HashSet::new(),
            completed_tool_calls: HashSet::new(),
        }
    }

    /// Project one documented Agents API event. Unmapped progress events return no output.
    pub fn map(
        &mut self,
        provider_event: &Value,
    ) -> Result<Vec<EventRequest>, AgentsApiPrototypeError> {
        let event_type = provider_event
            .get("type")
            .and_then(Value::as_str)
            .ok_or(AgentsApiPrototypeError::MissingEventType)?;
        let metadata = json!({
            "runtime_backend": "openai_agents_api",
            "provider_event_type": event_type,
            "provider_event_id": provider_event.get("id"),
            "provider_session_id": provider_event.pointer("/session/id")
                .or_else(|| provider_event.get("session_id")),
            "provider_turn_id": provider_event.pointer("/turn/id"),
            "provider_item_id": provider_event.pointer("/item/id"),
            "provider_usage": provider_event.pointer("/turn/usage")
                .or_else(|| provider_event.get("usage")),
        });
        let mut mapped = match event_type {
            "agent.session.turn.created" => vec![self.request(TurnStartedData {
                turn_id: self.turn_id,
                input_message_id: self.input_message_id,
                input_content: None,
                agent_id: None,
                agent_name: None,
                agent_description: None,
            })],
            "agent.session.turn.output_text.delta" => {
                let delta = string_field(provider_event, event_type, "delta")?;
                let mut events = self.start_output_if_needed();
                self.accumulated_output.push_str(&delta);
                events.push(self.request(OutputMessageDeltaData {
                    turn_id: self.turn_id,
                    message_id: self.output_message_id,
                    delta,
                    accumulated: self.accumulated_output.clone(),
                    phase: None,
                }));
                events
            }
            "agent.session.requires_action" => {
                let actions = provider_event
                    .pointer("/session/required_actions")
                    .and_then(Value::as_array)
                    .ok_or_else(|| AgentsApiPrototypeError::MissingEventField {
                        event_type: event_type.to_string(),
                        field: "session.required_actions",
                    })?;
                let mut calls = Vec::new();
                for action in actions {
                    if action.get("type").and_then(Value::as_str) != Some("function_call") {
                        continue;
                    }
                    let id = string_field(action, event_type, "call_id")?;
                    if !self.requested_tool_calls.insert(id.clone()) {
                        continue;
                    }
                    calls.push(ToolCall {
                        id,
                        name: string_field(action, event_type, "name")?,
                        arguments: action
                            .get("arguments")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    });
                }
                self.tool_call_count = self
                    .tool_call_count
                    .saturating_add(u32::try_from(calls.len()).unwrap_or(u32::MAX));
                if calls.is_empty() {
                    Vec::new()
                } else {
                    vec![self.request(ToolCallRequestedData {
                        tool_calls: calls,
                        tool_summaries: Vec::new(),
                        headline: None,
                        completed_headline: None,
                    })]
                }
            }
            "agent.session.turn.item.added" | "agent.session.turn.item.updated" => {
                self.map_item(provider_event, event_type)?
            }
            "agent.session.turn.completed" => self.map_completed(provider_event),
            "agent.session.idle" => vec![self.request(SessionIdledData {
                turn_id: self.turn_id,
                iterations: Some(1),
                usage: self.last_usage.clone(),
            })],
            "agent.session.turn.failed" => vec![self.request(TurnFailedData {
                turn_id: self.turn_id,
                error: provider_event
                    .pointer("/turn/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("OpenAI Agents API turn failed")
                    .to_string(),
                error_code: provider_event
                    .pointer("/turn/error/code")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                error_fields: None,
                error_disclosure: None,
            })],
            "agent.session.turn.cancelled" => vec![self.request(TurnCancelledData {
                turn_id: self.turn_id,
                reason: Some("OpenAI Agents API turn cancelled".to_string()),
                usage: usage_from(provider_event),
            })],
            _ => Vec::new(),
        };
        for event in &mut mapped {
            event.metadata = Some(metadata.clone());
        }
        Ok(mapped)
    }

    fn map_item(
        &mut self,
        provider_event: &Value,
        event_type: &str,
    ) -> Result<Vec<EventRequest>, AgentsApiPrototypeError> {
        let Some(item) = provider_event.get("item") else {
            return Ok(Vec::new());
        };
        if item.get("type").and_then(Value::as_str) != Some("mcp_call") {
            return Ok(Vec::new());
        }
        let call_id = string_field(item, event_type, "id")?;
        let server_label = string_field(item, event_type, "server_label")?;
        let source_name = string_field(item, event_type, "name")?;
        let tool_name = mcp_tool_name(&server_label, &source_name);
        let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let call = ToolCall {
            id: call_id.clone(),
            name: tool_name.clone(),
            arguments,
        };
        let mut events = Vec::new();
        if self.started_tool_calls.insert(call_id.clone()) {
            self.tool_call_count = self.tool_call_count.saturating_add(1);
            events.push(self.request(ToolStartedData {
                tool_call: call,
                tool_call_fingerprint: None,
                display_name: Some(format!("{server_label}: {source_name}")),
                narration: None,
            }));
        }
        if item.get("status").and_then(Value::as_str) == Some("completed")
            && self.completed_tool_calls.insert(call_id.clone())
        {
            let output = item.get("output").cloned().unwrap_or(Value::Null);
            events.push(self.request(
                ToolCompletedData::success(
                    call_id,
                    tool_name,
                    vec![ContentPart::tool_result_text(&output)],
                    item.get("duration_ms").and_then(Value::as_u64),
                )
                .with_display_name(Some(format!("{server_label}: {source_name}"))),
            ));
        }
        Ok(events)
    }

    fn map_completed(&mut self, provider_event: &Value) -> Vec<EventRequest> {
        let mut events = self.start_output_if_needed();
        let usage = usage_from(provider_event);
        self.last_usage = usage.clone();
        let message = RuntimeMessage::assistant(self.accumulated_output.clone())
            .with_id(self.output_message_id);
        events.push(self.request(
            OutputMessageCompletedData::new(message)
                .with_metadata(ModelMetadata {
                    model: self.model.clone(),
                    model_id: None,
                    provider_id: None,
                })
                .with_usage(usage.clone().unwrap_or_default()),
        ));
        events.push(self.request(LlmGenerationData::success_with_metadata(
            Vec::new(),
            Vec::new(),
            Some(self.accumulated_output.clone()),
            Vec::new(),
            self.model.clone(),
            Some("openai_agents_api".to_string()),
            usage.clone(),
            provider_event
                .pointer("/turn/duration_ms")
                .and_then(Value::as_u64),
            None,
            Some(vec!["stop".to_string()]),
            provider_event
                .pointer("/turn/id")
                .and_then(Value::as_str)
                .map(str::to_string),
        )));
        events.push(self.request(TurnCompletedData {
            turn_id: self.turn_id,
            iterations: 1,
            duration_ms: provider_event
                .pointer("/turn/duration_ms")
                .and_then(Value::as_u64),
            usage: usage.clone(),
            input_content: None,
            final_message_id: Some(self.output_message_id),
            final_answer_preview: Some(self.accumulated_output.chars().take(500).collect()),
            time_to_first_token_ms: None,
            tool_call_count: Some(self.tool_call_count),
            llm_call_count: None,
            status: Some("completed".to_string()),
        }));
        events
    }

    fn start_output_if_needed(&mut self) -> Vec<EventRequest> {
        if self.output_started {
            return Vec::new();
        }
        self.output_started = true;
        vec![self.request(OutputMessageStartedData {
            reasoning_state: None,
            turn_id: self.turn_id,
            message_id: self.output_message_id,
            model: Some(self.model.clone()),
            iteration: Some(1),
            phase: Some(ExecutionPhase::FinalAnswer),
        })]
    }

    fn request(&self, data: impl Into<everruns_core::events::EventData>) -> EventRequest {
        EventRequest::new(
            self.session_id,
            EventContext::turn(self.turn_id, self.input_message_id),
            data,
        )
    }
}

fn string_field(
    value: &Value,
    event_type: &str,
    field: &'static str,
) -> Result<String, AgentsApiPrototypeError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| AgentsApiPrototypeError::MissingEventField {
            event_type: event_type.to_string(),
            field,
        })
}

fn usage_from(event: &Value) -> Option<TokenUsage> {
    let usage = event
        .pointer("/turn/usage")
        .or_else(|| event.get("usage"))?;
    let count = |field: &str| {
        usage
            .get(field)
            .and_then(Value::as_u64)
            .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
            .unwrap_or_default()
    };
    Some(
        TokenUsage::with_cache(
            count("input_tokens"),
            count("output_tokens"),
            usage
                .get("cache_read_tokens")
                .and_then(Value::as_u64)
                .map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
            None,
        )
        .with_cost(usage.get("cost_usd").and_then(Value::as_f64), None),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::tool_types::ClientSideTool;

    fn prototype_config() -> AgentsApiSessionConfig {
        let mut agent = RuntimeAgent::new("Use both tools.", "gpt-6-astra");
        agent.tools.push(ToolDefinition::ClientSide(
            ClientSideTool::new(
                "lookup_customer",
                "Look up one customer",
                json!({
                    "type": "object",
                    "properties": {"customer_id": {"type": "string"}},
                    "required": ["customer_id"],
                    "additionalProperties": false
                }),
            ),
        ));
        let servers = ScopedMcpServers::from([(
            "docs".to_string(),
            ScopedMcpServer {
                url: "https://developers.openai.com/mcp".to_string(),
                ..ScopedMcpServer::default()
            },
        )]);
        build_session_config(&agent, &servers, "Use the function and MCP tools.", Some(2)).unwrap()
    }

    #[test]
    fn session_config_maps_one_function_and_one_mcp_tool() {
        let config = prototype_config();
        assert_eq!(config.agent.model, "gpt-6-astra");
        assert_eq!(config.agent.tools.len(), 2);
        assert_eq!(
            serde_json::to_value(&config.agent.tools).unwrap(),
            json!([
                {
                    "type": "function",
                    "name": "lookup_customer",
                    "description": "Look up one customer",
                    "parameters": {
                        "type": "object",
                        "properties": {"customer_id": {"type": "string"}},
                        "required": ["customer_id"],
                        "additionalProperties": false
                    }
                },
                {
                    "type": "mcp",
                    "server_label": "docs",
                    "transport": {
                        "type": "http",
                        "server_url": "https://developers.openai.com/mcp"
                    },
                    "connection_origin": "service",
                    "required": true
                }
            ])
        );
    }

    #[test]
    fn recorded_boundary_fixture_projects_to_canonical_session_events() {
        let fixture: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/agents_api_events.json")).unwrap();
        let mut mapper = AgentsApiEventMapper::new(
            SessionId::from_seed(1),
            TurnId::from_seed(2),
            MessageId::from_seed(3),
            "gpt-6-astra",
        );
        let events = fixture
            .iter()
            .flat_map(|event| mapper.map(event).unwrap())
            .collect::<Vec<_>>();
        assert!(mapper.map(&fixture[1]).unwrap().is_empty());
        assert!(mapper.map(&fixture[2]).unwrap().is_empty());
        let types = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                "turn.started",
                "tool.call_requested",
                "tool.started",
                "tool.completed",
                "output.message.started",
                "output.message.delta",
                "output.message.delta",
                "output.message.completed",
                "llm.generation",
                "turn.completed",
                "session.idled"
            ]
        );
        let requested = events
            .iter()
            .find(|event| event.event_type == "tool.call_requested")
            .unwrap();
        assert_eq!(
            serde_json::to_value(&requested.data).unwrap()["tool_calls"][0]["name"],
            "lookup_customer"
        );
        let mcp_completed = events
            .iter()
            .find(|event| event.event_type == "tool.completed")
            .unwrap();
        assert_eq!(
            serde_json::to_value(&mcp_completed.data).unwrap()["tool_name"],
            "mcp_docs__search"
        );
        let completed = events
            .iter()
            .find(|event| event.event_type == "turn.completed")
            .unwrap();
        assert_eq!(
            serde_json::to_value(&completed.data).unwrap()["usage"],
            json!({
                "input_tokens": 101,
                "output_tokens": 23,
                "cache_read_tokens": 40,
                "actual_cost_usd": 0.0042
            })
        );
        assert_eq!(
            completed.metadata.as_ref().unwrap()["runtime_backend"],
            "openai_agents_api"
        );
        assert_eq!(
            completed.metadata.as_ref().unwrap()["provider_turn_id"],
            "turn_fixture"
        );
        assert_eq!(
            completed.metadata.as_ref().unwrap()["provider_usage"]["container_seconds"],
            0
        );
    }

    #[test]
    fn function_result_builds_agents_api_continuation_input() {
        assert_eq!(
            build_tool_result_input("call_customer", json!({"name": "Ada"})),
            json!({
                "type": "agent.session.input.tool_result",
                "call_id": "call_customer",
                "output": {"name": "Ada"}
            })
        );
    }

    #[test]
    fn import_preserves_portable_tools_and_reports_nonportable_state() {
        let mut config = prototype_config();
        config.environment = AgentsApiEnvironment::OpenaiHosted;
        config.agent.tools.push(AgentsApiTool::Unsupported);
        let imported = import_session_config(config).unwrap();
        assert_eq!(imported.runtime_agent.tools.len(), 1);
        assert_eq!(imported.mcp_servers["docs"].url, "https://developers.openai.com/mcp");
        assert_eq!(imported.warnings.len(), 3);
    }

    #[test]
    fn unsupported_mcp_transport_fails_closed() {
        let agent = RuntimeAgent::new("Test.", "gpt-6-astra");
        let servers = ScopedMcpServers::from([(
            "local".to_string(),
            ScopedMcpServer {
                transport_type: McpServerTransportType::Stdio,
                command: Some("mcp-server".to_string()),
                ..ScopedMcpServer::default()
            },
        )]);
        assert!(matches!(
            build_session_config(&agent, &servers, "test", None),
            Err(AgentsApiPrototypeError::UnsupportedMcpServer(name)) if name == "local"
        ));
    }
}
