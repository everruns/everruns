use super::*;

/// Configuration for tool_search (deferred tool loading).
///
/// When enabled, the driver groups tools into namespaces and marks them with
/// `defer_loading: true` so the model only loads full schemas on-demand.
/// This reduces token usage for agents with many tools.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ToolSearchConfig {
    /// Enable tool_search for this request (requires model support)
    pub enabled: bool,
    /// Minimum number of tools before activating tool_search.
    /// Below this threshold, full schemas are sent even when enabled.
    pub threshold: usize,
}

/// Strategy for prompt caching.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheStrategy {
    /// Let each driver choose the safest provider-specific behavior.
    #[default]
    Auto,
    /// Cache only the developer-instruction prefix on supporting models.
    /// The changing conversation suffix is not written to cache.
    Explicit,
}

/// Configuration for prompt caching.
///
/// Drivers translate this into provider-specific request options when possible.
/// Unsupported providers or models should ignore it without failing the call.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PromptCacheConfig {
    /// Enable prompt caching for this request.
    pub enabled: bool,
    /// Strategy the driver should use when enabling prompt caching.
    #[serde(default)]
    pub strategy: PromptCacheStrategy,
    /// Existing Gemini cached content resource name (`cachedContents/{id}`).
    ///
    /// When set, the Gemini driver uses explicit caching via the
    /// `cachedContent` request field. When absent, Gemini falls back to its
    /// default provider behavior (for example implicit caching on supported
    /// models).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_cached_content: Option<String>,
}

/// Per-request prompt-cache diagnostics controls.
///
/// Anthropic's `cache-diagnosis` beta fingerprints each request and, on the
/// next one, reports where the prompt prefix diverged (model, system prompt,
/// tools, or message history) instead of leaving a silent cache miss. Drivers
/// that have no diagnostics protocol ignore this without failing the call.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CacheDiagnosticsConfig {
    /// Opt this request into diagnostics.
    pub enabled: bool,
    /// Provider response id of the request to compare this one against.
    ///
    /// `None` opts in without a prior request: Anthropic requires the field to
    /// be present and explicitly `null` on the first turn, so drivers must
    /// serialize it rather than skip it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_message_id: Option<String>,
}

