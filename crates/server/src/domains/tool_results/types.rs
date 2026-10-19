// Tool results domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A single tool result from the client
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct ClientToolResult {
    /// Tool call ID (correlates with the tool call from tool.call_requested event)
    #[schema(example = "toolu_01933b5a00007000800000000000001")]
    pub tool_call_id: String,
    /// Result value (any JSON — object, array, string, number, etc.). Null if the tool failed.
    /// Example: `{"url": "https://example.com/orders/42"}`.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    /// Error message if the tool failed
    #[serde(default)]
    #[schema(example = "Refund failed: order is outside refund window")]
    pub error: Option<String>,
}

impl ClientToolResult {
    /// The authority resolves the tool name from its canonical pending calls.
    pub(crate) fn completed(&self) -> everruns_core::events::ToolCompletedData {
        use everruns_core::{events::ToolCompletedData, message::ContentPart};
        if let Some(error) = &self.error {
            ToolCompletedData::failure(
                self.tool_call_id.clone(),
                String::new(),
                "error".into(),
                error.clone(),
                None,
            )
        } else {
            let content = self
                .result
                .as_ref()
                .map(|result| vec![ContentPart::tool_result_text(result)])
                .unwrap_or_default();
            ToolCompletedData::success(self.tool_call_id.clone(), String::new(), content, None)
        }
    }
}

/// Request to submit client-side tool results
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SubmitToolResultsRequest {
    /// Tool results from the client
    #[schema(example = json!([{"tool_call_id": "toolu_01933b5a00007000800000000000001", "result": {"url": "https://example.com/orders/42"}}]))]
    pub tool_results: Vec<ClientToolResult>,
}

/// Response from submitting tool results
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SubmitToolResultsResponse {
    /// Number of tool results accepted
    pub accepted: usize,
    /// Session status after submission
    pub status: String,
}
