// V2 Activity Implementations
//
// Activities are the units of work scheduled by the workflow.
// Each activity runs outside the workflow and returns a result.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::session_workflow::{MessageData, ToolCallData, ToolDefinitionData, ToolResultData};

// ============================================================================
// Activity Input/Output Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallModelInput {
    pub session_id: String,
    pub model: String,
    pub messages: Vec<MessageData>,
    pub tools: Vec<ToolDefinitionData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallModelOutput {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallData>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolsInput {
    pub session_id: String,
    pub tool_calls: Vec<ToolCallData>,
    pub tools: Vec<ToolDefinitionData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolsOutput {
    pub results: Vec<ToolResultData>,
}

// ============================================================================
// Activity Implementations
// ============================================================================

/// IMPLEMENT MODEL CALL HERE
///
/// This is where the actual LLM call happens.
/// The workflow schedules this activity with type "call-model".
///
/// Input: CallModelInput { session_id, model, messages, tools }
/// Output: CallModelOutput { text, tool_calls }
///
/// Example implementation:
/// ```ignore
/// pub async fn call_model_activity(
///     input: CallModelInput,
///     llm_provider: &impl LlmProvider,
/// ) -> Result<CallModelOutput> {
///     // Convert messages to LLM format
///     let llm_messages = input.messages.iter().map(|m| /* ... */).collect();
///
///     // Build config with tools
///     let config = LlmCallConfig {
///         model: input.model,
///         tools: input.tools.into_iter().map(/* ... */).collect(),
///         ..Default::default()
///     };
///
///     // Call the LLM
///     let response = llm_provider.chat_completion(llm_messages, &config).await?;
///
///     Ok(CallModelOutput {
///         text: response.text,
///         tool_calls: response.tool_calls.map(|calls| /* convert to ToolCallData */),
///     })
/// }
/// ```
pub fn call_model_activity_stub(input: CallModelInput) -> CallModelOutput {
    // TODO: Replace with real implementation using LlmProvider
    //
    // For now, return a fake response for testing
    let _ = input; // silence unused warning

    CallModelOutput {
        text: "This is a fake response. Implement call_model_activity with LlmProvider.".into(),
        tool_calls: None,
    }
}

/// IMPLEMENT TOOL EXECUTION HERE
///
/// This is where tools are executed (in parallel).
/// The workflow schedules this activity with type "execute-tools".
///
/// Input: ExecuteToolsInput { session_id, tool_calls, tools }
/// Output: ExecuteToolsOutput { results }
///
/// Example implementation:
/// ```ignore
/// pub async fn execute_tools_activity(
///     input: ExecuteToolsInput,
///     tool_executor: &impl ToolExecutor,
/// ) -> Result<ExecuteToolsOutput> {
///     // Execute all tools in parallel
///     let results = tool_executor
///         .execute_parallel(&input.tool_calls, &input.tools)
///         .await?;
///
///     Ok(ExecuteToolsOutput {
///         results: results.into_iter().map(|r| ToolResultData {
///             tool_call_id: r.tool_call_id,
///             result: r.result,
///             error: r.error,
///         }).collect(),
///     })
/// }
/// ```
pub fn execute_tools_activity_stub(input: ExecuteToolsInput) -> ExecuteToolsOutput {
    // TODO: Replace with real implementation using ToolExecutor
    //
    // For now, return fake results for testing
    let results = input
        .tool_calls
        .iter()
        .map(|tc| ToolResultData {
            tool_call_id: tc.id.clone(),
            result: Some(json!({"status": "ok", "note": "fake result"})),
            error: None,
        })
        .collect();

    ExecuteToolsOutput { results }
}

// ============================================================================
// Activity Dispatcher (for worker integration)
// ============================================================================

/// Activity type constants matching workflow's activity_names
pub mod activity_types {
    pub const CALL_MODEL: &str = "call-model";
    pub const EXECUTE_TOOLS: &str = "execute-tools";
    pub const LOAD_AGENT: &str = "load-agent";
    pub const LOAD_MESSAGES: &str = "load-messages";
    pub const SAVE_MESSAGE: &str = "save-message";
    pub const EMIT_EVENT: &str = "emit-event";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_model_stub() {
        let input = CallModelInput {
            session_id: "test".into(),
            model: "gpt-4".into(),
            messages: vec![MessageData {
                role: "user".into(),
                content: "Hello".into(),
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
        };

        let output = call_model_activity_stub(input);
        assert!(!output.text.is_empty());
    }

    #[test]
    fn test_execute_tools_stub() {
        let input = ExecuteToolsInput {
            session_id: "test".into(),
            tool_calls: vec![ToolCallData {
                id: "call_1".into(),
                name: "get_time".into(),
                arguments: json!({}),
            }],
            tools: vec![],
        };

        let output = execute_tools_activity_stub(input);
        assert_eq!(output.results.len(), 1);
        assert_eq!(output.results[0].tool_call_id, "call_1");
    }
}
