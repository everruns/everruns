//! Feature-gated prototype of OpenAI's Agents API as a runtime backend (EVE-1120).
//!
//! OpenAI owns the agent loop; Everruns owns the event ledger. This module maps
//! a resolved Everruns agent onto an Agents API session, projects the provider
//! stream onto canonical session events, and answers client function calls
//! through a caller-supplied handler. It is compiled only with the
//! `openai-agents-api-prototype` feature and nothing in the worker selects it
//! yet. Design, gaps, and the recommendation live in
//! `knowledge/execution/openai-agents-api-runtime.md`.

use std::collections::{HashMap, HashSet};

use eventsource_stream::Eventsource;
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
use futures::StreamExt;
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
    #[error("Agents API terminal event '{0}' is not supported")]
    UnsupportedTerminalEvent(String),
    #[error("Agents API stream closed before the root turn ended")]
    StreamClosedBeforeTurnEnded,
    #[error("Agents API request failed: {0}")]
    Http(String),
    #[error("Agents API returned HTTP {status}: {body}")]
    Api { status: u16, body: String },
    #[error("Agents API event '{event_type}' is missing {field}")]
    MissingEventField {
        event_type: String,
        field: &'static str,
    },
}

/// Build the documented continuation event for one client function result.
///
/// `turn_id` and `call_id` are copied from the pending `function_call` entry in
/// `session.required_actions`. OpenAI takes the output as a string, so a JSON
/// result is serialized by the caller; an `Err` reports a failed call the agent
/// can react to instead of retrying blind.
pub fn build_tool_result_input(
    action: &FunctionCallAction,
    result: Result<String, String>,
) -> Value {
    match result {
        Ok(output) => json!({
            "type": "agent.session.input.tool_result",
            "turn_id": action.turn_id,
            "call_id": action.call_id,
            "success": true,
            "output": output,
        }),
        Err(error) => json!({
            "type": "agent.session.input.tool_result",
            "turn_id": action.turn_id,
            "call_id": action.call_id,
            "success": false,
            "error": error,
        }),
    }
}

/// One pending client function call from `session.required_actions`.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionCallAction {
    pub turn_id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

