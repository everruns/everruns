// V2 Activities - Mock implementations for testing
//
// Decision: Activities are pure async functions that can be mocked for testing
// Decision: Each activity type has a trait for dependency injection

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::types::*;

/// Trait for loading agent configuration
#[async_trait]
pub trait AgentLoader: Send + Sync {
    async fn load_agent(&self, input: LoadAgentInput) -> Result<AgentConfig, ActivityError>;
}

/// Trait for calling LLM
#[async_trait]
pub trait LlmCaller: Send + Sync {
    async fn call_llm(&self, input: CallLlmInput) -> Result<LlmResponse, ActivityError>;
}

/// Trait for executing tools
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute_tool(
        &self,
        input: ExecuteSingleToolInput,
    ) -> Result<ExecuteSingleToolOutput, ActivityError>;
}

/// Activity error
#[derive(Debug, Clone)]
pub struct ActivityError {
    pub message: String,
    pub is_retryable: bool,
}

impl ActivityError {
    pub fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
            is_retryable: false,
        }
    }

    pub fn retryable(message: &str) -> Self {
        Self {
            message: message.to_string(),
            is_retryable: true,
        }
    }
}

impl std::fmt::Display for ActivityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ActivityError {}

// =============================================================================
// Mock Implementations
// =============================================================================

/// Mock agent loader that returns predefined configs
#[derive(Debug, Clone)]
pub struct MockAgentLoader {
    agents: Arc<Mutex<HashMap<Uuid, AgentConfig>>>,
}

impl MockAgentLoader {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register an agent config
    pub fn register(&self, config: AgentConfig) {
        self.agents.lock().unwrap().insert(config.agent_id, config);
    }
}

impl Default for MockAgentLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentLoader for MockAgentLoader {
    async fn load_agent(&self, input: LoadAgentInput) -> Result<AgentConfig, ActivityError> {
        self.agents
            .lock()
            .unwrap()
            .get(&input.agent_id)
            .cloned()
            .ok_or_else(|| ActivityError::new(&format!("Agent {} not found", input.agent_id)))
    }
}

/// Mock LLM caller with scripted responses
#[derive(Debug)]
pub struct MockLlmCaller {
    responses: Arc<Mutex<Vec<LlmResponse>>>,
    default_response: LlmResponse,
    call_count: Arc<Mutex<usize>>,
}

impl MockLlmCaller {
    pub fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(Vec::new())),
            default_response: LlmResponse::text("I don't know how to respond."),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Add a scripted response (FIFO)
    pub fn add_response(&self, response: LlmResponse) {
        self.responses.lock().unwrap().push(response);
    }

    /// Set the default response when no scripted responses are available
    pub fn set_default(&mut self, response: LlmResponse) {
        self.default_response = response;
    }

    /// Get the number of calls made
    pub fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

impl Default for MockLlmCaller {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmCaller for MockLlmCaller {
    async fn call_llm(&self, _input: CallLlmInput) -> Result<LlmResponse, ActivityError> {
        *self.call_count.lock().unwrap() += 1;

        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            Ok(self.default_response.clone())
        } else {
            Ok(responses.remove(0))
        }
    }
}

/// Mock tool executor with predefined results
#[derive(Debug)]
pub struct MockToolExecutor {
    results: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    errors: Arc<Mutex<HashMap<String, String>>>,
}

