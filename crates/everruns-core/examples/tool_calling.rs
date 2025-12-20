//! Tool Calling Example - Agent Loop with Tool Trait
//!
//! This example demonstrates tool calling using the Tool trait abstraction
//! and ToolRegistry for tool management. Uses OpenAI as the LLM provider.
//!
//! The OpenAI adapter is included inline to keep the example self-contained
//! within the core crate (no dependency on worker crate).
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example tool_calling

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use everruns_core::{
    config::AgentConfig,
    memory::{InMemoryEventEmitter, InMemoryMessageStore},
    message::{ConversationMessage, MessageContent, MessageRole},
    tools::{Tool, ToolExecutionResult, ToolRegistry},
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
// OpenAI Provider (Self-contained for this example)
// ============================================================================

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Minimal OpenAI provider for the example
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
                                    // Handle tool calls
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

                                    // Handle content delta
                                    if let Some(content) = &choice.delta.content {
                                        *total_tokens.lock().unwrap() += 1;
                                        return Ok(LlmStreamEvent::TextDelta(content.clone()));
                                    }

                                    // Handle finish reason
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

// OpenAI API types
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
// Custom Tools
// ============================================================================

/// Tool that returns the current date and time
struct GetCurrentTime;

#[async_trait]
impl Tool for GetCurrentTime {
    fn name(&self) -> &str {
        "get_current_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time in various formats. Use this when asked about the current time or date."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "format": {
                    "type": "string",
                    "description": "Output format: 'iso8601' for ISO format, 'unix' for Unix timestamp, 'human' for readable format",
                    "enum": ["iso8601", "unix", "human"]
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let format = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("human");

        let now = chrono::Utc::now();

        let result = match format {
            "unix" => json!({
                "timestamp": now.timestamp(),
                "format": "unix"
            }),
            "iso8601" => json!({
                "datetime": now.to_rfc3339(),
                "format": "iso8601"
            }),
            _ => json!({
                "datetime": now.format("%A, %B %d, %Y at %H:%M:%S UTC").to_string(),
                "format": "human"
            }),
        };

        ToolExecutionResult::success(result)
    }
}

/// Tool that performs basic arithmetic calculations
struct Calculator;

#[async_trait]
impl Tool for Calculator {
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

/// Tool that provides random facts
struct RandomFact;

#[async_trait]
impl Tool for RandomFact {
    fn name(&self) -> &str {
        "get_random_fact"
    }

    fn description(&self) -> &str {
        "Get a random interesting fact about a given topic."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "The topic to get a fact about (e.g., 'science', 'history', 'nature')"
                }
            },
            "required": ["topic"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let topic = arguments
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("general");

        let fact = match topic.to_lowercase().as_str() {
            "science" => "The human brain uses approximately 20% of the body's total energy.",
            "history" => "The Great Wall of China is not visible from space with the naked eye.",
            "nature" => "Honey never spoils. Archaeologists have found 3000-year-old honey in Egyptian tombs that was still edible.",
            "space" => "A day on Venus is longer than a year on Venus.",
            "animals" => "Octopuses have three hearts and blue blood.",
            _ => "The average person walks about 100,000 miles in their lifetime.",
        };

        ToolExecutionResult::success(json!({
            "topic": topic,
            "fact": fact
        }))
    }
}

// ============================================================================
// Helper to print conversation steps
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
    // Set up logging (WARN level to reduce noise)
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    // Check for API key
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("Error: OPENAI_API_KEY environment variable is not set");
        eprintln!("Please set it before running this example:");
        eprintln!("  export OPENAI_API_KEY=your-api-key");
        std::process::exit(1);
    }

    println!("=== Tool Calling Demo (everruns-core) ===");
    println!("(Using OpenAI API with Tool trait abstraction)\n");

    // Run examples
    example_time_query().await?;
    example_calculation().await?;
    example_multi_tool().await?;

    println!("=== Demo completed! ===");
    Ok(())
}

/// Example 1: Ask about the current time
async fn example_time_query() -> anyhow::Result<()> {
    println!("--- Example 1: Time Query ---\n");

    // Create tool registry
    let registry = ToolRegistry::builder().tool(GetCurrentTime).build();

    // Create agent config with tools
    let config = AgentConfig::new(
        "You are a helpful assistant with access to a time tool. When asked about time, use the get_current_time tool.",
        "gpt-4o-mini",
    )
    .with_tools(registry.tool_definitions())
    .with_max_iterations(5);

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAiProvider::new()?;

    // Seed with user message
    let session_id = Uuid::now_v7();
    let user_message = "What time is it right now?";
    message_store
        .seed(session_id, vec![ConversationMessage::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    // Create and run agent loop
    let agent_loop = AgentLoop::new(config, event_emitter, message_store, llm_provider, registry);

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

/// Example 2: Perform a calculation
async fn example_calculation() -> anyhow::Result<()> {
    println!("--- Example 2: Calculation ---\n");

    let registry = ToolRegistry::builder().tool(Calculator).build();

    let config = AgentConfig::new(
        "You are a helpful calculator assistant. Use the calculate tool for math operations.",
        "gpt-4o-mini",
    )
    .with_tools(registry.tool_definitions())
    .with_max_iterations(5);

    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAiProvider::new()?;

    let session_id = Uuid::now_v7();
    let user_message = "What is 42 multiplied by 17?";
    message_store
        .seed(session_id, vec![ConversationMessage::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    let agent_loop = AgentLoop::new(config, event_emitter, message_store, llm_provider, registry);

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

/// Example 3: Multiple tools available
async fn example_multi_tool() -> anyhow::Result<()> {
    println!("--- Example 3: Multi-Tool Query ---\n");

    // Register multiple tools
    let registry = ToolRegistry::builder()
        .tool(GetCurrentTime)
        .tool(Calculator)
        .tool(RandomFact)
        .build();

    println!("Available tools: {:?}\n", registry.tool_names());

    let config = AgentConfig::new(
        "You are a helpful assistant with access to multiple tools: get_current_time for time queries, calculate for math, and get_random_fact for interesting facts. Use the appropriate tool based on the user's request.",
        "gpt-4o-mini",
    )
    .with_tools(registry.tool_definitions())
    .with_max_iterations(5);

    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAiProvider::new()?;

    let session_id = Uuid::now_v7();
    let user_message = "Tell me a random fact about nature.";
    message_store
        .seed(session_id, vec![ConversationMessage::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    let agent_loop = AgentLoop::new(config, event_emitter, message_store, llm_provider, registry);

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}
