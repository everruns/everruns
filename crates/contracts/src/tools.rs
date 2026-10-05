//! Tool outcomes shared by runtimes and provider services.

use crate::tool_types::{
    ConnectionRequired, ConnectionRequiredSubject, ToolResult, ToolResultImage,
};
use serde_json::Value;
use tracing::error;

// ============================================================================
// Tool Execution Result - Error Handling Contract
// ============================================================================

/// Result of a tool execution.
///
/// This enum distinguishes between different outcomes:
/// - `Success`: Tool executed successfully, result is returned to LLM
/// - `SuccessWithImages`: Successful execution with JSON result plus images
/// - `ToolError`: Tool-level error that should be shown to the LLM
///   (e.g., "City not found", "Invalid date format")
/// - `InternalError`: System-level error that should NOT be exposed to the LLM
///   (e.g., database connection failure, API key issues)
///
/// # Security
///
/// Internal errors are logged but replaced with a generic message when
/// returned to the LLM. This prevents leaking sensitive information like
/// database errors, API keys, or internal system details.

#[derive(Debug)]
pub enum ToolExecutionResult {
    /// Successful execution with a JSON result
    Success(Value),

    /// Successful execution with a JSON result and images.
    /// Images are sent to the LLM as native image content blocks
    /// (not stringified JSON), enabling visual understanding.
    SuccessWithImages {
        result: Value,
        images: Vec<ToolResultImage>,
    },

    /// Tool-level error that is safe to show to the LLM
    ///
    /// Use this for expected error conditions that the LLM should know about,
    /// such as validation errors, resource not found, etc.
    ToolError(String),

    /// Internal/system error that should NOT be exposed to the LLM
    ///
    /// Use this for unexpected errors like network failures, database errors,
    /// or other internal issues. The error details will be logged but replaced
    /// with a generic message when returned to the LLM.
    InternalError(ToolInternalError),

    /// A connection is required to execute this tool.
    ///
    /// Instead of returning an error, this signals that the workflow should
    /// pause and ask the client to set up a connection for the given provider.
    /// The UI renders an inline setup prompt; once setup is confirmed (or
    /// skipped), a tool result is submitted and execution resumes.
    ConnectionRequired {
        /// Connection provider id (e.g. "daytona", "brave_search")
        provider: String,
        /// Whose grant is missing. Absent for provider-only integrations.
        subject: Option<ConnectionRequiredSubject>,
        /// Relative UI route where the missing grant can be configured.
        setup_url: Option<String>,
    },

    /// A policy gate settled the call without running it; record this outcome
    /// as is. It is a block, or a deferral whose structured result (a hosted
    /// approval request) the engine turns into a pause.
    ///
    /// For tools that dispatch another tool on the model's behalf
    /// (`spawn_background`): the nested call's gate outcome reaches the act
    /// phase exactly as it would for a direct call (EVE-1186).
    PolicyOutcome(Box<ToolResult>),
}

impl ToolExecutionResult {
    /// Create a successful result
    pub fn success(value: impl Into<Value>) -> Self {
        ToolExecutionResult::Success(value.into())
    }

    /// Create a successful result with pre-truncation raw output for VFS persistence.
    /// The raw output is transferred to `ToolResult.raw_output` during `into_tool_result()`.
    pub fn success_with_raw_output(value: impl Into<Value>, raw_output: String) -> Self {
        let mut value = value.into();
        // Embed raw output in a sidecar key — extracted in into_tool_result().
        // Non-object values are wrapped in a scalar carrier so raw_output still
        // flows through; the carrier is unwrapped on extraction.
        match value.as_object_mut() {
            Some(obj)
                if !obj.contains_key("_raw_output") && !obj.contains_key("_raw_output_scalar") =>
            {
                obj.insert("_raw_output".to_string(), Value::String(raw_output));
            }
            _ => {
                // Wrap colliding object keys too: caller data must not be
                // overwritten or mistaken for our scalar carrier on extraction.
                value = serde_json::json!({
                    "_raw_output_scalar": value,
                    "_raw_output": raw_output,
                });
            }
        }
        ToolExecutionResult::Success(value)
    }

    /// Create a successful result with images
    pub fn success_with_images(value: impl Into<Value>, images: Vec<ToolResultImage>) -> Self {
        ToolExecutionResult::SuccessWithImages {
            result: value.into(),
            images,
        }
    }

    /// Create a tool-level error (safe to show to LLM)
    pub fn tool_error(message: impl Into<String>) -> Self {
        ToolExecutionResult::ToolError(message.into())
    }

    /// Create an internal error (will be hidden from LLM)
    pub fn internal_error(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        ToolExecutionResult::InternalError(ToolInternalError::new(error))
    }

    /// Create an internal error from a string message
    pub fn internal_error_msg(message: impl Into<String>) -> Self {
        ToolExecutionResult::InternalError(ToolInternalError::from_message(message))
    }

