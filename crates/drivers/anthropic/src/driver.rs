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

use reqwest::Client;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::server_compaction;
use everruns_provider::RejectedProviderCapability;
use everruns_provider::credential_schema::CredentialFormSchema;
use everruns_provider::driver_helpers::{
    self, ANTHROPIC_NOT_FOUND_PATTERNS, ANTHROPIC_TOO_LARGE_PATTERNS, AUDIO_CONTENT_PLACEHOLDER,
    parse_data_url,
};
use everruns_provider::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmCompletionMetadata, LlmContentPart, LlmResponseStream, LlmStreamEvent, Message,
    MessageContent, MessageRole, ProviderCheckpointCandidate, fold_system_messages,
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
use everruns_provider::{ProviderOpaqueContent, ProviderOpaqueContext};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Beta flag that turns on Anthropic's prompt-cache diagnostics: the API
/// fingerprints each request and, on the next one, reports where the prompt
/// prefix diverged instead of leaving a silent cache miss.
const CACHE_DIAGNOSTICS_BETA: &str = "cache-diagnosis-2026-04-07";
const SERVER_COMPACTION_BETA: &str = "compact-2026-01-12";
const SERVER_COMPACTION_OPTION: &str = "anthropic/server_compaction";
const SERVER_COMPACTION_MIN_TOKENS: usize = 50_000;
const SERVER_COMPACTION_CHECKPOINT_FORMAT: u32 = 2;

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

#[derive(Debug, Serialize)]
struct AnthropicContextManagement {
    edits: Vec<AnthropicContextEdit>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum AnthropicContextEdit {
    #[serde(rename = "compact_20260112")]
    Compact {
        trigger: AnthropicCompactionTrigger,
        pause_after_compaction: bool,
    },
}

#[derive(Debug, Serialize)]
struct AnthropicCompactionTrigger {
    r#type: &'static str,
    value: usize,
}

fn append_nullable_raw_block_field(
    blocks: &Mutex<BTreeMap<u32, Value>>,
    index: u32,
    field: &str,
    fragment: &str,
) {
    let mut blocks = blocks.lock().unwrap();
    let Some(Value::Object(block)) = blocks.get_mut(&index) else {
        return;
    };
    let value = block
        .entry(field.to_string())
        .or_insert_with(|| Value::String(String::new()));
    if value.is_null() {
        *value = Value::String(String::new());
    }
    if let Value::String(value) = value {
        value.push_str(fragment);
    }
}

fn append_raw_block_field(
    blocks: &Mutex<BTreeMap<u32, Value>>,
    index: u32,
    field: &str,
    fragment: &str,
) {
    let mut blocks = blocks.lock().unwrap();
    let Some(Value::Object(block)) = blocks.get_mut(&index) else {
        return;
    };
    let value = block
        .entry(field.to_string())
        .or_insert_with(|| Value::String(String::new()));
    if let Value::String(value) = value {
        value.push_str(fragment);
    }
}

fn set_raw_block_field(
    blocks: &Mutex<BTreeMap<u32, Value>>,
    index: u32,
    field: &str,
    value: Value,
) {
    let mut blocks = blocks.lock().unwrap();
    if let Some(Value::Object(block)) = blocks.get_mut(&index) {
        block.insert(field.to_string(), value);
    }
}

fn anthropic_checkpoint_candidate(
    enabled: bool,
    request_messages: &[Value],
    response_content: &[Value],
) -> Result<Option<ProviderCheckpointCandidate>> {
    if !enabled {
        return Ok(None);
    }
    let mut observed_compaction = false;
    for block in response_content {
        if block.get("type").and_then(Value::as_str) != Some("compaction") {
            continue;
        }
        observed_compaction = true;
        let complete = block
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| !content.is_empty())
            && block
                .get("encrypted_content")
                .and_then(Value::as_str)
                .is_some_and(|content| !content.is_empty());
        if !complete {
            return Err(AgentLoopError::llm(
                "Anthropic completed a response with an incomplete compaction block",
            ));
        }
    }
    if !observed_compaction {
        return Ok(None);
    }
    let mut messages = request_messages.to_vec();
    messages.push(json!({
        "role": "assistant",
        "content": response_content,
    }));
    let messages_json = serde_json::to_string(&messages).map_err(|error| {
        AgentLoopError::llm(format!(
            "failed to serialize Anthropic messages-prefix checkpoint: {error}"
        ))
    })?;
    Ok(Some(ProviderCheckpointCandidate {
        format_version: SERVER_COMPACTION_CHECKPOINT_FORMAT,
        context: ProviderOpaqueContext::AnthropicMessagesPrefix { messages_json },
    }))
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
    wants_clear_at: bool,
    wants_server_compaction: bool,
    max_tokens_from_profile: bool,
    model: &'a str,
    /// Caller-supplied per-request headers, applied over everything the driver
    /// and the provider endpoint resolve.
    extra_headers: &'a [(String, String)],
}

