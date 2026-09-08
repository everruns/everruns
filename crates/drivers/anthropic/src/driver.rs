// Anthropic Claude Chat Driver
//
// Implementation of ChatDriver for Anthropic's Claude API.
// Uses the Messages API with streaming support.
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff, respecting the retry-after header if provided.
// Retry metadata is included in the response for observability.
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// llm.generation events are emitted by ReasonAtom, and OtelEventListener
// creates the appropriate gen-ai spans. No direct tracing in drivers.

use async_trait::async_trait;
use chrono::DateTime;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use everruns_provider::credential_schema::CredentialFormSchema;
use everruns_provider::driver_helpers::{
    self, ANTHROPIC_NOT_FOUND_PATTERNS, ANTHROPIC_TOO_LARGE_PATTERNS, AUDIO_CONTENT_PLACEHOLDER,
    parse_data_url,
};
use everruns_provider::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmCompletionMetadata, LlmContentPart, LlmMessage, LlmMessageContent, LlmMessageRole,
    LlmResponseStream, LlmStreamEvent, fold_system_messages,
};
use everruns_provider::error::{AgentLoopError, LlmErrorKind, Result};
use everruns_provider::is_provider_quota_message;
use everruns_provider::llm_retry::{
    LlmRetryConfig, RateLimitInfo, RetryDecision, RetryMetadata, SendOutcome, is_rate_limit_status,
    retry_request, send_error_message,
};
use everruns_provider::model::ReasoningEffort;
use everruns_provider::reasoning::{ReasoningContentPart, ReasoningText};
use everruns_provider::stream_reconnect::connect_sse_with_reconnect;
use everruns_provider::tool_types::{DeferrablePolicy, ToolCall, ToolDefinition};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Beta flag that turns on Anthropic's prompt-cache diagnostics: the API
/// fingerprints each request and, on the next one, reports where the prompt
/// prefix diverged instead of leaving a silent cache miss.
const CACHE_DIAGNOSTICS_BETA: &str = "cache-diagnosis-2026-04-07";

/// Ready-to-use Anthropic Messages provider assembly.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    api_key: impl Into<String>,
) -> everruns_provider::Provider {
    everruns_provider::Provider::new(id, AnthropicChatDriver::new())
        .base_url(DEFAULT_BASE_URL)
        .auth(everruns_provider::StaticHeaderAuth::new(
            "x-api-key",
            api_key,
        ))
}

/// Message-level prompt-cache breakpoints per request. Anthropic allows four
/// in total; the system prompt and the tool array take one each, leaving two
/// for the transcript. See `mark_recent_text_blocks_for_cache`.
const MESSAGE_CACHE_BREAKPOINTS: usize = 2;

/// Anthropic Claude Chat Driver
///
/// Implements `ChatDriver` for Anthropic's Messages API.
/// Supports streaming responses and tool calls.
///
/// Rate limit handling: On 429 errors, automatically retries with exponential
/// backoff, respecting the `retry-after` header if provided by Anthropic.
///
/// # Example
///
/// ```ignore
/// use everruns_anthropic::AnthropicChatDriver;
///
/// let driver = AnthropicChatDriver::new();
/// // Endpoint and auth belong to the runtime Provider.
/// let driver = AnthropicChatDriver::new()
///     .with_retry_config(LlmRetryConfig::aggressive());
/// ```
#[derive(Clone)]
pub struct AnthropicChatDriver {
    /// Retry configuration for rate limit errors
    retry_config: LlmRetryConfig,
}

#[derive(Clone, Copy)]
struct SendMessagesOptions<'a> {
    needs_interleaved_thinking: bool,
    wants_million_context: bool,
    wants_cache_diagnostics: bool,
    max_tokens_from_profile: bool,
    model: &'a str,
    /// Caller-supplied per-request headers, applied over everything the driver
    /// and the provider endpoint resolve.
    extra_headers: &'a [(String, String)],
}