    /// Signal a provider-only connection requirement.
    pub fn connection_required(provider: impl Into<String>) -> Self {
        ToolExecutionResult::ConnectionRequired {
            provider: provider.into(),
            subject: None,
            setup_url: None,
        }
    }

    /// Signal that a subject-specific connection is required before execution.
    pub fn connection_required_with_setup(
        provider: impl Into<String>,
        subject: ConnectionRequiredSubject,
        setup_url: impl Into<String>,
    ) -> Self {
        ToolExecutionResult::ConnectionRequired {
            provider: provider.into(),
            subject: Some(subject),
            setup_url: Some(setup_url.into()),
        }
    }

    /// Check if this is a successful result
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            ToolExecutionResult::Success(_) | ToolExecutionResult::SuccessWithImages { .. }
        )
    }

    /// Check if this is an error (either tool error or internal error)
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            ToolExecutionResult::ToolError(_) | ToolExecutionResult::InternalError(_)
        )
    }

    /// Check whether this requires connection setup.
    pub fn is_connection_required(&self) -> bool {
        matches!(self, ToolExecutionResult::ConnectionRequired { .. })
    }

    /// Convert to a ToolResult for the agent loop
    ///
    /// Both tool errors and internal errors are packaged as `{"error": "..."}` in the
    /// result field. This provides a consistent contract where the result field always
    /// contains the payload, and the agent loop continues the same way for all outcomes.
    ///
    /// Internal errors are logged but replaced with a generic message when returned.
    pub fn into_tool_result(self, tool_call_id: &str, tool_name: &str) -> ToolResult {
        match self {
            ToolExecutionResult::Success(mut value) => {
                // Extract sidecar raw output if present (from success_with_raw_output)
                let raw_output = value
                    .as_object_mut()
                    .and_then(|obj| obj.remove("_raw_output"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()));
                // Unwrap scalar carrier only when it matches the exact wrapper shape
                // set by success_with_raw_output for non-object inputs.
                let result_value = if let Some(obj) = value.as_object_mut() {
                    let is_scalar_carrier = raw_output.is_some()
                        && obj.len() == 1
                        && obj.contains_key("_raw_output_scalar");
                    if is_scalar_carrier {
                        obj.remove("_raw_output_scalar").unwrap_or(Value::Null)
                    } else {
                        value
                    }
                } else {
                    value
                };
                ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    result: Some(result_value),
                    images: None,
                    error: None,
                    connection_required: None,
                    raw_output,
                }
            }
            ToolExecutionResult::SuccessWithImages { result, images } => ToolResult {
                tool_call_id: tool_call_id.to_string(),
                result: Some(result),
                images: if images.is_empty() {
                    None
                } else {
                    Some(images)
                },
                error: None,
                connection_required: None,
                raw_output: None,
            },
            ToolExecutionResult::ToolError(message) => ToolResult {
                tool_call_id: tool_call_id.to_string(),
                result: Some(serde_json::json!({ "error": &message })),
                images: None,
                error: Some(message),
                connection_required: None,
                raw_output: None,
            },
            ToolExecutionResult::InternalError(err) => {
                // Log the full error details for debugging
                error!(
                    tool_name = %tool_name,
                    tool_call_id = %tool_call_id,
                    error = %err.message,
                    error_chain = %err.chain_string(),
                    "Tool internal error (details hidden from LLM)"
                );

                // Return generic error message to LLM, packaged as {"error": "..."}
                let generic_msg = "An internal error occurred while executing the tool";
                ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    result: Some(serde_json::json!({
                        "error": generic_msg
                    })),
                    images: None,
                    error: Some(generic_msg.to_string()),
                    connection_required: None,
                    raw_output: None,
                }
            }
            ToolExecutionResult::ConnectionRequired {
                provider,
                subject,
                setup_url,
            } => {
                let required = ConnectionRequired {
                    provider,
                    subject,
                    setup_url,
                };
                ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    result: Some(serde_json::json!({
                        "connection_required": &required,
                    })),
                    images: None,
                    error: None,
                    connection_required: Some(required),
                    raw_output: None,
                }
            }
            ToolExecutionResult::PolicyOutcome(result) => ToolResult {
                tool_call_id: tool_call_id.to_string(),
                ..*result
            },
        }
    }
}

/// Internal error details (logged but not exposed to LLM)
#[derive(Debug)]
pub struct ToolInternalError {
    /// Error message for logging
    pub message: String,
    /// Optional source error
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl ToolInternalError {
    /// Create from an error
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            message: error.to_string(),
            source: Some(Box::new(error)),
        }
    }

    /// Create from a string message
    pub fn from_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn chain_string(&self) -> String {
        let mut parts = vec![self.message.clone()];
        let mut current = <Self as std::error::Error>::source(self);
        while let Some(source) = current {
            let message = source.to_string();
            if parts.last() != Some(&message) {
                parts.push(message);
            }
            current = source.source();
        }
        parts.join(": ")
    }
}

impl std::fmt::Display for ToolInternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ToolInternalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}