/// Configuration for an LLM call
///
/// `#[non_exhaustive]`: new per-call knobs arrive most releases, and a field
/// addition must not break every downstream consumer (and force a breaking
/// bump that cascades through the whole publish cone). Construct with
/// [`LlmCallConfig::new`] or [`LlmCallConfig::default`] and assign fields, or
/// use [`LlmCallConfigBuilder`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct LlmCallConfig {
    /// Durable Astra baseline and effective effort; absent for other modes.
    pub reasoning_state: Option<crate::reasoning_updates::ReasoningState>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<ToolDefinition>,
    /// Reasoning effort for models that support it.
    ///
    /// `None` means unset — the provider keeps its default. `Some(None)` is the
    /// caller explicitly asking for no reasoning, which drivers honor by
    /// omitting the reasoning request fields rather than sending a default.
    pub reasoning_effort: Option<crate::model::ReasoningEffort>,
    /// Speed (service tier) for this call: "flex", "default", "priority",
    /// "fast" or "ultrafast". Serialized as OpenAI `service_tier`; omitted
    /// when `None` so the provider keeps its default ("auto") routing.
    pub speed: Option<String>,
    /// Verbosity for this call: "low", "medium", or "high". Serialized as
    /// OpenAI `verbosity`; omitted when `None` so the provider keeps its
    /// default ("medium") output length.
    pub verbosity: Option<String>,
    /// String metadata sent with the API request for tracking (OpenAI and Anthropic).
    /// Typically includes: session_id, agent_id, org_id, turn_id, exec_id.
    pub metadata: HashMap<String, String>,
    /// Previous response ID for stateful continuation (OpenAI Responses API).
    /// When set, the provider can skip re-encoding cached context.
    pub previous_response_id: Option<String>,
    /// Standalone, ordered native compact output for this request.
    ///
    /// This is mutually exclusive with `previous_response_id`. Provider
    /// drivers must serialize it as the request input without transcript-delta
    /// trimming or structural pruning.
    pub provider_opaque_context: Option<ProviderOpaqueContext>,
    /// Tool search configuration for deferred tool loading
    pub tool_search: Option<ToolSearchConfig>,
    /// Prompt caching configuration for provider-specific cache controls.
    pub prompt_cache: Option<PromptCacheConfig>,
    /// Driver-namespaced opaque per-call options (`"<driver-id>/<option>"`, e.g.
    /// `"openrouter/routing"`). Each entry's shape is owned by the driver crate
    /// named in the key; this crate never interprets the values.
    pub driver_options: HashMap<String, serde_json::Value>,
    /// Request-level parallel tool calling preference (EVE-598).
    ///
    /// Serialized onto the provider request when `Some(_)`: OpenAI sets
    /// `parallel_tool_calls`; Anthropic maps `Some(false)` →
    /// `tool_choice.disable_parallel_tool_use = true`. `None` preserves
    /// provider defaults (no field sent).
    pub parallel_tool_calls: Option<bool>,
    /// Number of trailing messages that are volatile (regenerated every turn)
    /// and must not anchor a message-level prompt-cache breakpoint.
    ///
    /// `ReasonAtom` sets it to 1 only when a `<facts>` message trails an answer
    /// (a continuation) and so will not be replayed. Drivers that place a
    /// message cache breakpoint on the last block (Anthropic) skip this many
    /// trailing messages so the breakpoint lands on the last *stable* block —
    /// otherwise a tail that changes each turn would evict the
    /// conversation-history cache. `0` (the default) changes nothing.
    pub volatile_suffix_len: usize,
    /// Extra HTTP headers to attach to every provider request made for this
    /// call.
    ///
    /// Merged case-insensitively over the driver's protocol headers and the
    /// provider's configured/auth headers, so a caller value replaces an
    /// existing header instead of appending a second copy. Connection-level
    /// headers are dropped (see
    /// [`merge_request_headers`](crate::driver_helpers::merge_request_headers)).
    pub extra_headers: Vec<(String, String)>,
    /// Prompt-cache diagnostics requested for this call.
    pub cache_diagnostics: Option<CacheDiagnosticsConfig>,
    /// Record the exact request body the driver sends, on
    /// [`LlmCompletionMetadata::request_body`].
    ///
    /// Off by default. A driver's serialization is its own — which fields it
    /// sends, how it shapes tools and reasoning — so a consumer that has to
    /// show or store what was actually asked cannot reconstruct it and ends up
    /// fabricating an approximation. This hands over the real thing.
    ///
    /// THREAT[TM-LLM-039]: the body carries the whole prompt, so turning this
    /// on is a deliberate choice about where that prompt may be written — off
    /// by default, and never enabled by the runtime on a caller's behalf.
    /// Credentials are not part of it: authentication travels in headers,
    /// which are not serialized into the body.
    pub capture_request: bool,
    /// Bounds on how long this call may run and how much it may return.
    ///
    /// Enforced wherever a stream is folded into a turn: the non-streaming
    /// path below, and any caller using
    /// [`collect_turn`](crate::turn_collector::collect_turn). Unbounded by
    /// default, so a driver that ignores them behaves as before.
    pub limits: crate::turn_collector::TurnLimits,
    /// Schema the reply must satisfy, enforced by the provider. A driver that
    /// cannot enforce it fails the call (see [`ChatDriver::supports_response_format`]).
    pub response_format: Option<crate::structured_output::ResponseFormat>,
    /// Durable journal and turn-cancel signal for background responses.
    pub background_call: crate::background_call::BackgroundCallContext,
}

impl LlmCallConfig {
    /// Create a config for `model`, leaving every other knob at its default.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    // `resolved_parallel_tool_calls` lives in `llm_call_config_builder.rs`.
}