impl MockToolExecutor {
    pub fn new() -> Self {
        Self {
            results: Arc::new(Mutex::new(HashMap::new())),
            errors: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a successful result for a tool
    pub fn register_result(&self, tool_name: &str, result: serde_json::Value) {
        self.results
            .lock()
            .unwrap()
            .insert(tool_name.to_string(), result);
    }

    /// Register an error for a tool
    pub fn register_error(&self, tool_name: &str, error: &str) {
        self.errors
            .lock()
            .unwrap()
            .insert(tool_name.to_string(), error.to_string());
    }
}

impl Default for MockToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for MockToolExecutor {
    async fn execute_tool(
        &self,
        input: ExecuteSingleToolInput,
    ) -> Result<ExecuteSingleToolOutput, ActivityError> {
        let tool_name = &input.tool_call.name;

        // Check for registered error
        if let Some(error) = self.errors.lock().unwrap().get(tool_name) {
            return Ok(ExecuteSingleToolOutput {
                result: ToolResult::error(&input.tool_call.id, error),
            });
        }

        // Check for registered result
        if let Some(result) = self.results.lock().unwrap().get(tool_name) {
            return Ok(ExecuteSingleToolOutput {
                result: ToolResult::success(&input.tool_call.id, result.clone()),
            });
        }

        // Default: return echo of arguments
        Ok(ExecuteSingleToolOutput {
            result: ToolResult::success(
                &input.tool_call.id,
                serde_json::json!({
                    "tool": tool_name,
                    "arguments": input.tool_call.arguments,
                    "echo": true
                }),
            ),
        })
    }
}

// =============================================================================
// Activity Context
// =============================================================================

/// Context for activity execution (holds all dependencies)
pub struct ActivityContext {
    pub agent_loader: Arc<dyn AgentLoader>,
    pub llm_caller: Arc<dyn LlmCaller>,
    pub tool_executor: Arc<dyn ToolExecutor>,
}

impl ActivityContext {
    /// Create a new context with mock implementations
    pub fn mock() -> Self {
        Self {
            agent_loader: Arc::new(MockAgentLoader::new()),
            llm_caller: Arc::new(MockLlmCaller::new()),
            tool_executor: Arc::new(MockToolExecutor::new()),
        }
    }

    /// Create a context with custom implementations
    pub fn new(
        agent_loader: Arc<dyn AgentLoader>,
        llm_caller: Arc<dyn LlmCaller>,
        tool_executor: Arc<dyn ToolExecutor>,
    ) -> Self {
        Self {
            agent_loader,
            llm_caller,
            tool_executor,
        }
    }
}

// =============================================================================
// Built-in Tools
// =============================================================================

/// Echo tool - returns the input arguments
pub fn echo_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "echo".to_string(),
        description: "Echoes back the input".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Message to echo"
                }
            },
            "required": ["message"]
        }),
    }
}

/// Get time tool - returns a mock timestamp
pub fn get_time_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "get_time".to_string(),
        description: "Gets the current time".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "Timezone (optional)"
                }
            }
        }),
    }
}

/// Calculator tool - performs basic math
pub fn calculator_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "calculator".to_string(),
        description: "Performs basic arithmetic operations".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["add", "subtract", "multiply", "divide"],
                    "description": "The operation to perform"
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
            "required": ["operation", "a", "b"]
        }),
    }
}

/// Built-in tool executor that handles common tools
pub struct BuiltinToolExecutor;

impl BuiltinToolExecutor {
    pub fn execute(tool_call: &ToolCall) -> Option<serde_json::Value> {
        match tool_call.name.as_str() {
            "echo" => {
                let message = tool_call
                    .arguments
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(no message)");
                Some(serde_json::json!({ "echo": message }))
            }
            "get_time" => {
                let timezone = tool_call
                    .arguments
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .unwrap_or("UTC");
                Some(serde_json::json!({
                    "time": "2025-01-15T12:00:00Z",
                    "timezone": timezone
                }))
            }
            "calculator" => {
                let op = tool_call
                    .arguments
                    .get("operation")
                    .and_then(|v| v.as_str());
                let a = tool_call.arguments.get("a").and_then(|v| v.as_f64());
                let b = tool_call.arguments.get("b").and_then(|v| v.as_f64());

                match (op, a, b) {
                    (Some("add"), Some(a), Some(b)) => Some(serde_json::json!({ "result": a + b })),
                    (Some("subtract"), Some(a), Some(b)) => {
                        Some(serde_json::json!({ "result": a - b }))
                    }
                    (Some("multiply"), Some(a), Some(b)) => {
                        Some(serde_json::json!({ "result": a * b }))
                    }
                    (Some("divide"), Some(a), Some(b)) if b != 0.0 => {
                        Some(serde_json::json!({ "result": a / b }))
                    }
                    (Some("divide"), Some(_), Some(_)) => {
                        Some(serde_json::json!({ "error": "Division by zero" }))
                    }
                    _ => Some(serde_json::json!({ "error": "Invalid arguments" })),
                }
            }
            _ => None,
        }
    }
}

