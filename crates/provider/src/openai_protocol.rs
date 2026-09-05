// OpenAI Protocol Chat Driver
//
// Base implementation of the OpenAI chat completion protocol.
// This driver can be used with any OpenAI-compatible API endpoint.
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff, respecting x-ratelimit-reset-* and retry-after headers.
// Retry metadata is included in the response for observability.
//
// This is the base protocol implementation used in examples.
// For production use with OpenAI-specific features, use OpenAIChatDriver from everruns-openai.
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// llm.generation events are emitted by ReasonAtom, and OtelEventListener
// creates the appropriate gen-ai spans. No direct tracing in drivers.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};

use crate::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmContentPart, LlmMessage,
    LlmMessageContent, LlmMessageRole, LlmResponseStream, LlmStreamEvent, disjoint_prompt_tokens,
};
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::llm_retry::{
    LlmRetryConfig, RateLimitInfo, RetryDecision, RetryMetadata, SendOutcome, is_rate_limit_status,
    retry_request, send_error_message,
};
use crate::runtime_provider::ProviderEndpoint;
use crate::stream_accumulator::StreamToolCallAccumulator;
use crate::stream_reconnect::connect_sse_with_reconnect;
use crate::tool_types::ToolDefinition;
use crate::user_facing_error::is_provider_quota_message;

pub fn is_azure_openai_api_url(api_url: &str) -> bool {
    Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| {
            host.ends_with(".openai.azure.com") || host.ends_with(".services.ai.azure.com")
        })
}

/// Whether `api_url` points at OpenAI's hosted API (`api.openai.com`).
///
/// Host-based (not prefix-based) so it tolerates ports and trailing paths.
pub fn is_openai_api_url(api_url: &str) -> bool {
    Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| host == "api.openai.com")
}

// ============================================================================
// Model-discovery helpers (shared by OpenAI-compatible provider crates)
// ============================================================================
//
// These are used by both `everruns-openai` and `everruns-openrouter` to derive
// a `/models` URL, normalize a base URL, authenticate the discovery request, and
// map a non-success status into an error. They live in core so the provider
// crates can reuse them without duplicating logic.

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

/// Whether `api_url`'s host equals `host` (case-insensitive), ignoring path/port.
pub fn url_host_eq(api_url: &str, host: &str) -> bool {
    Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|h| h.eq_ignore_ascii_case(host))
}

/// Normalize a base URL to a canonical endpoint URL, appending `endpoint_suffix`
/// (e.g. `/responses`) unless it is already present.
pub fn normalize_api_url(base_url: &str, endpoint_suffix: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with(endpoint_suffix) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{endpoint_suffix}")
    }
}

/// Derive the `/models` discovery URL from a chat/responses API URL.
pub fn models_url_for_api_url(api_url: &str) -> String {
    let trimmed = api_url.trim_end_matches('/');

    if let Some(prefix) = trimmed.strip_suffix("/responses") {
        return format!("{prefix}/models");
    }
    if let Some(prefix) = trimmed.strip_suffix("/chat/completions") {
        return format!("{prefix}/models");
    }
    if trimmed.ends_with("/models") {
        return trimmed.to_string();
    }
    if trimmed.ends_with("/v1") || trimmed.ends_with("/openai/v1") {
        return format!("{trimmed}/models");
    }

    OPENAI_MODELS_URL.to_string()
}

/// Build the error returned when the `/models` endpoint responds with a
/// non-success status.
pub fn models_api_status_error(status: reqwest::StatusCode) -> AgentLoopError {
    AgentLoopError::llm(format!("Models API returned status {status}"))
}

/// OpenAI Protocol Chat Driver
///
/// Base implementation of `ChatDriver` for OpenAI-compatible APIs.
/// Supports streaming responses and tool calls.
///
/// Rate limit handling: On 429 errors, automatically retries with exponential
/// backoff, respecting `x-ratelimit-reset-*` and `retry-after` headers.
///
/// This is the base protocol driver used in examples and for OpenAI-compatible endpoints.
/// For production use with OpenAI, consider using `OpenAIChatDriver` from the `everruns-openai` crate.
///
/// # Example
///
/// ```ignore
/// use everruns_provider::OpenAIProtocolChatDriver;
///
/// let driver = OpenAIProtocolChatDriver::new();
/// // Endpoint and authentication are configured on a runtime Provider.
/// // Retry policy remains a wire-protocol concern.
/// let driver = OpenAIProtocolChatDriver::new()
///     .with_retry_config(LlmRetryConfig::aggressive());
/// ```
#[derive(Clone)]
pub struct OpenAIProtocolChatDriver {
    client: Client,
    /// Retry configuration for rate limit errors
    retry_config: LlmRetryConfig,
}

