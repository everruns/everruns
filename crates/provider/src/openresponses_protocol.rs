// Open Responses Protocol Driver
//
// Implementation of the Open Responses specification (https://www.openresponses.org/)
// an open-source, vendor-neutral API standard for multi-provider LLM interfaces.
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff, respecting x-ratelimit-reset-* and retry-after headers.
// Retry metadata is included in the response for observability.
//
// The spec is inspired by and interoperable with the OpenAI Responses API, offering:
// - One spec, many providers (OpenAI, Anthropic, Gemini, local models)
// - Agentic loop support with tool calls and state machines
// - Semantic streaming events (not raw text deltas)
// - 40-80% better cache utilization vs Chat Completions API
// - Native stateful conversation support
//
// Specification: https://www.openresponses.org/specification
// GitHub: https://github.com/openresponses/openresponses
//
// The Chat Completions API remains supported for backward compatibility.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::{Client, header::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub use crate::compact::{
    CompactContent, CompactContentPart, CompactInputItem, CompactOutputItem, CompactRequest,
    CompactResponse, CompactUsage, messages_to_compact_input,
};
use crate::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmContentPart, LlmMessage,
    LlmMessageContent, LlmMessageRole, LlmResponseStream, LlmStreamEvent, disjoint_prompt_tokens,
    fold_system_messages,
};
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::llm_retry::{
    LlmRetryConfig, RateLimitInfo, RetryDecision, RetryMetadata, SendOutcome, is_rate_limit_status,
    retry_request, send_error_message,
};
use crate::openai_protocol::{is_openai_model_not_found, is_openai_request_too_large};
use crate::openresponses_types::{self as types, StreamingEvent};
use crate::stream_reconnect::connect_sse_with_reconnect;
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::user_facing_error::is_provider_quota_message;

const OPENAI_PROMPT_CACHE_KEY_MAX_LEN: usize = 64;
const PROMPT_CACHE_KEY_PREFIX: &str = "everruns:";

/// Open Responses Protocol Driver (OpenAI implementation)
///
/// Implements `ChatDriver` using the Open Responses specification
/// (<https://www.openresponses.org/>). This driver targets OpenAI's API
/// but follows the vendor-neutral Open Responses standard.
///
/// Rate limit handling: On 429 errors, automatically retries with exponential
/// backoff, respecting `x-ratelimit-reset-*` and `retry-after` headers.
///
/// The Open Responses spec is recommended for new projects, offering:
/// - Better performance with reasoning models (o1, o3, GPT-5)
/// - Provider-agnostic streaming events
/// - Native agentic loop support
///
/// # Example
///
/// ```ignore
/// use everruns_provider::OpenResponsesProtocolChatDriver;
///
/// let driver = OpenResponsesProtocolChatDriver::new();
/// // Endpoint and authentication are configured on a runtime Provider.
/// let driver = OpenResponsesProtocolChatDriver::new()
///     .with_retry_config(LlmRetryConfig::aggressive());
/// ```
/// Hook for provider-specific augmentation of an Open Responses request.
///
/// The Open Responses request shape this driver builds is vendor-neutral.
/// Providers reached through it (e.g. OpenRouter) layer extra top-level fields
/// onto the outgoing JSON or HTTP headers via this seam, so the core driver
/// stays free of provider branching. `decorate` and `decorate_headers` run once
/// per request, after the base body is serialized and before it is sent; either
/// may return an error to abort the request (e.g. failed routing validation).
pub trait OpenResponsesRequestExtension: Send + Sync {
    fn decorate(&self, body: &mut Value, config: &LlmCallConfig) -> Result<()>;

    /// Add provider-specific **non-auth** request headers (routing, attribution,
    /// `session_id`, `OpenAI-Beta`, `originator`, account ids, …).
    ///
    /// Authentication is owned by the runtime provider. The driver applies
    /// these decoration headers first, then the provider-resolved auth headers,
    /// so authentication wins on a name conflict. Do not set auth here.
    fn decorate_headers(&self, _headers: &mut HeaderMap, _config: &LlmCallConfig) -> Result<()> {
        Ok(())
    }

    /// Refine retry metadata from provider-specific rate limit response fields.
    fn update_rate_limit_info(
        &self,
        _info: &mut RateLimitInfo,
        _headers: &HeaderMap,
        _error_body: &str,
    ) {
    }
}

#[derive(Clone)]
pub struct OpenResponsesProtocolChatDriver {
    client: Client,
    /// Retry configuration for rate limit errors
    retry_config: LlmRetryConfig,
    /// Optional provider-specific request-body decorator (see
    /// [`OpenResponsesRequestExtension`]). `None` for vanilla OpenAI/Azure.
    request_extension: Option<Arc<dyn OpenResponsesRequestExtension>>,
    /// Explicit stateful-continuation support supplied by the service provider.
    stateful_responses: Option<bool>,
    native_phases: bool,
    hosted_tool_search: bool,
}

impl OpenResponsesProtocolChatDriver {
    /// Create a wire-only Open Responses protocol driver.
    pub fn new() -> Self {
        Self {
            // SSRF-hardened shared client (redirects disabled + DNS-pinned
            // resolver). The api_url is org-configurable, so a bare
            // `Client::new()` would leave this provider open to DNS-rebind /
            // redirect SSRF (TM-API-013, EVE-623).
            client: crate::driver_helpers::shared_streaming_http_client(),
            retry_config: LlmRetryConfig::default(),
            request_extension: None,
            stateful_responses: None,
            native_phases: false,
            hosted_tool_search: false,
        }
    }

    /// Enable optional protocol extensions implemented by this endpoint.
    pub fn with_native_features(mut self, phases: bool, hosted_tool_search: bool) -> Self {
        self.native_phases = phases;
        self.hosted_tool_search = hosted_tool_search;
        self
    }

    /// Attach a provider-specific request-body decorator. The decorator runs on
    /// every chat request just before it is sent (see
    /// [`OpenResponsesRequestExtension`]).
    pub fn with_request_extension(
        mut self,
        extension: Arc<dyn OpenResponsesRequestExtension>,
    ) -> Self {
        self.request_extension = Some(extension);
        self
    }

    /// Override whether this endpoint persists Responses continuation state.
    pub fn with_stateful_responses(mut self, supported: bool) -> Self {
        self.stateful_responses = Some(supported);
        self
    }