impl AnthropicChatDriver {
    /// Create a new provider with the given API key
    pub fn new() -> Self {
        Self {
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// The process-wide streaming HTTP client, resolved per request rather than
    /// held as a field. Building it loads the platform trust store (~1.3 ms),
    /// which would otherwise land in `Agent::build` on the startup path; after
    /// the first request this is a `OnceLock` read and an `Arc` clone.
    fn client(&self) -> Client {
        driver_helpers::shared_streaming_http_client()
    }

    /// Configure retry behavior for rate limit errors
    pub fn with_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Send one streaming Messages request, applying the shared header-phase
    /// retry loop (transient send failures, 429, 5xx, and the one-shot
    /// `max_tokens` fallback), and return the raw response plus its retry
    /// metadata.
    ///
    /// Invoked once per reconnect attempt by [`connect_sse_with_reconnect`]. The
    /// `request` is shared (`Arc<Mutex>`) so a fallback-corrected body carries
    /// across reconnects; it consumes no body bytes, so re-sending is
    /// idempotent. Terminal classification and error messages are preserved
    /// exactly.
    async fn send_messages_request(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
        request: Arc<Mutex<AnthropicRequest>>,
        options: SendMessagesOptions<'_>,
        retries_consumed: u32,
    ) -> Result<(reqwest::Response, RetryMetadata)> {
        let SendMessagesOptions {
            needs_interleaved_thinking,
            wants_million_context,
            wants_cache_diagnostics,
            max_tokens_from_profile,
            model,
            extra_headers,
        } = options;
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let max_tokens_fallback_attempted = Arc::new(Mutex::new(false));
        let beta_logged = Arc::new(Mutex::new(false));
        let mut retry_config = self.retry_config.clone();
        retry_config.max_retries = retry_config.max_retries.saturating_sub(retries_consumed);

        retry_request(
            &retry_config,
            "AnthropicDriver",
            || {
                let request = Arc::clone(&request);
                let beta_logged = Arc::clone(&beta_logged);
                async move {
                    // Build request with headers (must rebuild each iteration).
                    let url = endpoint.url("messages").ok_or_else(|| {
                        SendOutcome::Fatal(AgentLoopError::config("Anthropic provider has no base URL"))
                    })?;
                    let mut driver_headers: Vec<(String, String)> = vec![
                        ("anthropic-version".to_string(), ANTHROPIC_VERSION.to_string()),
                        ("Content-Type".to_string(), "application/json".to_string()),
                    ];

                    // Anthropic beta features, combined into a single
                    // comma-separated `anthropic-beta` header:
                    //  - interleaved-thinking: required for tool use with
                    //    budget-based extended thinking (adaptive thinking
                    //    interleaves on its own).
                    //  - context-1m: opts the `[1m]` model ids into the 1M
                    //    context window. It is GA / silently ignored on Opus 4.6+
                    //    and Fable 5.x (the only ids we attach it to), so always
                    //    sending it is safe.
                    //  - cache-diagnosis: required for the request-level
                    //    `diagnostics` object and the diagnostics payload on
                    //    the response.
                    let mut beta_features: Vec<&str> = Vec::new();
                    if needs_interleaved_thinking {
                        beta_features.push("interleaved-thinking-2025-05-14");
                    }
                    if wants_million_context {
                        beta_features.push("context-1m-2025-08-07");
                    }
                    if wants_cache_diagnostics {
                        beta_features.push(CACHE_DIAGNOSTICS_BETA);
                    }
                    if !beta_features.is_empty() {
                        let beta = beta_features.join(",");
                        let mut logged = beta_logged.lock().unwrap();
                        if !*logged {
                            tracing::info!(beta = %beta, "AnthropicDriver: enabling anthropic-beta features");
                            *logged = true;
                        }
                        drop(logged);
                        driver_headers.push(("anthropic-beta".to_string(), beta));
                    }

                    // Snapshot the (possibly fallback-mutated) request as JSON
                    // while holding the lock, then release it before awaiting the
                    // send so the guard never crosses an `.await` point.
                    let body = serde_json::to_vec(&*request.lock().unwrap())
                        .map_err(|e| {
                            SendOutcome::Fatal(AgentLoopError::llm(format!(
                                "Failed to serialize Anthropic request: {e}"
                            )))
                        })?;
                    let resolved = endpoint.resolve("POST", url, &body).await.map_err(SendOutcome::Fatal)?;
                    driver_headers.extend(resolved.headers);
                    let mut request_builder = self.client().post(&resolved.url);
                    for (name, value) in
                        driver_helpers::merge_request_headers(driver_headers, extra_headers)
                    {
                        request_builder = request_builder.header(name, value);
                    }
                    request_builder
                        .body(body)
                        .send()
                        .await
                        .map_err(SendOutcome::Send)
                }
            },
            |response, attempts, can_retry| {
                let request = Arc::clone(&request);
                let last_error = Arc::clone(&last_error);
                let max_tokens_fallback_attempted = Arc::clone(&max_tokens_fallback_attempted);
                let model = model.to_string();
                async move {
                    let status = response.status();

                    if can_retry {
                        // Parse rate limit info from headers before consuming body.
                        let rate_limit_info = if is_rate_limit_status(status) {
                            Some(RateLimitInfo::from_anthropic_headers(response.headers()))
                        } else {
                            None
                        };

                        let error_text = response.text().await.unwrap_or_default();

                        // Don't retry a request-too-large error (not transient).
                        if is_anthropic_request_too_large(status, &error_text) {
                            return RetryDecision::Terminal(AgentLoopError::request_too_large(
                                format!("Anthropic API error ({}): {}", status, error_text),
                            ));
                        }

                        // Exhausted billing quota is not transient — fail fast.
                        if is_provider_quota_message(&error_text) {
                            return RetryDecision::Terminal(AgentLoopError::llm_kind(
                                LlmErrorKind::QuotaExhausted,
                                format!("Anthropic API error ({}): {}", status, error_text),
                            ));
                        }

                        let wait = rate_limit_info
                            .as_ref()
                            .map(|info| info.recommended_wait(&self.retry_config, attempts))
                            .unwrap_or_else(|| self.retry_config.calculate_backoff(attempts));

                        *last_error.lock().unwrap() = Some(error_text);
                        return RetryDecision::Retry {
                            wait,
                            rate_limit_info,
                        };
                    }

                    // Non-retryable error or max retries exceeded
                    let error_text = response.text().await.unwrap_or_default();
                    let error_msg = format!("Anthropic API error ({}): {}", status, error_text);

                    // Graceful fallback: if max_tokens derived from profile
                    // exceeds the model limit (400 error), retry once with a safe
                    // fallback value instead of failing immediately. Modeled as
                    // `RetryNow` so it re-sends without counting an attempt or
                    // sleeping, matching the original `continue`.
                    {
                        let mut fallback_attempted = max_tokens_fallback_attempted.lock().unwrap();
                        if status == reqwest::StatusCode::BAD_REQUEST
                            && !*fallback_attempted
                            && max_tokens_from_profile
                            && (error_text.contains("max_tokens")
                                || error_text.contains("maximum output tokens"))
                        {
                            const FALLBACK_MAX_TOKENS: u32 = 16_384;
                            let mut req = request.lock().unwrap();
                            tracing::warn!(
                                attempted = req.max_tokens,
                                fallback = FALLBACK_MAX_TOKENS,
                                model = %model,
                                "max_tokens exceeds model limit, retrying with fallback. \
                                 Update model profile for {}.",
                                model,
                            );
                            req.max_tokens = FALLBACK_MAX_TOKENS;
                            *fallback_attempted = true;
                            return RetryDecision::RetryNow;
                        }
                    }

                    // Check if this is a model-not-found error (404 with not_found_error)
                    if is_anthropic_model_not_found(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::model_not_available(model));
                    }

                    // Check if this is a request-too-large error
                    if is_anthropic_request_too_large(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::request_too_large(error_msg));
                    }

                    // Attach the semantic error kind while the HTTP status and
                    // body are still available (see LlmErrorKind).
                    let kind = LlmErrorKind::from_provider_status(status.as_u16(), &error_text);

                    if attempts > 0 {
                        return RetryDecision::Terminal(AgentLoopError::llm_kind(
                            kind,
                            format!(
                                "{} (after {} retries, last error: {})",
                                error_msg,
                                attempts,
                                last_error.lock().unwrap().take().unwrap_or_default()
                            ),
                        ));
                    }

                    RetryDecision::Terminal(AgentLoopError::llm_kind(kind, error_msg))
                }
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await
    }

    fn convert_role(role: &LlmMessageRole) -> &'static str {
        match role {
            LlmMessageRole::System => "user", // System is handled separately in Anthropic
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "user", // Tool results are sent as user messages
        }
    }

    fn convert_content(content: &LlmMessageContent) -> Vec<AnthropicContentBlock> {
        match content {
            LlmMessageContent::Text(text) => {
                // Skip empty text to avoid Anthropic API error
                if text.is_empty() {
                    vec![]
                } else {
                    vec![AnthropicContentBlock::Text {
                        text: text.clone(),
                        cache_control: None,
                    }]
                }
            }
            LlmMessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    LlmContentPart::Text { text } => {
                        // Skip empty text to avoid Anthropic API error
                        if text.is_empty() {
                            None
                        } else {
                            Some(AnthropicContentBlock::Text {
                                text: text.clone(),
                                cache_control: None,
                            })
                        }
                    }
                    LlmContentPart::Image { url } => {
                        if let Some(parsed) = parse_data_url(url) {
                            Some(AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Base64 {
                                    media_type: parsed.media_type,
                                    data: parsed.data,
                                },
                            })
                        } else if url.starts_with("data:") {
                            // Malformed data URL — fall back to image/jpeg
                            Some(AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Base64 {
                                    media_type: "image/jpeg".to_string(),
                                    data: url.clone(),
                                },
                            })
                        } else {
                            // HTTP URL
                            Some(AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Url { url: url.clone() },
                            })
                        }
                    }
                    LlmContentPart::Audio { .. } => Some(AnthropicContentBlock::Text {
                        text: AUDIO_CONTENT_PLACEHOLDER.to_string(),
                        cache_control: None,
                    }),
                })
                .collect(),
        }
    }

    fn system_prompt_for_request(
        system_prompt: Option<String>,
        prompt_cache_enabled: bool,
    ) -> Option<AnthropicSystem> {
        system_prompt.map(|text| {
            if prompt_cache_enabled && !text.is_empty() {
                AnthropicSystem::Blocks(vec![AnthropicSystemBlock::Text {
                    text,
                    cache_control: Some(AnthropicCacheControl::ephemeral()),
                }])
            } else {
                AnthropicSystem::Text(text)
            }
        })
    }

    /// Place the message-level prompt-cache breakpoints on the last text
    /// blocks, skipping `volatile_suffix_len` trailing messages.
    ///
    /// The runtime appends volatile content (a live `<facts>` block that changes
    /// every turn) after the last stable message. Anchoring a breakpoint on
    /// that volatile tail would make the cached prefix diverge from the next
    /// turn's prefix right after the last stable message, evicting the
    /// conversation-history cache. Skipping the volatile suffix keeps the
    /// breakpoints on stable blocks, so the trailing block rides as an
    /// uncached suffix while everything before it stays cached.
    ///
    /// Two breakpoints, not one, and the pair is what makes caching
    /// *incremental*: the newest marks where this turn's history gets written,
    /// the one behind it sits at a position the previous turn already wrote, so
    /// each turn reads the cache its predecessor created instead of re-paying
    /// for the whole transcript. With the system prompt and the tool array this
    /// totals four, Anthropic's per-request maximum.
    fn mark_recent_text_blocks_for_cache(
        messages: &mut [AnthropicMessage],
        volatile_suffix_len: usize,
    ) {
        let anchor_len = messages.len().saturating_sub(volatile_suffix_len);
        let mut remaining = MESSAGE_CACHE_BREAKPOINTS;
        for msg in messages[..anchor_len].iter_mut().rev() {
            // At most one breakpoint per message: a second marker inside the
            // same message would spend a scarce breakpoint on a position the
            // first one already covers.
            for block in msg.content.iter_mut().rev() {
                if let AnthropicContentBlock::Text { cache_control, .. } = block {
                    *cache_control = Some(AnthropicCacheControl::ephemeral());
                    remaining -= 1;
                    break;
                }
            }
            if remaining == 0 {
                return;
            }
        }
    }

    fn convert_messages(
        messages: &[LlmMessage],
        prompt_cache_enabled: bool,
        volatile_suffix_len: usize,
    ) -> (Option<String>, Vec<AnthropicMessage>) {
        // Accumulate all system messages into Anthropic's separate top-level
        // `system` field. Overwriting on each System message would drop the agent
        // system prompt whenever a later notice/summary System message is present
        // (infinity_context / compaction). See `fold_system_messages`.
        let system_prompt = fold_system_messages(messages);
        let mut converted = Vec::new();
        let visible_tool_use_ids = visible_tool_call_ids(messages);

        for msg in messages {
            match msg.role {
                LlmMessageRole::System => {
                    // Folded above into the top-level `system` field; never emit a
                    // System-role entry into the Anthropic `messages` array.
                }
                LlmMessageRole::Tool => {
                    // Tool results in Anthropic are user messages with tool_result content blocks.
                    // When the message contains images, we use the array form with content blocks.
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        // Anthropic rejects tool_result blocks unless the matching tool_use
                        // is present in the visible request after context trimming.
                        if !visible_tool_use_ids.contains(tool_call_id.as_str()) {
                            continue;
                        }

                        let has_images = match &msg.content {
                            LlmMessageContent::Parts(parts) => parts
                                .iter()
                                .any(|p| matches!(p, LlmContentPart::Image { .. })),
                            _ => false,
                        };

                        let content = if has_images {
                            // Build array of text + image content blocks
                            let blocks = match &msg.content {
                                LlmMessageContent::Parts(parts) => parts
                                    .iter()
                                    .filter_map(|p| match p {
                                        LlmContentPart::Text { text } => {
                                            Some(AnthropicToolResultBlock::Text {
                                                text: text.clone(),
                                            })
                                        }
                                        LlmContentPart::Image { url } => {
                                            let source = if let Some(parsed) = parse_data_url(url) {
                                                AnthropicImageSource::Base64 {
                                                    media_type: parsed.media_type,
                                                    data: parsed.data,
                                                }
                                            } else {
                                                AnthropicImageSource::Url { url: url.clone() }
                                            };
                                            Some(AnthropicToolResultBlock::Image { source })
                                        }
                                        _ => None,
                                    })
                                    .collect(),
                                LlmMessageContent::Text(text) => {
                                    vec![AnthropicToolResultBlock::Text { text: text.clone() }]
                                }
                            };
                            AnthropicToolResultContent::Blocks(blocks)
                        } else {
                            AnthropicToolResultContent::Text(msg.content.to_text())
                        };

                        converted.push(AnthropicMessage {
                            role: "user".to_string(),
                            content: vec![AnthropicContentBlock::ToolResult {
                                tool_use_id: tool_call_id.clone(),
                                content,
                                is_error: None,
                            }],
                        });
                    }
                }
                LlmMessageRole::Assistant => {
                    let mut content = Vec::new();

                    tracing::debug!(
                        reasoning_blocks = msg.reasoning.len(),
                        has_tool_calls = msg.tool_calls.is_some(),
                        tool_calls_count = msg.tool_calls.as_ref().map(|tc| tc.len()),
                        "AnthropicDriver: converting assistant message"
                    );

                    // Thinking blocks lead the assistant message, as the API
                    // requires when thinking is enabled. Every block is
                    // replayed with the signature that signs *it*: a merged
                    // block paired with some other block's signature fails
                    // verification, which is exactly what interleaved thinking
                    // produces if the blocks are flattened.
                    for item in &msg.reasoning {
                        if item.provider != "anthropic" {
                            // Signatures are provider-scoped; replaying another
                            // provider's artifact is never valid.
                            continue;
                        }
                        match (&item.text, &item.signature, &item.encrypted) {
                            (Some(ReasoningText::Redacted), _, Some(data)) => {
                                content.push(AnthropicContentBlock::RedactedThinking {
                                    data: data.clone(),
                                });
                            }
                            (Some(ReasoningText::Plain { text }), Some(signature), _) => {
                                content.push(AnthropicContentBlock::Thinking {
                                    thinking: text.clone(),
                                    signature: signature.clone(),
                                });
                            }
                            _ => {
                                tracing::warn!(
                                    has_signature = item.signature.is_some(),
                                    has_text = item.text.is_some(),
                                    "AnthropicDriver: skipping unreplayable reasoning artifact"
                                );
                            }
                        }
                    }

                    // Add text/image content
                    content.extend(Self::convert_content(&msg.content));

                    // Add tool_use blocks if present
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            content.push(AnthropicContentBlock::ToolUse {
                                id: tc.id.clone(),
                                name: tc.name.clone(),
                                input: tc.arguments.clone(),
                            });
                        }
                    }

                    converted.push(AnthropicMessage {
                        role: Self::convert_role(&msg.role).to_string(),
                        content,
                    });
                }
                _ => {
                    converted.push(AnthropicMessage {
                        role: Self::convert_role(&msg.role).to_string(),
                        content: Self::convert_content(&msg.content),
                    });
                }
            }
        }

        if prompt_cache_enabled {
            Self::mark_recent_text_blocks_for_cache(&mut converted, volatile_suffix_len);
        }

        (system_prompt, converted)
    }

    /// Anthropic rejects `oneOf`, `allOf`, and `anyOf` at the top level of a
    /// tool `input_schema` (nested composition is accepted). Tool schemas can
    /// be caller-supplied JSON Schema (e.g. a spawn_agent `result_schema`
    /// becomes the child's `report_result` schema verbatim), so drop the
    /// keywords here instead of failing the whole request with a 400. This
    /// only loosens what the model sees; execute-time validation still
    /// enforces the full schema and rejects non-conforming calls.
    fn sanitize_input_schema(mut schema: Value) -> Value {
        if let Value::Object(obj) = &mut schema {
            let mut had_composition = false;
            for key in ["oneOf", "allOf", "anyOf"] {
                had_composition |= obj.remove(key).is_some();
            }
            if had_composition && !obj.contains_key("type") {
                obj.insert("type".to_string(), Value::String("object".to_string()));
            }
        }
        schema
    }

    fn convert_tools(
        tools: &[ToolDefinition],
        prompt_cache_enabled: bool,
    ) -> Vec<AnthropicToolEntry> {
        let last_index = tools.len().saturating_sub(1);
        tools
            .iter()
            .enumerate()
            .map(|(index, tool)| {
                AnthropicToolEntry::Function(AnthropicTool {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    input_schema: Self::sanitize_input_schema(tool.parameters().clone()),
                    cache_control: (prompt_cache_enabled && index == last_index)
                        .then(AnthropicCacheControl::ephemeral),
                    defer_loading: None,
                })
            })
            .collect()
    }

    /// Convert tools with Anthropic's hosted tool_search: mark deferrable tools
    /// `defer_loading: true` and prepend a `tool_search_tool_bm25_20251119` server
    /// tool entry. The model sees names + descriptions for deferred tools and
    /// loads their full schemas on demand via a natural-language search query.
    ///
    /// See `crates/drivers/anthropic/src/driver.rs` callers and
    /// docs.claude.com/.../tool-search-tool for the wire format. Mirrors the
    /// OpenAI Responses `convert_tools_with_search`, minus namespaces (Anthropic
    /// defers each tool individually).
    ///
    /// `DeferrablePolicy::Never` tools keep full schemas (hot-path tools the model
    /// shouldn't have to search for). The search-tool entry is always
    /// non-deferred, which also satisfies Anthropic's "at least one tool must be
    /// non-deferred" constraint even when every function tool is deferrable.
    ///
    /// When deferral is active (above threshold), per-tool prompt-cache
    /// breakpoints are skipped: deferred tools are not part of the cached prefix,
    /// so a `cache_control` marker on them is pointless (system-prompt caching
    /// still applies via `mark_recent_text_blocks_for_cache`). Below threshold this
    /// delegates to the standard `convert_tools`, so `prompt_cache_enabled` is
    /// threaded through — hardcoding it would silently drop the tools-list cache
    /// breakpoint whenever tool_search is configured but inactive.
    fn convert_tools_with_search(
        tools: &[ToolDefinition],
        threshold: usize,
        prompt_cache_enabled: bool,
    ) -> Vec<AnthropicToolEntry> {
        // Below threshold the full schemas fit comfortably; don't defer, and keep
        // the standard cache behavior.
        if tools.len() < threshold {
            return Self::convert_tools(tools, prompt_cache_enabled);
        }

        let mut entries: Vec<AnthropicToolEntry> = Vec::with_capacity(tools.len() + 1);

        // The hosted search tool — natural-language (BM25) variant. Never deferred.
        entries.push(AnthropicToolEntry::Search(AnthropicToolSearchTool {
            r#type: "tool_search_tool_bm25_20251119".to_string(),
            name: "tool_search_tool_bm25".to_string(),
        }));

        for tool in tools {
            let should_defer = match tool.deferrable() {
                DeferrablePolicy::Never => false,
                DeferrablePolicy::Automatic | DeferrablePolicy::Always => true,
            };
            entries.push(AnthropicToolEntry::Function(AnthropicTool {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: Self::sanitize_input_schema(tool.parameters().clone()),
                cache_control: None,
                defer_loading: should_defer.then_some(true),
            }));
        }

        entries
    }
}