impl AnthropicChatDriver {
    /// Create a new provider with the given API key
    pub fn new() -> Self {
        // EVE-924: choose the rustls backend on the startup path. The shared
        // client installs it as well, but that now happens on the first
        // request, and products expect the process-wide choice to be settled
        // while providers are being constructed.
        everruns_provider::install_default_crypto_provider();
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
            wants_clear_at,
            wants_server_compaction,
            max_tokens_from_profile,
            model,
            extra_headers,
        } = options;
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let max_tokens_fallback_attempted = Arc::new(Mutex::new(false));
        let beta_logged = Arc::new(Mutex::new(false));
        let binds_thinking = layout::binds_thinking_to_conversation(model);
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
                    if binds_thinking {
                        beta_features.push(layout::THINKING_BINDING_BETA);
                    }
                    if wants_clear_at {
                        beta_features.push("mid-conversation-system-clear-at-2026-08-21");
                    }
                    if wants_server_compaction {
                        beta_features.push(SERVER_COMPACTION_BETA);
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

                    if wants_server_compaction
                        && server_compaction::is_rejection_response(status, &error_text)
                        && server_compaction::record_rejection(endpoint, &model)
                    {
                        tracing::warn!(
                            model = %model,
                            status = status.as_u16(),
                            "AnthropicDriver: server compaction rejected; caching legacy fallback"
                        );
                        return RetryDecision::Terminal(
                            AgentLoopError::provider_capability_rejected(
                                RejectedProviderCapability::AnthropicServerCompaction,
                                status.as_u16(),
                                &error_text,
                                error_msg,
                            ),
                        );
                    }

                    // Classified, and its status preserved, while the HTTP
                    // response is still structured (see LlmError).
                    let message = if attempts > 0 {
                        format!(
                            "{} (after {} retries, last error: {})",
                            error_msg,
                            attempts,
                            last_error.lock().unwrap().take().unwrap_or_default()
                        )
                    } else {
                        error_msg
                    };
                    RetryDecision::Terminal(AgentLoopError::llm_http(
                        status.as_u16(),
                        &error_text,
                        message,
                    ))
                }
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await
    }

    fn convert_role(role: &MessageRole) -> &'static str {
        match role {
            MessageRole::System => "user", // System is handled separately in Anthropic
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "user", // Tool results are sent as user messages
        }
    }

    fn convert_content(content: &MessageContent) -> Vec<AnthropicContentBlock> {
        match content {
            MessageContent::Text(text) => {
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
            MessageContent::Parts(parts) => parts
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
                    LlmContentPart::File { url, .. } => {
                        if let Some(parsed) = parse_data_url(url) {
                            Some(AnthropicContentBlock::Document {
                                source: AnthropicDocumentSource::Base64 {
                                    media_type: parsed.media_type,
                                    data: parsed.data,
                                },
                            })
                        } else if url.starts_with("data:") {
                            // Malformed data URL — fall back to application/pdf
                            Some(AnthropicContentBlock::Document {
                                source: AnthropicDocumentSource::Base64 {
                                    media_type: "application/pdf".to_string(),
                                    data: url.clone(),
                                },
                            })
                        } else {
                            // File URL
                            Some(AnthropicContentBlock::Document {
                                source: AnthropicDocumentSource::Url { url: url.clone() },
                            })
                        }
                    }
                    // `LlmContentPart` is `#[non_exhaustive]`: a part this build
                    // does not know is dropped rather than failing the request.
                    _ => None,
                })
                .collect(),
        }
    }

    fn preserved_content(content: &MessageContent) -> Option<Value> {
        let MessageContent::Parts(parts) = content else {
            return None;
        };
        parts.iter().find_map(|part| {
            let LlmContentPart::ProviderOpaque(opaque) = part else {
                return None;
            };
            if opaque.provider != "anthropic" {
                return None;
            }
            match &opaque.content {
                Value::Array(_) => Some(opaque.content.clone()),
                _ => {
                    tracing::warn!("AnthropicDriver: preserved assistant content is not an array");
                    None
                }
            }
        })
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

    #[cfg(test)]
    fn convert_messages(
        messages: &[Message],
        prompt_cache_enabled: bool,
        volatile_suffix_len: usize,
    ) -> (Option<String>, Vec<AnthropicMessage>) {
        Self::convert_messages_with_options(
            messages,
            prompt_cache_enabled,
            volatile_suffix_len,
            false,
        )
    }

    fn convert_messages_with_options(
        messages: &[Message],
        prompt_cache_enabled: bool,
        volatile_suffix_len: usize,
        allow_unmatched_tool_results: bool,
    ) -> (Option<String>, Vec<AnthropicMessage>) {
        // Accumulate all system messages into Anthropic's separate top-level
        // `system` field. Overwriting on each System message would drop the agent
        // system prompt whenever a later notice/summary System message is present
        // (infinity_context / compaction). See `fold_system_messages`.
        let mut system_prompt = fold_system_messages(messages);
        let mut converted = Vec::new();
        let visible_tool_use_ids = visible_tool_call_ids(messages);

        for msg in messages {
            match msg.role {
                MessageRole::System => {
                    // Folded above into the top-level `system` field; never emit a
                    // System-role entry into the Anthropic `messages` array.
                }
                MessageRole::Tool => {
                    // Tool results in Anthropic are user messages with tool_result content blocks.
                    // When the message contains images, we use the array form with content blocks.
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        // Anthropic rejects tool_result blocks unless the matching tool_use
                        // is present in the visible request after context trimming.
                        if !allow_unmatched_tool_results
                            && !visible_tool_use_ids.contains(tool_call_id.as_str())
                        {
                            continue;
                        }

                        let has_images = match &msg.content {
                            MessageContent::Parts(parts) => parts
                                .iter()
                                .any(|p| matches!(p, LlmContentPart::Image { .. })),
                            _ => false,
                        };

                        let content = if has_images {
                            // Build array of text + image content blocks
                            let blocks = match &msg.content {
                                MessageContent::Parts(parts) => parts
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
                                MessageContent::Text(text) => {
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
                            clear_at: None,
                            preserved_content: None,
                        });
                    }
                }
                MessageRole::Assistant => {
                    if let Some(preserved_content) = Self::preserved_content(&msg.content) {
                        converted.push(AnthropicMessage {
                            role: Self::convert_role(&msg.role).to_string(),
                            content: Vec::new(),
                            clear_at: None,
                            preserved_content: Some(preserved_content),
                        });
                        continue;
                    }
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
                        clear_at: None,
                        preserved_content: None,
                    });
                }
                _ => {
                    converted.push(AnthropicMessage {
                        role: Self::convert_role(&msg.role).to_string(),
                        content: Self::convert_content(&msg.content),
                        clear_at: None,
                        preserved_content: None,
                    });
                }
            }
        }

        layout::place_system_messages(&mut system_prompt, &mut converted);
        if prompt_cache_enabled {
            layout::mark_recent_text_blocks_for_cache(&mut converted, volatile_suffix_len);
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

mod chat_driver;

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
/// This driver's descriptor: identity, services, and the credential schema
/// that declares its own environment variables.
pub fn descriptor() -> DriverDescriptor {
    DriverDescriptor {
        display_name: "Anthropic".into(),
        // Matches the anthropic SDK's own key variable.
        credential_schema: CredentialFormSchema::api_key(
            "ANTHROPIC_API_KEY",
            "Create an API key in the [Anthropic Console](https://console.anthropic.com/settings/keys).",
        ),
        // No endpoint variable, deliberately. Anthropic's ANTHROPIC_BASE_URL is
        // the host root (its SDK appends `/v1/messages`), while a Provider
        // base_url here is the versioned API root that drivers append bare
        // paths to — DEFAULT_BASE_URL ends in `/v1`. Importing the vendor's
        // value verbatim yields `https://api.anthropic.com/messages` and a 404.
        // Point at a proxy with an explicit `.base_url(...)` instead.
        base_url_env: None,
        ..DriverDescriptor::chat_only(DriverId::Anthropic, |config| {
            let provider = everruns_provider::Provider::new(
                config.provider.clone(),
                AnthropicChatDriver::new(),
            )
            .base_url(config.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL))
            .auth(everruns_provider::StaticHeaderAuth::new(
                "x-api-key",
                config.api_key.as_deref().unwrap_or(""),
            ));
            provider.into_boxed_driver()
        })
    }
}

