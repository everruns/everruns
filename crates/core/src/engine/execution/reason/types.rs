//! What a reason step takes and returns.

use super::*;

/// Input for ReasonAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonInput {
    /// Atom execution context
    pub context: ExecutionContext,
    /// Harness ID for loading base configuration
    pub harness_id: HarnessId,
    /// Agent ID for loading configuration (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Organization ID for multi-tenancy tracking
    #[serde(default)]
    pub org_id: i64,
    /// MCP tool definitions from agent's MCP capabilities (pre-resolved)
    /// These are passed from the control-plane since MCP capabilities
    /// are not in the CapabilityRegistry.
    #[serde(default)]
    pub mcp_tool_definitions: Vec<ToolDefinition>,
    /// Previous LLM response ID for stateful continuation.
    /// Enables server-side context caching across reason iterations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Current iteration number within this turn (1-based).
    /// Used for output.message.started events so UI can show progress.
    #[serde(default = "default_iteration")]
    pub iteration: u32,
}

fn default_iteration() -> u32 {
    1
}

/// Internal continuations accounted as part of one scheduled Reason activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeExecutionCounts {
    pub llm_calls: u32,
    pub tool_calls: u32,
}

/// Result of the ReasonAtom
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReasonResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_counts: Option<NativeExecutionCounts>,
    /// Whether the LLM call succeeded
    pub success: bool,
    /// Text response from the model
    pub text: String,
    /// The text is working notes, never something said: the agent talks
    /// explicitly, through `send_message`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub commentary: bool,
    /// Tool calls requested by the model
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Whether tool execution is needed
    pub has_tool_calls: bool,
    /// Tool definitions from applied capabilities (for tool execution)
    #[serde(default)]
    pub tool_definitions: Vec<ToolDefinition>,
    /// Maximum iterations configured for the agent
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Error message if the call failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Disclosed user-facing decision of the failure, already filtered
    /// through the resolved error-disclosure mode. Hosts must prefer this over
    /// re-classifying `error`/`text` strings so disclosure stays consistent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_facing_error: Option<UserFacingError>,
    /// Error-disclosure mode that was applied to `user_facing_error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_disclosure: Option<ErrorDisclosure>,
    /// Token usage from the LLM call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /// Assistant message emitted by `output.message.completed` for this generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_message_id: Option<MessageId>,
    /// Streaming latency for this LLM call, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
    /// LLM provider's response ID for chaining with `previous_response_id`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Raw provider finish reason for this generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// Resolved locale used for this turn's prompt and backend-authored strings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Merged network access list for URL filtering in tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<crate::engine::network_access::NetworkAccessList>,
    /// Request-level parallel tool calling preference (EVE-598), carried from
    /// the resolved agent config into `ActInput` so the act scheduler can honor
    /// `Some(false)` (force serialize). `None` preserves the default schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// A remote tool loop (OpenAI Agents API) paused on a tool call; the turn parks (EVE-1124).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waiting_for_tool_results: bool,
    /// The generation lost tool calls to truncation and the output-truncation
    /// gate retries: the turn runs another reason step even without calls.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncation_retry: bool,
}

impl ReasonResult {
    /// The text this step said to the conversation: empty when it is an
    /// explicit agent's working notes.
    pub fn said_text(&self) -> &str {
        if self.commentary { "" } else { &self.text }
    }
}