impl FunctionCallAction {
    /// Pending function calls in an `agent.session.requires_action` event (or a
    /// retrieved session). Other action kinds (environment connection, browser
    /// sign-in) are not function results and are skipped.
    pub fn from_required_actions(provider_event: &Value) -> Vec<Self> {
        provider_event
            .pointer("/session/required_actions")
            .or_else(|| provider_event.get("required_actions"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|action| action.get("type").and_then(Value::as_str) == Some("function_call"))
            .filter_map(|action| {
                Some(Self {
                    turn_id: action.get("turn_id")?.as_str()?.to_string(),
                    call_id: action.get("call_id")?.as_str()?.to_string(),
                    name: action.get("name")?.as_str()?.to_string(),
                    arguments: action
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                })
            })
            .collect()
    }
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
            return Err(AgentsApiPrototypeError::UnsupportedMcpServer(name.clone()));
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

    let mut runtime_agent = RuntimeAgent::new(
        config.agent.instructions.clone(),
        config.agent.model.clone(),
    );
    let mut mcp_servers = ScopedMcpServers::default();
    let mut warnings = Vec::new();

    for tool in config.agent.tools {
        match tool {
            AgentsApiTool::Function {
                name,
                description,
                parameters,
                ..
            } => runtime_agent
                .tools
                .push(ToolDefinition::function(name, description, parameters)),
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
            AgentsApiTool::Unsupported => warnings
                .push("OpenAI built-in tool has no native Everruns import mapping".to_string()),
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

/// How the root turn ended, as reported by the provider.
#[derive(Clone, Debug, PartialEq)]
pub enum RootTurnOutcome {
    Completed,
    Failed {
        code: Option<String>,
        message: String,
    },
    Cancelled,
}

/// Stateful projection from Agents API stream events to canonical Everruns events.
///
/// Only the root agent's turn is projected. Subagent turns carry a non-null
/// `subagent_id` and must never end or overwrite the root turn; their usage is
/// a follow-up (see the concept's cost section).
pub struct AgentsApiEventMapper {
    session_id: SessionId,
    turn_id: TurnId,
    input_message_id: MessageId,
    output_message_id: MessageId,
    model: String,
    accumulated_output: String,
    /// Provider message item the open Everruns output message mirrors.
    open_item: Option<String>,
    output_phase: Option<ExecutionPhase>,
    message_open: bool,
    /// Last non-commentary message: what the turn reports as its answer.
    final_answer: Option<(MessageId, String)>,
    tool_call_count: u32,
    last_usage: Option<TokenUsage>,
    last_error: Option<(Option<String>, String)>,
    outcome: Option<RootTurnOutcome>,
    provider_session_id: Option<String>,
    /// Client function calls by call id, with the tool name for the result.
    requested_tool_calls: HashMap<String, String>,
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
            open_item: None,
            output_phase: None,
            message_open: false,
            final_answer: None,
            tool_call_count: 0,
            last_usage: None,
            last_error: None,
            outcome: None,
            provider_session_id: None,
            requested_tool_calls: HashMap::new(),
            started_tool_calls: HashSet::new(),
            completed_tool_calls: HashSet::new(),
        }
    }

    /// The provider session id, once any event has carried it. A durable
    /// backend stores this with the turn so it can reconcile after a restart.
    pub fn provider_session_id(&self) -> Option<&str> {
        self.provider_session_id.as_deref()
    }

    /// How the root turn ended, once a terminal event has been projected.
    pub fn outcome(&self) -> Option<&RootTurnOutcome> {
        self.outcome.as_ref()
    }

    /// Close the projection when the provider stream ends. A stream that
    /// closes before the root turn reaches a terminal state is a failure: OpenAI
    /// does not replay missed events, so the caller must reconcile from the
    /// saved session rather than report success.
    pub fn finish(&self) -> Result<RootTurnOutcome, AgentsApiPrototypeError> {
        self.outcome
            .clone()
            .ok_or(AgentsApiPrototypeError::StreamClosedBeforeTurnEnded)
    }

    /// Project one Agents API stream event. Unmapped progress events return no
    /// output; an unrecognized terminal event fails closed.
    pub fn map(
        &mut self,
        provider_event: &Value,
    ) -> Result<Vec<EventRequest>, AgentsApiPrototypeError> {
        let event_type = provider_event
            .get("type")
            .and_then(Value::as_str)
            .ok_or(AgentsApiPrototypeError::MissingEventType)?;
        if let Some(id) = provider_event
            .pointer("/session/id")
            .or_else(|| provider_event.get("session_id"))
            .and_then(Value::as_str)
        {
            self.provider_session_id
                .get_or_insert_with(|| id.to_string());
        }
        if is_subagent_event(provider_event) {
            return Ok(Vec::new());
        }
        let metadata = json!({
            "runtime_backend": "openai_agents_api",
            "provider_event_type": event_type,
            "provider_event_id": provider_event.get("event_id"),
            "provider_session_id": self.provider_session_id,
            "provider_turn_id": provider_event.get("turn_id")
                .or_else(|| provider_event.pointer("/turn/id")),
            "provider_item_id": provider_event.pointer("/item/id")
                .or_else(|| provider_event.get("item_id")),
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
                let mut events = self.open_message(item_id(provider_event), None);
                self.accumulated_output.push_str(&delta);
                events.push(self.delta(delta));
                events
            }
            // Deltas may be absent; `done` carries the complete part. Emit only
            // the missing suffix so SSE clients that concatenate deltas agree
            // with the final message.
            "agent.session.turn.output_text.done" => {
                let text = string_field(provider_event, event_type, "text")?;
                let mut events = self.open_message(item_id(provider_event), None);
                if let Some(missing) = text.strip_prefix(self.accumulated_output.as_str()) {
                    if !missing.is_empty() {
                        self.accumulated_output.push_str(missing);
                        events.push(self.delta(missing.to_string()));
                    }
                } else {
                    self.accumulated_output = text;
                }
                events
            }
            "agent.session.requires_action" => {
                let calls = FunctionCallAction::from_required_actions(provider_event)
                    .into_iter()
                    .filter(|action| {
                        self.requested_tool_calls
                            .insert(action.call_id.clone(), action.name.clone())
                            .is_none()
                    })
                    .map(|action| ToolCall {
                        id: action.call_id,
                        name: action.name,
                        arguments: action.arguments,
                    })
                    .collect::<Vec<_>>();
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
            "agent.session.turn.item.added"
            | "agent.session.turn.item.updated"
            | "agent.session.turn.item.done" => self.map_item(provider_event, event_type)?,
            "error" => {
                // The live API reports the cause (e.g. `usage_limit_exceeded`)
                // on a standalone `error` event just before `turn.failed`.
                let error = provider_event.get("error");
                self.last_error = Some((
                    error
                        .and_then(|e| e.get("code"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    error
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("OpenAI Agents API error")
                        .to_string(),
                ));
                Vec::new()
            }
            "agent.session.turn.completed" => self.map_completed(provider_event),
            "agent.session.turn.failed" => self.fail(
                provider_event.pointer("/turn/error"),
                "OpenAI Agents API turn failed",
            ),
            "agent.session.failed" | "agent.session.environment.failed"
                if self.outcome.is_none() =>
            {
                self.fail(
                    provider_event.pointer("/session/error"),
                    "OpenAI Agents API session failed",
                )
            }
            "agent.session.failed" | "agent.session.environment.failed" => Vec::new(),
            "agent.session.turn.cancelled" => {
                self.outcome = Some(RootTurnOutcome::Cancelled);
                vec![self.request(TurnCancelledData {
                    turn_id: self.turn_id,
                    reason: Some("OpenAI Agents API turn cancelled".to_string()),
                    usage: usage_from(provider_event),
                })]
            }
            "agent.session.idle" if self.outcome.is_some() => {
                vec![self.request(SessionIdledData {
                    turn_id: self.turn_id,
                    iterations: Some(1),
                    usage: self.last_usage.clone(),
                })]
            }
            other if other.ends_with(".failed") || other.ends_with(".cancelled") => {
                return Err(AgentsApiPrototypeError::UnsupportedTerminalEvent(
                    other.to_string(),
                ));
            }
            _ => Vec::new(),
        };
        for event in &mut mapped {
            event.metadata = Some(metadata.clone());
        }
        Ok(mapped)
    }

    fn fail(&mut self, error: Option<&Value>, fallback: &str) -> Vec<EventRequest> {
        let code = error
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| self.last_error.as_ref().and_then(|(code, _)| code.clone()));
        let message = error
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| self.last_error.as_ref().map(|(_, message)| message.clone()))
            .unwrap_or_else(|| fallback.to_string());
        self.outcome = Some(RootTurnOutcome::Failed {
            code: code.clone(),
            message: message.clone(),
        });
        vec![self.request(TurnFailedData {
            turn_id: self.turn_id,
            error: message,
            error_code: code,
            error_fields: None,
            error_disclosure: None,
        })]
    }

    fn map_item(
        &mut self,
        provider_event: &Value,
        event_type: &str,
    ) -> Result<Vec<EventRequest>, AgentsApiPrototypeError> {
        let Some(item) = provider_event.get("item") else {
            return Ok(Vec::new());
        };
        match item.get("type").and_then(Value::as_str) {
            Some("mcp_call") => {}
            // The result Everruns submitted, echoed back by the provider: the
            // point where the session view shows the client function as done.
            Some("function_call_output") => {
                let call_id = string_field(item, event_type, "call_id")?;
                let Some(tool_name) = self.requested_tool_calls.get(&call_id).cloned() else {
                    return Ok(Vec::new());
                };
                if !self.completed_tool_calls.insert(call_id.clone()) {
                    return Ok(Vec::new());
                }
                let output = provider_output_text(item);
                return Ok(vec![self.request(
                    if item.get("status").and_then(Value::as_str) == Some("failed") {
                        ToolCompletedData::failure(call_id, tool_name, "error".into(), output, None)
                    } else {
                        ToolCompletedData::success(
                            call_id,
                            tool_name,
                            vec![ContentPart::text(output)],
                            None,
                        )
                    },
                )]);
            }
            // Each assistant message item (commentary or final answer) is its
            // own Everruns output message, as with the native runtime.
            Some("message") if item.get("role").and_then(Value::as_str) == Some("assistant") => {
                let id = item.get("id").and_then(Value::as_str);
                return Ok(if event_type == "agent.session.turn.item.done" {
                    if id.is_some() && id == self.open_item.as_deref() {
                        self.close_message()
                    } else {
                        Vec::new()
                    }
                } else {
                    let phase = match item.get("phase").and_then(Value::as_str) {
                        Some("commentary") => Some(ExecutionPhase::Commentary),
                        Some("final_answer") => Some(ExecutionPhase::FinalAnswer),
                        _ => None,
                    };
                    self.open_message(id, phase)
                });
            }
            _ => return Ok(Vec::new()),
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
        let status = item.get("status").and_then(Value::as_str);
        let error = item.get("error").filter(|error| !error.is_null());
        if (status == Some("failed") || error.is_some())
            && self.completed_tool_calls.insert(call_id.clone())
        {
            // A failed call reports its cause in `output`, with `error` null.
            let message = error
                .and_then(|e| e.get("message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| provider_output_text(item));
            events.push(
                self.request(
                    ToolCompletedData::failure(
                        call_id,
                        tool_name,
                        "error".to_string(),
                        message,
                        item.get("duration_ms").and_then(Value::as_u64),
                    )
                    .with_display_name(Some(format!("{server_label}: {source_name}"))),
                ),
            );
        } else if status == Some("completed") && self.completed_tool_calls.insert(call_id.clone()) {
            events.push(
                self.request(
                    ToolCompletedData::success(
                        call_id,
                        tool_name,
                        vec![ContentPart::text(provider_output_text(item))],
                        item.get("duration_ms").and_then(Value::as_u64),
                    )
                    .with_display_name(Some(format!("{server_label}: {source_name}"))),
                ),
            );
        }
        Ok(events)
    }

    fn map_completed(&mut self, provider_event: &Value) -> Vec<EventRequest> {
        self.outcome = Some(RootTurnOutcome::Completed);
        let mut events = self.close_message();
        // Usage is best-effort and usually still null here; a production
        // backend upserts it later from the turn resource (EVE-1125).
        let usage = usage_from(provider_event);
        self.last_usage = usage.clone();
        let duration_ms = turn_duration_ms(provider_event);
        let (final_message_id, final_text) = self
            .final_answer
            .clone()
            .map_or((None, String::new()), |(id, text)| (Some(id), text));
        events.push(
            self.request(LlmGenerationData::success_with_metadata(
                Vec::new(),
                Vec::new(),
                Some(final_text.clone()),
                Vec::new(),
                self.model.clone(),
                Some("openai_agents_api".to_string()),
                usage.clone(),
                duration_ms,
                None,
                Some(vec!["stop".to_string()]),
                provider_event
                    .get("turn_id")
                    .or_else(|| provider_event.pointer("/turn/id"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )),
        );
        events.push(self.request(TurnCompletedData {
            turn_id: self.turn_id,
            iterations: 1,
            duration_ms,
            usage: usage.clone(),
            input_content: None,
            final_message_id,
            final_answer_preview: Some(final_text.chars().take(500).collect()),
            time_to_first_token_ms: None,
            tool_call_count: Some(self.tool_call_count),
            llm_call_count: None,
            status: Some("completed".to_string()),
        }));
        events
    }

    /// Open an output message for provider item `item_id`, closing the
    /// previous one when the item changes. Text without an item id continues
    /// whatever message is open.
    fn open_message(
        &mut self,
        item_id: Option<&str>,
        phase: Option<ExecutionPhase>,
    ) -> Vec<EventRequest> {
        if self.message_open && (item_id.is_none() || item_id == self.open_item.as_deref()) {
            if phase.is_some() {
                self.output_phase = phase;
            }
            return Vec::new();
        }
        let mut events = self.close_message();
        self.open_item = item_id.map(str::to_string);
        self.output_message_id = MessageId::new();
        self.accumulated_output.clear();
        self.output_phase = phase;
        self.message_open = true;
        events.push(self.request(OutputMessageStartedData {
            reasoning_state: None,
            turn_id: self.turn_id,
            message_id: self.output_message_id,
            model: Some(self.model.clone()),
            iteration: Some(1),
            phase,
        }));
        events
    }

    fn close_message(&mut self) -> Vec<EventRequest> {
        if !self.message_open {
            return Vec::new();
        }
        self.message_open = false;
        let mut message = RuntimeMessage::assistant(self.accumulated_output.clone())
            .with_id(self.output_message_id);
        if let Some(phase) = self.output_phase {
            message = message.with_phase(phase);
        }
        if self.output_phase != Some(ExecutionPhase::Commentary) {
            self.final_answer = Some((self.output_message_id, self.accumulated_output.clone()));
        }
        vec![self.request(
            OutputMessageCompletedData::new(message).with_metadata(ModelMetadata {
                model: self.model.clone(),
                model_id: None,
                provider_id: None,
            }),
        )]
    }

    fn delta(&self, delta: String) -> EventRequest {
        self.request(OutputMessageDeltaData {
            turn_id: self.turn_id,
            message_id: self.output_message_id,
            delta,
            accumulated: self.accumulated_output.clone(),
            phase: self.output_phase,
        })
    }

    fn request(&self, data: impl Into<everruns_core::events::EventData>) -> EventRequest {
        EventRequest::new(
            self.session_id,
            EventContext::turn(self.turn_id, self.input_message_id),
            data,
        )
    }
}

/// The text of a provider tool output: MCP `{content: [{text}]}`, a function
/// output `[{text}]`, or anything else serialized.
fn provider_output_text(item: &Value) -> String {
    let output = item.get("output").unwrap_or(&Value::Null);
    let parts = output
        .get("content")
        .or(Some(output))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty());
    parts.unwrap_or_else(|| match output {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    })
}

fn item_id(provider_event: &Value) -> Option<&str> {
    provider_event.get("item_id").and_then(Value::as_str)
}

fn is_subagent_event(provider_event: &Value) -> bool {
    provider_event
        .pointer("/turn/subagent_id")
        .or_else(|| provider_event.get("subagent_id"))
        .is_some_and(|id| !id.is_null())
}

fn turn_duration_ms(provider_event: &Value) -> Option<u64> {
    let started = provider_event.pointer("/turn/started_at")?.as_u64()?;
    let completed = provider_event.pointer("/turn/completed_at")?.as_u64()?;
    completed.checked_sub(started).map(|seconds| seconds * 1000)
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

/// OpenAI reports cached tokens inside `input_tokens`; Everruns keeps disjoint
/// buckets (see [`TokenUsage`]), so the cached subset is subtracted here. The
/// provider usage is best-effort and may be null, which is not zero.
fn usage_from(event: &Value) -> Option<TokenUsage> {
    let usage = event
        .pointer("/turn/usage")
        .or_else(|| event.get("usage"))
        .filter(|usage| !usage.is_null())?;
    let count = |pointer: &str| {
        usage
            .pointer(pointer)
            .and_then(Value::as_u64)
            .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
    };
    let input = count("/input_tokens").unwrap_or_default();
    let cached = count("/input_tokens_details/cached_tokens");
    Some(TokenUsage::with_cache(
        input.saturating_sub(cached.unwrap_or_default()),
        count("/output_tokens").unwrap_or_default(),
        cached,
        None,
    ))
}

/// Default Agents API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
/// Beta header value the Agents API requires on every request.
pub const BETA_HEADER: &str = "agents=v1";

/// Minimal HTTP client for the three calls the prototype needs: create a
/// streamed session, send input events, and retrieve a session.
#[derive(Clone)]
pub struct AgentsApiClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl AgentsApiClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: api_key.into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base_url))
            .bearer_auth(&self.api_key)
            .header("OpenAI-Beta", BETA_HEADER)
    }

    async fn send(
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, AgentsApiPrototypeError> {
        let response = request
            .send()
            .await
            .map_err(|error| AgentsApiPrototypeError::Http(error.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(AgentsApiPrototypeError::Api {
            status: status.as_u16(),
            body: body.chars().take(2000).collect(),
        })
    }

    /// `POST /agents/sessions` with `stream: true`; yields each SSE `data` payload.
    pub async fn create_session_stream(
        &self,
        config: &AgentsApiSessionConfig,
    ) -> Result<
        impl futures::Stream<Item = Result<Value, AgentsApiPrototypeError>> + use<>,
        AgentsApiPrototypeError,
    > {
        let mut body = config.clone();
        body.stream = true;
        let response = Self::send(
            self.request(reqwest::Method::POST, "/agents/sessions")
                .header("Accept", "text/event-stream")
                .json(&body),
        )
        .await?;
        Ok(response.bytes_stream().eventsource().filter_map(|event| {
            futures::future::ready(match event {
                Ok(event) if event.data.trim().is_empty() || event.data == "[DONE]" => None,
                Ok(event) => Some(serde_json::from_str(&event.data).map_err(|error| {
                    AgentsApiPrototypeError::Http(format!("invalid SSE payload: {error}"))
                })),
                Err(error) => Some(Err(AgentsApiPrototypeError::Http(error.to_string()))),
            })
        }))
    }

    /// `POST /agents/sessions/{id}/events`: submit input, tool results, or cancel.
    pub async fn send_events(
        &self,
        session_id: &str,
        events: Vec<Value>,
    ) -> Result<(), AgentsApiPrototypeError> {
        Self::send(
            self.request(
                reqwest::Method::POST,
                &format!("/agents/sessions/{}/events", path_segment(session_id)?),
            )
            .json(&json!({ "events": events })),
        )
        .await?;
        Ok(())
    }

    /// `GET /agents/sessions/{id}`: the reconciliation source after a disconnect.
    pub async fn retrieve_session(
        &self,
        session_id: &str,
    ) -> Result<Value, AgentsApiPrototypeError> {
        Self::send(self.request(
            reqwest::Method::GET,
            &format!("/agents/sessions/{}", path_segment(session_id)?),
        ))
        .await?
        .json()
        .await
        .map_err(|error| AgentsApiPrototypeError::Http(error.to_string()))
    }
}

/// Provider ids are opaque; refuse anything that could alter the request path.
fn path_segment(id: &str) -> Result<&str, AgentsApiPrototypeError> {
    if !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(id)
    } else {
        Err(AgentsApiPrototypeError::Http(format!(
            "invalid provider id '{id}'"
        )))
    }
}

/// Run one root turn end to end: create the session, project every provider
/// event through `mapper` into `sink`, and answer each pending function call.
///
/// `handler` is the seam where Everruns keeps control of client functions: in
/// a production backend it runs permission checks, approval policy, `jev`
/// guardrails, and the durable tool-result claim before returning, and the
/// Agents API waits for it. MCP and built-in tools never reach it.
pub async fn run_root_turn<H, F>(
    client: &AgentsApiClient,
    config: &AgentsApiSessionConfig,
    mapper: &mut AgentsApiEventMapper,
    mut handler: H,
    mut sink: impl FnMut(EventRequest),
) -> Result<RootTurnOutcome, AgentsApiPrototypeError>
where
    H: FnMut(FunctionCallAction) -> F,
    F: std::future::Future<Output = Result<String, String>>,
{
    let stream = client.create_session_stream(config).await?;
    futures::pin_mut!(stream);
    let mut answered = HashSet::new();
    while let Some(provider_event) = stream.next().await {
        let provider_event = provider_event?;
        for event in mapper.map(&provider_event)? {
            sink(event);
        }
        match provider_event.get("type").and_then(Value::as_str) {
            Some("agent.session.requires_action") => {
                let session_id = mapper
                    .provider_session_id()
                    .ok_or(AgentsApiPrototypeError::MissingEventField {
                        event_type: "agent.session.requires_action".to_string(),
                        field: "session.id",
                    })?
                    .to_string();
                for action in FunctionCallAction::from_required_actions(&provider_event) {
                    if !answered.insert(action.call_id.clone()) {
                        continue;
                    }
                    let result = handler(action.clone()).await;
                    client
                        .send_events(&session_id, vec![build_tool_result_input(&action, result)])
                        .await?;
                }
            }
            Some("agent.session.idle") if mapper.outcome().is_some() => break,
            _ => {}
        }
    }
    mapper.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::tool_types::ClientSideTool;

    fn prototype_config() -> AgentsApiSessionConfig {
        let mut agent = RuntimeAgent::new("Use both tools.", "gpt-6-astra");
        agent
            .tools
            .push(ToolDefinition::ClientSide(ClientSideTool::new(
                "lookup_customer",
                "Look up one customer",
                json!({
                    "type": "object",
                    "properties": {"customer_id": {"type": "string"}},
                    "required": ["customer_id"],
                    "additionalProperties": false
                }),
            )));
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
    fn documented_boundary_fixture_projects_to_canonical_session_events() {
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
        // Replayed provider events are deduplicated by call id.
        assert!(mapper.map(&fixture[2]).unwrap().is_empty());
        assert!(mapper.map(&fixture[3]).unwrap().is_empty());
        assert_eq!(mapper.outcome(), Some(&RootTurnOutcome::Completed));
        assert_eq!(mapper.provider_session_id(), Some("sess_fixture"));
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
                "input_tokens": 61,
                "output_tokens": 23,
                "cache_read_tokens": 40
            })
        );
        assert_eq!(
            serde_json::to_value(&completed.data).unwrap()["final_answer_preview"],
            "Customer 123 is documented."
        );
        assert_eq!(
            serde_json::to_value(&completed.data).unwrap()["duration_ms"],
            1000
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
            completed.metadata.as_ref().unwrap()["provider_usage"]["total_tokens"],
            124
        );
    }

    #[test]
    fn live_round_trip_projects_messages_tools_and_the_final_answer() {
        // Recorded from the live API on 2026-09-30: one client function, one
        // HTTP MCP server (a search that succeeded and a fetch that failed),
        // a commentary preamble, and a final answer. MCP outputs are trimmed.
        let fixture: Vec<Value> = serde_json::from_str(include_str!(
            "../tests/fixtures/agents_api_live_round_trip.json"
        ))
        .unwrap();
        let mut mapper = mapper();
        let events = fixture
            .iter()
            .flat_map(|event| mapper.map(event).unwrap())
            .collect::<Vec<_>>();
        let data = |event: &EventRequest| serde_json::to_value(&event.data).unwrap();
        let types = events
            .iter()
            .map(|event| event.event_type.as_str())
            .filter(|kind| *kind != "output.message.delta")
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                "turn.started",
                "output.message.started",
                "output.message.completed",
                "tool.call_requested",
                "tool.completed",
                "tool.started",
                "tool.completed",
                "tool.started",
                "tool.completed",
                "output.message.started",
                "output.message.completed",
                "llm.generation",
                "turn.completed",
                "session.idled",
            ]
        );

        let messages = events
            .iter()
            .filter(|event| event.event_type == "output.message.completed")
            .map(data)
            .collect::<Vec<_>>();
        assert_eq!(messages[0]["message"]["phase"], "commentary");
        assert_eq!(messages[1]["message"]["phase"], "final_answer");
        assert_ne!(messages[0]["message"]["id"], messages[1]["message"]["id"]);

        let tools = events
            .iter()
            .filter(|event| event.event_type == "tool.completed")
            .map(data)
            .collect::<Vec<_>>();
        assert_eq!(tools[0]["tool_name"], "lookup_customer");
        assert_eq!(tools[1]["tool_name"], "mcp_docs__search_openai_docs");
        assert_eq!(tools[2]["tool_name"], "mcp_docs__fetch_openai_doc");
        assert!(
            tools[2]["error"]
                .as_str()
                .unwrap()
                .contains("404 Not Found")
        );

        let completed = data(events.iter().rev().nth(1).unwrap());
        assert_eq!(completed["final_message_id"], messages[1]["message"]["id"]);
        assert!(
            completed["final_answer_preview"]
                .as_str()
                .unwrap()
                .starts_with("Customer 123 is Ada Lovelace")
        );
        assert_eq!(completed["tool_call_count"], 3);
        // Usage had not arrived when the turn completed.
        assert!(completed.get("usage").is_none());
        assert_eq!(mapper.finish().unwrap(), RootTurnOutcome::Completed);
    }

    #[test]
    fn live_failed_turn_projects_the_error_cause() {
        // Recorded from the live API on 2026-09-30; the org had no credits left.
        let fixture: Vec<Value> = serde_json::from_str(include_str!(
            "../tests/fixtures/agents_api_live_failed_turn.json"
        ))
        .unwrap();
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
        let types = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(types, vec!["turn.started", "turn.failed", "session.idled"]);
        let failed = serde_json::to_value(&events[1].data).unwrap();
        assert_eq!(failed["error_code"], "usage_limit_exceeded");
        assert!(matches!(
            mapper.finish().unwrap(),
            RootTurnOutcome::Failed { code: Some(code), .. } if code == "usage_limit_exceeded"
        ));
        assert!(
            mapper
                .provider_session_id()
                .is_some_and(|id| id.starts_with("sess_"))
        );
    }

    fn mapper() -> AgentsApiEventMapper {
        AgentsApiEventMapper::new(
            SessionId::from_seed(1),
            TurnId::from_seed(2),
            MessageId::from_seed(3),
            "gpt-6-astra",
        )
    }

    #[test]
    fn stream_closing_before_a_terminal_root_turn_fails_closed() {
        let mut mapper = mapper();
        mapper
            .map(
                &json!({"type": "agent.session.turn.created", "turn_id": "turn_1",
                "turn": {"id": "turn_1", "subagent_id": null}}),
            )
            .unwrap();
        // Idle without a terminal turn is not success and emits nothing.
        assert!(
            mapper
                .map(&json!({"type": "agent.session.idle", "session": {"id": "sess_1"}}))
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            mapper.finish(),
            Err(AgentsApiPrototypeError::StreamClosedBeforeTurnEnded)
        ));
    }

    #[test]
    fn subagent_turns_never_end_the_root_turn() {
        let mut mapper = mapper();
        let events = mapper
            .map(
                &json!({"type": "agent.session.turn.failed", "turn_id": "turn_child",
                "turn": {"id": "turn_child", "subagent_id": "sub_1",
                    "error": {"message": "child failed"}}}),
            )
            .unwrap();
        assert!(events.is_empty());
        assert!(mapper.outcome().is_none());
    }

    #[test]
    fn unknown_terminal_event_fails_closed_and_unknown_progress_is_ignored() {
        let mut mapper = mapper();
        assert!(
            mapper
                .map(&json!({"type": "agent.session.turn.reasoning.delta"}))
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            mapper.map(&json!({"type": "agent.session.sandbox.failed"})),
            Err(AgentsApiPrototypeError::UnsupportedTerminalEvent(kind))
                if kind == "agent.session.sandbox.failed"
        ));
    }

