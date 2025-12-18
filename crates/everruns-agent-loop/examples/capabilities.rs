//! Capabilities Example - Agent Loop with Capability System
//!
//! This example demonstrates how to use the capabilities system to compose
//! agent functionality through modular units. Capabilities can contribute:
//! - System prompt additions
//! - Tools for the agent
//!
//! The example shows:
//! 1. Using built-in capabilities (CurrentTime)
//! 2. Creating custom capabilities
//! 3. Applying capabilities to build an AgentConfig
//! 4. Running the agent loop with capabilities
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-agent-loop --example capabilities

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use everruns_agent_loop::{
    apply_capabilities,
    capabilities::{Capability, CapabilityId, CapabilityRegistry, CapabilityStatus},
    config::AgentConfig,
    memory::{InMemoryEventEmitter, InMemoryMessageStore},
    message::{ConversationMessage, MessageContent, MessageRole},
    tools::{Tool, ToolExecutionResult},
    traits::{
        LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmMessageRole, LlmProvider,
        LlmResponseStream, LlmStreamEvent,
    },
    AgentLoop, AgentLoopError, Result, ToolCall, ToolDefinition,
};
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// ============================================================================
// OpenAI Provider (Same as tool_calling example)
// ============================================================================

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";

struct OpenAiProvider {
    client: Client,
    api_key: String,
}

impl OpenAiProvider {
    fn new() -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| AgentLoopError::llm("OPENAI_API_KEY environment variable not set"))?;
        Ok(Self {
            client: Client::new(),
            api_key,
        })
    }

    fn convert_message(msg: &LlmMessage) -> OpenAiMessage {
        let role = match msg.role {
            LlmMessageRole::System => "system",
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        };

        OpenAiMessage {
            role: role.to_string(),
            content: Some(msg.content.clone()),
            tool_calls: msg.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .map(|tc| OpenAiToolCall {
                        id: tc.id.clone(),
                        r#type: "function".to_string(),
                        function: OpenAiFunctionCall {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        },
                    })
                    .collect()
            }),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<OpenAiTool> {
        tools
            .iter()
            .map(|tool| {
                let (name, description, parameters) = match tool {
                    ToolDefinition::Webhook(webhook) => {
                        (&webhook.name, &webhook.description, &webhook.parameters)
                    }
                    ToolDefinition::Builtin(builtin) => {
                        (&builtin.name, &builtin.description, &builtin.parameters)
                    }
                };

                OpenAiTool {
                    r#type: "function".to_string(),
                    function: OpenAiFunction {
                        name: name.clone(),
                        description: description.clone(),
                        parameters: parameters.clone(),
                    },
                }
            })
            .collect()
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let openai_messages: Vec<OpenAiMessage> =
            messages.iter().map(Self::convert_message).collect();

        let tools = if config.tools.is_empty() {
            None
        } else {
            Some(Self::convert_tools(&config.tools))
        };

        let request = OpenAiRequest {
            model: config.model.clone(),
            messages: openai_messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            stream: true,
            tools,
        };

        let response = self
            .client
            .post(OPENAI_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AgentLoopError::llm(format!(
                "OpenAI API request failed with status {}: {}",
                status, error_text
            )));
        }

        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream.eventsource();

        let model = config.model.clone();
        let total_tokens = Arc::new(Mutex::new(0u32));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));

        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let total_tokens = Arc::clone(&total_tokens);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            async move {
                match result {
                    Ok(event) => {
                        if event.data == "[DONE]" {
                            let tokens = *total_tokens.lock().unwrap();
                            return Ok(LlmStreamEvent::Done(LlmCompletionMetadata {
                                total_tokens: Some(tokens),
                                prompt_tokens: None,
                                completion_tokens: Some(tokens),
                                model: Some(model.clone()),
                                finish_reason: Some("stop".to_string()),
                            }));
                        }

                        match serde_json::from_str::<OpenAiStreamChunk>(&event.data) {
                            Ok(chunk) => {
                                if let Some(choice) = chunk.choices.first() {
                                    if let Some(tool_calls) = &choice.delta.tool_calls {
                                        let mut acc = accumulated_tool_calls.lock().unwrap();

                                        for tc in tool_calls {
                                            let idx = tc.index as usize;
                                            while acc.len() <= idx {
                                                acc.push(ToolCall {
                                                    id: String::new(),
                                                    name: String::new(),
                                                    arguments: json!(""),
                                                });
                                            }

                                            if let Some(id) = &tc.id {
                                                acc[idx].id = id.clone();
                                            }
                                            if let Some(function) = &tc.function {
                                                if let Some(name) = &function.name {
                                                    acc[idx].name = name.clone();
                                                }
                                                if let Some(args) = &function.arguments {
                                                    let current =
                                                        acc[idx].arguments.as_str().unwrap_or("");
                                                    let combined = format!("{}{}", current, args);
                                                    acc[idx].arguments = json!(combined);
                                                }
                                            }
                                        }
                                        return Ok(LlmStreamEvent::TextDelta(String::new()));
                                    }

                                    if let Some(content) = &choice.delta.content {
                                        *total_tokens.lock().unwrap() += 1;
                                        return Ok(LlmStreamEvent::TextDelta(content.clone()));
                                    }

                                    if let Some(finish_reason) = &choice.finish_reason {
                                        let tokens = *total_tokens.lock().unwrap();

                                        if finish_reason == "tool_calls" {
                                            let tool_calls =
                                                accumulated_tool_calls.lock().unwrap().clone();
                                            if !tool_calls.is_empty() {
                                                let parsed_calls: Vec<ToolCall> = tool_calls
                                                    .into_iter()
                                                    .map(|mut tc| {
                                                        if let Some(args_str) =
                                                            tc.arguments.as_str()
                                                        {
                                                            tc.arguments =
                                                                serde_json::from_str(args_str)
                                                                    .unwrap_or(json!({}));
                                                        }
                                                        tc
                                                    })
                                                    .collect();
                                                return Ok(LlmStreamEvent::ToolCalls(parsed_calls));
                                            }
                                        }

                                        return Ok(LlmStreamEvent::Done(LlmCompletionMetadata {
                                            total_tokens: Some(tokens),
                                            prompt_tokens: None,
                                            completion_tokens: Some(tokens),
                                            model: Some(model.clone()),
                                            finish_reason: Some(finish_reason.clone()),
                                        }));
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            Err(e) => Ok(LlmStreamEvent::Error(format!(
                                "Failed to parse chunk: {}",
                                e
                            ))),
                        }
                    }
                    Err(e) => Ok(LlmStreamEvent::Error(format!("Stream error: {}", e))),
                }
            }
        }));

        Ok(converted_stream)
    }
}

