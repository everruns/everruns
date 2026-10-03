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
// For production use with OpenAI-specific features, use OpenAIChatDriver from everruns_drivers::openai.
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// llm.generation events are emitted by ReasonAtom, and OtelEventListener
// creates the appropriate gen-ai spans. No direct tracing in drivers.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::{Client, Url};
use std::sync::{Arc, Mutex};

use crate::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmResponse, LlmResponseStream,
    LlmStreamEvent, Message, MessageRole, disjoint_prompt_tokens,
};
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::llm_retry::{
    LlmRetryConfig, RateLimitInfo, RetryDecision, RetryMetadata, SendOutcome, is_rate_limit_status,
    retry_request, send_error_message,
};
use crate::openai_compat::{chat_completions_url, max_output_fields};
use crate::openai_message_convert::convert_messages;
use crate::openai_types::*;
use crate::runtime_provider::ProviderEndpoint;
use crate::stream_accumulator::StreamToolCallAccumulator;
use crate::stream_reconnect::connect_sse_with_reconnect;
use crate::tool_types::{ToolCall, ToolDefinition};
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
// These are used by both `everruns_drivers::openai` and `everruns_drivers::openrouter` to derive
// a `/models` URL, normalize a base URL, authenticate the discovery request, and
// map a non-success status into an error. They live in core so the provider
// crates can reuse them without duplicating logic.

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
    let Ok(mut url) = Url::parse(api_url) else {
        return api_url.to_owned();
    };
    // THREAT[TM-API-025]: discovery must never move provider credentials to a
    // fallback origin when a URL contains a query or a custom path.
    let path = url.path().trim_end_matches('/');
    let models_path = if path.ends_with("/models") {
        path.to_owned()
    } else {
        let base = path
            .strip_suffix("/responses")
            .or_else(|| path.strip_suffix("/chat/completions"))
            .unwrap_or(path);
        format!("{base}/models")
    };
    url.set_path(&models_path);
    url.to_string()
}

/// The request body, as JSON, when the call asked for it to be captured.
///
/// Serializing the same value that goes on the wire keeps the capture honest:
/// it cannot drift from what was actually sent. A serialization failure yields
/// `None` rather than failing the call — a diagnostic must never be the reason
/// a request does not happen.
fn capture_request_body(
    config: &LlmCallConfig,
    request: &impl serde::Serialize,
) -> Option<serde_json::Value> {
    config
        .capture_request
        .then(|| serde_json::to_value(request).ok())
        .flatten()
}