    #[test]
    fn unknown_usage_is_not_reported_as_zero() {
        assert!(usage_from(&json!({"turn": {"usage": null}})).is_none());
    }

    #[test]
    fn function_result_builds_agents_api_continuation_input() {
        let requires_action = json!({
            "type": "agent.session.requires_action",
            "session": {"id": "sess_1", "required_actions": [
                {"type": "environment_connection"},
                {"type": "function_call", "turn_id": "turn_1", "call_id": "call_customer",
                 "name": "lookup_customer", "arguments": {"customer_id": "123"}}
            ]}
        });
        let actions = FunctionCallAction::from_required_actions(&requires_action);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            build_tool_result_input(&actions[0], Ok(r#"{"name":"Ada"}"#.to_string())),
            json!({
                "type": "agent.session.input.tool_result",
                "turn_id": "turn_1",
                "call_id": "call_customer",
                "success": true,
                "output": "{\"name\":\"Ada\"}"
            })
        );
        assert_eq!(
            build_tool_result_input(&actions[0], Err("denied by approval policy".to_string())),
            json!({
                "type": "agent.session.input.tool_result",
                "turn_id": "turn_1",
                "call_id": "call_customer",
                "success": false,
                "error": "denied by approval policy"
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
        assert_eq!(
            imported.mcp_servers["docs"].url,
            "https://developers.openai.com/mcp"
        );
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