// OpenAI API types (same as tool_calling example)
#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiTool {
    r#type: String,
    function: OpenAiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    r#type: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: u32,
    id: Option<String>,
    function: Option<OpenAiStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

// ============================================================================
// Custom Capability: Calculator
// ============================================================================

/// A custom capability that provides a calculator tool
struct CalculatorCapability;

impl Capability for CalculatorCapability {
    fn id(&self) -> CapabilityId {
        // Using Noop as placeholder since we can't add new variants
        // In a real application, you'd extend CapabilityId
        CapabilityId::Noop
    }

    fn name(&self) -> &str {
        "Calculator"
    }

    fn description(&self) -> &str {
        "Provides a calculator tool for basic arithmetic operations."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("calculator")
    }

    fn category(&self) -> Option<&str> {
        Some("Utilities")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(CalculatorTool)]
    }
}

/// Calculator tool implementation
struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculate"
    }

    fn description(&self) -> &str {
        "Perform basic arithmetic calculations. Supports add, subtract, multiply, and divide operations."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "The operation to perform",
                    "enum": ["add", "subtract", "multiply", "divide"]
                },
                "a": {
                    "type": "number",
                    "description": "First operand"
                },
                "b": {
                    "type": "number",
                    "description": "Second operand"
                }
            },
            "required": ["operation", "a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let operation = arguments.get("operation").and_then(|v| v.as_str());
        let a = arguments.get("a").and_then(|v| v.as_f64());
        let b = arguments.get("b").and_then(|v| v.as_f64());

        match (operation, a, b) {
            (Some(op), Some(a), Some(b)) => {
                let result = match op {
                    "add" => a + b,
                    "subtract" => a - b,
                    "multiply" => a * b,
                    "divide" => {
                        if b == 0.0 {
                            return ToolExecutionResult::tool_error(
                                "Division by zero is not allowed",
                            );
                        }
                        a / b
                    }
                    _ => {
                        return ToolExecutionResult::tool_error(format!(
                            "Unknown operation: {}",
                            op
                        ))
                    }
                };

                ToolExecutionResult::success(json!({
                    "expression": format!("{} {} {}", a, op, b),
                    "result": result
                }))
            }
            _ => ToolExecutionResult::tool_error(
                "Missing required parameters: operation, a, and b are required",
            ),
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn print_conversation_steps(messages: &[ConversationMessage]) {
    println!("\n  Steps:");
    for (i, msg) in messages.iter().enumerate() {
        match msg.role {
            MessageRole::User => {
                println!("    {}. [User] {}", i + 1, msg.content.to_llm_string());
            }
            MessageRole::Assistant => {
                let text = msg.content.to_llm_string();
                if let Some(ref tool_calls) = msg.tool_calls {
                    if !tool_calls.is_empty() {
                        println!("    {}. [Assistant] Calling tool(s):", i + 1);
                        for tc in tool_calls {
                            println!("       -> {}({})", tc.name, tc.arguments);
                        }
                        if !text.is_empty() {
                            println!("       Text: {}", text);
                        }
                    } else if !text.is_empty() {
                        println!("    {}. [Assistant] {}", i + 1, text);
                    }
                } else if !text.is_empty() {
                    println!("    {}. [Assistant] {}", i + 1, text);
                }
            }
            MessageRole::ToolCall => {
                // Skip - already shown in assistant message
            }
            MessageRole::ToolResult => {
                if let MessageContent::ToolResult { result, error } = &msg.content {
                    if let Some(err) = error {
                        println!("    {}. [Tool Result] Error: {}", i + 1, err);
                    } else if let Some(res) = result {
                        println!("    {}. [Tool Result] {}", i + 1, res);
                    }
                }
            }
            MessageRole::System => {
                // Skip system messages
            }
        }
    }
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("Error: OPENAI_API_KEY environment variable is not set");
        eprintln!("Please set it before running this example:");
        eprintln!("  export OPENAI_API_KEY=your-api-key");
        std::process::exit(1);
    }

    println!("=== Capabilities Demo (agent-loop) ===\n");

    // Example 1: Using built-in CurrentTime capability
    example_builtin_capability().await?;

    // Example 2: Using custom capability
    example_custom_capability().await?;

    // Example 3: Multiple capabilities
    example_multiple_capabilities().await?;

    println!("=== Demo completed! ===");
    Ok(())
}

/// Example 1: Using the built-in CurrentTime capability
async fn example_builtin_capability() -> anyhow::Result<()> {
    println!("--- Example 1: Built-in CurrentTime Capability ---\n");

    // Create capability registry with built-in capabilities
    let registry = CapabilityRegistry::with_builtins();

    // Base agent config
    let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-4o-mini");

    // Apply the CurrentTime capability
    let capability_ids = vec![CapabilityId::CurrentTime];
    let applied = apply_capabilities(base_config, &capability_ids, &registry);

    println!("Applied capabilities: {:?}", applied.applied_ids);
    println!("Tools available: {:?}", applied.tool_registry.tool_names());
    println!();

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAiProvider::new()?;

    // Seed with user message
    let session_id = Uuid::now_v7();
    let user_message = "What's the current time?";
    message_store
        .seed(session_id, vec![ConversationMessage::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    // Create and run agent loop with capability tools
    let agent_loop = AgentLoop::new(
        applied.config,
        event_emitter,
        message_store,
        llm_provider,
        applied.tool_registry,
    );

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

/// Example 2: Using a custom Calculator capability
async fn example_custom_capability() -> anyhow::Result<()> {
    println!("--- Example 2: Custom Calculator Capability ---\n");

    // Create custom registry with our calculator capability
    let mut registry = CapabilityRegistry::new();
    registry.register(CalculatorCapability);

    // Base agent config
    let base_config = AgentConfig::new("You are a helpful math assistant.", "gpt-4o-mini");

    // Apply the custom capability (using Noop ID as placeholder)
    let capability_ids = vec![CapabilityId::Noop];
    let applied = apply_capabilities(base_config, &capability_ids, &registry);

    println!("Applied capabilities: {:?}", applied.applied_ids);
    println!("Tools available: {:?}", applied.tool_registry.tool_names());
    println!();

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAiProvider::new()?;

    // Seed with user message
    let session_id = Uuid::now_v7();
    let user_message = "What is 123 multiplied by 456?";
    message_store
        .seed(session_id, vec![ConversationMessage::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    // Create and run agent loop
    let agent_loop = AgentLoop::new(
        applied.config,
        event_emitter,
        message_store,
        llm_provider,
        applied.tool_registry,
    );

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

/// Example 3: Multiple capabilities combined
async fn example_multiple_capabilities() -> anyhow::Result<()> {
    println!("--- Example 3: Multiple Capabilities Combined ---\n");

    // Create registry with built-in capabilities
    let registry = CapabilityRegistry::with_builtins();

    // Base agent config
    let base_config = AgentConfig::new(
        "You are a helpful assistant with access to time and other utilities.",
        "gpt-4o-mini",
    );

    // Apply multiple capabilities
    let capability_ids = vec![CapabilityId::CurrentTime, CapabilityId::Noop];
    let applied = apply_capabilities(base_config, &capability_ids, &registry);

    println!("Applied capabilities: {:?}", applied.applied_ids);
    println!("Tools available: {:?}", applied.tool_registry.tool_names());
    println!();

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAiProvider::new()?;

    // Seed with user message
    let session_id = Uuid::now_v7();
    let user_message = "What time is it in human-readable format?";
    message_store
        .seed(session_id, vec![ConversationMessage::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    // Create and run agent loop
    let agent_loop = AgentLoop::new(
        applied.config,
        event_emitter,
        message_store,
        llm_provider,
        applied.tool_registry,
    );

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}