impl OpenAIProtocolChatDriver {
    /// Create a wire-only OpenAI Chat Completions protocol driver.
    pub fn new() -> Self {
        Self {
            client: crate::driver_helpers::shared_streaming_http_client(),
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// Configure retry behavior for rate limit errors
    pub fn with_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Get the HTTP client (for subclass access)
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Send one streaming chat-completion request, applying the shared
    /// header-phase retry loop (transient send failures, 429, and 5xx), and
    /// return the raw response plus its retry metadata.
    ///
    /// Invoked once per reconnect attempt by [`connect_sse_with_reconnect`]. It
    /// re-sends the identical request and consumes no body bytes, so retrying it
    /// is idempotent. The classifier preserves OpenAI's terminal classification
    /// and error messages exactly.
    async fn send_chat_completion_request(
        &self,
        endpoint: &ProviderEndpoint,
        api_url: &str,
        request: &OpenAiRequest,
        model: &str,
        extra_headers: &[(String, String)],
        retries_consumed: u32,
    ) -> Result<(reqwest::Response, RetryMetadata)> {
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let mut retry_config = self.retry_config.clone();
        retry_config.max_retries = retry_config.max_retries.saturating_sub(retries_consumed);

        let body = serde_json::to_vec(request)
            .map_err(|e| AgentLoopError::llm(format!("failed to serialize request: {e}")))?;
        retry_request(
            &retry_config,
            "OpenAIProtocolDriver",
            || async {
                let resolved = endpoint
                    .resolve("POST", api_url, &body)
                    .await
                    .map_err(SendOutcome::Fatal)?;
                let mut request_builder = self.client.post(&resolved.url);
                let mut headers = resolved.headers;
                headers.push(("Content-Type".to_string(), "application/json".to_string()));
                for (name, value) in
                    crate::driver_helpers::merge_request_headers(headers, extra_headers)
                {
                    request_builder = request_builder.header(name, value);
                }
                request_builder
                    .body(body.clone())
                    .send()
                    .await
                    .map_err(SendOutcome::Send)
            },
            |response, attempts, can_retry| {
                let last_error = Arc::clone(&last_error);
                let model = model.to_string();
                async move {
                    let status = response.status();

                    if can_retry {
                        // Parse rate limit info from headers before consuming body.
                        let rate_limit_info = if is_rate_limit_status(status) {
                            Some(RateLimitInfo::from_openai_headers(response.headers()))
                        } else {
                            None
                        };

                        let error_text = response.text().await.unwrap_or_default();

                        // Don't retry a request-too-large error (not transient).
                        if is_openai_request_too_large(status, &error_text) {
                            return RetryDecision::Terminal(AgentLoopError::request_too_large(
                                format!("OpenAI API error ({}): {}", status, error_text),
                            ));
                        }

                        // Exhausted billing quota is surfaced as a 429 but is not
                        // transient — fail fast instead of burning retries.
                        if is_provider_quota_message(&error_text) {
                            return RetryDecision::Terminal(AgentLoopError::llm_kind(
                                LlmErrorKind::QuotaExhausted,
                                format!("OpenAI API error ({}): {}", status, error_text),
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
                    let error_msg = format!("OpenAI API error ({}): {}", status, error_text);

                    // Check if this is a model-not-found error
                    if is_openai_model_not_found(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::model_not_available(model));
                    }

                    // Check if this is a request-too-large error
                    if is_openai_request_too_large(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::request_too_large(
                            error_msg,
                        ));
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
            LlmMessageRole::System => "system",
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        }
    }

    fn convert_message(msg: &LlmMessage) -> OpenAiMessage {
        let content = match &msg.content {
            LlmMessageContent::Text(text) => OpenAiContent::Text(text.clone()),
            LlmMessageContent::Parts(parts) => {
                let openai_parts: Vec<OpenAiContentPart> = parts
                    .iter()
                    .map(|part| match part {
                        LlmContentPart::Text { text } => OpenAiContentPart::Text {
                            r#type: "text".to_string(),
                            text: text.clone(),
                        },
                        LlmContentPart::Image { url } => OpenAiContentPart::ImageUrl {
                            r#type: "image_url".to_string(),
                            image_url: OpenAiImageUrl { url: url.clone() },
                        },
                        LlmContentPart::Audio { url } => OpenAiContentPart::InputAudio {
                            r#type: "input_audio".to_string(),
                            input_audio: OpenAiInputAudio {
                                data: url.clone(),
                                format: "wav".to_string(),
                            },
                        },
                    })
                    .collect();
                OpenAiContent::Parts(openai_parts)
            }
        };

        // OpenAI only accepts tool_calls on assistant messages
        let tool_calls = if msg.role == LlmMessageRole::Assistant {
            msg.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .map(|tc| OpenAiToolCall {
                        id: tc.id.clone(),
                        r#type: "function".to_string(),
                        function: OpenAiFunctionCall {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        },
                    })
                    .collect()
            })
        } else {
            None
        };

        OpenAiMessage {
            role: Self::convert_role(&msg.role).to_string(),
            content: Some(content),
            tool_calls,
            tool_call_id: msg.tool_call_id.clone(),
        }
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<OpenAiTool> {
        tools
            .iter()
            .map(|tool| {
                let strict_parameters =
                    crate::tool_schema_compat::strict_openai_tool_schema(tool.parameters());
                let strict = strict_parameters.is_some().then_some(true);
                OpenAiTool {
                    r#type: "function".to_string(),
                    function: OpenAiFunction {
                        name: tool.name().to_string(),
                        description: tool.description().to_string(),
                        parameters: strict_parameters.unwrap_or_else(|| {
                            crate::tool_schema_compat::sanitize_openai_tool_schema(
                                tool.parameters(),
                            )
                        }),
                        strict,
                    },
                }
            })
            .collect()
    }
}

impl Default for OpenAIProtocolChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Drop Tool-role messages whose tool_call_id has no matching assistant tool call in the
/// visible window. Chat Completions rejects payloads where a `tool`-role message references
/// a call that is absent from the conversation.
fn drop_orphaned_tool_messages(messages: &[LlmMessage]) -> Vec<LlmMessage> {
    use std::collections::HashSet;

    let visible_call_ids: HashSet<&str> = messages
        .iter()
        .filter(|m| m.role == LlmMessageRole::Assistant)
        .flat_map(|m| m.tool_calls.iter().flatten())
        .map(|tc| tc.id.as_str())
        .collect();

    if visible_call_ids.is_empty() {
        return messages
            .iter()
            .filter(|m| m.role != LlmMessageRole::Tool)
            .cloned()
            .collect();
    }

    messages
        .iter()
        .filter(|m| {
            if m.role == LlmMessageRole::Tool {
                return m
                    .tool_call_id
                    .as_deref()
                    .is_none_or(|id| visible_call_ids.contains(id));
            }
            true
        })
        .cloned()
        .collect()
}

#[async_trait]
impl ChatDriver for OpenAIProtocolChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Note: OTel instrumentation is handled via event listeners.
        // ReasonAtom emits llm.generation events, and OtelEventListener
        // creates gen-ai spans from those events.
        let messages = drop_orphaned_tool_messages(&messages);
        let openai_messages: Vec<OpenAiMessage> =
            messages.iter().map(Self::convert_message).collect();

        let tools = if config.tools.is_empty() {
            None
        } else {
            Some(Self::convert_tools(&config.tools))
        };

        // Build metadata for request tracking
        let metadata = if config.metadata.is_empty() {
            None
        } else {
            Some(config.metadata.clone())
        };

        let request = OpenAiRequest {
            model: config.model.clone(),
            messages: openai_messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            stream: true,
            stream_options: Some(OpenAiStreamOptions {
                include_usage: true,
            }),
            tools,
            parallel_tool_calls: config
                .resolved_parallel_tool_calls(self.supports_parallel_tool_calls(&config.model)),
            // An explicit "no reasoning" omits the field: sending it to a
            // non-thinking model is an API error.
            reasoning_effort: config
                .reasoning_effort
                .filter(crate::model::ReasoningEffort::requests_reasoning)
                .map(|effort| effort.as_str().to_string()),
            service_tier: config.speed.clone(),
            verbosity: config.verbosity.clone(),
            metadata,
        };

        // Establish the SSE stream, transparently reconnecting on a transport
        // failure that lands before the first event is decoded (the "error
        // decoding response body" flake). Header-phase retries (429/5xx and
        // transient send failures) are handled inside the per-attempt send;
        // this adds the body-phase reconnect the official SDKs get for free.
        let api_url = endpoint.url("chat/completions").ok_or_else(|| {
            AgentLoopError::Configuration(
                "OpenAI Chat Completions provider has no base URL".to_string(),
            )
        })?;
        let (event_stream, retry_metadata) =
            connect_sse_with_reconnect(&self.retry_config, "OpenAIProtocolDriver", |attempts| {
                self.send_chat_completion_request(
                    endpoint,
                    &api_url,
                    &request,
                    &config.model,
                    &config.extra_headers,
                    attempts,
                )
            })
            .await?;

        let model = config.model.clone();
        let total_tokens = Arc::new(Mutex::new(0u32));
        let prompt_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        // OpenAI-compatible gateways (e.g. OpenRouter) report an authoritative
        // per-request cost in `usage.cost`; direct OpenAI leaves it absent.
        let provider_cost_usd = Arc::new(Mutex::new(Option::<f64>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(StreamToolCallAccumulator::new()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));
        // Reasoning text accumulated across deltas. Chat Completions has no
        // per-item envelope for reasoning — it arrives as loose deltas with no
        // id, signature or terminator — so the durable artifact has to be
        // assembled here and emitted once the turn ends.
        let accumulated_reasoning = Arc::new(Mutex::new(String::new()));
        // Captured from the first streaming chunk that carries an id field.
        // OpenRouter sets this to a "gen-..." identifier on every completion.
        let response_id = Arc::new(Mutex::new(Option::<String>::None));
        // Share retry metadata with stream closure (only set if retries occurred)
        let shared_retry_metadata = if retry_metadata.had_retries() {
            Some(Arc::new(retry_metadata))
        } else {
            None
        };

        // Each SSE event maps to zero-or-more stream events (the [DONE] marker can
        // emit a flushed ToolCalls plus Done), so the closure yields a Vec that is
        // flattened back into the stream.
        let converted_stream: LlmResponseStream = Box::pin(
            event_stream
                .then(move |result| {
                    let model = model.clone();
                    let total_tokens = Arc::clone(&total_tokens);
                    let prompt_tokens = Arc::clone(&prompt_tokens);
                    let cache_read_tokens = Arc::clone(&cache_read_tokens);
                    let provider_cost_usd = Arc::clone(&provider_cost_usd);
                    let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
                    let finish_reason = Arc::clone(&finish_reason);
                    let accumulated_reasoning = Arc::clone(&accumulated_reasoning);
                    let response_id = Arc::clone(&response_id);
                    let retry_metadata_for_done = shared_retry_metadata.clone();

                    async move {
                        let event = match result {
                            Ok(event) => event,
                            Err(e) => {
                                return vec![Ok(LlmStreamEvent::Error(
                                    format!("Stream error: {}", e).into(),
                                ))];
                            }
                        };

                        if event.data == "[DONE]" {
                            let output_tokens = *total_tokens.lock().unwrap();
                            let input_tokens = *prompt_tokens.lock().unwrap();
                            let cached = *cache_read_tokens.lock().unwrap();
                            let cost = *provider_cost_usd.lock().unwrap();
                            let resp_id = response_id.lock().unwrap().clone();
                            let mut reason = finish_reason.lock().unwrap().clone();

                            let mut events = Vec::new();

                            // Defense in depth (EVE-522): flush any tool calls that
                            // were accumulated but never emitted before Done, so they
                            // are never silently dropped. The normal path drains the
                            // accumulator at the finish chunk, so this only fires as a
                            // fallback — e.g. a provider that ends the stream with
                            // [DONE] without a tool_calls finish chunk reaching the
                            // handler. When it fires, reflect the tool-call completion
                            // in the reported finish_reason.
                            {
                                let mut acc = accumulated_tool_calls.lock().unwrap();
                                if let Some(event) =
                                    take_pending_tool_calls(&mut acc, reason.as_deref())
                                {
                                    events.push(Ok(event));
                                    reason.get_or_insert_with(|| "tool_calls".to_string());
                                }
                            }

                            // The reasoning artifact is what persists and what
                            // replays; a delta alone reaches the UI and is then
                            // lost. Chat Completions exposes no replay handle
                            // (no id, no signature), so the artifact carries the
                            // text and nothing opaque.
                            {
                                let mut text = accumulated_reasoning.lock().unwrap();
                                if !text.trim().is_empty() {
                                    let part = crate::reasoning::ReasoningContentPart::opaque(
                                        "openai-protocol",
                                    )
                                    .with_text(
                                        crate::reasoning::ReasoningText::Plain {
                                            text: std::mem::take(&mut *text),
                                        },
                                    );
                                    events.push(Ok(LlmStreamEvent::ReasoningItem(part)));
                                }
                            }

                            events.push(Ok(LlmStreamEvent::Done(Box::new(
                                LlmCompletionMetadata {
                                    // `input_tokens` is OpenAI's cache-inclusive prompt count;
                                    // normalize to non-cached input for the disjoint convention.
                                    total_tokens: Some(input_tokens + output_tokens),
                                    prompt_tokens: Some(disjoint_prompt_tokens(
                                        input_tokens,
                                        cached,
                                    )),
                                    completion_tokens: Some(output_tokens),
                                    cache_read_tokens: cached,
                                    cache_creation_tokens: None,
                                    provider_cost_usd: cost,
                                    model: Some(model),
                                    finish_reason: reason.or_else(|| Some("stop".to_string())),
                                    retry_metadata: retry_metadata_for_done
                                        .map(|arc| (*arc).clone()),
                                    response_id: resp_id,
                                    phase: None,
                                    cache_diagnostics: None,
                                },
                            ))));

                            return events;
                        }

                        match serde_json::from_str::<OpenAiStreamChunk>(&event.data) {
                            Ok(chunk) => {
                                // Capture the completion ID from the first chunk that
                                // carries one. OpenRouter sets this to a "gen-..."
                                // identifier on every chunk; direct OpenAI uses
                                // "chatcmpl-..." style IDs.
                                if let Some(id) = &chunk.id {
                                    let mut rid = response_id.lock().unwrap();
                                    if rid.is_none() {
                                        *rid = Some(id.clone());
                                    }
                                }

                                // Capture usage from chunk if available
                                if let Some(usage) = &chunk.usage {
                                    if let Some(pt) = usage.prompt_tokens {
                                        *prompt_tokens.lock().unwrap() = pt;
                                    }
                                    if let Some(ct) = usage.completion_tokens {
                                        *total_tokens.lock().unwrap() = ct;
                                    }
                                    // Capture cached tokens from prompt_tokens_details
                                    if let Some(details) = &usage.prompt_tokens_details
                                        && details.cached_tokens.is_some()
                                    {
                                        *cache_read_tokens.lock().unwrap() = details.cached_tokens;
                                    }
                                    // Authoritative cost from OpenAI-compatible gateways
                                    // (e.g. OpenRouter `usage.cost`, in USD credits).
                                    if usage.cost.is_some() {
                                        *provider_cost_usd.lock().unwrap() = usage.cost;
                                    }
                                }

                                if let Some(choice) = chunk.choices.first() {
                                    let mut tt = total_tokens.lock().unwrap();
                                    let mut acc = accumulated_tool_calls.lock().unwrap();
                                    let mut fr = finish_reason.lock().unwrap();
                                    let stream_event =
                                        process_stream_choice(choice, &mut tt, &mut acc, &mut fr);
                                    // Mirror reasoning deltas into the artifact
                                    // buffer as they stream, so the durable item
                                    // assembled at [DONE] carries the whole text.
                                    if let LlmStreamEvent::ReasoningDelta { delta, .. } =
                                        &stream_event
                                    {
                                        accumulated_reasoning.lock().unwrap().push_str(delta);
                                    }
                                    return vec![Ok(stream_event)];
                                }
                                vec![Ok(LlmStreamEvent::TextDelta(String::new()))]
                            }
                            Err(e) => vec![Ok(LlmStreamEvent::Error(
                                format!("Failed to parse chunk: {}", e).into(),
                            ))],
                        }
                    }
                })
                .flat_map(futures::stream::iter),
        );

        Ok(converted_stream)
    }

    /// OpenAI-compatible Chat Completions accept the top-level
    /// `parallel_tool_calls` boolean, so the preference maps directly onto the
    /// wire for every model served through this protocol.
    fn supports_parallel_tool_calls(&self, _model: &str) -> bool {
        true
    }
}

impl std::fmt::Debug for OpenAIProtocolChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIProtocolChatDriver")
            .field("protocol", &"openai_chat_completions")
            .finish()
    }
}

// ============================================================================
// Error Detection Helpers
// ============================================================================

/// Check if the error indicates the model was not found.
///
/// OpenAI returns 404 or 400 with `"model_not_found"` code or `"does not exist"` message.
/// OpenAI can also return 403 with `"model_not_found"` for tier-gated models — these must
/// be classified as model_unavailable rather than provider_misconfigured.
/// Also handles Gemini/OpenAI-compatible endpoints with similar patterns.
pub fn is_openai_model_not_found(status: reqwest::StatusCode, error_text: &str) -> bool {
    let error_lower = error_text.to_lowercase();

    // OpenAI can return 404, 400, or 403 (tier-gated access) for nonexistent/inaccessible models
    if status == reqwest::StatusCode::NOT_FOUND
        || status == reqwest::StatusCode::BAD_REQUEST
        || status == reqwest::StatusCode::FORBIDDEN
    {
        // OpenAI: {"error":{"code":"model_not_found","message":"The model 'x' does not exist"}}
        if error_lower.contains("model_not_found") {
            return true;
        }
    }

    // 404 with generic model-not-found patterns
    if status == reqwest::StatusCode::NOT_FOUND {
        if error_lower.contains("does not exist") {
            return true;
        }
        if error_lower.contains("model") && error_lower.contains("not found") {
            return true;
        }
    }

    false
}

/// Check if an OpenAI API error indicates the request is too large.
///
/// Detects:
/// - 429 with "Request too large" or token limit messages
/// - 400 with "context_length_exceeded" code
/// - Any message about maximum context length being exceeded
pub fn is_openai_request_too_large(status: reqwest::StatusCode, error_text: &str) -> bool {
    let error_lower = error_text.to_lowercase();

    // HTTP 429 with token-related errors
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // "Request too large for gpt-4" pattern
        if error_lower.contains("request too large") {
            return true;
        }
        // Token limit errors: "tokens per min (TPM): Limit X, Requested Y"
        if error_lower.contains("tokens") && error_lower.contains("limit") {
            return true;
        }
    }

    // HTTP 400 with context length errors
    if status == reqwest::StatusCode::BAD_REQUEST {
        // "context_length_exceeded" error code
        if error_lower.contains("context_length_exceeded") {
            return true;
        }
        // "maximum context length" message
        if error_lower.contains("maximum context length") {
            return true;
        }
    }

    // Generic patterns that could appear with various status codes
    if error_lower.contains("tokens must be reduced")
        || error_lower.contains("reduce the length")
        || error_lower.contains("input is too long")
    {
        return true;
    }

    false
}

// ============================================================================
// OpenAI API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    /// Request usage info in streaming response (required for token counts)
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OpenAiStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    /// Request-level control over parallel tool calls. Omitted when unset so the
    /// provider default applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    /// Speed selector: OpenAI service tier ("flex", "default", "priority").
    /// Omitted when `None` so the provider keeps its default ("auto") routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
    /// Verbosity selector ("low", "medium", "high"). Top-level field on the
    /// Chat Completions API. Omitted when `None` so the provider keeps its
    /// default ("medium") output length.
    #[serde(skip_serializing_if = "Option::is_none")]
    verbosity: Option<String>,
    /// Metadata for tracking API usage (up to 16 key-value pairs).
    /// Useful for correlating requests with session_id, agent_id, org_id, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct OpenAiStreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum OpenAiContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum OpenAiContentPart {
    Text {
        r#type: String,
        text: String,
    },
    ImageUrl {
        r#type: String,
        image_url: OpenAiImageUrl,
    },
    InputAudio {
        r#type: String,
        input_audio: OpenAiInputAudio,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiImageUrl {
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiInputAudio {
    data: String,
    format: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OpenAiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiTool {
    r#type: String,
    function: OpenAiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    r#type: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // id and model are deserialized but used by event listeners, not directly
struct OpenAiStreamChunk {
    /// Unique identifier for this completion
    #[serde(default)]
    id: Option<String>,
    /// Model used for completion (may differ from requested)
    #[serde(default)]
    model: Option<String>,
    choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    /// Detailed breakdown of prompt tokens (includes cached tokens)
    #[serde(default)]
    prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
    /// Authoritative per-request cost in USD credits, returned by
    /// OpenAI-compatible gateways such as OpenRouter. Absent for direct OpenAI.
    #[serde(default)]
    cost: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiPromptTokensDetails {
    /// Number of tokens retrieved from cache
    #[serde(default)]
    cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDelta {
    #[serde(default)]
    content: Option<String>,
    /// Reasoning text on the Chat Completions wire. Reasoning models reached
    /// over this protocol (DeepSeek-R1, Qwen, Groq, Fireworks) stream it here;
    /// vendors split between two field names for the same thing.
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

impl OpenAiDelta {
    fn reasoning_text(&self) -> Option<&str> {
        self.reasoning_content
            .as_deref()
            .or(self.reasoning.as_deref())
            .filter(|text| !text.is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: u32,
    id: Option<String>,
    function: Option<OpenAiStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

/// Drains tool calls that were accumulated but not yet emitted, returning a
/// final `ToolCalls` event for the `[DONE]` handler. Returns `None` when nothing
/// is pending (the common case, since the finish chunk normally drains them).
///
/// The fallback may only emit calls when the provider omitted a finish reason or
/// reported `tool_calls`. Non-tool finish reasons such as `length` and
/// `content_filter` indicate an incomplete or rejected response, so pending
/// calls are discarded instead of being executed. Malformed streamed argument
/// JSON is likewise dropped (via the accumulator's strict flush) because this
/// fallback runs without an explicit final tool-call completion chunk.
fn take_pending_tool_calls(
    accumulated_tool_calls: &mut StreamToolCallAccumulator,
    finish_reason: Option<&str>,
) -> Option<LlmStreamEvent> {
    if accumulated_tool_calls.is_empty() {
        return None;
    }

    // A non-tool finish reason means the response was cut/rejected; drain the
    // accumulator (so a repeated flush cannot re-emit) but do not execute.
    if !matches!(finish_reason, None | Some("tool_calls")) {
        let _ = accumulated_tool_calls.take_finalized();
        return None;
    }

    let calls = accumulated_tool_calls.take_pending_strict();
    if calls.is_empty() {
        None
    } else {
        Some(LlmStreamEvent::ToolCalls(calls))
    }
}

/// Processes a single chat-completion stream choice, updating the running
/// accumulators and returning the event to emit.
///
/// EVE-522: some OpenAI-compatible providers (OpenRouter/DeepInfra) send an
/// empty `content: ""` delta in the *same* chunk that carries
/// `finish_reason: "tool_calls"`. The content branch must therefore ignore
/// empty content, otherwise it short-circuits before the finish handler and the
/// accumulated tool calls are silently dropped. Emitting drains the accumulator
/// so a repeated finish chunk does not re-emit the same calls.
fn process_stream_choice(
    choice: &OpenAiStreamChoice,
    total_tokens: &mut u32,
    accumulated_tool_calls: &mut StreamToolCallAccumulator,
    finish_reason: &mut Option<String>,
) -> LlmStreamEvent {
    // Accumulate streamed tool-call fragments, keyed by the chunk `index`. The
    // shared accumulator appends argument fragments in place (EVE-636: amortized
    // O(total)) and parses the JSON once at finalize.
    if let Some(tool_calls) = &choice.delta.tool_calls {
        for tc in tool_calls {
            accumulated_tool_calls.apply_indexed_delta(
                tc.index,
                tc.id.as_deref(),
                tc.function.as_ref().and_then(|f| f.name.as_deref()),
                tc.function.as_ref().and_then(|f| f.arguments.as_deref()),
            );
        }
        return LlmStreamEvent::TextDelta(String::new());
    }

    // Reasoning delta. Checked before content: a chunk carries one or the
    // other, and reasoning must reach the reasoning channel rather than being
    // dropped (which is what happened before this protocol parsed it at all).
    if let Some(reasoning) = choice.delta.reasoning_text() {
        return LlmStreamEvent::ReasoningDelta {
            delta: reasoning.to_string(),
            summary: false,
        };
    }

    // Content delta. Guard on non-empty: an empty-content delta that rides along
    // with finish_reason must not short-circuit the finish handler below.
    if let Some(content) = &choice.delta.content
        && !content.is_empty()
    {
        *total_tokens += 1;
        return LlmStreamEvent::TextDelta(content.clone());
    }

    // Finish reason. Store it for the [DONE] handler; for tool_calls, emit the
    // accumulated calls immediately so the agent can start working. Draining the
    // accumulator prevents a second finish chunk from re-emitting the calls.
    if let Some(fr) = &choice.finish_reason {
        *finish_reason = Some(fr.clone());

        if fr == "tool_calls" && !accumulated_tool_calls.is_empty() {
            return LlmStreamEvent::ToolCalls(accumulated_tool_calls.take_finalized());
        }
    }

    LlmStreamEvent::TextDelta(String::new())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_convert_message_preserves_multiple_system_messages() {
        // OpenAI chat-completions keeps the system role inline, so both the agent
        // system prompt and a later notice/summary System message (infinity_context
        // / compaction) pass through as separate `system` entries — neither is
        // dropped. Lock that in alongside the "separate system field" drivers.
        let messages = [
            LlmMessage::text(LlmMessageRole::System, "A"),
            LlmMessage::text(LlmMessageRole::User, "hi"),
            LlmMessage::text(LlmMessageRole::System, "B"),
        ];
        let converted: Vec<OpenAiMessage> = messages
            .iter()
            .map(OpenAIProtocolChatDriver::convert_message)
            .collect();
        let system_texts: Vec<String> = converted
            .iter()
            .filter(|m| m.role == "system")
            .filter_map(|m| match &m.content {
                Some(OpenAiContent::Text(t)) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(system_texts, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn test_is_azure_openai_api_url() {
        assert!(is_azure_openai_api_url(
            "https://example.openai.azure.com/openai/v1/chat/completions"
        ));
        assert!(is_azure_openai_api_url(
            "https://example.services.ai.azure.com/openai/v1/responses"
        ));
        assert!(!is_azure_openai_api_url(
            "https://api.openai.com/v1/chat/completions"
        ));
    }

    #[test]
    fn test_request_includes_stream_options_for_usage() {
        // OpenAI streaming API requires stream_options.include_usage=true
        // to return token usage in the response
        let request = OpenAiRequest {
            verbosity: None,
            service_tier: None,
            model: "gpt-5.2".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(OpenAiContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: true,
            stream_options: Some(OpenAiStreamOptions {
                include_usage: true,
            }),
            tools: None,
            parallel_tool_calls: None,
            reasoning_effort: None,
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["stream"], true);
        assert_eq!(json["stream_options"]["include_usage"], true);
    }

    #[test]
    fn test_request_includes_metadata() {
        // Metadata should be included when provided
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("session_id".to_string(), "session_abc123".to_string());
        metadata.insert("agent_id".to_string(), "agent_xyz789".to_string());

        let request = OpenAiRequest {
            verbosity: None,
            service_tier: None,
            model: "gpt-5.2".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(OpenAiContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: true,
            stream_options: None,
            tools: None,
            parallel_tool_calls: None,
            reasoning_effort: None,
            metadata: Some(metadata),
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["metadata"]["session_id"], "session_abc123");
        assert_eq!(json["metadata"]["agent_id"], "agent_xyz789");
    }

    #[test]
    fn test_usage_chunk_parsing() {
        // OpenAI sends usage in a separate chunk after finish_reason
        // This test verifies we can parse it correctly
        let usage_chunk = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-5.2",
            "choices": [],
            "usage": {
                "prompt_tokens": 150,
                "completion_tokens": 42,
                "total_tokens": 192
            }
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(usage_chunk).unwrap();
        assert!(chunk.usage.is_some());
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(150));
        assert_eq!(usage.completion_tokens, Some(42));
    }

    #[test]
    fn test_usage_chunk_with_cached_tokens() {
        // OpenAI includes cached_tokens in prompt_tokens_details
        let usage_chunk = r#"{
            "id": "chatcmpl-123",
            "choices": [],
            "usage": {
                "prompt_tokens": 150,
                "completion_tokens": 42,
                "prompt_tokens_details": {
                    "cached_tokens": 100
                }
            }
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(usage_chunk).unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(150));
        assert_eq!(usage.completion_tokens, Some(42));
        assert!(usage.prompt_tokens_details.is_some());
        assert_eq!(
            usage.prompt_tokens_details.unwrap().cached_tokens,
            Some(100)
        );
    }

    #[test]
    fn test_usage_chunk_with_openrouter_cost() {
        // OpenAI-compatible gateways like OpenRouter add `usage.cost` (USD credits).
        let usage_chunk = r#"{
            "id": "gen-123",
            "choices": [],
            "usage": {
                "prompt_tokens": 194,
                "completion_tokens": 2,
                "total_tokens": 196,
                "cost": 0.00095
            }
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(usage_chunk).unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.cost, Some(0.00095));
    }

    #[test]
    fn test_usage_chunk_without_cost_defaults_none() {
        // Direct OpenAI omits `cost`; it must deserialize to None, not error.
        let usage_chunk = r#"{
            "id": "chatcmpl-123",
            "choices": [],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(usage_chunk).unwrap();
        assert_eq!(chunk.usage.unwrap().cost, None);
    }

    #[test]
    fn test_chunk_id_is_captured() {
        let chunk_with_id: OpenAiStreamChunk =
            serde_json::from_str(r#"{"id":"gen-abc123","choices":[]}"#).unwrap();
        assert_eq!(chunk_with_id.id.as_deref(), Some("gen-abc123"));

        let chunk_no_id: OpenAiStreamChunk = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
        assert!(chunk_no_id.id.is_none());
    }

    #[test]
    fn test_finish_reason_chunk_parsing() {
        // Finish reason comes in a chunk BEFORE the usage chunk
        let finish_chunk = r#"{
            "id": "chatcmpl-123",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }]
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(finish_chunk).unwrap();
        assert!(chunk.usage.is_none()); // No usage in finish_reason chunk
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].finish_reason, Some("stop".to_string()));
    }

    // ========================================================================
    // Request-too-large detection tests
    // ========================================================================

    #[test]
    fn test_is_openai_request_too_large_429_request_too_large() {
        let error = r#"{"error":{"message":"Request too large for gpt-4o in organization org-xxx on tokens per min (TPM): Limit 500000, Requested 538772."}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_429_token_limit() {
        let error =
            r#"{"error":{"message":"tokens per min (TPM): Limit 500000, Requested 600000"}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_400_context_length() {
        let error = r#"{"error":{"code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens."}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_400_max_context() {
        let error =
            r#"{"error":{"message":"This model's maximum context length is 128000 tokens"}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_tokens_must_be_reduced() {
        let error = r#"{"error":{"message":"The input or output tokens must be reduced"}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_false_for_other_errors() {
        // Regular rate limit (not token-related)
        let error = r#"{"error":{"message":"Rate limit exceeded: too many requests per minute"}}"#;
        assert!(!is_openai_request_too_large(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            error
        ));

        // Internal server error
        let error = r#"{"error":{"message":"Internal server error"}}"#;
        assert!(!is_openai_request_too_large(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error
        ));

        // Generic 400 error
        let error = r#"{"error":{"message":"Invalid request"}}"#;
        assert!(!is_openai_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    // ========================================================================
    // Model-not-found detection tests
    // ========================================================================

    #[test]
    fn test_is_openai_model_not_found_real_error() {
        // Real OpenAI 404 response for nonexistent model
        let error = r#"{"error":{"code":"model_not_found","message":"The model 'gpt-99' does not exist or you do not have access to it.","type":"invalid_request_error","param":null}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_does_not_exist() {
        let error = r#"{"error":{"message":"The model 'fake-model' does not exist"}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_generic_not_found() {
        let error = r#"{"error":{"message":"Model not found"}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_400_with_model_not_found_code() {
        // OpenAI Responses API returns 400 (not 404) for nonexistent models
        let error = r#"{"error":{"code":"model_not_found","message":"The requested model 'gpt-99' does not exist.","type":"invalid_request_error","param":"model"}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_false_for_non_model_error() {
        // 400 without model_not_found code should not match
        let error = r#"{"error":{"code":"invalid_request","message":"Some other error"}}"#;
        assert!(!is_openai_model_not_found(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_false_for_other_404() {
        // 404 without model-related message
        let error = r#"{"error":{"message":"Endpoint not found"}}"#;
        assert!(!is_openai_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_403_tier_gated_model() {
        // OpenAI returns 403 for models that exist but require a higher API tier;
        // these must classify as model_unavailable, not provider_misconfigured.
        let error = r#"{"error":{"code":"model_not_found","message":"The model 'gpt-5.4-mini' does not exist or you do not have access to it.","type":"invalid_request_error","param":null}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::FORBIDDEN,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_403_plain_auth_error_is_not_model_not_found() {
        // A plain 403 without model_not_found code is a real auth error and must
        // NOT be classified as model_unavailable.
        let error = r#"{"error":{"message":"Invalid authentication credentials","type":"authentication_error"}}"#;
        assert!(!is_openai_model_not_found(
            reqwest::StatusCode::FORBIDDEN,
            error
        ));
    }

    // ========================================================================
    // Reasoning effort guard tests
    // ========================================================================

    #[test]
    fn test_reasoning_effort_none_is_omitted() {
        // When reasoning_effort is "none", it should be filtered out
        // to avoid "Unrecognized request argument" errors on non-thinking models
        let request = OpenAiRequest {
            verbosity: None,
            service_tier: None,
            model: "gpt-5.4-mini".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(OpenAiContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: true,
            stream_options: None,
            tools: None,
            parallel_tool_calls: None,
            reasoning_effort: Some(crate::model::ReasoningEffort::None)
                .filter(crate::model::ReasoningEffort::requests_reasoning)
                .map(|e| e.as_str().to_string()),
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert!(
            json.get("reasoning_effort").is_none(),
            "reasoning_effort should be omitted when effort is 'none'"
        );
    }

    #[test]
    fn test_reasoning_effort_high_is_included() {
        let request = OpenAiRequest {
            verbosity: None,
            service_tier: None,
            model: "o4-mini".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(OpenAiContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: true,
            stream_options: None,
            tools: None,
            parallel_tool_calls: None,
            reasoning_effort: Some(crate::model::ReasoningEffort::High)
                .filter(crate::model::ReasoningEffort::requests_reasoning)
                .map(|e| e.as_str().to_string()),
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["reasoning_effort"], "high");
    }

    /// EVE-598: the Chat Completions request (used by the OpenAI Chat driver,
    /// OpenRouter, and MAI) serializes `parallel_tool_calls` only when set, so
    /// the provider default applies when the operator leaves it unset.
    #[test]
    fn test_request_serializes_parallel_tool_calls() {
        fn build(flag: Option<bool>) -> serde_json::Value {
            let request = OpenAiRequest {
                verbosity: None,
                service_tier: None,
                model: "gpt-5.4-mini".to_string(),
                messages: vec![OpenAiMessage {
                    role: "user".to_string(),
                    content: Some(OpenAiContent::Text("Hello".to_string())),
                    tool_calls: None,
                    tool_call_id: None,
                }],
                temperature: None,
                max_tokens: None,
                stream: true,
                stream_options: None,
                tools: None,
                parallel_tool_calls: flag,
                reasoning_effort: None,
                metadata: None,
            };
            serde_json::to_value(&request).unwrap()
        }

        // Omitted when None.
        assert!(build(None).get("parallel_tool_calls").is_none());
        // Present and preserved for Some(_).
        assert_eq!(build(Some(true))["parallel_tool_calls"], true);
        assert_eq!(build(Some(false))["parallel_tool_calls"], false);
    }

    /// The speed selector serializes as `service_tier` only when set, so the
    /// provider's default ("auto") routing applies when unset.
    #[test]
    fn test_request_serializes_service_tier() {
        fn build(tier: Option<&str>) -> serde_json::Value {
            let request = OpenAiRequest {
                service_tier: tier.map(str::to_string),
                verbosity: None,
                model: "gpt-5.4-mini".to_string(),
                messages: vec![OpenAiMessage {
                    role: "user".to_string(),
                    content: Some(OpenAiContent::Text("Hello".to_string())),
                    tool_calls: None,
                    tool_call_id: None,
                }],
                temperature: None,
                max_tokens: None,
                stream: true,
                stream_options: None,
                tools: None,
                parallel_tool_calls: None,
                reasoning_effort: None,
                metadata: None,
            };
            serde_json::to_value(&request).unwrap()
        }

        assert!(build(None).get("service_tier").is_none());
        assert_eq!(build(Some("flex"))["service_tier"], "flex");
        assert_eq!(build(Some("priority"))["service_tier"], "priority");
    }

    /// Verbosity serializes as a top-level `verbosity` field only when set, so
    /// the provider's default output length applies when unset.
    #[test]
    fn test_request_serializes_verbosity() {
        fn build(verbosity: Option<&str>) -> serde_json::Value {
            let request = OpenAiRequest {
                service_tier: None,
                verbosity: verbosity.map(str::to_string),
                model: "gpt-5.6-sol".to_string(),
                messages: vec![OpenAiMessage {
                    role: "user".to_string(),
                    content: Some(OpenAiContent::Text("Hello".to_string())),
                    tool_calls: None,
                    tool_call_id: None,
                }],
                temperature: None,
                max_tokens: None,
                stream: true,
                stream_options: None,
                tools: None,
                parallel_tool_calls: None,
                reasoning_effort: None,
                metadata: None,
            };
            serde_json::to_value(&request).unwrap()
        }

        assert!(build(None).get("verbosity").is_none());
        assert_eq!(build(Some("low"))["verbosity"], "low");
        assert_eq!(build(Some("high"))["verbosity"], "high");
    }

    // ------------------------------------------------------------------
    // EVE-522: streaming chunk handling (process_stream_choice)
    // ------------------------------------------------------------------

    fn choice(json_str: &str) -> OpenAiStreamChoice {
        serde_json::from_str(json_str).unwrap()
    }

    /// EVE-522 regression: providers such as OpenRouter/DeepInfra send an empty
    /// `content: ""` in the same chunk that carries `finish_reason: "tool_calls"`.
    /// The accumulated tool calls must still be emitted exactly once.
    #[test]
    fn test_empty_content_finish_chunk_still_emits_tool_calls() {
        let mut total_tokens = 0u32;
        let mut acc = StreamToolCallAccumulator::new();
        let mut finish_reason: Option<String> = None;

        // Chunk 2: tool_calls delta opens the call (id + name).
        let e = process_stream_choice(
            &choice(
                r#"{"delta":{"content":null,"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}]},"finish_reason":null}"#,
            ),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        assert!(matches!(e, LlmStreamEvent::TextDelta(s) if s.is_empty()));

        // Chunk 3: tool_calls delta streams the arguments.
        let e = process_stream_choice(
            &choice(
                r#"{"delta":{"content":null,"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"Cargo.toml\"}"}}]},"finish_reason":null}"#,
            ),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        assert!(matches!(e, LlmStreamEvent::TextDelta(s) if s.is_empty()));

        // Chunk 4: content:"" alongside finish_reason:"tool_calls" — must NOT
        // short-circuit; emits the accumulated call with parsed JSON arguments.
        let e = process_stream_choice(
            &choice(r#"{"delta":{"content":""},"finish_reason":"tool_calls"}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        match e {
            LlmStreamEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "read_file");
                assert_eq!(calls[0].arguments, json!({"path": "Cargo.toml"}));
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
        assert_eq!(finish_reason.as_deref(), Some("tool_calls"));

        // Chunk 5: second finish chunk with content:"" — the accumulator was
        // drained, so the same call must not be emitted again.
        let e = process_stream_choice(
            &choice(r#"{"delta":{"content":""},"finish_reason":"tool_calls"}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        assert!(
            matches!(e, LlmStreamEvent::TextDelta(s) if s.is_empty()),
            "tool calls must only be emitted once"
        );
    }

    /// Non-empty content deltas are still emitted and counted as output tokens.
    #[test]
    fn test_non_empty_content_is_emitted() {
        let mut total_tokens = 0u32;
        let mut acc = StreamToolCallAccumulator::new();
        let mut finish_reason: Option<String> = None;

        let e = process_stream_choice(
            &choice(r#"{"delta":{"content":"hello"},"finish_reason":null}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        assert!(matches!(e, LlmStreamEvent::TextDelta(s) if s == "hello"));
        assert_eq!(total_tokens, 1);
    }

    /// EVE-636: streamed tool-call arguments must concatenate exactly across
    /// many small chunks (accumulated as a raw string, parsed zero times
    /// mid-stream) and be parsed exactly once at the `tool_calls` finish chunk.
    #[test]
    fn test_tool_call_arguments_accumulate_across_many_chunks() {
        let mut total_tokens = 0u32;
        let mut acc = StreamToolCallAccumulator::new();
        let mut finish_reason: Option<String> = None;

        // Open the call (id + name, empty initial arguments).
        process_stream_choice(
            &choice(
                r#"{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"write_file","arguments":""}}]},"finish_reason":null}"#,
            ),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );

        let payload = r#"{"path":"a.rs","contents":"a fairly long contents value streamed one character at a time to exceed one hundred chunks","n":987654321}"#;
        assert!(payload.chars().count() > 100);

        // Stream the arguments one character per chunk.
        let mut expected = String::new();
        for ch in payload.chars() {
            let frag = ch.to_string();
            let chunk = json!({
                "delta": {"tool_calls": [{"index": 0, "function": {"arguments": frag}}]},
                "finish_reason": null
            })
            .to_string();
            process_stream_choice(
                &choice(&chunk),
                &mut total_tokens,
                &mut acc,
                &mut finish_reason,
            );
            expected.push_str(&frag);
        }

        // Mid-stream the shared accumulator holds the fragments as a raw string
        // (parsed once at finalize); its own unit tests cover that internal, so
        // here we assert the observable finish-chunk result concatenates exactly.

        // Finish chunk: parsed exactly once into the structured value.
        let e = process_stream_choice(
            &choice(r#"{"delta":{},"finish_reason":"tool_calls"}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        match e {
            LlmStreamEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(
                    calls[0].arguments,
                    serde_json::from_str::<serde_json::Value>(payload).unwrap()
                );
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
    }

    /// OpenAI's native path sends `delta: {}` (no content key) in the finish
    /// chunk; the existing behavior of emitting tool calls there is preserved.
    #[test]
    fn test_finish_chunk_without_content_emits_tool_calls() {
        let mut total_tokens = 0u32;
        let mut acc = StreamToolCallAccumulator::new();
        let mut finish_reason: Option<String> = None;

        process_stream_choice(
            &choice(
                r#"{"delta":{"tool_calls":[{"index":0,"id":"call_9","function":{"name":"list_dir","arguments":"{}"}}]},"finish_reason":null}"#,
            ),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );

        let e = process_stream_choice(
            &choice(r#"{"delta":{},"finish_reason":"tool_calls"}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        match e {
            LlmStreamEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "list_dir");
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
    }

    /// Seed a single tool-call slot into an accumulator the way the streamed
    /// chunks would (id + name + raw argument buffer), so the fallback-flush
    /// tests exercise the real accumulation path.
    fn seeded_acc(id: &str, name: &str, arguments: &str) -> StreamToolCallAccumulator {
        let mut acc = StreamToolCallAccumulator::new();
        acc.apply_indexed_delta(0, Some(id), Some(name), Some(arguments));
        acc
    }

    /// The [DONE] fallback flushes accumulated-but-unemitted tool calls when no
    /// finish reason was reported and drains the accumulator; once drained it
    /// returns None.
    #[test]
    fn test_take_pending_tool_calls_flushes_then_drains_without_finish_reason() {
        let mut acc = seeded_acc("call_1", "read_file", r#"{"path":"Cargo.toml"}"#);

        match take_pending_tool_calls(&mut acc, None) {
            Some(LlmStreamEvent::ToolCalls(calls)) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "read_file");
                assert_eq!(calls[0].arguments, json!({"path": "Cargo.toml"}));
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
        assert!(acc.is_empty(), "accumulator must be drained after flush");
        assert!(take_pending_tool_calls(&mut acc, None).is_none());
    }

    #[test]
    fn test_take_pending_tool_calls_discards_non_tool_finish_reason() {
        let mut acc = seeded_acc("call_cut", "read_file", r#"{"path":"#);

        assert!(take_pending_tool_calls(&mut acc, Some("length")).is_none());
        assert!(
            acc.is_empty(),
            "discarded unsafe fallback calls must still drain the accumulator"
        );
    }

    #[test]
    fn test_take_pending_tool_calls_rejects_malformed_fallback_arguments() {
        let mut acc = seeded_acc("call_cut", "read_file", r#"{"path":"#);

        assert!(take_pending_tool_calls(&mut acc, None).is_none());
        assert!(
            acc.is_empty(),
            "malformed fallback calls must be drained instead of re-emitted"
        );
    }

    #[test]
    fn test_non_tool_finish_reason_leaves_pending_calls_for_done_discard() {
        let mut total_tokens = 0u32;
        let mut acc = StreamToolCallAccumulator::new();
        let mut finish_reason: Option<String> = None;

        process_stream_choice(
            &choice(
                r#"{"delta":{"tool_calls":[{"index":0,"id":"call_cut","function":{"name":"read_file","arguments":"{\"path\":"}}]},"finish_reason":null}"#,
            ),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );

        let e = process_stream_choice(
            &choice(r#"{"delta":{},"finish_reason":"length"}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );

        assert!(matches!(e, LlmStreamEvent::TextDelta(s) if s.is_empty()));
        assert_eq!(finish_reason.as_deref(), Some("length"));
        assert!(take_pending_tool_calls(&mut acc, finish_reason.as_deref()).is_none());
        assert!(acc.is_empty());
    }

    #[test]
    fn drop_orphaned_tool_messages_removes_unmatched_tool_results() {
        use crate::driver_registry::LlmMessageContent;

        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "hello"),
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("result".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_trimmed".to_string()),
                phase: None,
                reasoning: Vec::new(),
            },
        ];
        let filtered = drop_orphaned_tool_messages(&messages);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].role, LlmMessageRole::User);
    }

    #[test]
    fn drop_orphaned_tool_messages_keeps_matched_tool_results() {
        use crate::driver_registry::LlmMessageContent;
        use crate::tool_types::ToolCall;

        let messages = vec![
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text(String::new()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({}),
                }]),
                tool_call_id: None,
                phase: None,
                reasoning: Vec::new(),
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("file content".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                phase: None,
                reasoning: Vec::new(),
            },
        ];
        let filtered = drop_orphaned_tool_messages(&messages);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn function_tools_serialize_strict_only_for_compatible_schemas() {
        use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
        let make_tool = |parameters| {
            ToolDefinition::Builtin(BuiltinTool {
                name: "lookup".into(),
                display_name: None,
                description: "Lookup".into(),
                parameters,
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::Never,
                hints: ToolHints::default(),
                full_parameters: None,
            })
        };
        let compatible = OpenAIProtocolChatDriver::convert_tools(&[make_tool(json!({
            "type": "object", "properties": {"query": {"type": "string"}}
        }))]);
        let serialized = serde_json::to_value(&compatible[0]).unwrap();
        assert_eq!(serialized["function"]["strict"], true);
        assert_eq!(
            serialized["function"]["parameters"]["required"],
            json!(["query"])
        );
        assert_eq!(
            serialized["function"]["parameters"]["properties"]["query"]["type"],
            json!(["string", "null"])
        );

        let incompatible = OpenAIProtocolChatDriver::convert_tools(&[make_tool(json!({
            "type": "object", "allOf": [{"type": "object"}]
        }))]);
        let serialized = serde_json::to_value(&incompatible[0]).unwrap();
        assert!(serialized["function"].get("strict").is_none());
        assert!(serialized["function"]["parameters"].get("allOf").is_some());
    }
}
