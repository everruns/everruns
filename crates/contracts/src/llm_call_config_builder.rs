//! Fluent builder for [`LlmCallConfig`], split out of `driver_registry.rs`
//! (size ratchet); re-exported from there so every existing path works.

use crate::driver_registry::{
    CacheDiagnosticsConfig, LlmCallConfig, PromptCacheConfig, ProviderOpaqueContext,
    ToolSearchConfig,
};
use crate::tool_types::ToolDefinition;
use std::collections::HashMap;

/// Builder for LlmCallConfig with fluent API
///
/// Chain methods like `reasoning_effort()`, `temperature()`, etc. and call
/// `build()` to get the final config. To start from a core `RuntimeAgent`, use
/// the [runtime agent conversion helper](https://github.com/everruns/everruns/blob/main/crates/core/src/llm_conversions.rs).
pub struct LlmCallConfigBuilder {
    config: LlmCallConfig,
}

impl LlmCallConfigBuilder {
    /// Construct a builder wrapping an existing config.
    pub fn from_config(config: LlmCallConfig) -> Self {
        Self { config }
    }

    /// Set reasoning effort for models that support it.
    pub fn reasoning_effort(mut self, effort: crate::model::ReasoningEffort) -> Self {
        self.config.reasoning_effort = Some(effort);
        self
    }

    /// Set speed (service tier): "flex", "default", "priority", "fast" or "ultrafast"
    pub fn speed(mut self, speed: impl Into<String>) -> Self {
        self.config.speed = Some(speed.into());
        self
    }

    /// Set verbosity: "low", "medium", or "high"
    pub fn verbosity(mut self, verbosity: impl Into<String>) -> Self {
        self.config.verbosity = Some(verbosity.into());
        self
    }

    /// Set the model
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
        self
    }

    /// Set temperature
    pub fn temperature(mut self, temp: f32) -> Self {
        self.config.temperature = Some(temp);
        self
    }

    /// Set max tokens
    pub fn max_tokens(mut self, tokens: u32) -> Self {
        self.config.max_tokens = Some(tokens);
        self
    }

    /// Set tools
    pub fn tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.config.tools = tools;
        self
    }

    /// Set metadata for API tracking
    ///
    /// This metadata is sent to the LLM provider for tracking and debugging.
    /// Typically includes session_id, agent_id, org_id, turn_id, exec_id.
    pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.config.metadata = metadata;
        self
    }

    /// Add a single metadata key-value pair
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.metadata.insert(key.into(), value.into());
        self
    }

    /// Set previous response ID for stateful continuation
    pub fn previous_response_id(mut self, id: Option<String>) -> Self {
        self.config.previous_response_id = id;
        self
    }

    /// Set standalone provider-owned compact context for the request.
    pub fn provider_opaque_context(mut self, context: Option<ProviderOpaqueContext>) -> Self {
        self.config.provider_opaque_context = context;
        self
    }

    /// Set tool_search configuration
    pub fn tool_search(mut self, config: ToolSearchConfig) -> Self {
        self.config.tool_search = Some(config);
        self
    }

    /// Set prompt caching configuration
    pub fn prompt_cache(mut self, config: PromptCacheConfig) -> Self {
        self.config.prompt_cache = Some(config);
        self
    }

    /// Set a driver-namespaced opaque per-call option (`"<driver-id>/<option>"`).
    /// The value's shape is owned by the driver crate named in the key; this
    /// crate passes it through untouched.
    pub fn driver_option(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.config.driver_options.insert(key.into(), value);
        self
    }

    /// Set the request-level parallel tool calling preference (EVE-598).
    pub fn parallel_tool_calls(mut self, parallel_tool_calls: Option<bool>) -> Self {
        self.config.parallel_tool_calls = parallel_tool_calls;
        self
    }

    /// Set the number of trailing volatile messages that must not anchor a
    /// message-level prompt-cache breakpoint (see
    /// [`LlmCallConfig::volatile_suffix_len`]).
    pub fn volatile_suffix_len(mut self, len: usize) -> Self {
        self.config.volatile_suffix_len = len;
        self
    }

    /// Replace the extra HTTP headers sent with this call.
    pub fn extra_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.config.extra_headers = headers;
        self
    }

    /// Add one extra HTTP header to send with this call.
    pub fn extra_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.extra_headers.push((name.into(), value.into()));
        self
    }

    /// Record the exact request body this call sends.
    pub fn capture_request(mut self, capture: bool) -> Self {
        self.config.capture_request = capture;
        self
    }

    /// Request provider prompt-cache diagnostics for this call.
    pub fn cache_diagnostics(mut self, config: CacheDiagnosticsConfig) -> Self {
        self.config.cache_diagnostics = Some(config);
        self
    }

    /// Bound how long this call may run and how much it may return.
    pub fn limits(mut self, limits: crate::turn_collector::TurnLimits) -> Self {
        self.config.limits = limits;
        self
    }

    /// Constrain the reply to a JSON Schema (see [`crate::structured_output`]).
    pub fn response_format(mut self, format: crate::structured_output::ResponseFormat) -> Self {
        self.config.response_format = Some(format);
        self
    }

    /// Build the configuration
    pub fn build(self) -> LlmCallConfig {
        self.config
    }
}

// Moved out of `driver_registry.rs` (size ratchet).
impl LlmCallConfig {
    /// Resolve the effective wire value for `parallel_tool_calls`, gated by
    /// whether the driver/model can express it on the request.
    ///
    /// Returns `None` (omit the field, keep the provider default) when the
    /// preference is unset or `supported` is `false`. Drivers call this with
    /// `self.supports_parallel_tool_calls(&config.model)` so the preference is
    /// only serialized where the provider has a control for it. The local tool
    /// scheduler honors the preference independently, so `Some(false)` still
    /// serializes execution even when this returns `None`.
    pub fn resolved_parallel_tool_calls(&self, supported: bool) -> Option<bool> {
        if supported {
            self.parallel_tool_calls
        } else {
            None
        }
    }
}