/// Tool executor that uses builtin tools
pub struct BuiltinToolExecutorAdapter;

#[async_trait]
impl ToolExecutor for BuiltinToolExecutorAdapter {
    async fn execute_tool(
        &self,
        input: ExecuteSingleToolInput,
    ) -> Result<ExecuteSingleToolOutput, ActivityError> {
        match BuiltinToolExecutor::execute(&input.tool_call) {
            Some(result) => Ok(ExecuteSingleToolOutput {
                result: ToolResult::success(&input.tool_call.id, result),
            }),
            None => Ok(ExecuteSingleToolOutput {
                result: ToolResult::error(
                    &input.tool_call.id,
                    &format!("Unknown tool: {}", input.tool_call.name),
                ),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_agent_loader() {
        let loader = MockAgentLoader::new();
        let config = AgentConfig::test("test-agent");
        let agent_id = config.agent_id;
        loader.register(config);

        let result = loader
            .load_agent(LoadAgentInput { agent_id })
            .await
            .unwrap();
        assert_eq!(result.name, "test-agent");
    }

    #[tokio::test]
    async fn test_mock_agent_loader_not_found() {
        let loader = MockAgentLoader::new();
        let result = loader
            .load_agent(LoadAgentInput {
                agent_id: Uuid::now_v7(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_llm_caller_scripted() {
        let caller = MockLlmCaller::new();
        caller.add_response(LlmResponse::text("First response"));
        caller.add_response(LlmResponse::text("Second response"));

        let input = CallLlmInput {
            session_id: Uuid::now_v7(),
            agent_config: AgentConfig::test("test"),
            messages: vec![],
        };

        let result1 = caller.call_llm(input.clone()).await.unwrap();
        assert_eq!(result1.text, "First response");

        let result2 = caller.call_llm(input.clone()).await.unwrap();
        assert_eq!(result2.text, "Second response");

        // Default response after scripted ones are exhausted
        let result3 = caller.call_llm(input).await.unwrap();
        assert_eq!(result3.text, "I don't know how to respond.");

        assert_eq!(caller.call_count(), 3);
    }

    #[tokio::test]
    async fn test_mock_tool_executor() {
        let executor = MockToolExecutor::new();
        executor.register_result("my_tool", serde_json::json!({"status": "ok"}));

        let input = ExecuteSingleToolInput {
            session_id: Uuid::now_v7(),
            tool_call: ToolCall::new("my_tool", serde_json::json!({})),
            tool_definition: None,
        };

        let result = executor.execute_tool(input).await.unwrap();
        assert!(result.result.result.is_some());
        assert_eq!(result.result.result.unwrap()["status"], "ok");
    }

    #[tokio::test]
    async fn test_mock_tool_executor_error() {
        let executor = MockToolExecutor::new();
        executor.register_error("failing_tool", "Tool failed");

        let input = ExecuteSingleToolInput {
            session_id: Uuid::now_v7(),
            tool_call: ToolCall::new("failing_tool", serde_json::json!({})),
            tool_definition: None,
        };

        let result = executor.execute_tool(input).await.unwrap();
        assert!(result.result.error.is_some());
        assert_eq!(result.result.error.unwrap(), "Tool failed");
    }

    #[tokio::test]
    async fn test_builtin_echo_tool() {
        let tool_call = ToolCall::new("echo", serde_json::json!({"message": "hello"}));
        let result = BuiltinToolExecutor::execute(&tool_call).unwrap();
        assert_eq!(result["echo"], "hello");
    }

    #[tokio::test]
    async fn test_builtin_calculator_tool() {
        let tool_call = ToolCall::new(
            "calculator",
            serde_json::json!({
                "operation": "add",
                "a": 2,
                "b": 3
            }),
        );
        let result = BuiltinToolExecutor::execute(&tool_call).unwrap();
        assert_eq!(result["result"], 5.0);
    }

    #[tokio::test]
    async fn test_builtin_calculator_divide_by_zero() {
        let tool_call = ToolCall::new(
            "calculator",
            serde_json::json!({
                "operation": "divide",
                "a": 10,
                "b": 0
            }),
        );
        let result = BuiltinToolExecutor::execute(&tool_call).unwrap();
        assert!(result.get("error").is_some());
    }
}