/// Register the driver with a [`DriverRegistry`].
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(descriptor());
}

/// Build a provider from this driver's declared environment variables.
///
/// Standalone/CLI/dev only: server paths resolve credentials from storage and
/// must never read the environment.
pub fn from_env(
    id: impl Into<everruns_provider::ProviderKey>,
) -> std::result::Result<
    everruns_provider::Provider,
    everruns_provider::credential_provider::EnvCredentialError,
> {
    everruns_provider::credential_provider::provider_from_env(&descriptor(), id)
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

fn visible_tool_call_ids(messages: &[Message]) -> HashSet<&str> {
    messages
        .iter()
        .filter(|msg| msg.role == MessageRole::Assistant)
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
    messages: Vec<Value>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    context_management: Option<AnthropicContextManagement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<AnthropicCacheControl>,
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
/// Fable 5.x and Opus 5.5/5/4.8/4.7 (where `budget_tokens` returns 400) and is the
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
        /// Fable 5.x and Opus 5.5/5/4.8/4.7 omit thinking text by default
        /// (`display: "omitted"`); "summarized" restores it so assistant
        /// messages keep their thinking content like on budget-based models.
        display: &'static str,
        /// What the API does with a replayed block whose conversation prefix
        /// changed; set on preserved-thinking models (see `layout`).
        #[serde(skip_serializing_if = "Option::is_none")]
        block_binding: Option<AnthropicBlockBinding>,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
struct AnthropicBlockBinding {
    prefix_mismatch_behavior: &'static str,
}

impl AnthropicThinking {
    /// Create a budget-based thinking config from a reasoning effort level
    fn enabled_from_effort(effort: ReasoningEffort) -> Option<Self> {
        driver_helpers::thinking_budget::from_effort(effort)
            .map(|budget_tokens| Self::Enabled { budget_tokens })
    }

    /// Adaptive thinking, exposing its summary only when the caller opted in.
    fn adaptive(model: &str, summary: bool) -> Self {
        Self::Adaptive {
            display: if summary { "summarized" } else { "omitted" },
            block_binding: layout::binds_thinking_to_conversation(model).then_some(
                AnthropicBlockBinding {
                    prefix_mismatch_behavior: "drop_block",
                },
            ),
        }
    }
}

/// `output_config` request field — carries the effort level that controls
/// adaptive thinking depth.
#[derive(Debug, Serialize)]
struct AnthropicOutputConfig {
    effort: String,
}

/// Claude families that use adaptive thinking. On Fable 5.x, Opus 5.5/5/4.8/4.7,
/// and Sonnet 5 budget-based thinking is removed (400); on Opus 4.6 / Sonnet 4.6
/// it is deprecated and adaptive is the recommended form. Keep in sync with the
/// adaptive-thinking profiles in `everruns_provider::model_profiles`.
///
/// `claude-fable-5-1` is listed on its own: `normalize_anthropic_id` only
/// strips 8-digit date suffixes, so the `-1` does not collapse to Fable 5.
const ADAPTIVE_THINKING_FAMILIES: &[&str] = &[
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5-5",
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
    "claude-opus-5-5",
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
pub(crate) fn split_million_context(model_id: &str) -> (&str, bool) {
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
pub(crate) fn uses_adaptive_thinking(model_id: &str) -> bool {
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

#[derive(Debug)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
    clear_at: Option<String>,
    // Keep provider-returned blocks raw so future fields and block types survive replay.
    preserved_content: Option<Value>,
}

impl Serialize for AnthropicMessage {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut message = serializer.serialize_struct("AnthropicMessage", 3)?;
        message.serialize_field("role", &self.role)?;
        match &self.preserved_content {
            Some(content) => message.serialize_field("content", content)?,
            None => message.serialize_field("content", &self.content)?,
        }
        if let Some(clear_at) = &self.clear_at {
            message.serialize_field("clear_at", clear_at)?;
        }
        message.end()
    }
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
    #[serde(rename = "document")]
    Document { source: AnthropicDocumentSource },
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
    #[serde(rename = "server_tool_use")]
    ServerToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    #[serde(rename = "tool_search_tool_result")]
    ToolSearchToolResult {
        tool_use_id: String,
        content: Value,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicDocumentSource {
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
    #[serde(rename = "text")]
    Text { media_type: String, data: String },
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
    #[serde(default)]
    input_transformations: Vec<serde_json::Value>,
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
    index: u32,
    content_block: Value,
}

/// Completed content block from content_block_stop event
/// Includes the cryptographic signature for thinking blocks
#[derive(Debug, Deserialize)]
struct AnthropicContentBlockStop {
    index: u32,
    #[serde(default)]
    content_block: Option<Value>,
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
    #[serde(rename = "compaction")]
    Compaction,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockDeltaEvent {
    index: u32,
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
    #[serde(rename = "compaction_delta")]
    CompactionDelta {
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        encrypted_content: Option<String>,
    },
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
pub(crate) fn normalize_anthropic_id(model_id: &str) -> &str {
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
            // Discovery cannot prove the direct-endpoint and family contract.
            // Curated direct Anthropic profiles opt in explicitly.
            supports_server_compaction: false,
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

#[path = "driver_layout.rs"]
mod layout;

#[cfg(test)]
#[path = "driver_family_tests.rs"]
mod family_tests;

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