    /// Configure retry behavior for rate limit errors
    pub fn with_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Send one streaming Responses request, applying the shared header-phase
    /// retry loop (transient send failures, 429, and 5xx), and return the raw
    /// response plus its retry metadata.
    ///
    /// Invoked once per reconnect attempt by [`connect_sse_with_reconnect`]; it
    /// re-sends the identical request and consumes no body bytes, so retrying is
    /// idempotent. The classifier preserves the Responses API terminal
    /// classification and error messages exactly.
    async fn send_responses_request(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        api_url: &str,
        request_body: &Value,
        extension_headers: &HeaderMap,
        config: &LlmCallConfig,
        retries_consumed: u32,
    ) -> Result<(reqwest::Response, RetryMetadata)> {
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let mut retry_config = self.retry_config.clone();
        retry_config.max_retries = retry_config.max_retries.saturating_sub(retries_consumed);

        let body = serde_json::to_vec(request_body)
            .map_err(|e| AgentLoopError::llm(format!("failed to serialize request: {e}")))?;
        retry_request(
            &retry_config,
            "OpenResponsesProtocolDriver",
            || async {
                // Compose headers: provider decoration first, then the resolved
                // auth header (awaited each attempt so refreshable providers can
                // rotate tokens per retry). `insert` overrides any same-named
                // decoration header, so auth always wins on conflict. An auth
                // failure is fatal (no retry).
                let mut headers = extension_headers.clone();
                let service_headers = headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|value| (name.to_string(), value.to_string()))
                    })
                    .collect::<Vec<_>>();
                let resolved = endpoint
                    .resolve("POST", api_url, &body)
                    .await
                    .map_err(SendOutcome::Fatal)?;
                for (name, value) in service_headers.into_iter().chain(resolved.headers) {
                    let name =
                        reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
                            SendOutcome::Fatal(AgentLoopError::llm(format!(
                                "invalid header name: {e}"
                            )))
                        })?;
                    let mut value =
                        reqwest::header::HeaderValue::from_str(&value).map_err(|e| {
                            SendOutcome::Fatal(AgentLoopError::llm(format!(
                                "invalid header value: {e}"
                            )))
                        })?;
                    value.set_sensitive(true);
                    headers.insert(name, value);
                }

                // Caller-supplied per-request headers are applied last so they
                // override provider decoration and configured headers, matching
                // the `LlmCallConfig::extra_headers` contract.
                for (name, value) in
                    crate::driver_helpers::merge_request_headers(Vec::new(), &config.extra_headers)
                {
                    let name =
                        reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
                            SendOutcome::Fatal(AgentLoopError::llm(format!(
                                "invalid header name: {e}"
                            )))
                        })?;
                    let value = reqwest::header::HeaderValue::from_str(&value).map_err(|e| {
                        SendOutcome::Fatal(AgentLoopError::llm(format!(
                            "invalid header value: {e}"
                        )))
                    })?;
                    headers.insert(name, value);
                }

                self.client
                    .post(&resolved.url)
                    .headers(headers)
                    .header("Content-Type", "application/json")
                    .body(body.clone())
                    .send()
                    .await
                    .map_err(SendOutcome::Send)
            },
            |response, attempts, can_retry| {
                let last_error = Arc::clone(&last_error);
                let model = config.model.clone();
                async move {
                    let status = response.status();

                    if can_retry {
                        // Parse rate limit info from headers before consuming body.
                        let response_headers = response.headers().clone();
                        let mut rate_limit_info = if is_rate_limit_status(status) {
                            Some(RateLimitInfo::from_openai_headers(&response_headers))
                        } else {
                            None
                        };

                        let error_text = response.text().await.unwrap_or_default();
                        if let (Some(extension), Some(info)) =
                            (self.request_extension.as_ref(), rate_limit_info.as_mut())
                        {
                            extension.update_rate_limit_info(info, &response_headers, &error_text);
                        }

                        // Exhausted billing quota is surfaced as a 429 but is not
                        // transient — fail fast instead of burning retries.
                        if is_provider_quota_message(&error_text) {
                            return RetryDecision::Terminal(AgentLoopError::llm_kind(
                                LlmErrorKind::QuotaExhausted,
                                format!("OpenAI Responses API error ({}): {}", status, error_text),
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

                    // Check if this is a model-not-found error
                    if is_openai_model_not_found(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::model_not_available(model));
                    }

                    // Check if this is a request-too-large error (context length).
                    if is_openai_request_too_large(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::request_too_large(
                            format!("OpenAI Responses API ({}): {}", status, error_text),
                        ));
                    }

                    let error_msg =
                        format!("OpenAI Responses API error ({}): {}", status, error_text);

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

    /// Get the HTTP client (for subclass access)
    pub fn client(&self) -> &Client {
        &self.client
    }

    fn convert_role(role: &LlmMessageRole) -> &'static str {
        match role {
            LlmMessageRole::System => "developer", // Responses API uses "developer" for system
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        }
    }

    fn convert_message(msg: &LlmMessage, supports_phases: bool) -> ResponsesInputItem {
        // Handle tool result messages differently
        // Note: OpenAI Responses API function_call_output only supports text output.
        // Images in tool results are dropped with a warning.
        if msg.role == LlmMessageRole::Tool
            && let Some(tool_call_id) = &msg.tool_call_id
        {
            let mut has_images = false;
            let output = match &msg.content {
                LlmMessageContent::Text(text) => text.clone(),
                LlmMessageContent::Parts(parts) => {
                    has_images = parts
                        .iter()
                        .any(|p| matches!(p, LlmContentPart::Image { .. }));
                    parts
                        .iter()
                        .filter_map(|p| match p {
                            LlmContentPart::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("")
                }
            };
            if has_images {
                tracing::warn!(
                    tool_call_id = %tool_call_id,
                    "OpenResponses API does not support images in tool results; images dropped"
                );
            }
            return ResponsesInputItem::FunctionCallOutput {
                r#type: "function_call_output".to_string(),
                call_id: tool_call_id.clone(),
                output,
            };
        }

        let content = match &msg.content {
            LlmMessageContent::Text(text) => ResponsesContent::Text(text.clone()),
            LlmMessageContent::Parts(parts) => {
                let responses_parts: Vec<ResponsesContentPart> = parts
                    .iter()
                    .map(|part| match part {
                        LlmContentPart::Text { text } => ResponsesContentPart::InputText {
                            r#type: "input_text".to_string(),
                            text: text.clone(),
                        },
                        LlmContentPart::Image { url } => ResponsesContentPart::InputImage {
                            r#type: "input_image".to_string(),
                            image_url: url.clone(),
                        },
                        LlmContentPart::Audio { url } => ResponsesContentPart::InputAudio {
                            r#type: "input_audio".to_string(),
                            input_audio: ResponsesInputAudio {
                                data: url.clone(),
                                format: "wav".to_string(),
                            },
                        },
                    })
                    .collect();
                ResponsesContent::Parts(responses_parts)
            }
        };

        // Only include phase on assistant messages when the model supports it.
        // Map ExecutionPhase enum to the provider's wire format string.
        let phase = if supports_phases && msg.role == LlmMessageRole::Assistant {
            msg.phase.map(|p| p.as_provider_str().to_string())
        } else {
            None
        };

        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: Self::convert_role(&msg.role).to_string(),
            content,
            phase,
        }
    }

    /// Ensure an object-typed JSON Schema has a `properties` key.
    /// OpenAI rejects function schemas where `type: "object"` lacks `properties`.
    fn sanitize_parameters(params: &Value) -> Value {
        let mut p = crate::tool_schema_compat::sanitize_openai_tool_schema(params);
        if let Some(obj) = p.as_object_mut()
            && obj.get("type").and_then(|v| v.as_str()) == Some("object")
            && !obj.contains_key("properties")
        {
            obj.insert(
                "properties".to_string(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
        }
        p
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<ResponsesTool> {
        tools
            .iter()
            .map(|tool| Self::function_tool(tool, None))
            .collect()
    }

    fn function_tool(tool: &ToolDefinition, defer_loading: Option<bool>) -> ResponsesTool {
        let strict_parameters =
            crate::tool_schema_compat::strict_openai_tool_schema(tool.parameters());
        ResponsesTool::Function {
            r#type: "function".to_string(),
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: strict_parameters
                .clone()
                .unwrap_or_else(|| Self::sanitize_parameters(tool.parameters())),
            strict: strict_parameters.as_ref().map(|_| true),
            defer_loading,
        }
    }

    /// Convert tools with tool_search support: groups tools into namespaces,
    /// marks them as deferred, and appends a `tool_search` entry.
    fn convert_tools_with_search(tools: &[ToolDefinition], threshold: usize) -> Vec<ResponsesTool> {
        use crate::tool_types::DeferrablePolicy;
        use std::collections::BTreeMap;

        // Below threshold: fall back to standard conversion
        if tools.len() < threshold {
            return Self::convert_tools(tools);
        }

        // Stable namespace order also keeps the serialized prompt-cache fingerprint stable.
        let mut namespaces: BTreeMap<String, Vec<ResponsesTool>> = BTreeMap::new();
        let mut ungrouped = vec![];
        let mut never_defer = vec![];

        for tool in tools {
            let should_defer = match tool.deferrable() {
                DeferrablePolicy::Never => false,
                DeferrablePolicy::Automatic | DeferrablePolicy::Always => true,
            };

            let func = Self::function_tool(tool, if should_defer { Some(true) } else { None });

            if !should_defer {
                never_defer.push(func);
            } else {
                match tool.category() {
                    Some(cat) => {
                        namespaces.entry(cat.to_string()).or_default().push(func);
                    }
                    None => ungrouped.push(func),
                }
            }
        }

        let mut result: Vec<ResponsesTool> = Vec::new();

        // Non-deferred tools first (always visible to model)
        result.extend(never_defer);

        // Namespaced tools
        for (name, tools) in namespaces {
            let description = format!("Tools for {name}");
            result.push(ResponsesTool::Namespace {
                r#type: "namespace".to_string(),
                name,
                description,
                tools,
            });
        }

        // Ungrouped deferred tools
        result.extend(ungrouped);

        // Add tool_search activator
        result.push(ResponsesTool::ToolSearch {
            r#type: "tool_search".to_string(),
        });

        result
    }

    fn build_prompt_cache_key(
        config: &LlmCallConfig,
        _input_items: &[ResponsesInputItem],
        instructions: &Option<String>,
        tools: &Option<Vec<ResponsesTool>>,
    ) -> Option<String> {
        let prompt_cache = config.prompt_cache.as_ref().filter(|cfg| cfg.enabled)?;
        let cache_family = config
            .metadata
            .get("session_id")
            .or_else(|| config.metadata.get("agent_id"))
            .or_else(|| config.metadata.get("harness_id"))
            .or_else(|| config.metadata.get("org_id"));
        let fingerprint = json!({
            "strategy": prompt_cache.strategy,
            "model": config.model,
            "cache_family": cache_family,
            "instructions": instructions,
            "tools": tools,
        });
        let payload = serde_json::to_vec(&fingerprint).ok()?;
        let digest = hex::encode(Sha256::digest(payload));
        let digest_len = OPENAI_PROMPT_CACHE_KEY_MAX_LEN - PROMPT_CACHE_KEY_PREFIX.len();
        Some(format!(
            "{PROMPT_CACHE_KEY_PREFIX}{}",
            &digest[..digest_len]
        ))
    }

    /// Compact a conversation to reduce context size
    ///
    /// This method calls the /v1/responses/compact endpoint to compress the conversation
    /// history. User messages are kept verbatim, while assistant messages, tool calls,
    /// and tool results are replaced by an encrypted compaction item.
    ///
    /// # Arguments
    ///
    /// * `request` - The compact request containing the model and input items
    ///
    /// # Returns
    ///
    /// Returns a `CompactResponse` containing the compacted output items.
    /// The output can be used directly as input for the next /v1/responses call.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_provider::{OpenResponsesProtocolChatDriver, CompactRequest, CompactInputItem, CompactContent};
    ///
    /// let driver = OpenResponsesProtocolChatDriver::new();
    ///
    /// let request = CompactRequest {
    ///     model: "gpt-5.2".to_string(),
    ///     input: vec![
    ///         CompactInputItem::Message {
    ///             role: "user".to_string(),
    ///             content: CompactContent::Text("Hello!".to_string()),
    ///         },
    ///     ],
    ///     previous_response_id: None,
    ///     instructions: None,
    /// };
    ///
    /// let response = driver.compact(request).await?;
    /// // Use response.output as input for the next /v1/responses call
    /// ```
    pub async fn compact(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<CompactResponse> {
        // Build the compact endpoint URL
        // Replace /v1/responses with /v1/responses/compact
        let responses_url = endpoint.url("responses").ok_or_else(|| {
            AgentLoopError::Configuration("Open Responses provider has no base URL".to_string())
        })?;
        let compact_url = if responses_url.ends_with("/responses") {
            format!("{responses_url}/compact")
        } else if responses_url.ends_with("/responses/") {
            format!("{responses_url}compact")
        } else {
            // Custom URL - just append /compact
            format!("{}/compact", responses_url.trim_end_matches('/'))
        };
        let body = serde_json::to_vec(&request).map_err(|e| {
            AgentLoopError::llm(format!("failed to serialize compact request: {e}"))
        })?;

        // Retry loop for rate limit (429) and transient errors. Shared executor
        // owns the loop/backoff/send-error retry/exhaustion logging; the
        // classifier preserves the compact endpoint's terminal classification
        // and (compact-specific) error messages exactly.
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let (response, _retry_metadata) = retry_request(
            &self.retry_config,
            "OpenResponsesProtocolDriver(compact)",
            || async {
                // Auth is resolved per attempt so refreshable providers can
                // rotate tokens across retries (same seam as the streaming path).
                let resolved = endpoint
                    .resolve("POST", &compact_url, &body)
                    .await
                    .map_err(SendOutcome::Fatal)?;
                let mut builder = self.client.post(&resolved.url);
                for (name, value) in resolved.headers {
                    builder = builder.header(name, value);
                }
                builder
                    .header("Content-Type", "application/json")
                    .body(body.clone())
                    .send()
                    .await
                    .map_err(SendOutcome::Send)
            },
            |response, attempts, can_retry| {
                let last_error = Arc::clone(&last_error);
                let request_model = request.model.clone();
                async move {
                    let status = response.status();

                    if can_retry {
                        let response_headers = response.headers().clone();
                        let mut rate_limit_info = if is_rate_limit_status(status) {
                            Some(RateLimitInfo::from_openai_headers(&response_headers))
                        } else {
                            None
                        };

                        let error_text = response.text().await.unwrap_or_default();
                        if let (Some(extension), Some(info)) =
                            (self.request_extension.as_ref(), rate_limit_info.as_mut())
                        {
                            extension.update_rate_limit_info(info, &response_headers, &error_text);
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

                    // Check if this is a model-not-found error
                    if is_openai_model_not_found(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::model_not_available(
                            request_model,
                        ));
                    }

                    // Check if this is a request-too-large error (context length).
                    if is_openai_request_too_large(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::request_too_large(
                            format!("OpenAI Responses compact API ({}): {}", status, error_text),
                        ));
                    }

                    let error_msg = format!(
                        "OpenAI Responses compact API error ({}): {}",
                        status, error_text
                    );

                    if attempts > 0 {
                        return RetryDecision::Terminal(AgentLoopError::llm(format!(
                            "{} (after {} retries, last error: {})",
                            error_msg,
                            attempts,
                            last_error.lock().unwrap().take().unwrap_or_default()
                        )));
                    }

                    RetryDecision::Terminal(AgentLoopError::llm(error_msg))
                }
            },
            |e, attempts| {
                let suffix = if attempts > 0 {
                    format!(" (after {attempts} retries)")
                } else {
                    String::new()
                };
                AgentLoopError::llm(format!("Failed to send compact request: {e}{suffix}"))
            },
        )
        .await?;

        // Parse the response
        let compact_response: CompactResponse = response
            .json()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to parse compact response: {}", e)))?;

        Ok(compact_response)
    }

    /// Check if this driver supports the compact endpoint
    ///
    /// Returns true for OpenAI's Responses API. Custom endpoints may or may not
    /// support compaction.
    pub fn supports_compact(&self) -> bool {
        true
    }

    /// Build input items from messages, extracting system/developer instructions
    ///
    /// Handles the conversion of:
    /// - Assistant messages with tool_calls into separate FunctionCall items
    /// - Assistant messages with thinking into Reasoning items (for o-series/GPT-5 models)
    ///
    /// Note: this function always reconstructs the FULL transcript from the supplied
    /// messages. The caller is responsible for trimming to a delta window when a
    /// `previous_response_id` is in play — see [`compute_delta_input_items`]. The
    /// stateful Responses invariant is: a request must not mix `previous_response_id`
    /// with prior transcript input the provider already holds server-side.
    fn build_input(
        messages: &[LlmMessage],
        supports_phases: bool,
    ) -> (Option<String>, Vec<ResponsesInputItem>) {
        // Accumulate all system messages into `instructions`. Multiple system
        // messages legitimately occur in one request — the agent system prompt
        // plus, e.g., infinity context's hidden-history notice or compaction's
        // conversation summary. Overwriting would drop the real system prompt and
        // keep only the last notice. See `fold_system_messages`.
        let instructions: Option<String> = fold_system_messages(messages);
        let mut input_items = Vec::new();

        for msg in messages {
            if msg.role == LlmMessageRole::System {
                // Folded above into `instructions`; never emit the System message
                // as a separate input item.
            } else if msg.role == LlmMessageRole::Assistant {
                // Reasoning items precede the message content they belong to,
                // as the API requires for o-series and GPT-5 models.
                //
                // Every item is replayed, not just the last: a turn with
                // parallel tool calls emits several, and each is keyed by the
                // `rs_…` id OpenAI issued. Items without that id, or without
                // encrypted content, are dropped rather than reconstructed —
                // a synthesized id is not one the API can resolve.
                for item in &msg.reasoning {
                    let (Some(id), Some(encrypted_content)) = (&item.item_id, &item.encrypted)
                    else {
                        tracing::debug!(
                            provider = %item.provider,
                            has_id = item.item_id.is_some(),
                            has_encrypted = item.encrypted.is_some(),
                            "OpenResponses: skipping reasoning item without a replayable id/payload"
                        );
                        continue;
                    };
                    // Replay the curated summary the provider gave us, when it
                    // gave one. `summary` is required either way.
                    let summary = match &item.text {
                        Some(crate::reasoning::ReasoningText::Summary { parts }) => parts
                            .iter()
                            .map(|text| types::ContentPart::SummaryText { text: text.clone() })
                            .collect(),
                        _ => Vec::new(),
                    };
                    input_items.push(ResponsesInputItem::Reasoning {
                        r#type: "reasoning".to_string(),
                        id: id.clone(),
                        encrypted_content: encrypted_content.clone(),
                        summary,
                    });
                    tracing::debug!(
                        item_id = %id,
                        encrypted_len = encrypted_content.len(),
                        "OpenResponses: including reasoning item in request"
                    );
                }

                // Handle tool calls
                if msg.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) {
                    // First emit the message content if non-empty
                    let has_content = match &msg.content {
                        LlmMessageContent::Text(text) => !text.is_empty(),
                        LlmMessageContent::Parts(parts) => !parts.is_empty(),
                    };
                    if has_content {
                        input_items.push(Self::convert_message(msg, supports_phases));
                    }

                    // Then emit FunctionCall items for each tool call
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            input_items.push(ResponsesInputItem::FunctionCall {
                                r#type: "function_call".to_string(),
                                call_id: tc.id.clone(),
                                name: tc.name.clone(),
                                arguments: tc.arguments.to_string(),
                            });
                        }
                    }
                } else {
                    input_items.push(Self::convert_message(msg, supports_phases));
                }
            } else {
                input_items.push(Self::convert_message(msg, supports_phases));
            }
        }

        (instructions, input_items)
    }
}

impl Default for OpenResponsesProtocolChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Trim input items to the "delta" window for a stateful Responses continuation.
///
/// When a request carries `previous_response_id`, OpenAI already holds the prior
/// transcript server-side. Re-sending it in `input` double-counts context (charges
/// the user twice and inflates prompt-cache keys). The invariant is:
///
///   **A request must not mix `previous_response_id` with prior transcript input.**
///
/// "Delta" is everything strictly after the last item that belonged to a prior
/// assistant turn. Items that belong to a prior assistant turn are: assistant
/// `Message`, `Reasoning`, and `FunctionCall` (the assistant's own tool calls).
/// What remains as delta is typically `FunctionCallOutput` items (tool results
/// the client produced) plus any fresh user `Message`s.
///
/// Defensive behavior: if no prior-assistant item is found (e.g., the caller
/// passed only fresh user input), all items are treated as delta and kept. An
/// empty input is also valid — the provider can resume purely from
/// `previous_response_id`.
fn compute_delta_input_items(items: Vec<ResponsesInputItem>) -> Vec<ResponsesInputItem> {
    // Find the index of the last item that is part of a prior assistant turn.
    let last_assistant_turn_idx = items
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, item)| match item {
            ResponsesInputItem::Message { role, .. } if role == "assistant" => Some(i),
            ResponsesInputItem::Reasoning { .. } => Some(i),
            ResponsesInputItem::FunctionCall { .. } => Some(i),
            _ => None,
        });

    match last_assistant_turn_idx {
        Some(idx) => items.into_iter().skip(idx + 1).collect(),
        // No prior-assistant items in input — defensive: keep all items as delta.
        None => items,
    }
}

/// The single decision point for whether a Responses request `input` should be
/// trimmed to the delta window. Extracted so the call path can be regression-tested
/// without spinning up an HTTP mock — protects against accidentally removing the
/// `previous_response_id.is_some()` guard that enforces the stateful invariant.
fn finalize_input_for_request(
    input_items: Vec<ResponsesInputItem>,
    previous_response_id: &Option<String>,
) -> Vec<ResponsesInputItem> {
    if previous_response_id.is_some() {
        compute_delta_input_items(input_items)
    } else {
        repair_unpaired_function_call_items(input_items)
    }
}

/// Find `call_id`s that break the OpenAI/Codex Responses tool-pairing invariant
/// for a stateless full-replay `input`: a serialized `function_call` with no
/// matching `function_call_output` (EVE-597) or a `function_call_output` with
/// no matching `function_call` (EVE-519). An empty result means the input is
/// protocol-valid in both directions.
fn unpaired_function_call_ids(items: &[ResponsesInputItem]) -> Vec<String> {
    let call_ids: HashSet<&str> = items
        .iter()
        .filter_map(|item| match item {
            ResponsesInputItem::FunctionCall { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    let output_ids: HashSet<&str> = items
        .iter()
        .filter_map(|item| match item {
            ResponsesInputItem::FunctionCallOutput { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();

    items
        .iter()
        .filter_map(|item| match item {
            ResponsesInputItem::FunctionCall { call_id, .. }
                if !output_ids.contains(call_id.as_str()) =>
            {
                Some(call_id.clone())
            }
            ResponsesInputItem::FunctionCallOutput { call_id, .. }
                if !call_ids.contains(call_id.as_str()) =>
            {
                Some(call_id.clone())
            }
            _ => None,
        })
        .collect()
}

/// Repair a stateless full-replay Responses `input` so every `function_call` is
/// paired with its `function_call_output` and vice versa.
///
/// OpenAI/Codex Responses reject requests that contain a `function_call`
/// without a matching `function_call_output` ("No tool output found for
/// function call …", EVE-597) or a `function_call_output` without a matching
/// `function_call` ("No tool call found for function call output", EVE-519).
/// Long-session compaction / model-view masking can evict one side of a pair —
/// e.g. `keep_recent_tool_outputs = 3` drops an old tool result while its
/// assistant `function_call` survives — leaving the serialized request
/// protocol-invalid and producing a permanent 400 on every continuation.
///
/// Tool-call pairs are atomic here: when only one side survives we drop both so
/// the request stays valid rather than 400ing at the provider. Dropped dangling
/// items are logged with their `call_id` to point at the responsible
/// compaction/serialization stage.
fn repair_unpaired_function_call_items(
    input_items: Vec<ResponsesInputItem>,
) -> Vec<ResponsesInputItem> {
    let unpaired: HashSet<String> = unpaired_function_call_ids(&input_items)
        .into_iter()
        .collect();

    if unpaired.is_empty() {
        return input_items;
    }

    tracing::warn!(
        unpaired_call_ids = ?unpaired,
        "dropping unpaired function_call / function_call_output items before \
         stateless Responses replay; one side of the pair was likely evicted by \
         compaction or model-view masking (EVE-597/EVE-519)"
    );

    input_items
        .into_iter()
        .filter(|item| match item {
            ResponsesInputItem::FunctionCall { call_id, .. }
            | ResponsesInputItem::FunctionCallOutput { call_id, .. } => {
                !unpaired.contains(call_id.as_str())
            }
            _ => true,
        })
        .collect()
}

fn is_missing_tool_output_continuation_error(error: &AgentLoopError) -> bool {
    if !matches!(error.llm_error_kind(), Some(LlmErrorKind::InvalidRequest)) {
        return false;
    }
    let message = error.to_string().to_ascii_lowercase();
    message.contains("no tool output found for function call")
        || message.contains("no tool call found for function call output")
}

#[async_trait]
impl ChatDriver for OpenResponsesProtocolChatDriver {
    fn supports_stateful_responses(&self) -> bool {
        self.stateful_responses.unwrap_or(false)
    }

    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let api_url = endpoint.url("responses").ok_or_else(|| {
            AgentLoopError::Configuration("Open Responses provider has no base URL".to_string())
        })?;
        // Check the provider-specific model profile before sending native
        // Responses features. OpenAI-compatible gateways may share base model
        // metadata without supporting OpenAI-only extensions such as phases or
        // hosted tool_search.
        let supports_phases = self.native_phases;
        let supports_tool_search = self.hosted_tool_search;

        let (instructions, transcript_input_items) = Self::build_input(&messages, supports_phases);
        let full_replay_input_items = transcript_input_items.clone();

        // Only chain via `previous_response_id` when the endpoint actually persists
        // responses server-side. Stateless OpenAI-compatible gateways (OpenRouter,
        // Gemini compat, …) accept the field but ignore it, so chaining there drops
        // the conversation from turn 2 onward (EVE-523). For those we send no
        // continuation handle and replay the full transcript in `input` below.
        let mut previous_response_id = if self.stateful_responses.unwrap_or(false) {
            config.previous_response_id.clone()
        } else {
            None
        };

        // Native compact output replaces history through its durable source
        // boundary. Messages supplied here are the raw suffix written after that
        // boundary, so append them without rewriting or trimming the checkpoint.
        // This is mutually exclusive with server-side continuation state.
        let input_items = match &config.provider_opaque_context {
            Some(crate::driver_registry::ProviderOpaqueContext::OpenResponsesCompact {
                output,
            }) => {
                previous_response_id = None;
                let mut input_items: Vec<_> = output.iter().map(ResponsesInputItem::from).collect();
                input_items.extend(transcript_input_items);
                input_items
            }
            None => finalize_input_for_request(transcript_input_items, &previous_response_id),
        };

        let tools = if config.tools.is_empty() {
            None
        } else if let Some(ref ts_config) = config.tool_search {
            if ts_config.enabled && supports_tool_search {
                Some(Self::convert_tools_with_search(
                    &config.tools,
                    ts_config.threshold,
                ))
            } else {
                Some(Self::convert_tools(&config.tools))
            }
        } else {
            Some(Self::convert_tools(&config.tools))
        };

        // Build reasoning config if specified.
        // Skip when effort is "none" — sending reasoning params to models that
        // don't support them (or with effort=none) causes OpenAI API errors.
        let reasoning = config
            .reasoning_effort
            .filter(crate::model::ReasoningEffort::requests_reasoning)
            .map(|effort| ResponsesReasoning {
                effort: effort.as_str().to_string(),
                summary: "detailed".to_string(),
            });

        // Reasoning items are only replayable when the provider hands back
        // their encrypted payload, and it only does so on request.
        let include = reasoning
            .is_some()
            .then(|| vec!["reasoning.encrypted_content".to_string()]);

        // Build metadata for request tracking
        let metadata = if config.metadata.is_empty() {
            None
        } else {
            Some(config.metadata.clone())
        };
        let prompt_cache_key =
            Self::build_prompt_cache_key(config, &input_items, &instructions, &tools);
        let mut request = ResponsesRequest {
            model: config.model.clone(),
            input: input_items,
            instructions,
            previous_response_id,
            temperature: config.temperature,
            max_output_tokens: config.max_tokens,
            stream: true,
            tools,
            reasoning,
            metadata,
            prompt_cache_key,
            parallel_tool_calls: config
                .resolved_parallel_tool_calls(self.supports_parallel_tool_calls(&config.model)),
            service_tier: config.speed.clone(),
            text: config.verbosity.clone().map(|verbosity| ResponsesText {
                verbosity: Some(verbosity),
            }),
            include,
        };

        // Log request details for debugging LLM errors.
        // Only log request shape to avoid leaking prompt or metadata contents.
        {
            let tool_count = request.tools.as_ref().map_or(0, |t| t.len());
            let input_count = request.input.len();
            let has_instructions = request.instructions.is_some();
            let has_reasoning = request.reasoning.is_some();
            let has_previous_response = request.previous_response_id.is_some();
            tracing::debug!(
                model = %request.model,
                input_items = input_count,
                tool_count = tool_count,
                has_instructions = has_instructions,
                has_reasoning = has_reasoning,
                has_previous_response = has_previous_response,
                api_url = %api_url,
                "OpenResponsesDriver: sending request"
            );
        }

        // Serialize the vendor-neutral request, then let any provider-specific
        // extension (e.g. OpenRouter) layer extra fields and headers onto it.
        let mut request_body = serde_json::to_value(&request)
            .map_err(|e| AgentLoopError::llm(format!("Failed to serialize request: {}", e)))?;
        if let Some(extension) = &self.request_extension {
            extension.decorate(&mut request_body, config)?;
        }
        let mut extension_headers = HeaderMap::new();
        if let Some(extension) = &self.request_extension {
            extension.decorate_headers(&mut extension_headers, config)?;
        }

        // Establish the SSE stream, transparently reconnecting on a transport
        // failure that lands before the first event is decoded (the "error
        // decoding response body" flake). Header-phase retries (429/5xx and
        // transient send failures) are handled inside the per-attempt send.
        let first_connect = connect_sse_with_reconnect(
            &self.retry_config,
            "OpenResponsesProtocolDriver",
            |attempts| {
                self.send_responses_request(
                    endpoint,
                    &api_url,
                    &request_body,
                    &extension_headers,
                    config,
                    attempts,
                )
            },
        )
        .await;
        let (event_stream, retry_metadata) = match first_connect {
            Ok(connected) => connected,
            Err(error)
                if request.previous_response_id.is_some()
                    && is_missing_tool_output_continuation_error(&error) =>
            {
                // The provider lost or rejected its continuation state. The
                // rejected 400 executed no tools, so safely retry once without
                // the opaque handle and replay the locally complete transcript.
                // Full replay runs the same pair repair used by ordinary
                // stateless requests, preserving completed tool outputs without
                // re-running their side effects.
                tracing::warn!(
                    model = %request.model,
                    "stateful Responses continuation rejected for missing tool output; retrying once with repaired stateless replay"
                );
                request.previous_response_id = None;
                request.input = repair_unpaired_function_call_items(full_replay_input_items);
                request.prompt_cache_key = Self::build_prompt_cache_key(
                    config,
                    &request.input,
                    &request.instructions,
                    &request.tools,
                );
                request_body = serde_json::to_value(&request).map_err(|e| {
                    AgentLoopError::llm(format!("Failed to serialize recovery request: {e}"))
                })?;
                if let Some(extension) = &self.request_extension {
                    extension.decorate(&mut request_body, config)?;
                }
                connect_sse_with_reconnect(
                    &self.retry_config,
                    "OpenResponsesProtocolDriver",
                    |attempts| {
                        self.send_responses_request(
                            endpoint,
                            &api_url,
                            &request_body,
                            &extension_headers,
                            config,
                            attempts,
                        )
                    },
                )
                .await?
            }
            Err(error) => return Err(error),
        };

        let model = config.model.clone();
        let input_tokens = Arc::new(Mutex::new(0u32));
        let output_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCallAccumulator>::new()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));
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
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            let finish_reason = Arc::clone(&finish_reason);
            let retry_metadata_for_done = shared_retry_metadata.clone();

            async move {
                match result {
                    Ok(event) => {
                        let event_data = &event.data;

                        // OpenAI-compatible gateways (e.g. OpenRouter) terminate the
                        // Responses SSE stream with a chat-completions-style `[DONE]`
                        // sentinel, which OpenAI's native Responses API does not send.
                        // It is not JSON, so skip it instead of surfacing a spurious
                        // "Failed to parse event" error after the real completion.
                        if event_data == "[DONE]" {
                            return Ok(LlmStreamEvent::TextDelta(String::new()));
                        }

                        // Try to parse as typed StreamingEvent first for type safety
                        if let Ok(streaming_event) =
                            serde_json::from_str::<StreamingEvent>(event_data)
                        {
                            return Ok(handle_streaming_event(
                                streaming_event,
                                &input_tokens,
                                &output_tokens,
                                &cache_read_tokens,
                                &accumulated_tool_calls,
                                &finish_reason,
                                model,
                                retry_metadata_for_done,
                            ));
                        }

                        // Fallback: parse as generic JSON for backwards compatibility
                        let parsed: std::result::Result<Value, _> =
                            serde_json::from_str(event_data);

                        match parsed {
                            Ok(json) => {
                                let event_type = json.get("type").and_then(|t| t.as_str());

                                match event_type {
                                    Some("response.output_text.delta") => {
                                        // Text delta
                                        if let Some(delta) =
                                            json.get("delta").and_then(|d| d.as_str())
                                        {
                                            Ok(LlmStreamEvent::TextDelta(delta.to_string()))
                                        } else {
                                            Ok(LlmStreamEvent::TextDelta(String::new()))
                                        }
                                    }

                                    Some("response.function_call_arguments.delta") => {
                                        // Function call arguments delta
                                        if let (Some(item_id), Some(delta)) = (
                                            json.get("item_id").and_then(|c| c.as_str()),
                                            json.get("delta").and_then(|d| d.as_str()),
                                        ) {
                                            let mut acc = accumulated_tool_calls.lock().unwrap();
                                            // Find or create accumulator for this item_id
                                            if let Some(tc) =
                                                acc.iter_mut().find(|t| t.id == item_id)
                                            {
                                                tc.arguments.push_str(delta);
                                            } else {
                                                acc.push(ToolCallAccumulator {
                                                    id: item_id.to_string(),
                                                    call_id: String::new(),
                                                    name: String::new(),
                                                    arguments: delta.to_string(),
                                                });
                                            }
                                        }
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }

                                    Some("response.output_item.added") => {
                                        // New output item added - may be a function
                                        // call or an assistant message carrying a
                                        // native phase.
                                        let item_type = json
                                            .get("item")
                                            .and_then(|i| i.get("type"))
                                            .and_then(|t| t.as_str());
                                        if item_type == Some("function_call") {
                                            let item = json.get("item").unwrap();
                                            let id = item
                                                .get("id")
                                                .and_then(|c| c.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let call_id = item
                                                .get("call_id")
                                                .and_then(|c| c.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let name = item
                                                .get("name")
                                                .and_then(|n| n.as_str())
                                                .unwrap_or("")
                                                .to_string();

                                            let mut acc = accumulated_tool_calls.lock().unwrap();
                                            if let Some(tc) = acc.iter_mut().find(|t| t.id == id) {
                                                tc.name = name;
                                                tc.call_id = call_id;
                                            } else {
                                                acc.push(ToolCallAccumulator {
                                                    id,
                                                    call_id,
                                                    name,
                                                    arguments: String::new(),
                                                });
                                            }
                                        } else if item_type == Some("message") {
                                            // Surface the assistant item's native
                                            // phase mid-stream as a best-effort hint
                                            // (EVE-774); Done metadata stays
                                            // authoritative.
                                            if let Some(phase) = json
                                                .get("item")
                                                .and_then(|i| i.get("phase"))
                                                .and_then(|p| p.as_str())
                                                .and_then(
                                                    crate::execution_phase::ExecutionPhase::from_provider_str,
                                                )
                                            {
                                                return Ok(LlmStreamEvent::MessagePhase(phase));
                                            }
                                        }
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }

                                    Some("response.output_item.done") => {
                                        // Output item completed - check if it's a function call
                                        if let Some(item) = json.get("item")
                                            && item.get("type").and_then(|t| t.as_str())
                                                == Some("function_call")
                                        {
                                            // Function call completed, emit ToolCalls event
                                            let acc = accumulated_tool_calls.lock().unwrap();
                                            if !acc.is_empty() {
                                                let tool_calls: Vec<ToolCall> = acc
                                                    .iter()
                                                    .filter(|tc| !tc.name.is_empty())
                                                    .map(|tc| {
                                                        let arguments: Value =
                                                            serde_json::from_str(&tc.arguments)
                                                                .unwrap_or(json!({}));
                                                        ToolCall {
                                                            id: tc.call_id.clone(),
                                                            name: tc.name.clone(),
                                                            arguments,
                                                        }
                                                    })
                                                    .collect();

                                                if !tool_calls.is_empty() {
                                                    *finish_reason.lock().unwrap() =
                                                        Some("tool_calls".to_string());
                                                    return Ok(LlmStreamEvent::ToolCalls(
                                                        tool_calls,
                                                    ));
                                                }
                                            }
                                        }
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }

                                    Some("response.completed")
                                    | Some("response.incomplete")
                                    | Some("response.done") => {
                                        // Response completed - extract usage
                                        let response_obj = json.get("response").unwrap_or(&json);

                                        // Authoritative per-request cost from OpenAI-compatible
                                        // gateways (e.g. OpenRouter `usage.cost`, in USD credits).
                                        let mut provider_cost_usd: Option<f64> = None;
                                        if let Some(usage) = response_obj.get("usage") {
                                            if let Some(input) =
                                                usage.get("input_tokens").and_then(|t| t.as_u64())
                                            {
                                                *input_tokens.lock().unwrap() = input as u32;
                                            }
                                            if let Some(output) =
                                                usage.get("output_tokens").and_then(|t| t.as_u64())
                                            {
                                                *output_tokens.lock().unwrap() = output as u32;
                                            }
                                            // Check for cached tokens
                                            if let Some(details) = usage.get("input_tokens_details")
                                                && let Some(cached) = details
                                                    .get("cached_tokens")
                                                    .and_then(|t| t.as_u64())
                                            {
                                                *cache_read_tokens.lock().unwrap() =
                                                    Some(cached as u32);
                                            }
                                            provider_cost_usd =
                                                usage.get("cost").and_then(|c| c.as_f64());
                                        }

                                        // Determine finish reason from status
                                        let status = response_obj
                                            .get("status")
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("completed");

                                        let reason = match status {
                                            "completed" => {
                                                // Check if there were tool calls
                                                let existing_reason =
                                                    finish_reason.lock().unwrap().clone();
                                                existing_reason
                                                    .unwrap_or_else(|| "stop".to_string())
                                            }
                                            "failed" => {
                                                let error_detail = response_obj
                                                    .get("error")
                                                    .map(|e| e.to_string())
                                                    .unwrap_or_else(|| "no error detail".into());
                                                tracing::warn!(
                                                    response_error = %error_detail,
                                                    "OpenResponsesDriver: response completed with 'failed' status (fallback parser)"
                                                );
                                                "error".to_string()
                                            }
                                            "incomplete" => response_obj
                                                .get("incomplete_details")
                                                .and_then(|details| details.get("reason"))
                                                .and_then(|reason| reason.as_str())
                                                .map(|reason| match reason {
                                                    "max_output_tokens" | "max_tokens" => "length",
                                                    other => other,
                                                })
                                                .unwrap_or("stop")
                                                .to_string(),
                                            "cancelled" => "cancelled".to_string(),
                                            _ => "stop".to_string(),
                                        };

                                        // Extract phase from the last assistant message in output items
                                        let phase = response_obj
                                            .get("output")
                                            .and_then(|o| o.as_array())
                                            .and_then(|items| {
                                                items.iter().rev().find_map(|item| {
                                                    if item.get("type")?.as_str()? == "message"
                                                        && item.get("role")?.as_str()?
                                                            == "assistant"
                                                    {
                                                        item.get("phase")?
                                                            .as_str()
                                                            .map(String::from)
                                                    } else {
                                                        None
                                                    }
                                                })
                                            });

                                        let input = *input_tokens.lock().unwrap();
                                        let output = *output_tokens.lock().unwrap();
                                        let cached = *cache_read_tokens.lock().unwrap();

                                        Ok(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                                            // `input` is OpenAI's cache-inclusive prompt count;
                                            // normalize to non-cached input (disjoint convention).
                                            total_tokens: Some(input + output),
                                            prompt_tokens: Some(disjoint_prompt_tokens(input, cached)),
                                            completion_tokens: Some(output),
                                            cache_read_tokens: cached,
                                            cache_creation_tokens: None,
                                            provider_cost_usd,
                                            model: Some(model),
                                            finish_reason: Some(reason),
                                            retry_metadata: retry_metadata_for_done
                                                .map(|arc| (*arc).clone()),
                                            response_id: None,
                                            phase,
                                            cache_diagnostics: None,
                                        })))
                                    }

                                    Some("error") => {
                                        // Error event (fallback JSON path)
                                        let error_code = json
                                            .get("error")
                                            .and_then(|e| e.get("code"))
                                            .and_then(|c| c.as_str())
                                            .unwrap_or("unknown");
                                        let error_msg = json
                                            .get("error")
                                            .and_then(|e| e.get("message"))
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("Unknown error");
                                        tracing::warn!(
                                            error_code = error_code,
                                            error_message = error_msg,
                                            raw_error = %json.get("error").unwrap_or(&json),
                                            "OpenResponsesDriver: received streaming error event (fallback parser)"
                                        );
                                        Ok(LlmStreamEvent::Error(
                                            crate::driver_registry::LlmStreamError::provider(
                                                (error_code != "unknown")
                                                    .then_some(error_code.to_string()),
                                                None,
                                                error_msg,
                                            ),
                                        ))
                                    }

                                    _ => {
                                        // Other event types - ignore
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }
                                }
                            }
                            Err(e) => Ok(LlmStreamEvent::Error(
                                format!("Failed to parse event: {}", e).into(),
                            )),
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

    fn supports_compact(&self) -> bool {
        // Delegate to the inherent method
        OpenResponsesProtocolChatDriver::supports_compact(self)
    }

    /// The Responses API accepts the top-level `parallel_tool_calls` boolean.
    fn supports_parallel_tool_calls(&self, _model: &str) -> bool {
        true
    }

    async fn compact(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: crate::openresponses_protocol::CompactRequest,
    ) -> Result<Option<crate::openresponses_protocol::CompactResponse>> {
        // Delegate to the inherent method and wrap in Some
        Ok(Some(
            OpenResponsesProtocolChatDriver::compact(self, endpoint, request).await?,
        ))
    }
}

impl std::fmt::Debug for OpenResponsesProtocolChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenResponsesProtocolChatDriver")
            .field("stateful_responses", &self.stateful_responses)
            .field("native_phases", &self.native_phases)
            .field("hosted_tool_search", &self.hosted_tool_search)
            .finish_non_exhaustive()
    }
}

// ============================================================================
// Helper Types
// ============================================================================

/// Accumulator for tool call arguments during streaming
#[derive(Clone, Default)]
struct ToolCallAccumulator {
    /// Item ID in the stream
    id: String,
    /// Unique call ID for the function call
    call_id: String,
    /// Function name
    name: String,
    /// Accumulated JSON arguments
    arguments: String,
}

/// Handle typed streaming events from the OpenResponses API
#[allow(clippy::too_many_arguments)]
fn handle_streaming_event(
    event: StreamingEvent,
    input_tokens: &Mutex<u32>,
    output_tokens: &Mutex<u32>,
    cache_read_tokens: &Mutex<Option<u32>>,
    accumulated_tool_calls: &Mutex<Vec<ToolCallAccumulator>>,
    finish_reason: &Mutex<Option<String>>,
    model: String,
    retry_metadata: Option<Arc<RetryMetadata>>,
) -> LlmStreamEvent {
    match event {
        StreamingEvent::OutputTextDelta { delta, .. } => LlmStreamEvent::TextDelta(delta),

        StreamingEvent::ReasoningDelta { delta, .. } => LlmStreamEvent::ReasoningDelta {
            delta,
            summary: false,
        },

        StreamingEvent::ReasoningTextDelta { delta, .. } => LlmStreamEvent::ReasoningDelta {
            delta,
            summary: false,
        },

        StreamingEvent::ReasoningSummaryDelta { delta, .. } => {
            // A reasoning summary is a reasoning artifact, so it belongs on the
            // reasoning channel — flagged as a summary rather than raw
            // chain-of-thought. It must not become assistant text: that would
            // persist it as the model's answer and replay it as the model's own
            // prior output. See `knowledge/execution/events.md`.
            LlmStreamEvent::ReasoningDelta {
                delta,
                summary: true,
            }
        }

        StreamingEvent::FunctionCallArgumentsDelta { item_id, delta, .. } => {
            let mut acc = accumulated_tool_calls.lock().unwrap();
            if let Some(tc) = acc.iter_mut().find(|t| t.id == item_id) {
                tc.arguments.push_str(&delta);
            } else {
                acc.push(ToolCallAccumulator {
                    id: item_id,
                    call_id: String::new(),
                    name: String::new(),
                    arguments: delta,
                });
            }
            LlmStreamEvent::TextDelta(String::new())
        }

        StreamingEvent::OutputItemAdded { item, .. } => {
            match item {
                Some(types::OutputItem::FunctionCall {
                    id, call_id, name, ..
                }) => {
                    let mut acc = accumulated_tool_calls.lock().unwrap();
                    if let Some(tc) = acc.iter_mut().find(|t| t.id == id) {
                        tc.name = name;
                        tc.call_id = call_id;
                    } else {
                        acc.push(ToolCallAccumulator {
                            id,
                            call_id,
                            name,
                            arguments: String::new(),
                        });
                    }
                    LlmStreamEvent::TextDelta(String::new())
                }
                // OpenAI Responses stamps the assistant item's phase on
                // `response.output_item.added`, i.e. before any text delta of
                // that item. Surface it as a best-effort streamed hint (EVE-774)
                // so consumers can classify commentary vs final answer while
                // streaming; the terminal Done metadata stays authoritative.
                Some(types::OutputItem::Message {
                    phase: Some(phase_str),
                    ..
                }) => match crate::execution_phase::ExecutionPhase::from_provider_str(&phase_str) {
                    Some(phase) => LlmStreamEvent::MessagePhase(phase),
                    None => LlmStreamEvent::TextDelta(String::new()),
                },
                _ => LlmStreamEvent::TextDelta(String::new()),
            }
        }

        StreamingEvent::OutputItemDone { item, .. } => {
            match item {
                Some(types::OutputItem::FunctionCall { .. }) => {
                    let acc = accumulated_tool_calls.lock().unwrap();
                    if !acc.is_empty() {
                        let tool_calls: Vec<ToolCall> = acc
                            .iter()
                            .filter(|tc| !tc.name.is_empty())
                            .map(|tc| {
                                let arguments: Value =
                                    serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
                                ToolCall {
                                    id: tc.call_id.clone(),
                                    name: tc.name.clone(),
                                    arguments,
                                }
                            })
                            .collect();

                        if !tool_calls.is_empty() {
                            *finish_reason.lock().unwrap() = Some("tool_calls".to_string());
                            return LlmStreamEvent::ToolCalls(tool_calls);
                        }
                    }
                    LlmStreamEvent::TextDelta(String::new())
                }
                Some(types::OutputItem::Reasoning {
                    id,
                    summary,
                    content: _, // plaintext reasoning content is intentionally not propagated
                    encrypted_content,
                }) => {
                    // Plaintext reasoning content from the provider is intentionally
                    // dropped here so it never reaches persisted events. Only the
                    // provider's opaque encrypted artifact and curated summary text
                    // travel forward.
                    let safe_summary: Vec<String> = summary
                        .into_iter()
                        .filter_map(|part| match part {
                            types::ContentPart::SummaryText { text } => Some(text),
                            _ => None,
                        })
                        .collect();
                    tracing::debug!(
                        item_id = %id,
                        encrypted_len = encrypted_content.as_ref().map(|s| s.len()).unwrap_or(0),
                        summary_segments = safe_summary.len(),
                        "OpenResponses: received reasoning item"
                    );
                    let mut item =
                        crate::reasoning::ReasoningContentPart::opaque("openai").with_item_id(id);
                    if let Some(encrypted) = encrypted_content {
                        item = item.with_encrypted(encrypted);
                    }
                    if !safe_summary.is_empty() {
                        item = item.with_text(crate::reasoning::ReasoningText::Summary {
                            parts: safe_summary,
                        });
                    }
                    LlmStreamEvent::ReasoningItem(item)
                }
                _ => LlmStreamEvent::TextDelta(String::new()),
            }
        }

        StreamingEvent::ResponseCompleted { response, .. }
        | StreamingEvent::ResponseIncomplete { response, .. } => {
            // Extract usage
            if let Some(usage) = &response.usage {
                *input_tokens.lock().unwrap() = usage.input_tokens;
                *output_tokens.lock().unwrap() = usage.output_tokens;
                if let Some(details) = &usage.input_tokens_details {
                    *cache_read_tokens.lock().unwrap() = Some(details.cached_tokens);
                }
            }

            let reason = match response.status {
                types::ResponseStatus::Completed => {
                    let existing = finish_reason.lock().unwrap().clone();
                    existing.unwrap_or_else(|| "stop".to_string())
                }
                types::ResponseStatus::Failed => {
                    tracing::warn!(
                        response_id = %response.id,
                        error = ?response.error,
                        "OpenResponsesDriver: response completed with 'failed' status"
                    );
                    "error".to_string()
                }
                types::ResponseStatus::Cancelled => "cancelled".to_string(),
                types::ResponseStatus::Incomplete => response
                    .incomplete_details
                    .as_ref()
                    .map(|details| match details.reason.as_str() {
                        "max_output_tokens" | "max_tokens" => "length",
                        other => other,
                    })
                    .unwrap_or("stop")
                    .to_string(),
                _ => "stop".to_string(),
            };

            // Extract phase from the last assistant message in output items.
            // The API assigns the phase; we preserve it as-is for subsequent requests.
            let phase = response.output.iter().rev().find_map(|item| {
                if let types::OutputItem::Message { phase, .. } = item {
                    phase.clone()
                } else {
                    None
                }
            });

            let input = *input_tokens.lock().unwrap();
            let output = *output_tokens.lock().unwrap();
            let cached = *cache_read_tokens.lock().unwrap();
            let provider_cost_usd = response.usage.as_ref().and_then(|u| u.cost);

            LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                // `input` is OpenAI's cache-inclusive prompt count; normalize to
                // non-cached input (disjoint convention).
                total_tokens: Some(input + output),
                prompt_tokens: Some(disjoint_prompt_tokens(input, cached)),
                completion_tokens: Some(output),
                cache_read_tokens: cached,
                cache_creation_tokens: None,
                provider_cost_usd,
                model: Some(model),
                finish_reason: Some(reason),
                retry_metadata: retry_metadata.map(|arc| (*arc).clone()),
                response_id: Some(response.id),
                phase,
                cache_diagnostics: None,
            }))
        }

        StreamingEvent::Error { error, .. } => {
            tracing::warn!(
                error_code = error.code.as_deref().unwrap_or("none"),
                error_message = %error.message,
                "OpenResponsesDriver: received streaming error event from provider"
            );
            LlmStreamEvent::Error(crate::driver_registry::LlmStreamError::provider(
                error.code,
                None,
                error.message,
            ))
        }

        StreamingEvent::ResponseFailed { response, .. } => {
            let error = response.error.unwrap_or(types::Error {
                code: "processing_error".to_string(),
                message: "The provider failed while processing the response".to_string(),
            });
            tracing::warn!(
                response_id = %response.id,
                error_code = %error.code,
                error_message = %error.message,
                "OpenResponsesDriver: response failed in stream"
            );
            LlmStreamEvent::Error(crate::driver_registry::LlmStreamError::provider(
                Some(error.code),
                None,
                error.message,
            ))
        }

        StreamingEvent::RefusalDelta { delta, .. } => {
            // Treat refusal as an error message
            LlmStreamEvent::Error(format!("Model refused: {}", delta).into())
        }

        // All other events: emit empty delta to maintain stream continuity
        _ => LlmStreamEvent::TextDelta(String::new()),
    }
}

// ============================================================================
// OpenAI Responses API Types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
struct ResponsesRequest {
    model: String,
    input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ResponsesReasoning>,
    /// Metadata for tracking API usage (up to 16 key-value pairs).
    /// Useful for correlating requests with session_id, agent_id, org_id, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    /// Request-level parallel tool calling preference (EVE-598). Omitted when
    /// `None` to preserve the provider default.
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    /// Speed selector: OpenAI service tier ("flex", "default", "priority").
    /// Omitted when `None` so the provider keeps its default ("auto") routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
    /// Text output controls, currently just `verbosity`. Omitted when there is
    /// nothing to configure so the provider keeps its default output length.
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<ResponsesText>,
    /// Opt-in response fields. `reasoning.encrypted_content` is what makes
    /// reasoning replayable without server-side state: without it the API
    /// returns reasoning items carrying no payload, so a stateless follow-up
    /// (after compaction, a model switch, or router failover) silently loses
    /// the reasoning chain. Omitted when there is nothing to include.
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
}

/// `text` request block for the Responses API. Verbosity ("low"/"medium"/"high")
/// controls output length independently of reasoning effort.
#[derive(Debug, Clone, Serialize)]
struct ResponsesText {
    #[serde(skip_serializing_if = "Option::is_none")]
    verbosity: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ResponsesReasoning {
    effort: String,
    /// Request reasoning summary to get thinking tokens streamed back.
    /// Without this, reasoning happens internally but tokens are not exposed.
    summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum ResponsesInputItem {
    Message {
        r#type: String,
        role: String,
        content: ResponsesContent,
        /// Execution phase for assistant messages (e.g., "in_progress", "completed").
        /// Helps GPT-5.x distinguish intermediate working commentary from final answers.
        /// Only set on assistant messages; must be preserved when replaying history.
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    FunctionCall {
        r#type: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        r#type: String,
        call_id: String,
        output: String,
    },
    /// Reasoning item for o-series and GPT-5 models
    /// Contains encrypted reasoning content that preserves reasoning context across turns
    /// (similar to Anthropic's thinking signature).
    ///
    /// Stateless requests must re-send prior `Reasoning` items in `input` so the model can
    /// continue from them. Stateful continuations (those carrying `previous_response_id`)
    /// rely on OpenAI to hold the prior reasoning chain server-side, so [`compute_delta_input_items`]
    /// intentionally drops `Reasoning` items that belong to a prior assistant turn — re-sending
    /// them alongside `previous_response_id` would violate the no-mixing invariant.
    Reasoning {
        r#type: String,
        /// Unique ID for this reasoning item
        id: String,
        /// Encrypted reasoning content (required for multi-turn conversations)
        encrypted_content: String,
        /// Provider-curated summary segments. The API rejects a reasoning input
        /// item without this key (`400 … missing required field \`summary\``),
        /// so it is always serialized — an empty list when the artifact carried
        /// no summary, which is the common case since summaries arrive only
        /// when the request asked for them.
        summary: Vec<types::ContentPart>,
    },
    /// Opaque native context returned by `/responses/compact`.
    Compaction {
        r#type: String,
        encrypted_content: String,
    },
}

impl From<&CompactOutputItem> for ResponsesInputItem {
    fn from(item: &CompactOutputItem) -> Self {
        match item {
            CompactOutputItem::Message { role, content } => Self::Message {
                r#type: "message".to_string(),
                role: role.clone(),
                content: match content {
                    CompactContent::Text(text) => ResponsesContent::Text(text.clone()),
                    CompactContent::Parts(parts) => ResponsesContent::Parts(
                        parts
                            .iter()
                            .map(|part| match part {
                                CompactContentPart::InputText { text } => {
                                    ResponsesContentPart::InputText {
                                        r#type: "input_text".to_string(),
                                        text: text.clone(),
                                    }
                                }
                                CompactContentPart::InputImage { image_url } => {
                                    ResponsesContentPart::InputImage {
                                        r#type: "input_image".to_string(),
                                        image_url: image_url.clone(),
                                    }
                                }
                            })
                            .collect(),
                    ),
                },
                phase: None,
            },
            CompactOutputItem::Compaction { encrypted_content } => Self::Compaction {
                r#type: "compaction".to_string(),
                encrypted_content: encrypted_content.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum ResponsesContent {
    Text(String),
    Parts(Vec<ResponsesContentPart>),
}

// The "Input" prefix matches OpenAI's Responses API naming convention
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[allow(clippy::enum_variant_names)]
enum ResponsesContentPart {
    InputText {
        r#type: String,
        text: String,
    },
    InputImage {
        r#type: String,
        image_url: String,
    },
    InputAudio {
        r#type: String,
        input_audio: ResponsesInputAudio,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResponsesInputAudio {
    data: String,
    format: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum ResponsesTool {
    /// Standard function tool (or deferred function with defer_loading)
    Function {
        r#type: String,
        name: String,
        description: String,
        parameters: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        defer_loading: Option<bool>,
    },
    /// Namespace grouping for tool_search (groups related deferred tools)
    Namespace {
        r#type: String,
        name: String,
        description: String,
        tools: Vec<ResponsesTool>,
    },
    /// Activates tool_search on the request
    ToolSearch { r#type: String },
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let request = ResponsesRequest {
            include: None,
            text: None,
            service_tier: None,
            model: "gpt-5.2".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Hello".to_string()),
                phase: None,
            }],
            instructions: Some("You are helpful".to_string()),
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: None,
            metadata: None,
            prompt_cache_key: None,
            parallel_tool_calls: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "gpt-5.2");
        assert_eq!(json["stream"], true);
        assert_eq!(json["instructions"], "You are helpful");
        assert!(json["input"].is_array());
    }

    #[test]
    fn test_request_with_reasoning() {
        let request = ResponsesRequest {
            include: None,
            text: None,
            service_tier: None,
            model: "o3".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Think about this".to_string()),
                phase: None,
            }],
            instructions: None,
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: Some(ResponsesReasoning {
                effort: "high".to_string(),
                summary: "detailed".to_string(),
            }),
            metadata: None,
            prompt_cache_key: None,
            parallel_tool_calls: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["reasoning"]["effort"], "high");
        assert_eq!(json["reasoning"]["summary"], "detailed");
    }

    #[test]
    fn test_request_with_metadata() {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("session_id".to_string(), "session_abc123".to_string());
        metadata.insert("agent_id".to_string(), "agent_xyz789".to_string());

        let request = ResponsesRequest {
            include: None,
            text: None,
            service_tier: None,
            model: "gpt-5.2".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Hello".to_string()),
                phase: None,
            }],
            instructions: None,
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: None,
            metadata: Some(metadata),
            prompt_cache_key: None,
            parallel_tool_calls: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["metadata"]["session_id"], "session_abc123");
        assert_eq!(json["metadata"]["agent_id"], "agent_xyz789");
    }

    /// EVE-598: the Responses request serializes `parallel_tool_calls` only when
    /// the config sets it, preserving provider defaults when `None`.
    #[test]
    fn test_request_serializes_parallel_tool_calls() {
        let make = |flag: Option<bool>| ResponsesRequest {
            include: None,
            text: None,
            service_tier: None,
            model: "gpt-5.4".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Hello".to_string()),
                phase: None,
            }],
            instructions: None,
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: None,
            metadata: None,
            prompt_cache_key: None,
            parallel_tool_calls: flag,
        };

        // None → field omitted entirely (provider default preserved).
        let json = serde_json::to_value(make(None)).unwrap();
        assert!(json.get("parallel_tool_calls").is_none());

        // Some(true) → field present and true.
        let json = serde_json::to_value(make(Some(true))).unwrap();
        assert_eq!(json["parallel_tool_calls"], true);

        // Some(false) → field present and false.
        let json = serde_json::to_value(make(Some(false))).unwrap();
        assert_eq!(json["parallel_tool_calls"], false);
    }

    /// The speed selector serializes as `service_tier` only when set,
    /// preserving the provider's default ("auto") routing when `None`.
    #[test]
    fn test_request_serializes_service_tier() {
        let make = |tier: Option<&str>| ResponsesRequest {
            service_tier: tier.map(str::to_string),
            model: "gpt-5.4".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Hello".to_string()),
                phase: None,
            }],
            instructions: None,
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: None,
            metadata: None,
            prompt_cache_key: None,
            parallel_tool_calls: None,
            text: None,
            include: None,
        };

        let json = serde_json::to_value(make(None)).unwrap();
        assert!(json.get("service_tier").is_none());

        let json = serde_json::to_value(make(Some("priority"))).unwrap();
        assert_eq!(json["service_tier"], "priority");

        let json = serde_json::to_value(make(Some("flex"))).unwrap();
        assert_eq!(json["service_tier"], "flex");
    }

    /// Verbosity serializes as a nested `text.verbosity` object only when set,
    /// preserving the provider's default output length when `None`.
    #[test]
    fn test_request_serializes_verbosity() {
        let make = |verbosity: Option<&str>| ResponsesRequest {
            include: None,
            service_tier: None,
            text: verbosity.map(|v| ResponsesText {
                verbosity: Some(v.to_string()),
            }),
            model: "gpt-5.6-sol".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Hello".to_string()),
                phase: None,
            }],
            instructions: None,
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: None,
            metadata: None,
            prompt_cache_key: None,
            parallel_tool_calls: None,
        };

        let json = serde_json::to_value(make(None)).unwrap();
        assert!(json.get("text").is_none());

        let json = serde_json::to_value(make(Some("low"))).unwrap();
        assert_eq!(json["text"]["verbosity"], "low");

        let json = serde_json::to_value(make(Some("high"))).unwrap();
        assert_eq!(json["text"]["verbosity"], "high");
    }

    #[test]
    fn test_function_call_output_serialization() {
        let item = ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".to_string(),
            call_id: "call_123".to_string(),
            output: r#"{"result": 42}"#.to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "function_call_output");
        assert_eq!(json["call_id"], "call_123");
        assert_eq!(json["output"], r#"{"result": 42}"#);
    }

    #[test]
    fn test_multipart_content_serialization() {
        let content = ResponsesContent::Parts(vec![
            ResponsesContentPart::InputText {
                r#type: "input_text".to_string(),
                text: "Look at this image".to_string(),
            },
            ResponsesContentPart::InputImage {
                r#type: "input_image".to_string(),
                image_url: "data:image/png;base64,abc123".to_string(),
            },
        ]);

        let json = serde_json::to_value(&content).unwrap();
        assert!(json.is_array());
        assert_eq!(json[0]["type"], "input_text");
        assert_eq!(json[1]["type"], "input_image");
    }

    #[test]
    fn test_tool_serialization() {
        let tool = ResponsesTool::Function {
            r#type: "function".to_string(),
            name: "get_weather".to_string(),
            description: "Get weather for a location".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }),
            strict: Some(true),
            defer_loading: None,
        };

        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["type"], "function");
        assert_eq!(json["name"], "get_weather");
        assert!(json["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn test_build_input_extracts_system_as_instructions() {
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are a helpful assistant"),
            LlmMessage::text(LlmMessageRole::User, "Hello"),
        ];

        let (instructions, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        assert_eq!(
            instructions,
            Some("You are a helpful assistant".to_string())
        );
        assert_eq!(input.len(), 1); // Only user message, system converted to instructions
    }

    #[test]
    fn test_build_input_concatenates_multiple_system_messages() {
        // The agent system prompt plus a later system message (e.g. infinity
        // context's hidden-history notice or compaction's summary) must both
        // survive — the later one must not overwrite the real system prompt.
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are a helpful assistant"),
            LlmMessage::text(LlmMessageRole::User, "Hello"),
            LlmMessage::text(
                LlmMessageRole::System,
                "[IMPORTANT: 3 earlier messages are NOT visible in this context.]",
            ),
        ];

        let (instructions, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        assert_eq!(
            instructions,
            Some(
                "You are a helpful assistant\n\n[IMPORTANT: 3 earlier messages are NOT visible in this context.]"
                    .to_string()
            )
        );
        assert_eq!(input.len(), 1); // Only the user message remains as input
    }

    #[test]
    fn test_convert_role() {
        assert_eq!(
            OpenResponsesProtocolChatDriver::convert_role(&LlmMessageRole::System),
            "developer"
        );
        assert_eq!(
            OpenResponsesProtocolChatDriver::convert_role(&LlmMessageRole::User),
            "user"
        );
        assert_eq!(
            OpenResponsesProtocolChatDriver::convert_role(&LlmMessageRole::Assistant),
            "assistant"
        );
        assert_eq!(
            OpenResponsesProtocolChatDriver::convert_role(&LlmMessageRole::Tool),
            "tool"
        );
    }

    #[test]
    fn test_function_call_serialization() {
        let item = ResponsesInputItem::FunctionCall {
            r#type: "function_call".to_string(),
            call_id: "call_abc123".to_string(),
            name: "get_current_time".to_string(),
            arguments: r#"{"timezone":"UTC"}"#.to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_abc123");
        assert_eq!(json["name"], "get_current_time");
        assert_eq!(json["arguments"], r#"{"timezone":"UTC"}"#);
    }

    #[test]
    fn test_build_input_with_tool_calls() {
        use crate::tool_types::ToolCall;

        // Simulate a conversation with tool calls:
        // 1. User asks a question
        // 2. Assistant calls a tool
        // 3. Tool returns result
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are helpful"),
            LlmMessage::text(LlmMessageRole::User, "What time is it?"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text(String::new()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_xyz789".to_string(),
                    name: "get_current_time".to_string(),
                    arguments: json!({"timezone": "UTC"}),
                }]),
                tool_call_id: None,
                phase: None,
                reasoning: Vec::new(),
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("2025-01-19T10:30:00Z".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_xyz789".to_string()),
                phase: None,
                reasoning: Vec::new(),
            },
        ];

        let (instructions, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        // System message becomes instructions
        assert_eq!(instructions, Some("You are helpful".to_string()));

        // Should have: user message, function_call, function_call_output
        assert_eq!(input.len(), 3);

        // Verify the function_call is present (second item, since assistant had empty content)
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_xyz789");
        assert_eq!(json["name"], "get_current_time");

        // Verify the function_call_output is present
        let json = serde_json::to_value(&input[2]).unwrap();
        assert_eq!(json["type"], "function_call_output");
        assert_eq!(json["call_id"], "call_xyz789");
        assert_eq!(json["output"], "2025-01-19T10:30:00Z");
    }

    #[test]
    fn test_build_input_with_tool_calls_and_text() {
        use crate::tool_types::ToolCall;

        // Assistant message with both text content and tool calls
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "What time is it?"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Let me check the time for you.".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_abc".to_string(),
                    name: "get_time".to_string(),
                    arguments: json!({}),
                }]),
                tool_call_id: None,
                phase: None,
                reasoning: Vec::new(),
            },
        ];

        let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        // Should have: user message, assistant message, function_call
        assert_eq!(input.len(), 3);

        // First is user message
        let json = serde_json::to_value(&input[0]).unwrap();
        assert_eq!(json["role"], "user");

        // Second is assistant message with text
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["role"], "assistant");

        // Third is function_call
        let json = serde_json::to_value(&input[2]).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_abc");
    }

    // ========================================================================
    // EVE-488: Stateful Responses continuations must not double-send context.
    //
    // When `previous_response_id` is set, the OpenAI Responses provider already
    // holds the prior transcript server-side. Re-sending it in `input` causes
    // double-counting. These tests pin the invariant that the delta-trim helper
    // only keeps items strictly after the most recent assistant turn, and
    // that the request-building path applies the trim when (and only when) a
    // continuation handle is present.
    // ========================================================================

    /// Issue reproducer: a stateful continuation must not carry the full prior
    /// transcript in `input` alongside `previous_response_id`. After trimming,
    /// only the new tool result and any fresh user input should remain.
    #[test]
    fn openresponses_requests_should_not_mix_previous_response_id_with_full_transcript() {
        use crate::tool_types::ToolCall;

        // Simulate a multi-turn transcript: system + user + assistant(tool_call) + tool result.
        // This is the exact shape that gets reconstructed on a follow-up turn when
        // the runtime has a `previous_response_id` from the prior assistant turn.
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are helpful"),
            LlmMessage::text(LlmMessageRole::User, "What time is it?"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Let me check.".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_xyz789".to_string(),
                    name: "get_current_time".to_string(),
                    arguments: json!({"timezone": "UTC"}),
                }]),
                tool_call_id: None,
                phase: None,
                reasoning: Vec::new(),
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("2025-01-19T10:30:00Z".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_xyz789".to_string()),
                phase: None,
                reasoning: Vec::new(),
            },
        ];

        // Build the full transcript the same way the driver does.
        let (instructions, full_input) =
            OpenResponsesProtocolChatDriver::build_input(&messages, false);

        // Without trimming the full transcript leaks user + assistant + function_call
        // + function_call_output — exactly the bug.
        assert!(
            full_input.len() > 1,
            "sanity: full transcript has multi items"
        );

        // The trim performed when `previous_response_id` is present in the request
        // path must drop everything up to and including the last prior-assistant item.
        let delta = compute_delta_input_items(full_input);

        // Only the tool result (function_call_output) should remain.
        assert_eq!(
            delta.len(),
            1,
            "stateful continuation must only send delta items; got {} items",
            delta.len()
        );
        let json = serde_json::to_value(&delta[0]).unwrap();
        assert_eq!(json["type"], "function_call_output");
        assert_eq!(json["call_id"], "call_xyz789");
        assert_eq!(json["output"], "2025-01-19T10:30:00Z");

        // Instructions (system message) are NOT part of `input`; they're still sent
        // separately and that is correct — they don't count toward the invariant.
        assert_eq!(instructions, Some("You are helpful".to_string()));
    }

    /// Stateless mode (no previous_response_id): all input items are kept.
    /// The trim helper is only invoked by the call path when previous_response_id
    /// is set; this test pins that the helper produces correct delta output
    /// regardless, leaving the fresh user message that follows the assistant turn.
    #[test]
    fn compute_delta_keeps_tail_after_assistant_message() {
        let items = vec![
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("hi".to_string()),
                phase: None,
            },
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "assistant".to_string(),
                content: ResponsesContent::Text("hello".to_string()),
                phase: None,
            },
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("follow up".to_string()),
                phase: None,
            },
        ];
        let trimmed = compute_delta_input_items(items);
        assert_eq!(trimmed.len(), 1);
        let json = serde_json::to_value(&trimmed[0]).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(
            json["content"], "follow up",
            "trim keeps the fresh user message that arrived after the assistant turn"
        );
    }

    /// Stateful continuation with parallel tool calls: every tool output that
    /// follows the prior assistant's function_call items is kept. The function_call
    /// items themselves belong to server-side state and are dropped.
    #[test]
    fn compute_delta_keeps_tool_results_after_last_assistant_turn() {
        let items = vec![
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("do two things".to_string()),
                phase: None,
            },
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "assistant".to_string(),
                content: ResponsesContent::Text("ok".to_string()),
                phase: None,
            },
            ResponsesInputItem::FunctionCall {
                r#type: "function_call".to_string(),
                call_id: "call_a".to_string(),
                name: "tool_a".to_string(),
                arguments: "{}".to_string(),
            },
            ResponsesInputItem::FunctionCall {
                r#type: "function_call".to_string(),
                call_id: "call_b".to_string(),
                name: "tool_b".to_string(),
                arguments: "{}".to_string(),
            },
            ResponsesInputItem::FunctionCallOutput {
                r#type: "function_call_output".to_string(),
                call_id: "call_a".to_string(),
                output: "a result".to_string(),
            },
            ResponsesInputItem::FunctionCallOutput {
                r#type: "function_call_output".to_string(),
                call_id: "call_b".to_string(),
                output: "b result".to_string(),
            },
        ];

        let trimmed = compute_delta_input_items(items);

        // The function_call items live in server-side state. The delta carries
        // only the tool outputs the client produced for them.
        assert_eq!(trimmed.len(), 2);
        for item in &trimmed {
            let json = serde_json::to_value(item).unwrap();
            assert_eq!(json["type"], "function_call_output");
        }
    }

    /// Empty input with previous_response_id is valid: the provider can resume
    /// purely from the continuation handle, no input needed.
    #[test]
    fn compute_delta_allows_empty_input_for_stateful_continuation() {
        let trimmed = compute_delta_input_items(vec![]);
        assert!(trimmed.is_empty());
    }

    /// Defensive: if no prior-assistant item is present (caller passed only fresh
    /// user input), all items are kept as delta.
    #[test]
    fn compute_delta_keeps_all_items_when_no_assistant_turn_present() {
        let items = vec![
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("one".to_string()),
                phase: None,
            },
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("two".to_string()),
                phase: None,
            },
        ];
        let trimmed = compute_delta_input_items(items);
        assert_eq!(trimmed.len(), 2);
    }

    /// Reasoning items from a prior assistant turn are also dropped by the trim.
    #[test]
    fn compute_delta_drops_prior_reasoning_items() {
        let items = vec![
            ResponsesInputItem::Reasoning {
                r#type: "reasoning".to_string(),
                id: "rs_00000001".to_string(),
                encrypted_content: "encrypted-blob".to_string(),
                summary: Vec::new(),
            },
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "assistant".to_string(),
                content: ResponsesContent::Text("prior".to_string()),
                phase: None,
            },
            ResponsesInputItem::FunctionCallOutput {
                r#type: "function_call_output".to_string(),
                call_id: "call_z".to_string(),
                output: "result".to_string(),
            },
        ];
        let trimmed = compute_delta_input_items(items);
        assert_eq!(trimmed.len(), 1);
        let json = serde_json::to_value(&trimmed[0]).unwrap();
        assert_eq!(json["type"], "function_call_output");
    }

    // ------------------------------------------------------------------------
    // Request-builder integration: `finalize_input_for_request` is the single
    // gate that chooses whether the request `input` is trimmed. These tests
    // pin the exact decision the call path makes — they catch regressions
    // where the `previous_response_id`-presence check is accidentally dropped
    // or inverted, which is what would re-introduce the bug even if the trim
    // helper itself stays correct.
    // ------------------------------------------------------------------------

    fn sample_full_transcript_items() -> Vec<ResponsesInputItem> {
        vec![
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("first request".to_string()),
                phase: None,
            },
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "assistant".to_string(),
                content: ResponsesContent::Text("first reply".to_string()),
                phase: None,
            },
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("follow-up".to_string()),
                phase: None,
            },
        ]
    }

    #[test]
    fn finalize_input_skips_trim_when_previous_response_id_is_none() {
        let items = sample_full_transcript_items();
        let original_len = items.len();
        let out = finalize_input_for_request(items, &None);
        assert_eq!(
            out.len(),
            original_len,
            "stateless mode keeps the full transcript so the model has context"
        );
    }

    #[test]
    fn finalize_input_drops_locally_orphaned_tool_output_without_previous_response_id() {
        let items = vec![
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("fresh".to_string()),
                phase: None,
            },
            ResponsesInputItem::FunctionCallOutput {
                r#type: "function_call_output".to_string(),
                call_id: "call_trimmed".to_string(),
                output: "result".to_string(),
            },
        ];

        let out = finalize_input_for_request(items, &None);

        assert_eq!(out.len(), 1);
        let json = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(json["type"], "message");
    }

    #[test]
    fn finalize_input_keeps_tool_output_with_previous_response_id_even_without_local_call() {
        let items = vec![
            ResponsesInputItem::FunctionCallOutput {
                r#type: "function_call_output".to_string(),
                call_id: "call_server_side".to_string(),
                output: "stateful result".to_string(),
            },
            ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("follow-up".to_string()),
                phase: None,
            },
        ];

        let out = finalize_input_for_request(items, &Some("resp_prev_42".to_string()));

        assert_eq!(out.len(), 2);
        let json = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(json["type"], "function_call_output");
        assert_eq!(json["call_id"], "call_server_side");
    }

    #[test]
    fn finalize_input_trims_when_previous_response_id_is_set() {
        let items = sample_full_transcript_items();
        let out = finalize_input_for_request(items, &Some("resp_prev_42".to_string()));
        assert_eq!(
            out.len(),
            1,
            "stateful continuation must drop everything up to and including the prior assistant message"
        );
        let json = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(json["type"], "message");
        assert_eq!(json["role"], "user");
        // Only the post-assistant follow-up survives.
        let txt = json["content"].as_str().unwrap_or("");
        assert_eq!(txt, "follow-up");
    }

    #[test]
    fn finalize_input_allows_empty_input_with_previous_response_id() {
        let out = finalize_input_for_request(vec![], &Some("resp_anything".to_string()));
        assert!(
            out.is_empty(),
            "empty delta is valid — the provider can resume purely from the response id"
        );
    }

    // ------------------------------------------------------------------------
    // EVE-597: stateless full-replay must not serialize a `function_call` whose
    // `function_call_output` was evicted by compaction / model-view masking.
    // OpenAI/Codex Responses 400 with "No tool output found for function call …"
    // and the session wedges permanently. This is the sibling of EVE-519 (orphan
    // output, covered above); the repair drops both sides of a broken pair.
    // ------------------------------------------------------------------------

    fn function_call(call_id: &str, name: &str) -> ResponsesInputItem {
        ResponsesInputItem::FunctionCall {
            r#type: "function_call".to_string(),
            call_id: call_id.to_string(),
            name: name.to_string(),
            arguments: "{}".to_string(),
        }
    }

    fn function_call_output(call_id: &str) -> ResponsesInputItem {
        ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".to_string(),
            call_id: call_id.to_string(),
            output: "result".to_string(),
        }
    }

    fn user_message(text: &str) -> ResponsesInputItem {
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text(text.to_string()),
            phase: None,
        }
    }

    #[test]
    fn finalize_input_drops_dangling_function_call_without_previous_response_id() {
        // The exact incident: an early `read_file` call survived compaction but
        // its tool output was evicted (keep_recent_tool_outputs), leaving a
        // dangling `function_call`.
        let items = vec![
            user_message("fresh"),
            function_call("call_pHJNxIuwzLppFsQK5nJrDOpZ", "read_file"),
        ];

        let out = finalize_input_for_request(items, &None);

        assert_eq!(out.len(), 1);
        assert!(
            unpaired_function_call_ids(&out).is_empty(),
            "the dangling function_call must be dropped"
        );
        let json = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(json["type"], "message");
    }

    #[test]
    fn finalize_input_preserves_paired_function_call_and_output() {
        let items = vec![
            user_message("what time is it?"),
            function_call("call_ok", "get_current_time"),
            function_call_output("call_ok"),
        ];

        let out = finalize_input_for_request(items, &None);

        assert_eq!(out.len(), 3, "an intact call/output pair must survive");
        assert!(unpaired_function_call_ids(&out).is_empty());
    }

    #[test]
    fn finalize_input_compaction_drops_only_the_dangling_old_call() {
        // Post-compaction model view equivalent to keep_recent_tool_outputs = 3:
        // one old call whose output was masked away, followed by three intact
        // recent pairs. Only the dangling old call is dropped; the recent pairs
        // and the surrounding messages are preserved.
        let mut items = vec![
            user_message("long session"),
            function_call("call_old", "read_file"),
        ];
        for i in 0..3 {
            let id = format!("call_recent_{i}");
            items.push(function_call(&id, "tool"));
            items.push(function_call_output(&id));
        }

        let out = finalize_input_for_request(items, &None);

        assert!(
            unpaired_function_call_ids(&out).is_empty(),
            "no dangling function_call may remain after repair"
        );
        assert!(
            !out.iter().any(|item| matches!(
                item,
                ResponsesInputItem::FunctionCall { call_id, .. } if call_id == "call_old"
            )),
            "the old dangling call must be removed"
        );
        // 1 user message + 3 intact recent pairs (6 items) = 7.
        assert_eq!(out.len(), 7);
    }

    #[test]
    fn unpaired_function_call_ids_reports_both_directions() {
        let items = vec![
            function_call("call_no_output", "read_file"), // EVE-597: dangling call
            function_call_output("out_no_call"),          // EVE-519: orphan output
            function_call("paired", "tool"),
            function_call_output("paired"),
        ];

        let mut ids = unpaired_function_call_ids(&items);
        ids.sort();
        assert_eq!(
            ids,
            vec!["call_no_output".to_string(), "out_no_call".to_string()]
        );
    }

    // ========================================================================
    // Provider-declared statefulness (EVE-523)
    // ========================================================================

    #[test]
    fn provider_can_enable_stateful_responses() {
        assert!(
            OpenResponsesProtocolChatDriver::new()
                .with_stateful_responses(true)
                .supports_stateful_responses()
        );
    }

    #[test]
    fn wire_protocol_defaults_to_stateless() {
        assert!(!OpenResponsesProtocolChatDriver::new().supports_stateful_responses());
    }

    /// End-to-end shape of the call path: against a stateless gateway, a request
    /// that carries a `previous_response_id` in config must still send the FULL
    /// transcript in `input` (no trim) because the gateway will not have stored
    /// the prior response. This is the core EVE-523 regression guard.
    #[test]
    fn stateless_gateway_replays_full_transcript_despite_previous_response_id() {
        let prev_id: Option<String> = Some("gen-turn-1".to_string());

        let driver = OpenResponsesProtocolChatDriver::new();
        let effective_prev_id = if driver.supports_stateful_responses() {
            prev_id.clone()
        } else {
            None
        };
        assert!(
            effective_prev_id.is_none(),
            "stateless gateway must not chain via previous_response_id"
        );

        let items = sample_full_transcript_items();
        let original_len = items.len();
        let out = finalize_input_for_request(items, &effective_prev_id);
        assert_eq!(
            out.len(),
            original_len,
            "stateless gateway must replay the full transcript so the model keeps context"
        );
    }

    /// The same transcript against OpenAI's hosted API trims to the delta window
    /// and keeps the continuation handle — confirming the optimization is intact
    /// for genuinely stateful endpoints.
    #[test]
    fn stateful_endpoint_still_trims_and_chains() {
        let prev_id: Option<String> = Some("resp_turn_1".to_string());

        let driver = OpenResponsesProtocolChatDriver::new().with_stateful_responses(true);
        let effective_prev_id = if driver.supports_stateful_responses() {
            prev_id.clone()
        } else {
            None
        };
        assert_eq!(
            effective_prev_id, prev_id,
            "stateful endpoint keeps the continuation handle"
        );

        let out = finalize_input_for_request(sample_full_transcript_items(), &effective_prev_id);
        assert_eq!(out.len(), 1, "stateful endpoint trims to the delta window");
    }

    /// Wire-level EVE-523 reproducer: drive the real `chat_completion_stream`
    /// against a mock endpoint on a non-OpenAI host. Even with a
    /// `previous_response_id` in config, the request on the wire must omit it and
    /// carry the FULL transcript (user task + assistant turn + tool result), so a
    /// stateless gateway that ignores `previous_response_id` still sees the task.
    #[tokio::test]
    async fn stateless_gateway_request_replays_full_transcript_on_the_wire() {
        use crate::tool_types::ToolCall;
        use serde_json::json;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Any 200 lets the request through; we inspect the captured request, not
        // the (empty) streamed body.
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "stateless-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(crate::runtime_provider::BearerAuth::new("test-key"));
        let driver = OpenResponsesProtocolChatDriver::new();

        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are helpful"),
            LlmMessage::text(LlmMessageRole::User, "upgrade dependencies"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Let me look.".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": "Cargo.toml"}),
                }]),
                tool_call_id: None,
                phase: None,
                reasoning: Vec::new(),
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("[package]…".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                phase: None,
                reasoning: Vec::new(),
            },
        ];

        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "some/model".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: std::collections::HashMap::new(),
            // Continuation handle from a prior turn — must be ignored on a
            // stateless gateway.
            previous_response_id: Some("gen-turn-1".to_string()),
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        // Fire the request. The stream body is irrelevant for this assertion.
        let _ = driver
            .chat_completion_stream(endpoint.endpoint(), messages, &config)
            .await;

        let requests = server
            .received_requests()
            .await
            .expect("mock server recorded requests");
        assert_eq!(requests.len(), 1, "exactly one request should be sent");
        let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");

        // previous_response_id must be absent (skipped) — the gateway would ignore it.
        assert!(
            body.get("previous_response_id").is_none(),
            "stateless gateway request must omit previous_response_id; body: {body}"
        );

        // The full transcript must be replayed: user message, assistant message,
        // function_call, and function_call_output (instructions carry the system msg).
        let input = body["input"].as_array().expect("input is an array");
        assert_eq!(
            input.len(),
            4,
            "full transcript must be replayed on a stateless gateway; got {input:?}"
        );
        assert_eq!(body["instructions"], "You are helpful");
        let has_user_task = input
            .iter()
            .any(|item| item["type"] == "message" && item["role"] == "user");
        assert!(
            has_user_task,
            "the original user task must be replayed; got {input:?}"
        );
        let has_tool_output = input
            .iter()
            .any(|item| item["type"] == "function_call_output");
        assert!(
            has_tool_output,
            "the latest tool result must still be present; got {input:?}"
        );
    }

    #[tokio::test]
    async fn rejected_stateful_continuation_replays_repaired_transcript_once() {
        use crate::tool_types::ToolCall;
        use futures::StreamExt;
        use serde_json::json;
        use wiremock::matchers::{body_partial_json, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({
                "previous_response_id": "resp_tool_turn"
            })))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": {
                    "type": "invalid_request_error",
                    "message": "No tool output found for function call call_1"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let completed = r#"data: {"type":"response.completed","response":{"id":"resp_recovered","status":"completed","model":"gpt-5.4","output":[],"usage":{"input_tokens":4,"output_tokens":1,"total_tokens":5}}}