/// Store a `diagnostics` payload from the stream and log its headline.
///
/// The payload shape is Anthropic's, so it is carried verbatim on the
/// completion metadata; the log line exists because a cache divergence is the
/// kind of thing an operator wants to see without reading stored metadata.
fn record_cache_diagnostics(slot: &Arc<Mutex<Option<serde_json::Value>>>, diagnostics: Value) {
    tracing::info!(
        diagnostics = %diagnostics,
        "AnthropicDriver: received prompt cache diagnostics"
    );
    *slot.lock().unwrap() = Some(diagnostics);
}

#[async_trait]
impl ChatDriver for AnthropicChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Note: OTel instrumentation is handled via event listeners.
        // ReasonAtom emits llm.generation events, and OtelEventListener
        // creates gen-ai spans from those events.
        let prompt_cache_enabled = config.prompt_cache.as_ref().is_some_and(|cfg| cfg.enabled);
        let (system_prompt, anthropic_messages) =
            Self::convert_messages(&messages, prompt_cache_enabled, config.volatile_suffix_len);
        let system = Self::system_prompt_for_request(system_prompt, prompt_cache_enabled);

        // `[1m]` model ids (e.g. `claude-opus-4-8[1m]`) are the gateway's
        // large-context twins of the 200K base models. Anthropic's wire `model`
        // field only accepts the bare id; the 1M window is requested via the
        // `context-1m` beta header (added in the retry loop below). Strip the
        // suffix for everything that reasons about the canonical model, and
        // keep the flag for the header.
        let (wire_model, wants_million_context) = split_million_context(&config.model);

        let profile = everruns_provider::get_model_profile(
            &everruns_provider::DriverId::Anthropic,
            wire_model,
        );

        // Hosted tool_search (deferred tool loading) is gated on the Anthropic
        // model profile. When a hosted `ToolSearchConfig` is present and the
        // model supports it, defer tool schemas server-side via
        // `tool_search_tool_bm25_20251119` + per-tool `defer_loading`; otherwise
        // send full schemas. The config is provider-agnostic — set by the
        // `claude_tool_search` / `auto_tool_search` capability — and reaches here
        // on `config.tool_search`.
        let supports_tool_search = profile.as_ref().is_some_and(|p| p.tool_search);
        let tools = if config.tools.is_empty() {
            None
        } else if let Some(ref ts_config) = config.tool_search {
            if ts_config.enabled && supports_tool_search {
                Some(Self::convert_tools_with_search(
                    &config.tools,
                    ts_config.threshold,
                    prompt_cache_enabled,
                ))
            } else {
                Some(Self::convert_tools(&config.tools, prompt_cache_enabled))
            }
        } else {
            Some(Self::convert_tools(&config.tools, prompt_cache_enabled))
        };

        // Sampling parameters are removed on Fable 5.x and Opus 5/4.8/4.7 —
        // sending `temperature` returns 400 ("`temperature` is deprecated for
        // this model"). The model profile's `temperature` flag is the source
        // of truth; drop the parameter for models that reject it.
        let temperature = config.temperature.filter(|_| {
            let supported = profile.as_ref().is_none_or(|p| p.temperature);
            if !supported {
                tracing::warn!(
                    model = %config.model,
                    "AnthropicDriver: dropping temperature — not supported by this model"
                );
            }
            supported
        });

        // Build thinking config from reasoning effort.
        //
        // Recent Claude models (Fable 5.x, Opus 5/4.8/4.7, and the 4.6 family)
        // use adaptive thinking: `thinking: {type: "adaptive"}` plus
        // `output_config.effort`. On Fable 5.x and Opus 5/4.8/4.7 the budget-based
        // `thinking: {type: "enabled", budget_tokens}` form is removed and
        // returns 400, so this split is load-bearing, not stylistic.
        let (thinking, output_config) = match config.reasoning_effort {
            Some(effort) if uses_adaptive_thinking(wire_model) => {
                match adaptive_effort_level(effort) {
                    Some(level) => (
                        Some(AnthropicThinking::adaptive()),
                        Some(AnthropicOutputConfig {
                            effort: level.to_string(),
                        }),
                    ),
                    None => (None, None),
                }
            }
            Some(effort) => (AnthropicThinking::enabled_from_effort(effort), None),
            None => (None, None),
        };

        tracing::info!(
            model = %config.model,
            reasoning_effort = ?config.reasoning_effort,
            thinking = ?thinking,
            adaptive_effort = ?output_config.as_ref().map(|c| c.effort.as_str()),
            "AnthropicDriver: building request with thinking config"
        );

        // Calculate max_tokens - use caller's limit, or model's max output from profile, or 16384 fallback.
        // Anthropic requires max_tokens (can't omit), so we look up the model's native limit.
        let max_tokens_from_profile = config.max_tokens.is_none();
        let base_max_tokens = config.max_tokens.unwrap_or_else(|| {
            profile
                .as_ref()
                .and_then(|p| {
                    p.limits.as_ref().and_then(|l| {
                        u32::try_from(l.output)
                            .ok()
                            .and_then(|v| if v > 0 { Some(v) } else { None })
                    })
                })
                .unwrap_or(16_384)
        });
        let max_tokens = if let Some(AnthropicThinking::Enabled { budget_tokens }) = thinking {
            // max_tokens must be > thinking.budget_tokens per Anthropic requirements
            // Only increase if the caller's limit is too low for the thinking budget
            let min_for_thinking = budget_tokens + 1024; // minimum headroom for response
            base_max_tokens.max(min_for_thinking)
        } else {
            base_max_tokens
        };

        // Budget-based thinking with tools needs the interleaved-thinking beta
        // header; adaptive thinking interleaves automatically (no header).
        let needs_interleaved_thinking =
            matches!(thinking, Some(AnthropicThinking::Enabled { .. })) && tools.is_some();

        // Map the request-level parallel preference (EVE-598) onto Anthropic's
        // `tool_choice.disable_parallel_tool_use`. `tool_choice` is only valid
        // when tools are present, so skip it for tool-less requests.
        let tool_choice = if tools.is_some() {
            AnthropicToolChoice::from_parallel_preference(
                config
                    .resolved_parallel_tool_calls(self.supports_parallel_tool_calls(&config.model)),
            )
        } else {
            None
        };

        // Prompt-cache diagnostics (`cache-diagnosis` beta): opt in per request
        // and, from the second turn on, name the response the API should
        // compare this request against.
        let cache_diagnostics = config
            .cache_diagnostics
            .as_ref()
            .filter(|diagnostics| diagnostics.enabled);
        let diagnostics = cache_diagnostics.map(|diagnostics| AnthropicDiagnosticsRequest {
            previous_message_id: diagnostics.previous_message_id.clone(),
        });
        let wants_cache_diagnostics = diagnostics.is_some();

        let request = AnthropicRequest {
            model: wire_model.to_string(),
            messages: anthropic_messages,
            max_tokens,
            temperature,
            system,
            stream: true,
            tools,
            tool_choice,
            thinking,
            output_config,
            diagnostics,
        };

        // Share the (possibly fallback-mutated) request across reconnect
        // attempts: the classify closure mutates it for the one-shot max_tokens
        // fallback, and reusing the Arc means a reconnect re-sends the corrected
        // request.
        let request = Arc::new(Mutex::new(request));

        // Establish the SSE stream, transparently reconnecting on a transport
        // failure that lands before the first event (the "error decoding
        // response body" flake). Header-phase retries (429/5xx, transient send
        // failures, and the max_tokens fallback) are handled inside the
        // per-attempt send.
        let (event_stream, retry_metadata) =
            connect_sse_with_reconnect(&self.retry_config, "AnthropicDriver", |attempts| {
                self.send_messages_request(
                    endpoint,
                    Arc::clone(&request),
                    SendMessagesOptions {
                        needs_interleaved_thinking,
                        wants_million_context,
                        wants_cache_diagnostics,
                        max_tokens_from_profile,
                        model: &config.model,
                        extra_headers: &config.extra_headers,
                    },
                    attempts,
                )
            })
            .await?;

        let model = config.model.clone();
        let input_tokens = Arc::new(Mutex::new(0u32));
        let output_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let cache_creation_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let current_tool_call = Arc::new(Mutex::new(Option::<ToolCall>::None));
        let current_thinking = Arc::new(Mutex::new(Option::<OpenThinkingBlock>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));
        let response_id = Arc::new(Mutex::new(Option::<String>::None));
        let diagnostics_payload = Arc::new(Mutex::new(Option::<serde_json::Value>::None));
        // Share retry metadata with stream closure (only set if retries occurred)
        let shared_retry_metadata = if retry_metadata.had_retries() {
            Some(Arc::new(retry_metadata))
        } else {
            None
        };

        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let input_tokens = Arc::clone(&input_tokens);
            let output_tokens = Arc::clone(&output_tokens);
            let cache_read_tokens = Arc::clone(&cache_read_tokens);
            let cache_creation_tokens = Arc::clone(&cache_creation_tokens);
            let current_tool_call = Arc::clone(&current_tool_call);
            let current_thinking = Arc::clone(&current_thinking);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            let finish_reason = Arc::clone(&finish_reason);
            let response_id = Arc::clone(&response_id);
            let diagnostics_payload = Arc::clone(&diagnostics_payload);
            let retry_metadata_for_done = shared_retry_metadata.clone();

            async move {
                match result {
                    Ok(event) => {
                        // Anthropic uses different event types
                        match event.event.as_str() {
                            "message_start" => {
                                // Parse message_start for the message id, input
                                // token count, cache tokens, and prompt-cache
                                // diagnostics.
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicMessageStart>(&event.data)
                                {
                                    if let Some(id) = data.message.id {
                                        // The message id is what a following
                                        // request passes as
                                        // `diagnostics.previous_message_id`.
                                        *response_id.lock().unwrap() = Some(id);
                                    }
                                    if let Some(diagnostics) =
                                        data.message.diagnostics.or(data.diagnostics)
                                    {
                                        record_cache_diagnostics(
                                            &diagnostics_payload,
                                            diagnostics,
                                        );
                                    }
                                    if let Some(usage) = data.message.usage {
                                        *input_tokens.lock().unwrap() = usage.input_tokens;
                                        if let Some(cache_read) = usage.cache_read_input_tokens {
                                            *cache_read_tokens.lock().unwrap() = Some(cache_read);
                                        }
                                        if let Some(cache_creation) =
                                            usage.cache_creation_input_tokens
                                        {
                                            *cache_creation_tokens.lock().unwrap() =
                                                Some(cache_creation);
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_start" => {
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicContentBlockStart>(&event.data)
                                {
                                    match data.content_block {
                                        AnthropicContentBlockDelta::ToolUse { id, name } => {
                                            let mut current = current_tool_call.lock().unwrap();
                                            *current = Some(ToolCall {
                                                id,
                                                name,
                                                arguments: json!(""),
                                            });
                                        }
                                        AnthropicContentBlockDelta::Thinking { thinking } => {
                                            // Opens a block; text arrives as
                                            // thinking_delta and the signature
                                            // as signature_delta.
                                            *current_thinking.lock().unwrap() =
                                                Some(OpenThinkingBlock {
                                                    text: thinking,
                                                    ..Default::default()
                                                });
                                        }
                                        AnthropicContentBlockDelta::RedactedThinking { data } => {
                                            *current_thinking.lock().unwrap() =
                                                Some(OpenThinkingBlock {
                                                    redacted_payload: Some(data),
                                                    ..Default::default()
                                                });
                                        }
                                        AnthropicContentBlockDelta::Text { .. } => {}
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_delta" => {
                                if let Ok(data) = serde_json::from_str::<
                                    AnthropicContentBlockDeltaEvent,
                                >(&event.data)
                                {
                                    match data.delta {
                                        AnthropicDelta::TextDelta { text } => {
                                            // EVE-636: do not count deltas as tokens here —
                                            // deltas != tokens, and this took a mutex on every
                                            // token. Authoritative `output_tokens` is set from
                                            // the terminal `message_delta` usage event below.
                                            return Ok(LlmStreamEvent::TextDelta(text));
                                        }
                                        AnthropicDelta::InputJsonDelta { partial_json } => {
                                            // EVE-636: accumulate tool-input JSON in place via
                                            // push_str (amortized O(total)) instead of
                                            // re-copying + re-boxing into a Value per delta
                                            // (O(n^2)). Parsed once at content_block_stop.
                                            let mut current = current_tool_call.lock().unwrap();
                                            if let Some(ref mut tc) = *current {
                                                append_tool_input_delta(tc, &partial_json);
                                            }
                                            return Ok(LlmStreamEvent::TextDelta(String::new()));
                                        }
                                        AnthropicDelta::ThinkingDelta { thinking } => {
                                            let mut open = current_thinking.lock().unwrap();
                                            open.get_or_insert_with(OpenThinkingBlock::default)
                                                .text
                                                .push_str(&thinking);
                                            return Ok(LlmStreamEvent::ReasoningDelta {
                                                delta: thinking,
                                                summary: false,
                                            });
                                        }
                                        AnthropicDelta::SignatureDelta { signature } => {
                                            // Signs the block currently open, and
                                            // only that block.
                                            tracing::debug!(
                                                signature_len = signature.len(),
                                                "AnthropicDriver: received signature_delta from API"
                                            );
                                            let mut open = current_thinking.lock().unwrap();
                                            open.get_or_insert_with(OpenThinkingBlock::default)
                                                .signature = Some(signature);
                                            return Ok(LlmStreamEvent::TextDelta(String::new()));
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_stop" => {
                                // Debug: log raw content_block_stop data
                                tracing::debug!(
                                    raw_data = %event.data,
                                    "AnthropicDriver: received content_block_stop event"
                                );

                                // Finalize current tool call if any
                                {
                                    let mut current = current_tool_call.lock().unwrap();
                                    if let Some(mut tc) = current.take() {
                                        // EVE-636: parse the accumulated JSON string exactly once.
                                        finalize_tool_arguments(&mut tc);
                                        accumulated_tool_calls.lock().unwrap().push(tc);
                                    }
                                }

                                // Some responses carry the completed block
                                // inline; prefer its signature when present,
                                // otherwise the one accumulated from
                                // signature_delta.
                                let completed = serde_json::from_str::<AnthropicContentBlockStop>(
                                    &event.data,
                                )
                                .ok()
                                .and_then(|data| data.content_block);

                                let mut open = current_thinking.lock().unwrap();
                                if let Some(mut block) = open.take() {
                                    match completed {
                                        Some(AnthropicCompletedContentBlock::Thinking {
                                            thinking,
                                            signature,
                                        }) => {
                                            if !thinking.is_empty() {
                                                block.text = thinking;
                                            }
                                            block.signature = Some(signature);
                                        }
                                        Some(AnthropicCompletedContentBlock::RedactedThinking {
                                            data,
                                        }) => {
                                            block.redacted_payload = Some(data);
                                        }
                                        _ => {}
                                    }
                                    // A block without a signature cannot be
                                    // replayed: Anthropic rejects thinking it
                                    // did not sign. Drop it rather than send
                                    // an artifact that will fail verification.
                                    if block.signature.is_none()
                                        && block.redacted_payload.is_none()
                                    {
                                        tracing::warn!(
                                            thinking_len = block.text.len(),
                                            "AnthropicDriver: thinking block closed without a signature; not replayable"
                                        );
                                        return Ok(LlmStreamEvent::TextDelta(String::new()));
                                    }
                                    return Ok(LlmStreamEvent::ReasoningItem(
                                        block.into_reasoning_part(),
                                    ));
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "message_delta" => {
                                // Check for stop_reason and output tokens
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicMessageDelta>(&event.data)
                                {
                                    if let Some(diagnostics) = data.diagnostics {
                                        record_cache_diagnostics(&diagnostics_payload, diagnostics);
                                    }
                                    if let Some(usage) = data.usage {
                                        *output_tokens.lock().unwrap() = usage.output_tokens;
                                        // Cache tokens may also appear in delta
                                        if usage.cache_read_input_tokens.is_some() {
                                            *cache_read_tokens.lock().unwrap() =
                                                usage.cache_read_input_tokens;
                                        }
                                        if usage.cache_creation_input_tokens.is_some() {
                                            *cache_creation_tokens.lock().unwrap() =
                                                usage.cache_creation_input_tokens;
                                        }
                                    }

                                    if let Some(stop_reason) = data.delta.stop_reason {
                                        let normalized = match stop_reason.as_str() {
                                            "max_tokens" => "length",
                                            "tool_use" => "tool_calls",
                                            "refusal" => "refusal",
                                            _ => "stop",
                                        };
                                        *finish_reason.lock().unwrap() =
                                            Some(normalized.to_string());

                                        if stop_reason == "tool_use" {
                                            let tool_calls =
                                                accumulated_tool_calls.lock().unwrap().clone();
                                            if !tool_calls.is_empty() {
                                                return Ok(LlmStreamEvent::ToolCalls(tool_calls));
                                            }
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "message_stop" => {
                                let in_tokens = *input_tokens.lock().unwrap();
                                let out_tokens = *output_tokens.lock().unwrap();
                                let cache_read = *cache_read_tokens.lock().unwrap();
                                let cache_creation = *cache_creation_tokens.lock().unwrap();

                                Ok(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                                    total_tokens: Some(in_tokens + out_tokens),
                                    prompt_tokens: Some(in_tokens),
                                    completion_tokens: Some(out_tokens),
                                    cache_read_tokens: cache_read,
                                    cache_creation_tokens: cache_creation,
                                    provider_cost_usd: None,
                                    model: Some(model),
                                    finish_reason: finish_reason
                                        .lock()
                                        .unwrap()
                                        .clone()
                                        .or_else(|| Some("stop".to_string())),
                                    retry_metadata: retry_metadata_for_done
                                        .map(|arc| (*arc).clone()),
                                    response_id: response_id.lock().unwrap().clone(),
                                    phase: None,
                                    cache_diagnostics: diagnostics_payload
                                        .lock()
                                        .unwrap()
                                        .clone(),
                                })))
                            }
                            "error" => Ok(LlmStreamEvent::Error(
                                format!("Anthropic stream error: {}", event.data).into(),
                            )),
                            "ping" => {
                                // Keep-alive ping, ignore
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            _ => {
                                // Unknown event type, ignore
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                        }
                    }
                    Err(e) => Ok(LlmStreamEvent::Error(
                        format!("Stream error: {}", e).into(),
                    )),
                }
            }
        }));

        Ok(converted_stream)
    }

    /// Anthropic maps the preference onto `tool_choice.disable_parallel_tool_use`
    /// for every tool-capable Claude model.
    fn supports_parallel_tool_calls(&self, _model: &str) -> bool {
        true
    }

    async fn list_models(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs (proxies, self-hosted)
        if endpoint.base_url() != Some(DEFAULT_BASE_URL) {
            return Ok(None);
        }

        let url = endpoint
            .url("models")
            .ok_or_else(|| AgentLoopError::config("Anthropic provider has no base URL"))?;
        let resolved = endpoint.resolve("GET", url, &[]).await?;
        let mut request = self
            .client()
            .get(&resolved.url)
            .header("anthropic-version", ANTHROPIC_VERSION);
        for (name, value) in resolved.headers {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AgentLoopError::llm(format!(
                "Models API returned {}: {}",
                status, body
            )));
        }

        let models_response: AnthropicModelsResponse = response
            .json()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to parse models response: {}", e)))?;

        // All Anthropic models are chat models, no filtering needed
        let discovered: Vec<DiscoveredModel> = models_response
            .data
            .into_iter()
            .map(|m| {
                let profile = Some(m.to_discovered_profile());
                DiscoveredModel {
                    capabilities: vec!["chat".to_string()],
                    model_id: m.id,
                    display_name: Some(m.display_name),
                    created_at: m
                        .created_at
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc)),
                    owned_by: Some("anthropic".to_string()),
                    discovered_profile: profile,
                }
            })
            .collect();

        Ok(Some(discovered))
    }
}

impl std::fmt::Debug for AnthropicChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicChatDriver")
            .finish_non_exhaustive()
    }
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the Anthropic driver with the driver registry
///
/// This should be called at application startup to enable Anthropic model support.
///
/// # Example
///
/// ```ignore
/// use everruns_provider::DriverRegistry;
/// use everruns_anthropic::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(DriverDescriptor {
        display_name: "Anthropic".into(),
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key in the [Anthropic Console](https://console.anthropic.com/settings/keys).",
        ),
        ..DriverDescriptor::chat_only(DriverId::Anthropic, |config| {
            let provider = everruns_provider::Provider::new(config.provider.clone(), AnthropicChatDriver::new())
                .base_url(config.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL))
                .auth(everruns_provider::StaticHeaderAuth::new("x-api-key", config.api_key.as_deref().unwrap_or("")));
            provider.into_boxed_driver()
        })
    });
}

