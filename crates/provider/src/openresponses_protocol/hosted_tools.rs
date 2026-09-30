//! OpenAI hosted tools on the Open Responses driver (EVE-1115).
//!
//! Request side: render [`OpenAiHostedTools`] into the `tools` array, or fail
//! when this endpoint cannot run them. Response side: hosted calls arrive as
//! `web_search_call` output items and `response.web_search_call.*` events.
//! They are provider-executed, so they never become agent tool calls; the
//! stream skips them and the answer text carries the citations.

use serde_json::Value;

use crate::driver_registry::LlmCallConfig;
use crate::error::{AgentLoopError, Result};
use crate::openai_hosted_tools::{OpenAiHostedTools, count_hosted_tool_calls};

use super::OpenResponsesProtocolChatDriver;

impl OpenResponsesProtocolChatDriver {
    /// Wire entries for the hosted tools this call asked for.
    ///
    /// An endpoint without hosted tools (an OpenAI-compatible gateway) errors
    /// rather than sending a request that silently lacks them.
    pub(crate) fn hosted_tools_for(&self, config: &LlmCallConfig) -> Result<Vec<Value>> {
        let requested =
            OpenAiHostedTools::from_driver_options(&config.driver_options).map_err(|error| {
                AgentLoopError::Configuration(format!("invalid OpenAI hosted tools: {error}"))
            })?;
        let Some(requested) = requested else {
            return Ok(Vec::new());
        };
        if !self.hosted_tools {
            return Err(AgentLoopError::Configuration(
                "OpenAI hosted tools need the OpenAI or Azure OpenAI Responses API; \
                 this provider does not run them"
                    .to_string(),
            ));
        }
        Ok(requested.wire_tools())
    }
}

/// Record hosted tool calls from a terminal `response` object. Hosted tools
/// bill per call on top of tokens; the counts go to tracing until usage
/// accounting carries them.
pub(crate) fn record_hosted_tool_calls(response: &Value) {
    let Some(output) = response.get("output").and_then(Value::as_array) else {
        return;
    };
    let response_id = response.get("id").and_then(Value::as_str).unwrap_or("");
    for (kind, count) in count_hosted_tool_calls(output) {
        tracing::info!(
            hosted_tool = %kind,
            calls = count,
            response_id,
            "OpenResponsesDriver: hosted tool calls"
        );
    }
}