"#;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(completed),
            )
            .expect(1)
            .mount(&server)
            .await;

        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "stateful-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(crate::runtime_provider::BearerAuth::new("test-key"));
        let driver = OpenResponsesProtocolChatDriver::new()
            .with_stateful_responses(true)
            .with_retry_config(LlmRetryConfig::no_retry());
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "inspect the project"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text(String::new()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": "Cargo.toml"}),
                }]),
                tool_call_id: None,
                phase: None,
                reasoning: Vec::new(),
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("[package]".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                phase: None,
                reasoning: Vec::new(),
            },
        ];
        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "gpt-5.4".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: Some("resp_tool_turn".to_string()),
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        let mut stream = driver
            .chat_completion_stream(endpoint.endpoint(), messages, &config)
            .await
            .expect("continuation should recover");
        while let Some(event) = stream.next().await {
            event.expect("valid recovered event");
        }

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 2);
        let first: serde_json::Value = requests[0].body_json().expect("first body");
        let second: serde_json::Value = requests[1].body_json().expect("second body");
        assert_eq!(first["previous_response_id"], "resp_tool_turn");
        assert!(second.get("previous_response_id").is_none());
        let replay = second["input"].as_array().expect("replay input");
        assert!(replay.iter().any(|item| item["type"] == "function_call"));
        assert!(
            replay
                .iter()
                .any(|item| item["type"] == "function_call_output")
        );
    }

    #[tokio::test]
    async fn openrouter_provider_does_not_send_hosted_tool_search() {
        use crate::tool_types::DeferrablePolicy;
        use serde_json::json;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "openrouter-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(crate::runtime_provider::BearerAuth::new("test-key"));
        let driver = OpenResponsesProtocolChatDriver::new();

        let tools: Vec<ToolDefinition> = (0..16)
            .map(|i| {
                make_tool(
                    &format!("tool_{i}"),
                    Some("General"),
                    DeferrablePolicy::Automatic,
                )
            })
            .collect();

        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "gpt-5.4".to_string(),
            temperature: None,
            max_tokens: None,
            tools,
            reasoning_effort: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: Some(crate::driver_registry::ToolSearchConfig {
                enabled: true,
                threshold: 15,
            }),
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
        let _ = driver
            .chat_completion_stream(endpoint.endpoint(), messages, &config)
            .await;

        let requests = server
            .received_requests()
            .await
            .expect("mock server recorded requests");
        assert_eq!(requests.len(), 1, "exactly one request should be sent");
        let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");
        let tools = body["tools"].as_array().expect("tools is an array");

        assert!(
            tools.iter().all(|tool| tool["type"] == "function"),
            "OpenRouter should receive regular function tools, not hosted tool_search payloads: {tools:?}"
        );
        assert!(
            tools.iter().all(|tool| tool.get("defer_loading").is_none()),
            "OpenRouter tool schemas should not be deferred by hosted tool_search: {tools:?}"
        );
        assert_eq!(
            body["input"],
            json!([{"type": "message", "role": "user", "content": "hello"}])
        );
    }

    #[tokio::test]
    async fn openai_provider_omits_openrouter_routing_controls() {
        use crate::driver_registry::{OpenRouterRoute, OpenRouterRoutingConfig};
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "openai-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(crate::runtime_provider::BearerAuth::new("test-key"));
        let driver = OpenResponsesProtocolChatDriver::new();

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("session_id".to_string(), "session_abc123".to_string());
        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "gpt-5-mini".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata,
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: Some(OpenRouterRoutingConfig {
                models: vec!["openai/gpt-5-mini".to_string()],
                route: Some(OpenRouterRoute::Fallback),
                provider: None,
                ..Default::default()
            }),
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
        let _ = driver
            .chat_completion_stream(endpoint.endpoint(), messages, &config)
            .await;

        let requests = server
            .received_requests()
            .await
            .expect("mock server recorded requests");
        assert_eq!(requests.len(), 1, "exactly one request should be sent");
        let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");

        assert!(body.get("models").is_none(), "body: {body}");
        assert!(body.get("route").is_none(), "body: {body}");
        assert!(body.get("provider").is_none(), "body: {body}");
        // The top-level session_id is OpenRouter-only; OpenAI must not receive it
        // even though the session id rides along in `metadata`.
        assert!(body.get("session_id").is_none(), "body: {body}");
        assert_eq!(body["metadata"]["session_id"], "session_abc123");
    }

    /// OpenAI-compatible gateways (e.g. OpenRouter) terminate the Responses SSE
    /// stream with a chat-completions-style `[DONE]` sentinel that OpenAI's
    /// native API does not send. It must be skipped, not surfaced as a spurious
    /// `Error` event after the real completion. (EVE: caught by the OpenRouter
    /// live chat smoke test.)
    #[tokio::test]
    async fn openresponses_stream_skips_done_sentinel() {
        use futures::StreamExt;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // A normal text delta followed by the trailing `[DONE]` sentinel.
        let body =
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\ndata: [DONE]\n\n";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "stream-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(crate::runtime_provider::BearerAuth::new("test-key"));
        let driver = OpenResponsesProtocolChatDriver::new();
        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "openai/gpt-5.6-luna".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        let stream = driver
            .chat_completion_stream(
                endpoint.endpoint(),
                vec![LlmMessage::text(LlmMessageRole::User, "hi")],
                &config,
            )
            .await
            .expect("stream should start");
        let events: Vec<_> = stream.collect().await;

        let mut text = String::new();
        for ev in &events {
            match ev.as_ref().expect("no transport error") {
                LlmStreamEvent::TextDelta(d) => text.push_str(d),
                LlmStreamEvent::Error(e) => {
                    panic!("[DONE] sentinel must not surface as an error: {e}")
                }
                _ => {}
            }
        }
        assert_eq!(text, "hi");
    }

    /// Deterministic contract coverage for the live matrix's stochastic
    /// `get_current_time` case: prove the usable tool schema reaches the wire
    /// and a fragmented OpenResponses tool call survives streaming parse.
    #[tokio::test]
    async fn tool_call_contract_covers_request_wire_and_stream_parser() {
        use futures::StreamExt;
        use serde_json::json;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let body = concat!(
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"get_current_time\",\"arguments\":\"\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"timezone\\\":\\\"UTC\\\"\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\",\\\"format\\\":\\\"iso8601\\\"}\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"get_current_time\",\"arguments\":\"{\\\"timezone\\\":\\\"UTC\\\",\\\"format\\\":\\\"iso8601\\\"}\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"model\":\"openai/gpt-5.6-luna\",\"output\":[],\"usage\":{\"input_tokens\":4,\"output_tokens\":2,\"total_tokens\":6}}}\n\n",
            "data: [DONE]\n\n",
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "openrouter-contract-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(crate::runtime_provider::BearerAuth::new("test-key"));
        let driver = OpenResponsesProtocolChatDriver::new();
        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "openai/gpt-5.6-luna".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
                name: "get_current_time".to_string(),
                display_name: None,
                description: "Get the current time in a timezone.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "timezone": { "type": "string" },
                        "format": { "type": "string", "enum": ["iso8601", "unix", "human"] }
                    },
                    "required": ["timezone"]
                }),
                policy: crate::tool_types::ToolPolicy::Auto,
                category: None,
                deferrable: crate::tool_types::DeferrablePolicy::Never,
                hints: crate::tool_types::ToolHints::default(),
                full_parameters: None,
            })],
            reasoning_effort: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        let stream = driver
            .chat_completion_stream(
                endpoint.endpoint(),
                vec![LlmMessage::text(LlmMessageRole::User, "What time is it?")],
                &config,
            )
            .await
            .expect("stream should start");
        let events: Vec<_> = stream.collect().await;

        let tool_calls = events
            .iter()
            .filter_map(|event| match event.as_ref().expect("valid stream event") {
                LlmStreamEvent::ToolCalls(calls) => Some(calls),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
        assert_eq!(tool_calls[0].name, "get_current_time");
        assert_eq!(
            tool_calls[0].arguments,
            json!({"timezone": "UTC", "format": "iso8601"})
        );
        assert!(events.iter().any(|event| matches!(
            event.as_ref(),
            Ok(LlmStreamEvent::Done(metadata))
                if metadata.finish_reason.as_deref() == Some("tool_calls")
        )));

        let requests = server.received_requests().await.expect("captured request");
        let request: serde_json::Value = requests[0].body_json().expect("request JSON");
        let tool = &request["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["name"], "get_current_time");
        assert_eq!(tool["parameters"]["type"], "object");
        assert_eq!(tool["strict"], true);
        assert_eq!(
            tool["parameters"]["required"],
            json!(["format", "timezone"])
        );
        assert_eq!(
            tool["parameters"]["properties"]["format"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(tool["parameters"]["additionalProperties"], false);
    }

    // ========================================================================
    // Compact endpoint tests
    // ========================================================================

    #[test]
    fn test_compact_request_serialization() {
        let request = CompactRequest {
            model: "gpt-5.2".to_string(),
            input: vec![
                CompactInputItem::Message {
                    role: "user".to_string(),
                    content: CompactContent::Text("Hello!".to_string()),
                },
                CompactInputItem::Message {
                    role: "assistant".to_string(),
                    content: CompactContent::Text("Hi there!".to_string()),
                },
            ],
            previous_response_id: None,
            instructions: Some("Be helpful".to_string()),
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "gpt-5.2");
        assert_eq!(json["instructions"], "Be helpful");
        assert!(json["input"].is_array());
        assert_eq!(json["input"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_compact_input_item_message_serialization() {
        let item = CompactInputItem::Message {
            role: "user".to_string(),
            content: CompactContent::Text("Test message".to_string()),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "message");
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Test message");
    }

    #[test]
    fn test_compact_input_item_function_call_serialization() {
        let item = CompactInputItem::FunctionCall {
            call_id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: r#"{"city":"NYC"}"#.to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_123");
        assert_eq!(json["name"], "get_weather");
        assert_eq!(json["arguments"], r#"{"city":"NYC"}"#);
    }

    #[test]
    fn test_compact_input_item_compaction_serialization() {
        let item = CompactInputItem::Compaction {
            encrypted_content: "encrypted_data_here".to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "compaction");
        assert_eq!(json["encrypted_content"], "encrypted_data_here");
    }

    #[test]
    fn test_compact_output_item_deserialization() {
        let json = r#"{
            "type": "message",
            "role": "user",
            "content": "Hello"
        }"#;

        let item: CompactOutputItem = serde_json::from_str(json).unwrap();
        match item {
            CompactOutputItem::Message { role, content } => {
                assert_eq!(role, "user");
                match content {
                    CompactContent::Text(text) => assert_eq!(text, "Hello"),
                    _ => panic!("Expected text content"),
                }
            }
            _ => panic!("Expected Message item"),
        }
    }

    #[test]
    fn test_compact_output_compaction_deserialization() {
        let json = r#"{
            "type": "compaction",
            "encrypted_content": "abc123encrypted"
        }"#;

        let item: CompactOutputItem = serde_json::from_str(json).unwrap();
        match item {
            CompactOutputItem::Compaction { encrypted_content } => {
                assert_eq!(encrypted_content, "abc123encrypted");
            }
            _ => panic!("Expected Compaction item"),
        }
    }

    #[test]
    fn test_compact_response_deserialization() {
        let json = r#"{
            "output": [
                {"type": "message", "role": "user", "content": "Hello"},
                {"type": "compaction", "encrypted_content": "xyz789"}
            ],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "total_tokens": 150
            }
        }"#;

        let response: CompactResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.output.len(), 2);
        assert!(response.usage.is_some());
        let usage = response.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.total_tokens, Some(150));
    }

    #[test]
    fn test_compact_content_parts_serialization() {
        let content = CompactContent::Parts(vec![
            CompactContentPart::InputText {
                text: "Check this image".to_string(),
            },
            CompactContentPart::InputImage {
                image_url: "data:image/png;base64,abc".to_string(),
            },
        ]);

        let json = serde_json::to_value(&content).unwrap();
        assert!(json.is_array());
        assert_eq!(json[0]["type"], "input_text");
        assert_eq!(json[0]["text"], "Check this image");
        assert_eq!(json[1]["type"], "input_image");
    }

    #[test]
    fn test_wire_protocol_supports_compact() {
        let driver = OpenResponsesProtocolChatDriver::new();
        assert!(driver.supports_compact());
    }

    // ========================================================================
    // OpenAI Thinking/Reasoning Support Tests
    // ========================================================================

    #[test]
    fn test_reasoning_input_item_serialization() {
        let item = ResponsesInputItem::Reasoning {
            r#type: "reasoning".to_string(),
            id: "rs_00000001".to_string(),
            encrypted_content: "encrypted_reasoning_context_here".to_string(),
            summary: Vec::new(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "reasoning");
        // The API rejects a reasoning input item with no `summary` key, so an
        // empty summary must still serialize as `[]` rather than vanish.
        assert_eq!(
            json["summary"],
            serde_json::json!([]),
            "summary is required even when empty"
        );
        assert_eq!(json["id"], "rs_00000001");
        assert_eq!(
            json["encrypted_content"],
            "encrypted_reasoning_context_here"
        );
    }

    /// Every replayed reasoning item carries `summary`, and carries the
    /// provider's own summary segments when it had them.
    ///
    /// The Responses API rejects a reasoning input item without the key —
    /// `400 … \`input[1]\` missing required field \`summary\`` — which took
    /// `main` red against a live provider once reasoning replay went out under
    /// provider-issued ids. Most artifacts carry no summary (the provider only
    /// sends one when the request asks), so the empty case is the common one.
    #[test]
    fn test_build_input_reasoning_items_always_carry_a_summary() {
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "Think"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("No summary on this one.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                reasoning: vec![
                    crate::reasoning::ReasoningContentPart::opaque("openai")
                        .with_item_id("rs_bare")
                        .with_encrypted("enc_bare"),
                ],
            },
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("This one was summarized.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                reasoning: vec![
                    crate::reasoning::ReasoningContentPart::opaque("openai")
                        .with_item_id("rs_summarized")
                        .with_encrypted("enc_summarized")
                        .with_text(crate::reasoning::ReasoningText::Summary {
                            parts: vec!["First I checked.".to_string(), "Then I read.".to_string()],
                        }),
                ],
            },
        ];

        let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);
        let reasoning: Vec<serde_json::Value> = input
            .iter()
            .map(|item| serde_json::to_value(item).unwrap())
            .filter(|json| json["type"] == "reasoning")
            .collect();
        assert_eq!(reasoning.len(), 2, "both artifacts must be replayed");

        for item in &reasoning {
            assert!(
                item.get("summary").is_some(),
                "summary is required on every reasoning input item: {item}"
            );
        }

        assert_eq!(
            reasoning[0]["summary"],
            serde_json::json!([]),
            "an artifact with no summary replays an empty one, not a missing key"
        );
        assert_eq!(
            reasoning[1]["summary"],
            serde_json::json!([
                { "type": "summary_text", "text": "First I checked." },
                { "type": "summary_text", "text": "Then I read." },
            ]),
            "the provider's own summary segments replay verbatim"
        );
    }

    #[test]
    fn test_build_input_replays_reasoning_before_its_message() {
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "Think about this deeply"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("I have thought about this.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                reasoning: vec![
                    crate::reasoning::ReasoningContentPart::opaque("openai")
                        .with_item_id("rs_reply")
                        .with_encrypted("encrypted_reasoning_token_123"),
                ],
            },
            LlmMessage::text(LlmMessageRole::User, "What else?"),
        ];

        let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        // Should have: user message, reasoning item, assistant message, user message
        assert_eq!(input.len(), 4);

        // First is user message
        let json = serde_json::to_value(&input[0]).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Think about this deeply");

        // Second is the reasoning item, ahead of the message it belongs to, and
        // keyed by the id the provider issued.
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["type"], "reasoning");
        assert_eq!(json["id"], "rs_reply");
        assert_eq!(json["encrypted_content"], "encrypted_reasoning_token_123");

        // Third is assistant message
        let json = serde_json::to_value(&input[2]).unwrap();
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "I have thought about this.");

        // Fourth is second user message
        let json = serde_json::to_value(&input[3]).unwrap();
        assert_eq!(json["role"], "user");
    }

    #[test]
    fn test_build_input_replays_reasoning_with_tool_calls() {
        use crate::tool_types::ToolCall;

        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "What time is it? Think carefully."),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Let me check.".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_123".to_string(),
                    name: "get_time".to_string(),
                    arguments: json!({}),
                }]),
                tool_call_id: None,
                phase: None,
                reasoning: vec![
                    crate::reasoning::ReasoningContentPart::opaque("openai")
                        .with_item_id("rs_tool")
                        .with_encrypted("encrypted_token_xyz"),
                ],
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("10:30 AM".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_123".to_string()),
                phase: None,
                reasoning: Vec::new(),
            },
        ];

        let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        // Should have: user, reasoning, assistant, function_call, function_call_output
        assert_eq!(input.len(), 5);

        // Reasoning item comes before assistant message
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["type"], "reasoning");
        assert_eq!(json["id"], "rs_tool");
        assert_eq!(json["encrypted_content"], "encrypted_token_xyz");

        // Assistant message
        let json = serde_json::to_value(&input[2]).unwrap();
        assert_eq!(json["role"], "assistant");

        // Function call
        let json = serde_json::to_value(&input[3]).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_123");

        // Function call output
        let json = serde_json::to_value(&input[4]).unwrap();
        assert_eq!(json["type"], "function_call_output");
    }

    #[test]
    fn test_build_input_without_thinking_signature() {
        // Assistant message with thinking but NO thinking_signature should not emit reasoning item
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "Hello"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Hi there!".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                reasoning: Vec::new(),
            },
        ];

        let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        // Should have: user message, assistant message (no reasoning item)
        assert_eq!(input.len(), 2);

        // Verify no reasoning item
        let json = serde_json::to_value(&input[0]).unwrap();
        assert_eq!(json["role"], "user");

        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["role"], "assistant");
    }

    #[test]
    fn test_handle_streaming_event_reasoning_encrypted_content() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // Create an OutputItemDone event with Reasoning item containing encrypted_content
        let event = StreamingEvent::OutputItemDone {
            sequence_number: 5,
            output_index: 0,
            item: Some(types::OutputItem::Reasoning {
                id: "rs_001".to_string(),
                summary: vec![],
                content: None,
                encrypted_content: Some("encrypted_reasoning_data".to_string()),
            }),
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "gpt-5".to_string(),
            None,
        );

        // Should emit a reasoning artifact carrying the provider id and the
        // encrypted payload needed to replay it.
        match result {
            LlmStreamEvent::ReasoningItem(item) => {
                assert_eq!(item.provider, "openai");
                assert_eq!(item.item_id.as_deref(), Some("rs_001"));
                assert_eq!(item.encrypted.as_deref(), Some("encrypted_reasoning_data"));
                assert!(item.text.is_none());
                assert!(item.tokens.is_none());
            }
            other => panic!("Expected ReasoningItem event, got {:?}", other),
        }
    }

    #[test]
    fn output_item_added_message_surfaces_native_phase_hint() {
        use std::sync::Mutex;

        // EVE-774: OpenAI Responses stamps the assistant item's phase on
        // `response.output_item.added` (before any text delta). The driver must
        // surface it as a mid-stream `MessagePhase` hint.
        for (wire, expected) in [
            (
                "commentary",
                crate::execution_phase::ExecutionPhase::Commentary,
            ),
            (
                "final_answer",
                crate::execution_phase::ExecutionPhase::FinalAnswer,
            ),
        ] {
            let event: StreamingEvent = serde_json::from_value(serde_json::json!({
                "type": "response.output_item.added",
                "sequence_number": 1,
                "output_index": 0,
                "item": {
                    "type": "message",
                    "id": "msg_001",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": [],
                    "phase": wire,
                }
            }))
            .expect("output_item.added should deserialize");

            let result = handle_streaming_event(
                event,
                &Mutex::new(0),
                &Mutex::new(0),
                &Mutex::new(None),
                &Mutex::new(Vec::new()),
                &Mutex::new(None),
                "gpt-5".to_string(),
                None,
            );

            match result {
                LlmStreamEvent::MessagePhase(phase) => assert_eq!(phase, expected),
                other => panic!("Expected MessagePhase({expected:?}), got {other:?}"),
            }
        }
    }

    #[test]
    fn output_item_added_message_without_phase_is_noop() {
        use std::sync::Mutex;

        // A message item that carries no phase yields no hint (empty text delta),
        // never a fabricated phase.
        let event: StreamingEvent = serde_json::from_value(serde_json::json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": {
                "type": "message",
                "id": "msg_002",
                "status": "in_progress",
                "role": "assistant",
                "content": [],
            }
        }))
        .expect("output_item.added should deserialize");

        let result = handle_streaming_event(
            event,
            &Mutex::new(0),
            &Mutex::new(0),
            &Mutex::new(None),
            &Mutex::new(Vec::new()),
            &Mutex::new(None),
            "gpt-5".to_string(),
            None,
        );

        match result {
            LlmStreamEvent::TextDelta(d) => assert!(d.is_empty()),
            other => panic!("Expected empty TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn response_failed_preserves_provider_error_code() {
        use std::sync::Mutex;

        let event: StreamingEvent = serde_json::from_value(serde_json::json!({
            "type": "response.failed",
            "sequence_number": 7,
            "response": {
                "id": "resp_failed",
                "object": "response",
                "created_at": 1,
                "status": "failed",
                "model": "gpt-5",
                "output": [],
                "tools": [],
                "error": {
                    "code": "processing_error",
                    "message": "An error occurred while processing your request."
                }
            }
        }))
        .expect("response.failed should deserialize");

        let result = handle_streaming_event(
            event,
            &Mutex::new(0),
            &Mutex::new(0),
            &Mutex::new(None),
            &Mutex::new(Vec::new()),
            &Mutex::new(None),
            "gpt-5".to_string(),
            None,
        );

        let LlmStreamEvent::Error(error) = result else {
            panic!("expected structured stream error");
        };
        assert_eq!(error.code.as_deref(), Some("processing_error"));
        assert!(crate::llm_retry::is_transient_stream_error(&error));
    }

    #[test]
    fn test_handle_streaming_event_reasoning_without_encrypted_content() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // Create an OutputItemDone event with Reasoning item but NO encrypted_content
        let event = StreamingEvent::OutputItemDone {
            sequence_number: 5,
            output_index: 0,
            item: Some(types::OutputItem::Reasoning {
                id: "rs_001".to_string(),
                summary: vec![types::ContentPart::SummaryText {
                    text: "Some summary".to_string(),
                }],
                content: None,
                encrypted_content: None, // No encrypted content
            }),
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "gpt-5".to_string(),
            None,
        );

        // Should still emit the artifact carrying the safe summary even when no
        // encrypted content is present so the durable reasoning record survives.
        match result {
            LlmStreamEvent::ReasoningItem(item) => {
                assert_eq!(item.provider, "openai");
                assert_eq!(item.item_id.as_deref(), Some("rs_001"));
                assert!(item.encrypted.is_none());
                assert_eq!(
                    item.text,
                    Some(crate::reasoning::ReasoningText::Summary {
                        parts: vec!["Some summary".to_string()],
                    })
                );
            }
            other => panic!("Expected ReasoningItem event, got {:?}", other),
        }
    }

    #[test]
    fn test_handle_streaming_event_reasoning_drops_plaintext_content() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // Reasoning item with plaintext content and a non-summary content part in `summary`.
        // Both must be excluded from the emitted ReasonItem.
        let event = StreamingEvent::OutputItemDone {
            sequence_number: 5,
            output_index: 0,
            item: Some(types::OutputItem::Reasoning {
                id: "rs_002".to_string(),
                summary: vec![
                    types::ContentPart::SummaryText {
                        text: "safe summary".to_string(),
                    },
                    types::ContentPart::ReasoningText {
                        text: "SECRET hidden reasoning".to_string(),
                    },
                ],
                content: Some(vec![types::ContentPart::ReasoningText {
                    text: "SECRET hidden reasoning".to_string(),
                }]),
                encrypted_content: Some("opaque".to_string()),
            }),
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "gpt-5".to_string(),
            None,
        );

        match result {
            LlmStreamEvent::ReasoningItem(item) => {
                assert_eq!(
                    item.text,
                    Some(crate::reasoning::ReasoningText::Summary {
                        parts: vec!["safe summary".to_string()],
                    })
                );
                assert_eq!(item.encrypted.as_deref(), Some("opaque"));
            }
            other => panic!("Expected ReasoningItem event, got {:?}", other),
        }
    }

    #[test]
    fn test_handle_streaming_event_reasoning_delta() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // Raw reasoning from o-series reaches the reasoning channel, not text.
        let event = StreamingEvent::ReasoningDelta {
            sequence_number: 3,
            item_id: "rs_001".to_string(),
            output_index: 0,
            content_index: 0,
            delta: "Let me reason about this...".to_string(),
            obfuscation: None,
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "o3".to_string(),
            None,
        );

        match result {
            LlmStreamEvent::ReasoningDelta { delta, summary } => {
                assert_eq!(delta, "Let me reason about this...");
                assert!(!summary, "raw chain-of-thought is not a summary");
            }
            _ => panic!("Expected ReasoningDelta, got {:?}", result),
        }
    }

    #[test]
    fn test_handle_streaming_event_reasoning_summary_delta() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // A reasoning summary is a reasoning artifact. Routing it to the
        // assistant-text channel persisted it as the model's answer and
        // replayed it as the model's own prior output.
        let event = StreamingEvent::ReasoningSummaryDelta {
            sequence_number: 4,
            item_id: "rs_002".to_string(),
            output_index: 0,
            summary_index: 0,
            delta: "Breaking down the problem...".to_string(),
            obfuscation: None,
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "gpt-5.2".to_string(),
            None,
        );

        match result {
            LlmStreamEvent::ReasoningDelta { delta, summary } => {
                assert_eq!(delta, "Breaking down the problem...");
                assert!(
                    summary,
                    "a reasoning summary must be labelled as such, not passed \
                     off as raw chain-of-thought"
                );
            }
            other => panic!(
                "reasoning summary must reach the reasoning channel, never \
                 assistant text; got {other:?}"
            ),
        }
    }

    #[test]
    fn test_request_reasoning_none_is_omitted() {
        // When reasoning effort is "none", the reasoning field should be omitted
        // to avoid API errors on models that don't support reasoning params
        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "gpt-5.2".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: Some(crate::model::ReasoningEffort::None),
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        // Simulate the driver's filter logic
        let reasoning = config
            .reasoning_effort
            .filter(crate::model::ReasoningEffort::requests_reasoning)
            .map(|effort| ResponsesReasoning {
                effort: effort.as_str().to_string(),
                summary: "detailed".to_string(),
            });

        assert!(
            reasoning.is_none(),
            "reasoning should be None for effort=none"
        );
    }

    #[test]
    fn test_request_reasoning_high_is_included() {
        // When reasoning effort is "high", the reasoning field should be present
        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "gpt-5.2".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: Some(crate::model::ReasoningEffort::High),
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        let reasoning = config
            .reasoning_effort
            .filter(crate::model::ReasoningEffort::requests_reasoning)
            .map(|effort| ResponsesReasoning {
                effort: effort.as_str().to_string(),
                summary: "detailed".to_string(),
            });

        assert!(
            reasoning.is_some(),
            "reasoning should be present for effort=high"
        );
        let r = reasoning.unwrap();
        assert_eq!(r.effort, "high");
        assert_eq!(r.summary, "detailed");
    }

    #[test]
    fn test_request_reasoning_none_case_insensitive() {
        // "None", "NONE", "none" should all be filtered out
        for effort in &["none", "None", "NONE"] {
            let reasoning = Some(effort.to_string())
                .as_ref()
                .filter(|e| !e.eq_ignore_ascii_case("none"))
                .cloned();

            assert!(
                reasoning.is_none(),
                "effort={effort:?} should be filtered out"
            );
        }
    }

    #[test]
    fn test_build_input_assistant_without_thinking_or_tools() {
        // Plain assistant message (no thinking, no tool calls) should just be a message
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "Hello"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Hi there!".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                reasoning: Vec::new(),
            },
        ];

        let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        assert_eq!(input.len(), 2);
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["role"], "assistant");
        assert!(json.get("type").is_none() || json["type"] == "message");
    }

    /// Each reasoning item replays under the id the provider issued for it.
    ///
    /// This previously asserted only that synthesized ids were *unique*, which
    /// a counter satisfies. Uniqueness was never the requirement: the API
    /// resolves reasoning items by the `rs_…` id it handed out, so an id the
    /// provider never issued is not usable however distinct it is.
    #[test]
    fn test_build_input_reasoning_items_keep_provider_ids() {
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "First question"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("First answer.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                reasoning: vec![
                    crate::reasoning::ReasoningContentPart::opaque("openai")
                        .with_item_id("rs_alpha")
                        .with_encrypted("encrypted_1"),
                ],
            },
            LlmMessage::text(LlmMessageRole::User, "Second question"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Second answer.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                reasoning: vec![
                    crate::reasoning::ReasoningContentPart::opaque("openai")
                        .with_item_id("rs_beta")
                        .with_encrypted("encrypted_2"),
                ],
            },
        ];

        let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

        // user, reasoning_1, assistant, user, reasoning_2, assistant
        assert_eq!(input.len(), 6);

        let r1 = serde_json::to_value(&input[1]).unwrap();
        let r2 = serde_json::to_value(&input[4]).unwrap();

        assert_eq!(r1["type"], "reasoning");
        assert_eq!(r1["id"], "rs_alpha");
        assert_eq!(r1["encrypted_content"], "encrypted_1");
        assert_eq!(r2["type"], "reasoning");
        assert_eq!(r2["id"], "rs_beta");
        assert_eq!(r2["encrypted_content"], "encrypted_2");
    }

    #[test]
    fn test_build_input_with_phases_enabled() {
        use crate::execution_phase::ExecutionPhase;

        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are helpful"),
            LlmMessage::text(LlmMessageRole::User, "Hello"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Working on it...".to_string()),
                tool_calls: Some(vec![crate::tool_types::ToolCall {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    arguments: json!({}),
                }]),
                tool_call_id: None,
                phase: Some(ExecutionPhase::Commentary),
                reasoning: Vec::new(),
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("result".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                phase: None,
                reasoning: Vec::new(),
            },
        ];

        // With supports_phases=true, assistant message should include phase
        let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, true);
        let assistant_json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(assistant_json["phase"], "commentary");

        // With supports_phases=false, phase should be absent
        let (_, input_no_phases) = OpenResponsesProtocolChatDriver::build_input(&messages, false);
        let assistant_json_no = serde_json::to_value(&input_no_phases[1]).unwrap();
        assert!(assistant_json_no.get("phase").is_none() || assistant_json_no["phase"].is_null());
    }

    // ========================================================================
    // tool_search / convert_tools_with_search tests
    // ========================================================================

    /// Helper: create a ToolDefinition with optional category and deferrable policy
    fn make_tool(
        name: &str,
        category: Option<&str>,
        deferrable: crate::tool_types::DeferrablePolicy,
    ) -> ToolDefinition {
        ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: format!("{} description", name),
            parameters: json!({"type": "object", "properties": {}}),
            policy: crate::tool_types::ToolPolicy::Auto,
            category: category.map(|s| s.to_string()),
            deferrable,
            hints: crate::tool_types::ToolHints::default(),
            full_parameters: None,
        })
    }

    #[test]
    fn test_hosted_tool_search_completed_event_preserves_response_id() {
        let event_json = r#"{
            "type": "response.completed",
            "sequence_number": 8,
            "response": {
                "id": "resp_tool_search",
                "object": "response",
                "created_at": 1780000000,
                "status": "completed",
                "model": "gpt-5.5",
                "output": [
                    {
                        "type": "tool_search_call",
                        "execution": "server",
                        "call_id": null,
                        "status": "completed",
                        "arguments": { "paths": ["Math"] }
                    },
                    {
                        "type": "tool_search_output",
                        "execution": "server",
                        "call_id": null,
                        "status": "completed",
                        "tools": [
                            {
                                "type": "namespace",
                                "name": "Math",
                                "description": "Tools for Math",
                                "tools": [
                                    {
                                        "type": "function",
                                        "name": "add",
                                        "description": "Add numbers.",
                                        "defer_loading": true,
                                        "parameters": {
                                            "type": "object",
                                            "properties": {
                                                "a": { "type": "number" },
                                                "b": { "type": "number" }
                                            },
                                            "required": ["a", "b"],
                                            "additionalProperties": false
                                        }
                                    }
                                ]
                            }
                        ]
                    },
                    {
                        "type": "function_call",
                        "id": "fc_123",
                        "call_id": "call_123",
                        "name": "add",
                        "namespace": "Math",
                        "arguments": "{\"a\":7,\"b\":3}",
                        "status": "completed"
                    }
                ],
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "total_tokens": 15
                }
            }
        }"#;

        let event: StreamingEvent = serde_json::from_str(event_json).unwrap();
        let stream_event = handle_streaming_event(
            event,
            &Mutex::new(0),
            &Mutex::new(0),
            &Mutex::new(None),
            &Mutex::new(Vec::new()),
            &Mutex::new(Some("tool_calls".to_string())),
            "gpt-5.5".to_string(),
            None,
        );

        match stream_event {
            LlmStreamEvent::Done(metadata) => {
                assert_eq!(metadata.response_id.as_deref(), Some("resp_tool_search"));
                assert_eq!(metadata.finish_reason.as_deref(), Some("tool_calls"));
            }
            other => panic!("expected Done event, got {other:?}"),
        }
    }

    #[test]
    fn test_completed_event_normalizes_cache_inclusive_prompt_tokens() {
        // OpenAI reports `input_tokens` inclusive of cached reads. The driver
        // must normalize to the disjoint convention: prompt_tokens carries only
        // the non-cached remainder (input − cached), with cache reported on top.
        let event_json = r#"{
            "type": "response.completed",
            "sequence_number": 9,
            "response": {
                "id": "resp_cache",
                "object": "response",
                "created_at": 1780000000,
                "status": "completed",
                "model": "gpt-5.5",
                "output": [],
                "usage": {
                    "input_tokens": 1000,
                    "output_tokens": 20,
                    "total_tokens": 1020,
                    "input_tokens_details": { "cached_tokens": 800 }
                }
            }
        }"#;

        let event: StreamingEvent = serde_json::from_str(event_json).unwrap();
        let stream_event = handle_streaming_event(
            event,
            &Mutex::new(0),
            &Mutex::new(0),
            &Mutex::new(None),
            &Mutex::new(Vec::new()),
            &Mutex::new(None),
            "gpt-5.5".to_string(),
            None,
        );

        match stream_event {
            LlmStreamEvent::Done(metadata) => {
                // 1000 reported − 800 cached = 200 non-cached input.
                assert_eq!(metadata.prompt_tokens, Some(200));
                assert_eq!(metadata.cache_read_tokens, Some(800));
                // total_tokens stays the true prompt+output total (1000 + 20).
                assert_eq!(metadata.total_tokens, Some(1020));
            }
            other => panic!("expected Done event, got {other:?}"),
        }
    }

    #[test]
    fn test_incomplete_event_maps_output_limit_to_length() {
        let event_json = r#"{
            "type": "response.incomplete",
            "sequence_number": 10,
            "response": {
                "id": "resp_incomplete",
                "object": "response",
                "created_at": 1780000000,
                "status": "incomplete",
                "incomplete_details": { "reason": "max_output_tokens" },
                "model": "gpt-5.5",
                "output": [],
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "total_tokens": 15
                }
            }
        }"#;

        let event: StreamingEvent = serde_json::from_str(event_json).unwrap();
        let stream_event = handle_streaming_event(
            event,
            &Mutex::new(0),
            &Mutex::new(0),
            &Mutex::new(None),
            &Mutex::new(Vec::new()),
            &Mutex::new(None),
            "gpt-5.5".to_string(),
            None,
        );

        match stream_event {
            LlmStreamEvent::Done(metadata) => {
                assert_eq!(metadata.finish_reason.as_deref(), Some("length"));
            }
            other => panic!("expected Done event, got {other:?}"),
        }
    }

    #[test]
    fn test_sanitize_parameters_adds_missing_properties() {
        let params = json!({"type": "object", "additionalProperties": false});
        let sanitized = OpenResponsesProtocolChatDriver::sanitize_parameters(&params);
        assert_eq!(
            sanitized,
            json!({"type": "object", "properties": {}, "additionalProperties": false})
        );
    }

    #[test]
    fn test_sanitize_parameters_preserves_existing_properties() {
        let params = json!({"type": "object", "properties": {"x": {"type": "string"}}, "additionalProperties": false});
        let sanitized = OpenResponsesProtocolChatDriver::sanitize_parameters(&params);
        assert_eq!(sanitized, params);
    }

    #[test]
    fn test_sanitize_parameters_ignores_non_object_types() {
        let params = json!({"type": "string"});
        let sanitized = OpenResponsesProtocolChatDriver::sanitize_parameters(&params);
        assert_eq!(sanitized, params);
    }

    #[test]
    fn test_sanitize_parameters_rewrites_resend_email_lookaround() {
        let params = json!({
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "pattern": "^(?!\\.)(?!.*\\.\\.)([A-Za-z0-9_'+\\-\\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\\-]*\\.)+[A-Za-z]{2,}$"
                }
            }
        });

        let sanitized = OpenResponsesProtocolChatDriver::sanitize_parameters(&params);
        let pattern = sanitized["properties"]["email"]["pattern"]
            .as_str()
            .unwrap();

        assert!(!pattern.contains("(?!"));
        assert!(pattern.contains('@'));
    }

    // ========================================================================
    // Provider-owned request auth (EVE-618 / EVE-856)
    // ========================================================================

    /// Minimal `LlmCallConfig` for wire tests.
    fn auth_test_config() -> LlmCallConfig {
        LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "gpt-5.4".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        }
    }

    /// Static auth provider that records how many times it was awaited, so tests
    /// can assert per-attempt resolution (refreshable providers).
    struct CountingAuth {
        header: (String, String),
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl crate::runtime_provider::ProviderAuth for CountingAuth {
        async fn headers(
            &self,
            _request: crate::runtime_provider::ProviderAuthRequest<'_>,
        ) -> Result<Vec<(String, String)>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(vec![self.header.clone()])
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    /// Extension that injects a non-auth header and (deliberately) a conflicting
    /// `Authorization` header, to prove the auth seam wins on conflict.
    struct HeaderInjectingExtension;

    impl OpenResponsesRequestExtension for HeaderInjectingExtension {
        fn decorate(&self, _body: &mut Value, _config: &LlmCallConfig) -> Result<()> {
            Ok(())
        }

        fn decorate_headers(&self, headers: &mut HeaderMap, _config: &LlmCallConfig) -> Result<()> {
            headers.insert(
                "x-openrouter-route",
                reqwest::header::HeaderValue::from_static("fallback"),
            );
            // Decoration must never override auth — the driver applies auth last.
            headers.insert(
                "authorization",
                reqwest::header::HeaderValue::from_static("Bearer decoration"),
            );
            Ok(())
        }
    }

    #[tokio::test]
    async fn provider_resolves_bearer_auth() {
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "openai-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url("https://api.openai.com/v1")
        .auth(crate::runtime_provider::BearerAuth::new("secret-key"));
        let resolved = provider
            .endpoint()
            .resolve("POST", "https://api.openai.com/v1/responses", b"{}")
            .await
            .expect("auth resolves");
        assert_eq!(
            resolved.headers,
            vec![("authorization".into(), "Bearer secret-key".into())]
        );
    }

    #[tokio::test]
    async fn provider_selects_auth_independently_of_host() {
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "azure-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url("https://my-resource.openai.azure.com/openai/v1")
        .auth(crate::runtime_provider::StaticHeaderAuth::new(
            "api-key",
            "secret-key",
        ));
        let resolved = provider
            .endpoint()
            .resolve(
                "POST",
                "https://my-resource.openai.azure.com/openai/v1/responses",
                b"{}",
            )
            .await
            .expect("auth resolves");
        assert_eq!(
            resolved.headers,
            vec![("api-key".into(), "secret-key".into())]
        );
    }

    #[tokio::test]
    async fn refreshable_provider_auth_is_resolved() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "refreshable-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url("https://service.example/v1")
        .auth_arc(std::sync::Arc::new(CountingAuth {
            header: (
                "Authorization".to_string(),
                "Bearer minted-token".to_string(),
            ),
            calls: calls.clone(),
        }));
        let resolved = provider
            .endpoint()
            .resolve("POST", "https://service.example/v1/responses", b"{}")
            .await
            .expect("auth resolves");
        assert_eq!(
            resolved.headers,
            vec![("authorization".into(), "Bearer minted-token".into())]
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn default_static_auth_applied_on_the_wire() {
        use wiremock::matchers::{header, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("authorization", "Bearer wire-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "wire-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(crate::runtime_provider::BearerAuth::new("wire-key"));
        let driver = OpenResponsesProtocolChatDriver::new();
        let messages = vec![LlmMessage::text(LlmMessageRole::User, "hi")];
        let _ = driver
            .chat_completion_stream(endpoint.endpoint(), messages, &auth_test_config())
            .await;

        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "default static key must authenticate the request"
        );
    }

    #[tokio::test]
    async fn auth_provider_header_wins_over_extension_header() {
        use wiremock::matchers::{header, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // The request only matches if the auth header is the minted token (not the
        // extension's decoration value) AND the non-auth decoration is present.
        Mock::given(method("POST"))
            .and(header("authorization", "Bearer minted-token"))
            .and(header("x-openrouter-route", "fallback"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "auth-wins-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth_arc(std::sync::Arc::new(CountingAuth {
            header: (
                "Authorization".to_string(),
                "Bearer minted-token".to_string(),
            ),
            calls: calls.clone(),
        }));
        let driver = OpenResponsesProtocolChatDriver::new()
            .with_request_extension(std::sync::Arc::new(HeaderInjectingExtension));

        let messages = vec![LlmMessage::text(LlmMessageRole::User, "hi")];
        let _ = driver
            .chat_completion_stream(endpoint.endpoint(), messages, &auth_test_config())
            .await;

        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "auth header must win over a conflicting decoration header"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn auth_provider_awaited_on_each_retry_attempt() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Always 503 (transient): the driver exhausts its retries, awaiting auth
        // before every attempt.
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
            .mount(&server)
            .await;

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fast_retry = LlmRetryConfig {
            max_retries: 1,
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(1),
            backoff_multiplier: 1.0,
            jitter_factor: 0.0,
            ..Default::default()
        };
        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "retry-auth-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth_arc(std::sync::Arc::new(CountingAuth {
            header: (
                "Authorization".to_string(),
                "Bearer minted-token".to_string(),
            ),
            calls: calls.clone(),
        }));
        let driver = OpenResponsesProtocolChatDriver::new().with_retry_config(fast_retry);

        let messages = vec![LlmMessage::text(LlmMessageRole::User, "hi")];
        let _ = driver
            .chat_completion_stream(endpoint.endpoint(), messages, &auth_test_config())
            .await;

        // Initial attempt + one retry = two auth resolutions.
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "refreshable auth must be resolved per HTTP attempt, including retries"
        );
    }

    #[test]
    fn function_tools_serialize_strict_only_for_compatible_schemas() {
        let mut compatible = make_tool("lookup", None, crate::tool_types::DeferrablePolicy::Never);
        match &mut compatible {
            ToolDefinition::Builtin(tool) => {
                tool.parameters = json!({
                    "type": "object", "properties": {"query": {"type": "string"}}
                })
            }
            ToolDefinition::ClientSide(_) => unreachable!(),
        }
        let serialized =
            serde_json::to_value(&OpenResponsesProtocolChatDriver::convert_tools(&[compatible])[0])
                .unwrap();
        assert_eq!(serialized["strict"], true);
        assert_eq!(serialized["parameters"]["required"], json!(["query"]));

        let mut incompatible =
            make_tool("lookup", None, crate::tool_types::DeferrablePolicy::Never);
        match &mut incompatible {
            ToolDefinition::Builtin(tool) => {
                tool.parameters = json!({
                    "type": "object", "allOf": [{"type": "object"}]
                })
            }
            ToolDefinition::ClientSide(_) => unreachable!(),
        }
        let serialized = serde_json::to_value(
            &OpenResponsesProtocolChatDriver::convert_tools(&[incompatible])[0],
        )
        .unwrap();
        assert!(serialized.get("strict").is_none());
        assert!(serialized["parameters"].get("allOf").is_some());
    }
    fn cache_config() -> LlmCallConfig {
        let mut config = auth_test_config();
        config
            .metadata
            .insert("session_id".into(), "session-one".into());
        config.prompt_cache = Some(crate::driver_registry::PromptCacheConfig {
            enabled: true,
            strategy: crate::driver_registry::PromptCacheStrategy::Auto,
            gemini_cached_content: None,
        });
        config
    }

    #[test]
    fn cache_key_tracks_stable_prefix_and_family_but_not_turn_input() {
        let base = cache_config();
        let instructions = Some("stable system prompt".into());
        let key = |config: &LlmCallConfig,
                   instructions: &Option<String>,
                   tools: &Option<Vec<ResponsesTool>>,
                   input: &[ResponsesInputItem]| {
            OpenResponsesProtocolChatDriver::build_prompt_cache_key(
                config,
                input,
                instructions,
                tools,
            )
        };
        let expected = key(&base, &instructions, &None, &[]).unwrap();
        assert_eq!(expected.len(), 64);
        assert!(expected.starts_with("everruns:"));
        assert!(expected[9..].bytes().all(|byte| byte.is_ascii_hexdigit()));
        let (_, changed_input) = OpenResponsesProtocolChatDriver::build_input(
            &[LlmMessage::text(LlmMessageRole::User, "different turn")],
            false,
        );
        assert_eq!(
            key(&base, &instructions, &None, &changed_input),
            Some(expected.clone())
        );
        let mut disabled = base.clone();
        disabled.prompt_cache.as_mut().unwrap().enabled = false;
        assert_eq!(key(&disabled, &instructions, &None, &[]), None);
        disabled.prompt_cache = None;
        assert_eq!(key(&disabled, &instructions, &None, &[]), None);
        for field in ["session_id", "model", "instructions", "tools"] {
            let mut config = base.clone();
            let mut prompt = instructions.clone();
            let mut tools = None;
            match field {
                "session_id" => {
                    config
                        .metadata
                        .insert("session_id".into(), "session-two".into());
                }
                "model" => config.model = "other-model".into(),
                "instructions" => prompt = Some("different system prompt".into()),
                "tools" => {
                    tools = Some(OpenResponsesProtocolChatDriver::convert_tools(&[
                        make_tool("lookup", None, crate::tool_types::DeferrablePolicy::Never),
                    ]))
                }
                _ => unreachable!(),
            }
            assert_ne!(
                key(&config, &prompt, &tools, &[]).unwrap(),
                expected,
                "{field}"
            );
        }
        // More specific scopes take precedence; unrelated metadata is not part of the prefix.
        let mut scoped = base.clone();
        scoped.metadata.extend([
            ("agent_id".into(), "agent".into()),
            ("harness_id".into(), "harness".into()),
            ("org_id".into(), "org".into()),
            ("trace_id".into(), "trace".into()),
        ]);
        assert_eq!(
            key(&scoped, &instructions, &None, &[]),
            Some(expected.clone())
        );
        let mut previous = Some(expected);
        for scope in ["session_id", "agent_id", "harness_id", "org_id"] {
            scoped.metadata.remove(scope);
            let current = key(&scoped, &instructions, &None, &[]).unwrap();
            if let Some(previous) = previous {
                assert_ne!(current, previous);
            }
            previous = Some(current);
        }
    }

    fn search_tools() -> Vec<ToolDefinition> {
        use crate::tool_types::DeferrablePolicy::{Always, Automatic, Never};
        vec![
            make_tool("z", Some("Zeta"), Automatic),
            make_tool("first", Some("HiddenCategory"), Never),
            make_tool("a", Some("Alpha"), Always),
            make_tool("loose", None, Automatic),
            make_tool("second", None, Never),
            make_tool("b", Some("Alpha"), Automatic),
        ]
    }

    fn expected_search_tools() -> Value {
        // Independent literal wire contract; only repeated fixture names are parameterized.
        let function = |name: &str, deferred: bool| {
            let mut value = json!({"type":"function","name":name,"description":format!("{name} description"),"parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false},"strict":true});
            if deferred {
                value["defer_loading"] = json!(true);
            }
            value
        };
        json!([
            function("first", false), function("second", false),
            {"type":"namespace","name":"Alpha","description":"Tools for Alpha","tools":[function("a", true),function("b", true)]},
            {"type":"namespace","name":"Zeta","description":"Tools for Zeta","tools":[function("z", true)]},
            function("loose", true), {"type":"tool_search"}
        ])
    }

    #[test]
    fn tool_search_has_complete_stable_wire_order_and_threshold_boundary() {
        let tools = search_tools();
        let expected = expected_search_tools();
        let generated: Vec<_> = (0..32)
            .map(|_| OpenResponsesProtocolChatDriver::convert_tools_with_search(&tools, 6))
            .collect();
        let keys: HashSet<_> = generated
            .iter()
            .map(|tools| {
                OpenResponsesProtocolChatDriver::build_prompt_cache_key(
                    &cache_config(),
                    &[],
                    &None,
                    &Some(tools.clone()),
                )
                .unwrap()
            })
            .collect();
        assert_eq!(
            keys.len(),
            1,
            "identical tool sets must produce one cache key"
        );
        for actual in generated {
            assert_eq!(serde_json::to_value(actual).unwrap(), expected);
        }
        let fallback = OpenResponsesProtocolChatDriver::convert_tools_with_search(&tools, 7);
        let expected_fallback: Vec<Value> = ["z", "first", "a", "loose", "second", "b"].into_iter().map(|name| json!({"type":"function","name":name,"description":format!("{name} description"),"parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false},"strict":true})).collect();
        assert_eq!(
            serde_json::to_value(fallback).unwrap(),
            json!(expected_fallback)
        );
        assert_eq!(
            serde_json::to_value(OpenResponsesProtocolChatDriver::convert_tools_with_search(
                &[],
                1
            ))
            .unwrap(),
            json!([])
        );
    }

    #[tokio::test]
    async fn equivalent_search_requests_keep_cache_key_and_complete_tool_payload() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-cache\",\"status\":\"completed\",\"output\":[]}}\n\n")).expect(2).mount(&server).await;
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "cache-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(server.uri());
        let driver = OpenResponsesProtocolChatDriver::new()
            .with_native_features(false, true)
            .with_retry_config(LlmRetryConfig::no_retry());
        let mut config = cache_config();
        config.tools = search_tools();
        config.tool_search = Some(crate::driver_registry::ToolSearchConfig {
            enabled: true,
            threshold: 6,
        });
        for input in ["first turn", "second turn"] {
            let mut stream = driver
                .chat_completion_stream(
                    provider.endpoint(),
                    vec![
                        LlmMessage::text(LlmMessageRole::System, "stable system prompt"),
                        LlmMessage::text(LlmMessageRole::User, input),
                    ],
                    &config,
                )
                .await
                .unwrap();
            let mut completions = 0;
            while let Some(event) = stream.next().await {
                match event.unwrap() {
                    LlmStreamEvent::Done(metadata) => {
                        assert_eq!(metadata.finish_reason.as_deref(), Some("stop"));
                        completions += 1;
                    }
                    other => panic!("unexpected event: {other:?}"),
                }
            }
            assert_eq!(completions, 1);
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let bodies: Vec<Value> = requests
            .iter()
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect();
        for body in &bodies {
            assert_eq!(body["tools"], expected_search_tools());
            assert_eq!(body["instructions"], "stable system prompt");
            assert_eq!(body["prompt_cache_key"].as_str().unwrap().len(), 64);
        }
        assert_ne!(bodies[0]["input"], bodies[1]["input"]);
        assert_eq!(bodies[0]["prompt_cache_key"], bodies[1]["prompt_cache_key"]);
    }
}