impl Default for AnthropicChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Streaming tool-call accumulation (EVE-636)
// ============================================================================

/// Appends a streamed tool-input JSON fragment onto the accumulating arguments
/// in place. During streaming `ToolCall::arguments` is kept as a
/// `Value::String`, so `push_str` grows it in amortized O(total) — avoiding the
/// O(n^2) re-copy + re-box (`format!` + `json!`) that the per-delta path used.
/// The string is parsed once at `content_block_stop` via
/// [`finalize_tool_arguments`].
fn append_tool_input_delta(tool_call: &mut ToolCall, fragment: &str) {
    if let serde_json::Value::String(s) = &mut tool_call.arguments {
        s.push_str(fragment);
    }
}

/// Parses the accumulated tool-input JSON string into a structured value once a
/// tool-use content block completes. Empty/invalid JSON falls back to `{}`.
fn finalize_tool_arguments(tool_call: &mut ToolCall) {
    if let Some(args_str) = tool_call.arguments.as_str() {
        tool_call.arguments = serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
    }
}

// ============================================================================
// Error Detection Helpers
// ============================================================================

fn is_anthropic_model_not_found(status: reqwest::StatusCode, error_text: &str) -> bool {
    if driver_helpers::is_model_not_found(status, error_text, ANTHROPIC_NOT_FOUND_PATTERNS) {
        return true;
    }
    // Compound check: both "model" and "not found" must appear together
    if status == reqwest::StatusCode::NOT_FOUND {
        let lower = error_text.to_lowercase();
        if lower.contains("model") && lower.contains("not found") {
            return true;
        }
    }
    false
}

fn visible_tool_call_ids(messages: &[LlmMessage]) -> HashSet<&str> {
    messages
        .iter()
        .filter(|msg| msg.role == LlmMessageRole::Assistant)
        .flat_map(|msg| msg.tool_calls.iter().flatten())
        .map(|tool_call| tool_call.id.as_str())
        .collect()
}

fn is_anthropic_request_too_large(status: reqwest::StatusCode, error_text: &str) -> bool {
    driver_helpers::is_request_too_large(status, error_text, ANTHROPIC_TOO_LARGE_PATTERNS)
}

// ============================================================================
// Anthropic API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<AnthropicSystem>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicToolEntry>>,
    /// Tool-choice controls. Carries `disable_parallel_tool_use` to map the
    /// request-level `parallel_tool_calls` preference (EVE-598). Only sent when
    /// the request has tools and a parallel preference is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<AnthropicToolChoice>,
    /// Extended thinking configuration (for Claude models that support it)
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
    /// Output configuration — carries `effort` for adaptive thinking
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<AnthropicOutputConfig>,
    /// Prompt-cache diagnostics opt-in (`cache-diagnosis` beta).
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<AnthropicDiagnosticsRequest>,
}

/// Request-level `diagnostics` object.
///
/// `previous_message_id` must be serialized even when it is `null`: sending it
/// as an explicit null is how a first turn opts in without a prior message to
/// compare against.
#[derive(Debug, Serialize)]
struct AnthropicDiagnosticsRequest {
    previous_message_id: Option<String>,
}

/// Anthropic `tool_choice` object.
///
/// We always use `type: "auto"` (the model decides whether/which tools to call)
/// and only set this when mapping the request-level `parallel_tool_calls`
/// preference: `Some(false)` → `disable_parallel_tool_use = true`, `Some(true)`
/// → `disable_parallel_tool_use = false` (explicitly allow parallel use).
#[derive(Debug, Serialize)]
struct AnthropicToolChoice {
    r#type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    disable_parallel_tool_use: Option<bool>,
}