/// Build the error returned when the `/models` endpoint responds with a
/// non-success status.
///
/// The status is classified here, at the provider boundary, so callers that
/// act on the *kind* of failure (credential checks distinguishing a rejected
/// key from an unreachable provider) do not have to re-parse the message.
pub fn models_api_status_error(status: reqwest::StatusCode) -> AgentLoopError {
    AgentLoopError::llm_http(
        status.as_u16(),
        "",
        format!("Models API returned status {status}"),
    )
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
/// For production use with OpenAI, consider using `OpenAIChatDriver` from the `everruns_drivers::openai` crate.
///
/// # Example
///
/// ```ignore
/// use everruns_contracts::OpenAIProtocolChatDriver;
///
/// let driver = OpenAIProtocolChatDriver::new();
/// // Endpoint and authentication are configured on a runtime Provider.
/// // Retry policy remains a wire-protocol concern.
/// let driver = OpenAIProtocolChatDriver::new()
///     .with_retry_config(LlmRetryConfig::aggressive());
/// ```
#[derive(Clone)]
pub struct OpenAIProtocolChatDriver {
    /// Retry configuration for rate limit errors
    retry_config: LlmRetryConfig,
}

impl OpenAIProtocolChatDriver {
    /// Create a wire-only OpenAI Chat Completions protocol driver.
    pub fn new() -> Self {
        // EVE-924: choose the rustls backend on the startup path. The shared
        // client installs it as well, but that now happens on the first
        // request, and products expect the process-wide choice to be settled
        // while providers are being constructed.
        crate::install_default_crypto_provider();
        Self {
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// Configure retry behavior for rate limit errors
    pub fn with_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// The process-wide streaming HTTP client, resolved per request rather than
    /// held as a field. Building it loads the platform trust store (~1.3 ms),
    /// which would otherwise land on the agent startup path; after the first
    /// request this is a `OnceLock` read and an `Arc` clone.
    ///
    /// Returned by value for subclass access; a `reqwest::Client` is an `Arc`
    /// handle, so cloning it shares the same connection pool.
    pub fn client(&self) -> Client {
        crate::driver_helpers::shared_streaming_http_client()
    }

    /// Send one streaming chat-completion request, applying the shared
    /// header-phase retry loop (transient send failures, 429, and 5xx), and
    /// return the raw response plus its retry metadata.
    ///
    /// Invoked once per reconnect attempt by [`connect_sse_with_reconnect`]. It
    /// re-sends the identical request and consumes no body bytes, so retrying it
    /// is idempotent. The classifier preserves OpenAI's terminal classification
    /// and error messages exactly.
    #[expect(
        clippy::unwrap_used,
        reason = "Streaming metadata locks contain only internal state and never invoke user code; retain fail-closed poison handling"
    )]
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

        crate::openai_compat::validate_body(
            &serde_json::to_value(request)
                .map_err(|e| AgentLoopError::Configuration(e.to_string()))?,
            endpoint,
            false,
        )?;
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
                let mut request_builder = self.client().post(&resolved.url);
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
                            return RetryDecision::Terminal(
                                AgentLoopError::llm_kind(
                                    LlmErrorKind::QuotaExhausted,
                                    format!("OpenAI API error ({}): {}", status, error_text),
                                )
                                .with_status(status.as_u16()),
                            );
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

                    // Non-retryable error or max retries exceeded. The
                    // rate-limit headers are read before the body is consumed
                    // so a terminal 429 still carries the provider's own
                    // retry delay.
                    let retry_after = is_rate_limit_status(status)
                        .then(|| RateLimitInfo::from_openai_headers(response.headers()))
                        .and_then(|info| info.retry_after_secs);
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

                    // Classify and preserve the status/code while the HTTP
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
                    let mut error = AgentLoopError::llm_http(status.as_u16(), &error_text, message);
                    if let Some(secs) = retry_after {
                        error = error.with_retry_after_secs(secs);
                    }
                    RetryDecision::Terminal(error)
                }
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await
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

// Provider usage is authoritative, including zero; estimates must never be
// added to it when usage and text share a frame or more text follows usage.
#[derive(Default)]
struct CompletionTokenCount {
    estimated: u32,
    reported: Option<u32>,
}

impl Default for OpenAIProtocolChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Drop Tool-role messages whose tool_call_id has no matching assistant tool call in the
/// visible window. Chat Completions rejects payloads where a `tool`-role message references
/// a call that is absent from the conversation.
fn drop_orphaned_tool_messages(messages: &[Message]) -> Vec<Message> {
    use std::collections::HashSet;

    let visible_call_ids: HashSet<&str> = messages
        .iter()
        .filter(|m| m.role == MessageRole::Assistant)
        .flat_map(|m| m.tool_calls.iter().flatten())
        .map(|tc| tc.id.as_str())
        .collect();

    if visible_call_ids.is_empty() {
        return messages
            .iter()
            .filter(|m| m.role != MessageRole::Tool)
            .cloned()
            .collect();
    }

    messages
        .iter()
        .filter(|m| {
            if m.role == MessageRole::Tool {
                return m
                    .tool_call_id
                    .as_deref()
                    .is_some_and(|id| visible_call_ids.contains(id));
            }
            true
        })
        .cloned()
        .collect()
}

#[async_trait]
impl ChatDriver for OpenAIProtocolChatDriver {
    fn supports_native_non_streaming(&self) -> bool {
        true
    }

    async fn chat_completion_non_streaming(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        // Same request as the streaming path, but with streaming disabled so
        // the provider answers with one JSON body instead of SSE.
        let openai_messages: Vec<OpenAiMessage> = convert_messages(&messages);
        let tools = if config.tools.is_empty() {
            None
        } else {
            Some(Self::convert_tools(&config.tools))
        };
        let metadata = if config.metadata.is_empty() {
            None
        } else {
            Some(config.metadata.clone())
        };
        let api_url = chat_completions_url(endpoint)?;
        let (max_tokens, max_completion_tokens) = max_output_fields(&api_url, config.max_tokens);
        let request = OpenAiRequest {
            model: config.model.clone(),
            messages: openai_messages,
            temperature: config.temperature,
            max_tokens,
            max_completion_tokens,
            stream: false,
            stream_options: None,
            tools,
            parallel_tool_calls: config
                .resolved_parallel_tool_calls(self.supports_parallel_tool_calls(&config.model)),
            reasoning_effort: OpenAiRequest::reasoning_effort_for(config),
            service_tier: config.speed.clone(),
            verbosity: config.verbosity.clone(),
            metadata,
            response_format: OpenAiRequest::response_format_for(config),
        };
        let captured_request = capture_request_body(config, &request);
        let (response, retry_metadata) = self
            .send_chat_completion_request(
                endpoint,
                &api_url,
                &request,
                &config.model,
                &config.extra_headers,
                0,
            )
            .await?;
        let body: OpenAiChatCompletionResponse = response.json().await.map_err(|error| {
            AgentLoopError::llm(format!("failed to decode non-streaming response: {error}"))
        })?;
        if let Some(error) = body.error {
            return Err(error.into_stream_error().into_agent_error());
        }
        let response_model = body.model.clone();
        let (text, tool_calls, reasoning, finish_reason) = match body.choices.into_iter().next() {
            Some(choice) => {
                let text = match choice.message.content {
                    Some(OpenAiContent::Text(text)) => text,
                    Some(OpenAiContent::Parts(parts)) => parts
                        .into_iter()
                        .filter_map(|part| match part {
                            OpenAiContentPart::Text { text, .. } => Some(text),
                            _ => None,
                        })
                        .collect(),
                    None => String::new(),
                };
                // Mirror the streaming path: tool calls whose arguments are not
                // complete JSON values cannot execute, so drop them.
                let tool_calls: Vec<ToolCall> = choice
                    .message
                    .tool_calls
                    .into_iter()
                    .filter_map(|tool_call| {
                        let arguments = serde_json::from_str(&tool_call.function.arguments).ok()?;
                        Some(ToolCall {
                            id: tool_call.id,
                            name: tool_call.function.name,
                            arguments,
                        })
                    })
                    .collect();
                let reasoning = choice.message.reasoning_content.map(|text| {
                    crate::reasoning::ReasoningContentPart::opaque("openai-protocol")
                        .with_text(crate::reasoning::ReasoningText::Plain { text })
                });
                (text, tool_calls, reasoning, choice.finish_reason)
            }
            None => (String::new(), Vec::new(), None, None),
        };
        let (prompt_tokens, completion_tokens, cached_tokens, reasoning_tokens, cost) = body
            .usage
            .map(|usage| {
                let cached = usage
                    .prompt_tokens_details
                    .as_ref()
                    .and_then(|details| details.cached_tokens);
                let reasoning = usage
                    .completion_tokens_details
                    .as_ref()
                    .and_then(|details| details.reasoning_tokens);
                let prompt = usage.prompt_tokens.unwrap_or(0);
                (
                    Some(prompt),
                    usage.completion_tokens,
                    Some(disjoint_prompt_tokens(prompt, cached)),
                    reasoning,
                    usage.cost,
                )
            })
            .unwrap_or((None, None, None, None, None));
        Ok(LlmResponse {
            text,
            reasoning: reasoning.into_iter().collect(),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            metadata: LlmCompletionMetadata {
                total_tokens: prompt_tokens
                    .unwrap_or(0)
                    .checked_add(completion_tokens.unwrap_or(0)),
                prompt_tokens,
                completion_tokens,
                cache_read_tokens: cached_tokens,
                cache_creation_tokens: None,
                reasoning_tokens,
                provider_cost_usd: cost,
                model: Some(config.model.clone()),
                response_model,
                finish_reason,
                retry_metadata: if retry_metadata.had_retries() {
                    Some(retry_metadata)
                } else {
                    None
                },
                response_id: body.id,
                phase: None,
                request_body: captured_request,
                cache_diagnostics: None,
                ..Default::default()
            },
        })
    }

    #[expect(
        clippy::unwrap_used,
        reason = "Streaming metadata locks contain only internal state and never invoke user code; retain fail-closed poison handling"
    )]
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        crate::openai_compat::validate_config(config)?;
        // Note: OTel instrumentation is handled via event listeners.
        // ReasonAtom emits llm.generation events, and OtelEventListener
        // creates gen-ai spans from those events.
        let messages = drop_orphaned_tool_messages(&messages);
        let openai_messages: Vec<OpenAiMessage> = convert_messages(&messages);

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

        let api_url = chat_completions_url(endpoint)?;
        let (max_tokens, max_completion_tokens) = max_output_fields(&api_url, config.max_tokens);
        let request = OpenAiRequest {
            model: config.model.clone(),
            messages: openai_messages,
            temperature: config.temperature,
            max_tokens,
            max_completion_tokens,
            stream: true,
            stream_options: Some(OpenAiStreamOptions {
                include_usage: true,
            }),
            tools,
            parallel_tool_calls: config
                .resolved_parallel_tool_calls(self.supports_parallel_tool_calls(&config.model)),
            reasoning_effort: OpenAiRequest::reasoning_effort_for(config),
            service_tier: config.speed.clone(),
            verbosity: config.verbosity.clone(),
            metadata,
            response_format: OpenAiRequest::response_format_for(config),
        };

        // Establish the SSE stream, transparently reconnecting on a transport
        // failure that lands before the first event is decoded (the "error
        // decoding response body" flake). Header-phase retries (429/5xx and
        // transient send failures) are handled inside the per-attempt send;
        // this adds the body-phase reconnect the official SDKs get for free.
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

        let captured_request = Arc::new(capture_request_body(config, &request));
        let model = config.model.clone();
        let response_model = Arc::new(Mutex::new(Option::<String>::None));
        let completion_tokens = Arc::new(Mutex::new(CompletionTokenCount::default()));
        let prompt_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let reasoning_tokens = Arc::new(Mutex::new(Option::<u32>::None));
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
                    let response_model = Arc::clone(&response_model);
                    let completion_tokens = Arc::clone(&completion_tokens);
                    let prompt_tokens = Arc::clone(&prompt_tokens);
                    let cache_read_tokens = Arc::clone(&cache_read_tokens);
                    let reasoning_tokens = Arc::clone(&reasoning_tokens);
                    let provider_cost_usd = Arc::clone(&provider_cost_usd);
                    let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
                    let finish_reason = Arc::clone(&finish_reason);
                    let accumulated_reasoning = Arc::clone(&accumulated_reasoning);
                    let response_id = Arc::clone(&response_id);
                    let retry_metadata_for_done = shared_retry_metadata.clone();
                    let captured_request = Arc::clone(&captured_request);

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
                            let output_tokens = {
                                let counts = completion_tokens.lock().unwrap();
                                counts.reported.unwrap_or(counts.estimated)
                            };
                            let input_tokens = *prompt_tokens.lock().unwrap();
                            let cached = *cache_read_tokens.lock().unwrap();
                            let reasoning_used = *reasoning_tokens.lock().unwrap();
                            let cost = *provider_cost_usd.lock().unwrap();
                            let served_model = response_model.lock().unwrap().clone();
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
                                    reasoning_tokens: reasoning_used,
                                    cache_creation_tokens: None,
                                    provider_cost_usd: cost,
                                    model: Some(model),
                                    response_model: served_model,
                                    // Never defaulted: `None` says the provider sent none.
                                    finish_reason: reason,
                                    retry_metadata: retry_metadata_for_done
                                        .map(|arc| (*arc).clone()),
                                    response_id: resp_id,
                                    phase: None,
                                    request_body: (*captured_request).clone(),
                                    cache_diagnostics: None,
                                    ..Default::default()
                                },
                            ))));

                            return events;
                        }

                        match serde_json::from_str::<OpenAiStreamChunk>(&event.data) {
                            Ok(chunk) if chunk.error.is_some() => {
                                let error = chunk.error.unwrap_or_default();
                                vec![Ok(LlmStreamEvent::Error(error.into_stream_error()))]
                            }
                            Ok(chunk) => {
                                if chunk.model.is_some() {
                                    *response_model.lock().unwrap() = chunk.model.clone();
                                }
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
                                        completion_tokens.lock().unwrap().reported = Some(ct);
                                    }
                                    // Capture cached tokens from prompt_tokens_details
                                    if let Some(details) = &usage.prompt_tokens_details
                                        && details.cached_tokens.is_some()
                                    {
                                        *cache_read_tokens.lock().unwrap() = details.cached_tokens;
                                    }
                                    // Reasoning is billed inside completion_tokens;
                                    // keep the breakdown so callers can attribute it.
                                    if let Some(details) = &usage.completion_tokens_details
                                        && details.reasoning_tokens.is_some()
                                    {
                                        *reasoning_tokens.lock().unwrap() =
                                            details.reasoning_tokens;
                                    }
                                    // Authoritative cost from OpenAI-compatible gateways
                                    // (e.g. OpenRouter `usage.cost`, in USD credits).
                                    if usage.cost.is_some() {
                                        *provider_cost_usd.lock().unwrap() = usage.cost;
                                    }
                                }

                                if let Some(choice) = chunk.choices.first() {
                                    let mut counts = completion_tokens.lock().unwrap();
                                    let mut acc = accumulated_tool_calls.lock().unwrap();
                                    let mut fr = finish_reason.lock().unwrap();
                                    let Some(stream_event) = process_stream_choice(
                                        choice,
                                        &mut counts.estimated,
                                        &mut acc,
                                        &mut fr,
                                    ) else {
                                        return Vec::new();
                                    };
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
                                Vec::new() // usage- or role-only chunk
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

    /// Structured output goes out as `response_format: {type: "json_schema"}`.
    fn supports_response_format(&self, _model: &str) -> bool {
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

pub use crate::openai_errors::{is_openai_model_not_found, is_openai_request_too_large};

/// Drains tool calls still pending at `[DONE]`; see
/// [`StreamToolCallAccumulator::take_at_stream_end`] for which calls survive.
fn take_pending_tool_calls(
    accumulated_tool_calls: &mut StreamToolCallAccumulator,
    finish_reason: Option<&str>,
) -> Option<LlmStreamEvent> {
    let calls = accumulated_tool_calls.take_at_stream_end(finish_reason);
    (!calls.is_empty()).then_some(LlmStreamEvent::ToolCalls(calls))
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
/// `None` when the chunk has nothing to emit: no empty `TextDelta` filler.
fn process_stream_choice(
    choice: &OpenAiStreamChoice,
    total_tokens: &mut u32,
    accumulated_tool_calls: &mut StreamToolCallAccumulator,
    finish_reason: &mut Option<String>,
) -> Option<LlmStreamEvent> {
    // THREAT[TM-TOOL-036]: a terminal reason can share a final tool fragment.
    // Preserve it before any delta branch returns, especially for rejected calls.
    if let Some(reason) = &choice.finish_reason {
        *finish_reason = Some(reason.clone());
    }

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
        return None;
    }

    // Reasoning delta. Checked before content: a chunk carries one or the
    // other, and reasoning must reach the reasoning channel rather than being
    // dropped (which is what happened before this protocol parsed it at all).
    if let Some(reasoning) = choice.delta.reasoning_text() {
        return Some(LlmStreamEvent::ReasoningDelta {
            delta: reasoning.to_string(),
            summary: false,
        });
    }

    // Content delta. Guard on non-empty: an empty-content delta that rides along
    // with finish_reason must not short-circuit the finish handler below.
    if let Some(content) = &choice.delta.content
        && !content.is_empty()
    {
        *total_tokens += 1;
        return Some(LlmStreamEvent::TextDelta(content.clone()));
    }

    // Emit completed calls immediately. Draining prevents repeated finish
    // chunks from emitting the same calls again.
    if choice.finish_reason.as_deref() == Some("tool_calls") && !accumulated_tool_calls.is_empty() {
        return Some(LlmStreamEvent::ToolCalls(
            accumulated_tool_calls.take_finalized(),
        ));
    }

    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "openai_protocol_tests.rs"]
mod tests;