impl AnthropicToolChoice {
    /// Build an `auto` tool choice that encodes the parallel preference, or
    /// `None` when no preference is set (preserve the provider default).
    fn from_parallel_preference(parallel_tool_calls: Option<bool>) -> Option<Self> {
        parallel_tool_calls.map(|parallel| Self {
            r#type: "auto",
            disable_parallel_tool_use: Some(!parallel),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicCacheControl {
    r#type: String,
}

impl AnthropicCacheControl {
    fn ephemeral() -> Self {
        Self {
            r#type: "ephemeral".to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicSystem {
    Text(String),
    Blocks(Vec<AnthropicSystemBlock>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum AnthropicSystemBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
}

/// Thinking configuration for Claude.
///
/// `Enabled` is the legacy budget-based form; `Adaptive` is required on
/// Fable 5.x and Opus 5/4.8/4.7 (where `budget_tokens` returns 400) and is the
/// recommended form on the 4.6 family. "No thinking" is always expressed by
/// omitting the field — an explicit `{type: "disabled"}` is rejected by
/// Fable 5.x.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum AnthropicThinking {
    Enabled {
        /// Budget tokens for thinking (varies by effort level)
        budget_tokens: u32,
    },
    Adaptive {
        /// Fable 5.x and Opus 5/4.8/4.7 omit thinking text by default
        /// (`display: "omitted"`); "summarized" restores it so assistant
        /// messages keep their thinking content like on budget-based models.
        display: &'static str,
    },
}

impl AnthropicThinking {
    /// Create a budget-based thinking config from a reasoning effort level
    fn enabled_from_effort(effort: ReasoningEffort) -> Option<Self> {
        driver_helpers::thinking_budget::from_effort(effort)
            .map(|budget_tokens| Self::Enabled { budget_tokens })
    }

    /// Adaptive thinking with summarized (visible) thinking content
    fn adaptive() -> Self {
        Self::Adaptive {
            display: "summarized",
        }
    }
}

/// `output_config` request field — carries the effort level that controls
/// adaptive thinking depth.
#[derive(Debug, Serialize)]
struct AnthropicOutputConfig {
    effort: String,
}

/// Claude families that use adaptive thinking. On Fable 5.x, Opus 5/4.8/4.7,
/// and Sonnet 5 budget-based thinking is removed (400); on Opus 4.6 / Sonnet 4.6
/// it is deprecated and adaptive is the recommended form. Keep in sync with the
/// adaptive-thinking profiles in `everruns_provider::model_profiles`.
///
/// `claude-fable-5-1` is listed on its own: `normalize_anthropic_id` only
/// strips 8-digit date suffixes, so the `-1` does not collapse to Fable 5.
const ADAPTIVE_THINKING_FAMILIES: &[&str] = &[
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
];

/// Anthropic families that support the 1M context window (Anthropic docs:
/// context-windows / long-context pricing). Gates `[1m]` suffix handling. These
/// coincide with `ADAPTIVE_THINKING_FAMILIES` today but are a distinct
/// capability — kept separate so a future divergence (1M without adaptive
/// thinking, or vice versa) cannot silently mis-gate either path.
const MILLION_CONTEXT_FAMILIES: &[&str] = &[
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
];

/// Split a `[1m]`-suffixed Anthropic model id (e.g. `claude-opus-4-8[1m]`) into
/// the bare wire id Anthropic accepts and a flag marking the 1M-context twin.
///
/// The suffix is honored only when the bare id belongs to a family that
/// actually supports the 1M window (`MILLION_CONTEXT_FAMILIES`). A
/// manually-configured id that merely ends in `[1m]` but is not 1M-capable —
/// e.g. `claude-haiku-4-5[1m]` or `claude-sonnet-4-5[1m]` — is left untouched. We
/// must never rewrite an arbitrary configured id or send the `context-1m` beta
/// header to a model that does not support the 1M window (it can 400 or
/// silently truncate on models where the header was retired). Date-suffixed 1M
/// ids (`claude-opus-4-8-20260101[1m]`) are still honored via family
/// normalization.
fn split_million_context(model_id: &str) -> (&str, bool) {
    match model_id.strip_suffix("[1m]") {
        Some(bare) if is_million_context_family(bare) => (bare, true),
        _ => (model_id, false),
    }
}

/// Whether a bare (optionally date-suffixed) Anthropic model id belongs to a
/// family that supports the 1M context window.
fn is_million_context_family(model_id: &str) -> bool {
    let family = normalize_anthropic_id(model_id);
    MILLION_CONTEXT_FAMILIES
        .iter()
        .any(|f| family.eq_ignore_ascii_case(f))
}

/// Whether a model id (optionally date-suffixed) belongs to an
/// adaptive-thinking family.
fn uses_adaptive_thinking(model_id: &str) -> bool {
    let family = normalize_anthropic_id(model_id);
    ADAPTIVE_THINKING_FAMILIES
        .iter()
        .any(|f| family.eq_ignore_ascii_case(f))
}

/// Map an everruns reasoning-effort level to the `output_config.effort` value
/// used with adaptive thinking. `xhigh` is surfaced as "Max" in the model
/// profiles and maps to the API's `max` level.
/// Map a reasoning effort onto Anthropic's adaptive `output_config.effort`.
///
/// Anthropic's scale tops out at `max` rather than `xhigh`, and has no separate
/// `minimal`, so the lowest non-zero effort maps to `low`. Previously `minimal`
/// fell through to `None` here and disabled thinking entirely, matching the same
/// hole the budget-based path had.
fn adaptive_effort_level(effort: ReasoningEffort) -> Option<&'static str> {
    match effort {
        ReasoningEffort::None => None,
        ReasoningEffort::Minimal | ReasoningEffort::Low => Some("low"),
        ReasoningEffort::Medium => Some("medium"),
        ReasoningEffort::High => Some("high"),
        // `Max` (OpenAI-only, above `Xhigh`) collapses onto the same "max"
        // level: Anthropic's own profiles never offer it as a distinct choice.
        ReasoningEffort::Xhigh | ReasoningEffort::Max => Some("max"),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        /// Cryptographic signature required when sending thinking back to the API
        signature: String,
    },
    /// Withheld reasoning, replayed verbatim as the API returned it.
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: AnthropicToolResultContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// Content of a tool_result block - either a simple string or array of content blocks.
/// Anthropic API accepts both forms; we use the array form when images are present.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicToolResultContent {
    /// Simple text content
    Text(String),
    /// Array of content blocks (text + images)
    Blocks(Vec<AnthropicToolResultBlock>),
}

/// A content block inside a tool_result (text or image)
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicToolResultBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicImageSource {
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
    #[serde(rename = "url")]
    Url { url: String },
}

/// A tools-array entry: either a regular function tool or the hosted
/// `tool_search_tool_*_20251119` server tool. Untagged so each variant
/// serializes to its own object shape (the server tool has only `type`/`name`,
/// no `input_schema`).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicToolEntry {
    Search(AnthropicToolSearchTool),
    Function(AnthropicTool),
}

/// The hosted tool-search server tool, e.g.
/// `{"type": "tool_search_tool_bm25_20251119", "name": "tool_search_tool_bm25"}`.
#[derive(Debug, Serialize)]
struct AnthropicToolSearchTool {
    r#type: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<AnthropicCacheControl>,
    /// When `Some(true)`, the tool's schema is loaded on demand via hosted
    /// tool_search instead of being sent in the prefix. Omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    defer_loading: Option<bool>,
}

// Streaming response types

#[derive(Debug, Deserialize)]
struct AnthropicMessageStart {
    message: AnthropicMessageInfo,
    /// Some beta payloads carry `diagnostics` beside `message` rather than
    /// inside it; accept both placements.
    #[serde(default)]
    diagnostics: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // model is deserialized but used by event listeners, not directly
struct AnthropicMessageInfo {
    /// Unique identifier for this message
    #[serde(default)]
    id: Option<String>,
    /// Model used for this message
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    /// Prompt-cache diagnostics for this request, returned verbatim when the
    /// request opted into the `cache-diagnosis` beta.
    #[serde(default)]
    diagnostics: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    /// Tokens read from cache (reduces cost)
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    /// Tokens written to cache
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockStart {
    content_block: AnthropicContentBlockDelta,
}

/// Completed content block from content_block_stop event
/// Includes the cryptographic signature for thinking blocks
#[derive(Debug, Deserialize)]
struct AnthropicContentBlockStop {
    #[serde(default)]
    content_block: Option<AnthropicCompletedContentBlock>,
}

/// Completed content block variants (from content_block_stop)
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)] // Fields used for JSON deserialization
enum AnthropicCompletedContentBlock {
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        /// Cryptographic signature for the thinking content (required to send it back)
        signature: String,
    },
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

/// The thinking block currently being streamed.
///
/// Anthropic signs each thinking block separately, and interleaved thinking
/// emits several per response. Accumulating per block — rather than into one
/// buffer for the whole message — is what keeps each signature paired with the
/// text it actually signs.
#[derive(Debug, Default)]
struct OpenThinkingBlock {
    text: String,
    signature: Option<String>,
    redacted_payload: Option<String>,
}

impl OpenThinkingBlock {
    fn into_reasoning_part(self) -> ReasoningContentPart {
        let mut part = ReasoningContentPart::opaque("anthropic");
        if let Some(signature) = self.signature {
            part = part.with_signature(signature);
        }
        if let Some(data) = self.redacted_payload {
            // The redacted payload is the replay artifact; it is opaque and
            // carries no readable text.
            part = part.with_encrypted(data).with_text(ReasoningText::Redacted);
        } else {
            part = part.with_text(ReasoningText::Plain { text: self.text });
        }
        part
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)] // Fields used for deserialization
enum AnthropicContentBlockDelta {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    /// Reasoning the provider withheld. Arrives whole on `content_block_start`
    /// and carries no readable text, but must still be replayed verbatim.
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockDeltaEvent {
    delta: AnthropicDelta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)] // Delta suffix matches Anthropic's API naming
enum AnthropicDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    /// Cryptographic signature for thinking content (sent after thinking_delta completes)
    #[serde(rename = "signature_delta")]
    SignatureDelta { signature: String },
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDelta {
    delta: AnthropicMessageDeltaData,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    /// Diagnostics may also arrive on the terminal `message_delta`; the last
    /// payload seen wins.
    #[serde(default)]
    diagnostics: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDeltaData {
    stop_reason: Option<String>,
}

// ============================================================================
// Models API Types
// ============================================================================

/// Response from Anthropic's /v1/models endpoint
#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModelInfo>,
}

/// Individual model info from Anthropic's models API
///
/// Includes structured capabilities and token limits returned by the
/// `/v1/models` endpoint, used to build `ModelProfile` at discovery time.
#[derive(Debug, Deserialize)]
struct AnthropicModelInfo {
    /// Model identifier (e.g., "claude-opus-4-5-20251101")
    id: String,
    /// Human-readable display name (e.g., "Claude Opus 4.5")
    display_name: String,
    /// ISO 8601 timestamp when the model was created
    #[serde(default)]
    created_at: Option<String>,
    /// Maximum input context window size in tokens
    #[serde(default)]
    max_input_tokens: Option<u32>,
    /// Maximum output tokens
    #[serde(default)]
    max_tokens: Option<u32>,
    /// Model capabilities
    #[serde(default)]
    capabilities: Option<AnthropicModelCapabilities>,
}

/// Capability support flag from Anthropic's models API
#[derive(Debug, Deserialize, Default)]
struct CapabilitySupport {
    #[serde(default)]
    supported: bool,
}

/// Effort capability with per-level support
#[derive(Debug, Deserialize, Default)]
struct EffortCapability {
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    low: Option<CapabilitySupport>,
    #[serde(default)]
    medium: Option<CapabilitySupport>,
    #[serde(default)]
    high: Option<CapabilitySupport>,
    #[serde(default)]
    max: Option<CapabilitySupport>,
}

/// Thinking capability with type configurations
#[derive(Debug, Deserialize, Default)]
struct ThinkingCapability {
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    types: Option<ThinkingTypes>,
}

/// Supported thinking type configurations
#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)] // Fields deserialized from API; stored as metadata
struct ThinkingTypes {
    #[serde(default)]
    enabled: Option<CapabilitySupport>,
    #[serde(default)]
    adaptive: Option<CapabilitySupport>,
}

/// Model capabilities from Anthropic's models API
#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)] // Fields deserialized from API; not all used yet but stored as metadata
struct AnthropicModelCapabilities {
    #[serde(default)]
    image_input: Option<CapabilitySupport>,
    #[serde(default)]
    pdf_input: Option<CapabilitySupport>,
    #[serde(default)]
    structured_outputs: Option<CapabilitySupport>,
    #[serde(default)]
    thinking: Option<ThinkingCapability>,
    #[serde(default)]
    effort: Option<EffortCapability>,
    #[serde(default)]
    citations: Option<CapabilitySupport>,
    #[serde(default)]
    code_execution: Option<CapabilitySupport>,
    #[serde(default)]
    batch: Option<CapabilitySupport>,
}

/// Normalize Anthropic model ID to a family base name by stripping trailing
/// date suffix (e.g., "claude-opus-4-5-20251101" -> "claude-opus-4-5").
fn normalize_anthropic_id(model_id: &str) -> &str {
    // Anthropic date suffixes are always -YYYYMMDD (8 digits after a dash)
    if let Some((base, suffix)) = model_id.rsplit_once('-')
        && !base.is_empty()
        && suffix.len() == 8
        && suffix.bytes().all(|b| b.is_ascii_digit())
    {
        return base;
    }
    model_id
}

impl AnthropicModelInfo {
    /// Build an `ModelProfile` from the API-provided metadata.
    ///
    /// This profile contains limits and capability flags discovered from the API.
    /// Cost data is NOT available from the API and remains in hardcoded profiles.
    fn to_discovered_profile(&self) -> everruns_provider::model::ModelProfile {
        use everruns_provider::model::*;

        let caps = self.capabilities.as_ref();

        // Build token limits from API fields
        let limits = match (self.max_input_tokens, self.max_tokens) {
            (Some(input), Some(output)) => Some(ModelLimits {
                context: i32::try_from(input).unwrap_or(i32::MAX),
                input: None,
                output: i32::try_from(output).unwrap_or(i32::MAX),
                max_media: None,
            }),
            _ => None,
        };

        // Determine if model supports image/PDF input (implies attachment support)
        let image_input = caps
            .and_then(|c| c.image_input.as_ref())
            .is_some_and(|c| c.supported);
        let pdf_input = caps
            .and_then(|c| c.pdf_input.as_ref())
            .is_some_and(|c| c.supported);
        let supports_attachments = image_input || pdf_input;

        // Build modalities from capabilities
        let modalities = {
            let mut input_mods = vec![Modality::Text];
            if image_input {
                input_mods.push(Modality::Image);
            }
            if pdf_input {
                input_mods.push(Modality::Pdf);
            }
            Some(ModelModalities {
                input: input_mods,
                output: vec![Modality::Text],
            })
        };

        // Build reasoning effort config from thinking + effort capabilities
        let reasoning = caps
            .and_then(|c| c.thinking.as_ref())
            .is_some_and(|t| t.supported);

        let reasoning_effort = if reasoning {
            self.build_reasoning_effort(caps)
        } else {
            None
        };

        let structured_output = caps
            .and_then(|c| c.structured_outputs.as_ref())
            .is_some_and(|c| c.supported);

        ModelProfile {
            name: self.display_name.clone(),
            family: normalize_anthropic_id(&self.id).to_string(),
            description: None,
            release_date: self
                .created_at
                .as_ref()
                .and_then(|s| s.get(..10))
                .map(|s| s.to_string()),
            last_updated: None,
            attachment: supports_attachments,
            reasoning,
            temperature: true, // All Claude models support temperature
            knowledge: None,   // Not available from API
            tool_call: true,   // All Claude models support tool use
            structured_output,
            open_weights: false,
            cost: None, // Not available from API; hardcoded profiles provide this
            limits,
            modalities,
            reasoning_effort,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }
    }

    fn build_reasoning_effort(
        &self,
        caps: Option<&AnthropicModelCapabilities>,
    ) -> Option<everruns_provider::model::ReasoningEffortConfig> {
        use everruns_provider::model::*;

        let thinking = caps?.thinking.as_ref()?;
        if !thinking.supported {
            return None;
        }

        let types = thinking.types.as_ref();
        let supports_adaptive = types
            .and_then(|t| t.adaptive.as_ref())
            .is_some_and(|c| c.supported);

        // Check effort capability for supported levels
        let effort_cap = caps?.effort.as_ref();
        let effort_supported = effort_cap.is_some_and(|e| e.supported);

        if !effort_supported {
            // Thinking supported but no effort levels — basic extended thinking
            return Some(ReasoningEffortConfig {
                values: vec![
                    ReasoningEffortValue {
                        value: ReasoningEffort::Low,
                        name: "Low (1K tokens)".into(),
                    },
                    ReasoningEffortValue {
                        value: ReasoningEffort::Medium,
                        name: "Medium (4K tokens)".into(),
                    },
                    ReasoningEffortValue {
                        value: ReasoningEffort::High,
                        name: "High (16K tokens)".into(),
                    },
                    ReasoningEffortValue {
                        value: ReasoningEffort::Xhigh,
                        name: "Extra High (32K tokens)".into(),
                    },
                ],
                default: ReasoningEffort::Medium,
            });
        }

        // Build effort values from per-level support flags
        let ec = effort_cap.unwrap();
        let mut values = Vec::new();

        if ec.low.as_ref().is_some_and(|c| c.supported) {
            let name = if supports_adaptive {
                "Low"
            } else {
                "Low (1K tokens)"
            };
            values.push(ReasoningEffortValue {
                value: ReasoningEffort::Low,
                name: name.into(),
            });
        }
        if ec.medium.as_ref().is_some_and(|c| c.supported) {
            let name = if supports_adaptive {
                "Medium"
            } else {
                "Medium (4K tokens)"
            };
            values.push(ReasoningEffortValue {
                value: ReasoningEffort::Medium,
                name: name.into(),
            });
        }
        if ec.high.as_ref().is_some_and(|c| c.supported) {
            let name = if supports_adaptive {
                "High"
            } else {
                "High (16K tokens)"
            };
            values.push(ReasoningEffortValue {
                value: ReasoningEffort::High,
                name: name.into(),
            });
        }
        if ec.max.as_ref().is_some_and(|c| c.supported) {
            let name = if supports_adaptive {
                "Max"
            } else {
                "Extra High (32K tokens)"
            };
            values.push(ReasoningEffortValue {
                value: ReasoningEffort::Xhigh,
                name: name.into(),
            });
        }

        if values.is_empty() {
            return None;
        }

        // Prefer the usual default only when the catalog advertises it.
        let preferred = if supports_adaptive {
            ReasoningEffort::High
        } else {
            ReasoningEffort::Medium
        };
        let default = if values.iter().any(|v| v.value == preferred) {
            preferred
        } else {
            values[0].value
        };

        Some(ReasoningEffortConfig { values, default })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::ChatDriver;
    use everruns_provider::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};

    fn contract_config(model: &str, max_tokens: Option<u32>) -> LlmCallConfig {
        LlmCallConfig {
            model: model.into(),
            temperature: None,
            max_tokens,
            tools: vec![],
            reasoning_effort: None,
            reasoning_state: None,
            metadata: Default::default(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: vec![],
            cache_diagnostics: None,
            speed: None,
            verbosity: None,
        }
    }

    fn contract_tool(name: &str, deferrable: DeferrablePolicy) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.into(),
            display_name: None,
            description: "Search records".into(),
            parameters: json!({"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable,
            hints: ToolHints::default(),
            full_parameters: None,
        })
    }

    async fn assert_contract_request(config: LlmCallConfig, registered: bool, expected: Value) {
        use everruns_provider::{Provider, StaticHeaderAuth};
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "synthetic-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(body_json(expected))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error":{"message":"request captured"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let base = format!("{}/v1", server.uri());
        let driver = if registered {
            let mut registry = DriverRegistry::new();
            register_driver(&mut registry);
            registry
                .create_chat_driver(
                    &everruns_provider::driver_registry::ProviderConfig::new(DriverId::Anthropic)
                        .with_api_key("synthetic-key")
                        .with_base_url(base),
                )
                .unwrap()
        } else {
            Provider::new("test", AnthropicChatDriver::new())
                .base_url(base)
                .auth(StaticHeaderAuth::new("x-api-key", "synthetic-key"))
                .into_boxed_driver()
        };
        let error = match driver
            .chat_completion_stream(
                &everruns_provider::ProviderEndpoint::default(),
                vec![LlmMessage::text(LlmMessageRole::User, "hello")],
                &config,
            )
            .await
        {
            Ok(_) => panic!("expected capture response"),
            Err(error) => error,
        };
        assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::InvalidRequest));
        assert!(error.to_string().contains("request captured"), "{error}");
        server.verify().await;
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].headers.contains_key("authorization"));
    }

    #[tokio::test]
    async fn registered_and_direct_requests_apply_parallel_preferences_only_with_tools() {
        for registered in [false, true] {
            for tools in [false, true] {
                for preference in [None, Some(true), Some(false)] {
                    let mut config = contract_config("claude-test", Some(32));
                    config.parallel_tool_calls = preference;
                    let mut expected = json!({"model":"claude-test","max_tokens":32,"stream":true,
                        "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]});
                    if tools {
                        config
                            .tools
                            .push(contract_tool("lookup", DeferrablePolicy::Never));
                        expected["tools"] = json!([{"name":"lookup","description":"Search records","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}}]);
                        if let Some(allow) = preference {
                            expected["tool_choice"] =
                                json!({"type":"auto","disable_parallel_tool_use":!allow});
                        }
                    }
                    assert_contract_request(config, registered, expected).await;
                }
            }
        }
    }

    #[tokio::test]
    async fn requests_resolve_model_limits_and_complete_reasoning_policies() {
        for (model, requested, expected_limit) in [
            ("claude-sonnet-4-5-20250514", None, 64000),
            ("claude-test", None, 16384),
            ("claude-sonnet-4-5", Some(99), 99),
            ("claude-test", Some(99), 99),
        ] {
            assert_contract_request(
                contract_config(model, requested),
                false,
                json!({"model":model,"max_tokens":expected_limit,"stream":true,
                "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}),
            )
            .await;
        }
        for (effort, budget, adaptive) in [
            (ReasoningEffort::None, None, None),
            (ReasoningEffort::Minimal, Some(1024), Some("low")),
            (ReasoningEffort::Low, Some(1024), Some("low")),
            (ReasoningEffort::Medium, Some(4096), Some("medium")),
            (ReasoningEffort::High, Some(16384), Some("high")),
            (ReasoningEffort::Xhigh, Some(32768), Some("max")),
            (ReasoningEffort::Max, Some(32768), Some("max")),
        ] {
            for model in ["claude-sonnet-4-5", "claude-opus-4-8"] {
                let mut config = contract_config(model, Some(1));
                config.reasoning_effort = Some(effort);
                let mut expected = json!({"model":model,"max_tokens":1,"stream":true,
                    "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]});
                if model == "claude-sonnet-4-5" {
                    if let Some(budget) = budget {
                        expected["max_tokens"] = json!(budget + 1024);
                        expected["thinking"] = json!({"type":"enabled","budget_tokens":budget});
                    }
                } else if let Some(level) = adaptive {
                    expected["thinking"] = json!({"type":"adaptive","display":"summarized"});
                    expected["output_config"] = json!({"effort":level});
                }
                assert_contract_request(config, false, expected).await;
            }
        }
    }

    #[test]
    fn cache_markers_preserve_system_text_and_target_stable_message_blocks() {
        for (text, enabled, expected) in [
            (None, true, Value::Null),
            (Some(""), true, json!("")),
            (Some("prompt"), false, json!("prompt")),
            (
                Some("prompt"),
                true,
                json!([{"type":"text","text":"prompt","cache_control":{"type":"ephemeral"}}]),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(AnthropicChatDriver::system_prompt_for_request(
                    text.map(str::to_owned),
                    enabled
                ))
                .unwrap(),
                expected
            );
        }
        let mut first = LlmMessage::text(LlmMessageRole::User, "");
        first.content = LlmMessageContent::Parts(vec![
            LlmContentPart::Text {
                text: "first".into(),
            },
            LlmContentPart::Text {
                text: "last".into(),
            },
            LlmContentPart::Image {
                url: "https://example.com/a.png".into(),
            },
        ]);
        let messages = [
            first,
            LlmMessage::text(LlmMessageRole::Assistant, "reply"),
            LlmMessage::text(LlmMessageRole::User, "question"),
            LlmMessage::text(LlmMessageRole::Assistant, "volatile"),
        ];
        for (enabled, volatile, positions) in [
            (false, 0, vec![]),
            (true, 0, vec![(2, 0), (3, 0)]),
            (true, 1, vec![(1, 0), (2, 0)]),
            (true, 2, vec![(0, 1), (1, 0)]),
            (true, 3, vec![(0, 1)]),
            (true, 4, vec![]),
            (true, usize::MAX, vec![]),
        ] {
            let mut expected = json!([
                {"role":"user","content":[{"type":"text","text":"first"},{"type":"text","text":"last"},{"type":"image","source":{"type":"url","url":"https://example.com/a.png"}}]},
                {"role":"assistant","content":[{"type":"text","text":"reply"}]},
                {"role":"user","content":[{"type":"text","text":"question"}]},
                {"role":"assistant","content":[{"type":"text","text":"volatile"}]}
            ]);
            for (message, block) in positions {
                expected[message]["content"][block]["cache_control"] = json!({"type":"ephemeral"});
            }
            let (_, actual) = AnthropicChatDriver::convert_messages(&messages, enabled, volatile);
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                expected,
                "enabled={enabled} volatile={volatile}"
            );
        }
        let (_, empty) = AnthropicChatDriver::convert_messages(&[], true, 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn tool_search_preserves_complete_schemas_and_cache_thresholds() {
        let tools = [
            contract_tool("automatic", DeferrablePolicy::Automatic),
            contract_tool("always", DeferrablePolicy::Always),
            contract_tool("never", DeferrablePolicy::Never),
        ];
        for enabled in [false, true] {
            assert!(AnthropicChatDriver::convert_tools(&[], enabled).is_empty());
            for threshold in [2, 3, 4] {
                let mut expected = json!([
                    {"name":"automatic","description":"Search records","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}},
                    {"name":"always","description":"Search records","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}},
                    {"name":"never","description":"Search records","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}}
                ]);
                if threshold <= 3 {
                    expected[0]["defer_loading"] = json!(true);
                    expected[1]["defer_loading"] = json!(true);
                    expected.as_array_mut().unwrap().insert(0,json!({"type":"tool_search_tool_bm25_20251119","name":"tool_search_tool_bm25"}));
                } else if enabled {
                    expected[2]["cache_control"] = json!({"type":"ephemeral"});
                }
                assert_eq!(
                    serde_json::to_value(AnthropicChatDriver::convert_tools_with_search(
                        &tools, threshold, enabled
                    ))
                    .unwrap(),
                    expected,
                    "threshold={threshold} cache={enabled}"
                );
            }
        }
    }

    #[test]
    fn message_conversion_folds_system_text_without_losing_the_transcript() {
        for with_system in [false, true] {
            let mut messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
            if with_system {
                messages.insert(
                    0,
                    LlmMessage::text(LlmMessageRole::System, "first instruction"),
                );
                messages.push(LlmMessage::text(LlmMessageRole::System, "later summary"));
            }
            messages.push(LlmMessage::text(LlmMessageRole::Assistant, "reply"));
            let (system, converted) = AnthropicChatDriver::convert_messages(&messages, false, 0);
            assert_eq!(
                system.as_deref(),
                with_system.then_some("first instruction\n\nlater summary")
            );
            assert_eq!(
                serde_json::to_value(converted).unwrap(),
                json!([
                    {"role":"user","content":[{"type":"text","text":"hello"}]},
                    {"role":"assistant","content":[{"type":"text","text":"reply"}]}
                ])
            );
        }
    }

    #[test]
    fn tool_exchanges_preserve_identity_and_filter_only_orphan_results() {
        let mut assistant = LlmMessage::text(LlmMessageRole::Assistant, "");
        assistant.tool_calls = Some(vec![ToolCall {
            id: "call_123".into(),
            name: "get_weather".into(),
            arguments: json!({"city":"London"}),
        }]);
        let mut valid = LlmMessage::text(LlmMessageRole::Tool, "{\"temp\":20}");
        valid.tool_call_id = Some("call_123".into());
        let mut orphan = LlmMessage::text(LlmMessageRole::Tool, "orphan result");
        orphan.tool_call_id = Some("trimmed_call".into());
        let missing_id = LlmMessage::text(LlmMessageRole::Tool, "missing ID");
        let (system, converted) = AnthropicChatDriver::convert_messages(
            &[orphan, assistant, missing_id, valid],
            false,
            0,
        );
        assert!(system.is_none());
        assert_eq!(
            serde_json::to_value(converted).unwrap(),
            json!([
                {"role":"assistant","content":[{"type":"tool_use","id":"call_123","name":"get_weather","input":{"city":"London"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_123","content":"{\"temp\":20}"}]}
            ])
        );
    }

    #[test]
    fn model_family_normalization_requires_a_nonempty_base_and_eight_ascii_digits() {
        for (input, expected) in [
            ("claude-opus-4-5-20251101", "claude-opus-4-5"),
            ("claude-sonnet-4-6-20260217", "claude-sonnet-4-6"),
            ("claude-opus-4-5", "claude-opus-4-5"),
            ("claude-sonnet-4-6", "claude-sonnet-4-6"),
            ("claudé-opus-20251101", "claudé-opus"),
            ("claudé-opus-experimental", "claudé-opus-experimental"),
            ("", ""),
            ("-20251101", "-20251101"),
            ("claude-2025110", "claude-2025110"),
            ("claude-202511011", "claude-202511011"),
            ("claude-2025x101", "claude-2025x101"),
            ("claude-２０２５１１０１", "claude-２０２５１１０１"),
        ] {
            assert_eq!(normalize_anthropic_id(input), expected, "{input}");
        }
    }

    #[test]
    fn fragmented_tool_arguments_preserve_payloads_and_handle_incomplete_json() {
        let payload = r#"{"path":"src/main.rs","contents":"fn main() { println!(\"hello — 世界\"); }","count":1234567}"#;
        let mut call = ToolCall {
            id: "call".into(),
            name: "write_file".into(),
            arguments: json!(""),
        };
        for (offset, ch) in payload.char_indices() {
            append_tool_input_delta(&mut call, &ch.to_string());
            assert_eq!(
                call.arguments.as_str(),
                Some(&payload[..offset + ch.len_utf8()])
            );
        }
        finalize_tool_arguments(&mut call);
        assert_eq!(
            serde_json::to_value(&call).unwrap(),
            json!({
                "id":"call","name":"write_file","arguments":{"path":"src/main.rs","contents":"fn main() { println!(\"hello — 世界\"); }","count":1234567}
            })
        );
        for incomplete in ["", "{", r#"{"x":"unterminated"#] {
            call.arguments = json!(incomplete);
            finalize_tool_arguments(&mut call);
            assert_eq!(call.arguments, json!({}), "{incomplete}");
        }
    }

    // These tests verify that empty text blocks are filtered out to avoid
    // Anthropic API error: "text content blocks must be non-empty"

    #[test]
    fn content_conversion_preserves_text_and_filters_only_empty_blocks() {
        for text in ["", "Hello, world!", "   ", "\n\t", "héllo 世界"] {
            let expected = if text.is_empty() {
                json!([])
            } else {
                json!([{"type":"text","text":text}])
            };
            for content in [
                LlmMessageContent::Text(text.into()),
                LlmMessageContent::Parts(vec![
                    LlmContentPart::Text {
                        text: String::new(),
                    },
                    LlmContentPart::Text { text: text.into() },
                    LlmContentPart::Text {
                        text: String::new(),
                    },
                ]),
            ] {
                assert_eq!(
                    serde_json::to_value(AnthropicChatDriver::convert_content(&content)).unwrap(),
                    expected,
                    "content={content:?}"
                );
            }
        }
        assert_eq!(
            serde_json::to_value(AnthropicChatDriver::convert_content(
                &LlmMessageContent::Parts(vec![])
            ))
            .unwrap(),
            json!([])
        );
    }

    #[test]
    fn content_conversion_preserves_order_and_complete_media_payloads() {
        let content = LlmMessageContent::Parts(vec![
            LlmContentPart::Text {
                text: String::new(),
            },
            LlmContentPart::Text {
                text: "caption".into(),
            },
            LlmContentPart::Image {
                url: "data:image/png;base64,iVBORw0KGgo=".into(),
            },
            LlmContentPart::Text {
                text: String::new(),
            },
            LlmContentPart::Image {
                url: "https://example.com/photo.jpg?size=large".into(),
            },
            LlmContentPart::Audio {
                url: "data:audio/wav;base64,AAAA".into(),
            },
            LlmContentPart::Text { text: "  ".into() },
        ]);
        assert_eq!(
            serde_json::to_value(AnthropicChatDriver::convert_content(&content)).unwrap(),
            json!([
                {"type":"text","text":"caption"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}},
                {"type":"image","source":{"type":"url","url":"https://example.com/photo.jpg?size=large"}},
                {"type":"text","text":"[Audio content not supported]"},
                {"type":"text","text":"  "}
            ])
        );
    }

    #[test]
    fn test_uses_adaptive_thinking_by_family() {
        // Adaptive-only / adaptive-recommended families, with and without
        // dated suffixes.
        assert!(uses_adaptive_thinking("claude-fable-5-1"));
        assert!(uses_adaptive_thinking("claude-fable-5-1-20260901"));
        assert!(uses_adaptive_thinking("claude-fable-5"));
        assert!(uses_adaptive_thinking("claude-fable-5-20260601"));
        assert!(uses_adaptive_thinking("claude-opus-5"));
        assert!(uses_adaptive_thinking("claude-opus-5-20260101"));
        assert!(uses_adaptive_thinking("claude-opus-4-8"));
        assert!(uses_adaptive_thinking("claude-opus-4-7-20260416"));
        assert!(uses_adaptive_thinking("claude-opus-4-6"));
        assert!(uses_adaptive_thinking("claude-sonnet-5"));
        assert!(uses_adaptive_thinking("claude-sonnet-4-6"));
        // Budget-based families stay on extended thinking.
        assert!(!uses_adaptive_thinking("claude-opus-4-5"));
        assert!(!uses_adaptive_thinking("claude-sonnet-4-5"));
        assert!(!uses_adaptive_thinking("claude-haiku-4-5-20251001"));
    }

    #[test]
    fn test_thinking_config_serialization() {
        // Adaptive must not carry budget_tokens (400 on Fable 5.x / Opus 4.8 /
        // 4.7); display:"summarized" opts back into visible thinking text,
        // which those models omit by default.
        let adaptive = serde_json::to_value(AnthropicThinking::adaptive()).unwrap();
        assert_eq!(
            adaptive,
            json!({"type": "adaptive", "display": "summarized"})
        );

        let enabled = serde_json::to_value(AnthropicThinking::Enabled {
            budget_tokens: 4096,
        })
        .unwrap();
        assert_eq!(enabled, json!({"type": "enabled", "budget_tokens": 4096}));
    }

    #[test]
    fn test_convert_tools_strips_top_level_composition_keywords() {
        let make_tool = |name: &str, parameters: Value| {
            ToolDefinition::Builtin(BuiltinTool {
                name: name.to_string(),
                display_name: None,
                description: "test tool".to_string(),
                parameters,
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: ToolHints::default(),
                full_parameters: None,
            })
        };
        let tools = vec![
            make_tool(
                "top_level_one_of",
                json!({
                    "type": "object",
                    "properties": {
                        "target": {
                            "type": "object",
                            // Nested composition is accepted by Anthropic and
                            // must survive.
                            "oneOf": [{"required": ["id"]}]
                        }
                    },
                    "required": ["target"],
                    "oneOf": [{"required": ["name"]}],
                    "anyOf": [{"required": ["name"]}],
                    "allOf": [{"required": ["name"]}]
                }),
            ),
            // Degenerate caller-supplied schema: nothing but composition.
            make_tool("bare_any_of", json!({"anyOf": [{"type": "object"}]})),
        ];

        let converted = AnthropicChatDriver::convert_tools(&tools, false);
        let json = serde_json::to_value(&converted).unwrap();

        let schema = &json[0]["input_schema"];
        assert!(schema.get("oneOf").is_none());
        assert!(schema.get("anyOf").is_none());
        assert!(schema.get("allOf").is_none());
        assert_eq!(schema["required"], json!(["target"]));
        assert_eq!(
            schema["properties"]["target"]["oneOf"],
            json!([{"required": ["id"]}])
        );

        let bare = &json[1]["input_schema"];
        assert!(bare.get("anyOf").is_none());
        assert_eq!(bare["type"], "object");
    }

    #[test]
    fn test_tool_result_with_images_conversion() {
        // Tool result with text + image content
        let msg = LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Parts(vec![
                LlmContentPart::Text {
                    text: "{\"status\": \"ok\"}".to_string(),
                },
                LlmContentPart::Image {
                    url: "data:image/png;base64,AAAA".to_string(),
                },
            ]),
            tool_calls: None,
            tool_call_id: Some("call_img".to_string()),
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
        };

        let assistant = LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text(String::new()),
            tool_calls: Some(vec![ToolCall {
                id: "call_img".to_string(),
                name: "capture".to_string(),
                arguments: json!({}),
            }]),
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
        };
        let (_, converted) = AnthropicChatDriver::convert_messages(&[assistant, msg], false, 0);

        assert_eq!(converted.len(), 2);
        assert_eq!(converted[1].role, "user");
        assert_eq!(converted[1].content.len(), 1);

        match &converted[1].content[0] {
            AnthropicContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "call_img");
                match content {
                    AnthropicToolResultContent::Blocks(blocks) => {
                        assert_eq!(blocks.len(), 2);
                        match &blocks[0] {
                            AnthropicToolResultBlock::Text { text } => {
                                assert_eq!(text, "{\"status\": \"ok\"}");
                            }
                            _ => panic!("Expected text block"),
                        }
                        match &blocks[1] {
                            AnthropicToolResultBlock::Image { source } => match source {
                                AnthropicImageSource::Base64 { media_type, data } => {
                                    assert_eq!(media_type, "image/png");
                                    assert_eq!(data, "AAAA");
                                }
                                _ => panic!("Expected base64 image source"),
                            },
                            _ => panic!("Expected image block"),
                        }
                    }
                    _ => panic!("Expected Blocks content for multimodal tool result"),
                }
            }
            _ => panic!("Expected ToolResult block"),
        }
    }

    // ========================================================================
    // HTTP error classification
    // ========================================================================

    #[tokio::test]
    async fn http_errors_preserve_semantic_classification_without_retrying() {
        use everruns_provider::{Provider, StaticHeaderAuth};
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let config = LlmCallConfig {
            model: "claude-test".into(),
            temperature: None,
            max_tokens: Some(32),
            tools: vec![],
            reasoning_effort: None,
            reasoning_state: None,
            metadata: Default::default(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: vec![],
            cache_diagnostics: None,
            speed: None,
            verbosity: None,
        };
        for (status, message, category) in [
            (413, "Request too large", "size"),
            (
                400,
                "prompt is too long: 250000 tokens > 200000 maximum",
                "size",
            ),
            (400, "request size exceeded maximum", "size"),
            (400, "too many tokens in request", "size"),
            (401, "Invalid API key", "auth"),
            (429, "Rate limit exceeded", "rate"),
            (500, "Internal server error", "unavailable"),
            (404, "not_found_error: model: claude-test", "model"),
            (404, "Model not found", "model"),
            (404, "Endpoint not found", "invalid"),
            (400, "not_found_error: model: claude-test", "invalid"),
            (500, "prompt is too long", "unavailable"),
        ] {
            let server = MockServer::builder().start().await;
            let body = json!({"error":{"message":message}});
            Mock::given(method("POST")).and(path("/v1/messages"))
                .and(header("x-api-key","synthetic-key"))
                .and(header("anthropic-version","2023-06-01"))
                .and(body_json(json!({"model":"claude-test","max_tokens":32,"stream":true,"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]})))
                .respond_with(ResponseTemplate::new(status).set_body_json(body.clone())).expect(1).mount(&server).await;
            let provider = Provider::new(
                "test",
                AnthropicChatDriver::new().with_retry_config(LlmRetryConfig::no_retry()),
            )
            .base_url(format!("{}/v1", server.uri()))
            .auth(StaticHeaderAuth::new("x-api-key", "synthetic-key"));
            let error = match provider
                .chat_completion_stream(
                    vec![LlmMessage::text(LlmMessageRole::User, "hello")],
                    &config,
                )
                .await
            {
                Ok(_) => panic!("expected HTTP error for {status}: {message}"),
                Err(error) => error,
            };
            match category {
                "size" => assert!(error.is_request_too_large(), "{error:?}"),
                "model" => assert_eq!(error.model_not_available_id(), Some("claude-test")),
                kind => {
                    let expected = match kind {
                        "auth" => LlmErrorKind::Authentication,
                        "rate" => LlmErrorKind::RateLimited,
                        "unavailable" => LlmErrorKind::Unavailable,
                        _ => LlmErrorKind::InvalidRequest,
                    };
                    assert_eq!(
                        error.llm_error_kind(),
                        Some(expected),
                        "{status}: {message}"
                    );
                    assert!(!error.is_request_too_large());
                    assert!(!error.is_model_not_available());
                }
            }
            if category != "model" {
                assert!(error.to_string().contains(message), "{error:?}");
            }
            server.verify().await;
        }
    }

    // ========================================================================
    // Discovered profile construction tests
    // ========================================================================

    #[test]
    fn test_split_million_context() {
        // Registered `[1m]` twins: stripped to the wire id and flagged.
        assert_eq!(
            split_million_context("claude-opus-4-8[1m]"),
            ("claude-opus-4-8", true)
        );
        assert_eq!(
            split_million_context("claude-fable-5-1[1m]"),
            ("claude-fable-5-1", true)
        );
        assert_eq!(
            split_million_context("claude-fable-5[1m]"),
            ("claude-fable-5", true)
        );
        assert_eq!(
            split_million_context("claude-opus-5[1m]"),
            ("claude-opus-5", true)
        );
        assert_eq!(
            split_million_context("claude-opus-4-6[1m]"),
            ("claude-opus-4-6", true)
        );
        assert_eq!(
            split_million_context("claude-sonnet-5[1m]"),
            ("claude-sonnet-5", true)
        );

        // Date-suffixed 1M-capable id is still honored (family normalization).
        assert_eq!(
            split_million_context("claude-opus-4-8-20260101[1m]"),
            ("claude-opus-4-8-20260101", true)
        );

        // Bare ids are unchanged and not flagged.
        assert_eq!(
            split_million_context("claude-opus-4-8"),
            ("claude-opus-4-8", false)
        );

        // Models that merely end in `[1m]` but are NOT 1M-capable must be left
        // untouched — never strip them or send `context-1m` (it can 400 or
        // silently truncate, e.g. on sonnet-4-5 where the header was retired).
        for not_1m in [
            "claude-haiku-4-5[1m]",
            "claude-haiku-4-5-20251001[1m]",
            "claude-sonnet-4-5[1m]",
            "totally-made-up[1m]",
        ] {
            assert_eq!(split_million_context(not_1m), (not_1m, false), "{not_1m}");
        }
    }

    #[test]
    fn discovered_profile_preserves_complete_catalog_metadata() {
        let info: AnthropicModelInfo = serde_json::from_value(json!({
            "id":"claude-sonnet-4-6-20260217", "display_name":"Claude Sonnet 4.6",
            "created_at":"2026-02-17T00:00:00Z", "max_input_tokens":200000,
            "max_tokens":64000
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(info.to_discovered_profile()).unwrap(),
            json!({
                "name":"Claude Sonnet 4.6", "family":"claude-sonnet-4-6", "release_date":"2026-02-17",
                "attachment":false, "reasoning":false, "temperature":true, "tool_call":true,
                "structured_output":false, "open_weights":false,
                "limits":{"context":200000,"output":64000},
                "modalities":{"input":["text"],"output":["text"]},
                "tool_search":false, "supports_phases":false
            })
        );
    }

    #[test]
    fn discovered_limits_do_not_wrap_and_require_both_bounds() {
        for (input, output, expected) in [
            (Some(0_u32), Some(0_u32), Some((0, 0))),
            (Some(2147483647), Some(64000), Some((2147483647, 64000))),
            (
                Some(2147483648),
                Some(u32::MAX),
                Some((2147483647, 2147483647)),
            ),
            (Some(u32::MAX), Some(1), Some((2147483647, 1))),
            (None, Some(64000), None),
            (Some(200000), None, None),
            (None, None, None),
        ] {
            let info: AnthropicModelInfo = serde_json::from_value(json!({
                "id":"claude-test", "display_name":"Test", "max_input_tokens":input, "max_tokens":output
            })).unwrap();
            let profile = info.to_discovered_profile();
            assert_eq!(
                profile.limits.map(|l| (l.context, l.output)),
                expected,
                "input={input:?} output={output:?}"
            );
        }
    }

    #[test]
    fn discovered_capabilities_and_effort_defaults_match_supported_choices() {
        for capabilities in [
            json!({"thinking":{"supported":false},"effort":{"supported":true,"high":{"supported":true}}}),
            json!({"thinking":{"supported":true},"effort":{"supported":true}}),
            json!({"thinking":{"supported":true},"effort":{"supported":true,"low":{"supported":false}}}),
        ] {
            let info: AnthropicModelInfo = serde_json::from_value(json!({
                "id":"claude-test","display_name":"Test","capabilities":capabilities
            }))
            .unwrap();
            assert!(info.to_discovered_profile().reasoning_effort.is_none());
        }
        for effort in [
            Value::Null,
            json!({"supported":false,"high":{"supported":true}}),
        ] {
            let info: AnthropicModelInfo = serde_json::from_value(json!({
                "id":"claude-test","display_name":"Test","capabilities":{
                    "thinking":{"supported":true},"effort":effort
                }
            }))
            .unwrap();
            assert_eq!(
                serde_json::to_value(info.to_discovered_profile().reasoning_effort).unwrap(),
                json!({
                    "values":[
                        {"value":"low","name":"Low (1K tokens)"},
                        {"value":"medium","name":"Medium (4K tokens)"},
                        {"value":"high","name":"High (16K tokens)"},
                        {"value":"xhigh","name":"Extra High (32K tokens)"}
                    ],"default":"medium"
                })
            );
        }
        for (image, pdf, expected) in [
            (false, false, json!(["text"])),
            (true, false, json!(["text", "image"])),
            (false, true, json!(["text", "pdf"])),
            (true, true, json!(["text", "image", "pdf"])),
        ] {
            let info: AnthropicModelInfo = serde_json::from_value(json!({
                "id":"claude-test", "display_name":"Test", "capabilities":{
                    "image_input":{"supported":image},"pdf_input":{"supported":pdf},
                    "structured_outputs":{"supported":true}
                }
            }))
            .unwrap();
            let profile = info.to_discovered_profile();
            assert_eq!(profile.attachment, image || pdf);
            assert!(profile.structured_output);
            assert_eq!(
                serde_json::to_value(profile.modalities).unwrap(),
                json!({"input":expected,"output":["text"]})
            );
            assert!(!profile.reasoning);
            assert!(profile.reasoning_effort.is_none());
        }
        for adaptive in [false, true] {
            for (levels, default) in [
                (vec!["low"], "low"),
                (vec!["medium"], "medium"),
                (vec!["high"], "high"),
                (vec!["max"], "xhigh"),
                (
                    vec!["low", "medium", "high", "max"],
                    if adaptive { "high" } else { "medium" },
                ),
            ] {
                let mut effort = json!({"supported":true});
                for level in &levels {
                    effort[*level] = json!({"supported":true});
                }
                let info: AnthropicModelInfo = serde_json::from_value(json!({
                    "id":"claude-test", "display_name":"Test", "capabilities":{
                        "thinking":{"supported":true,"types":{"adaptive":{"supported":adaptive}}},
                        "effort":effort
                    }
                }))
                .unwrap();
                let profile = info.to_discovered_profile();
                assert!(profile.reasoning);
                let expected:Vec<Value>=levels.iter().map(|level|json!({
                    "value":if *level=="max" {"xhigh"} else {level},
                    "name":match (*level,adaptive) {
                        ("low",true)=>"Low",("medium",true)=>"Medium",("high",true)=>"High",("max",true)=>"Max",
                        ("low",false)=>"Low (1K tokens)",("medium",false)=>"Medium (4K tokens)",("high",false)=>"High (16K tokens)",_=>"Extra High (32K tokens)"
                    }
                })).collect();
                assert_eq!(
                    serde_json::to_value(profile.reasoning_effort).unwrap(),
                    json!({"values":expected,"default":default}),
                    "adaptive={adaptive} levels={levels:?}"
                );
            }
        }
    }
}
