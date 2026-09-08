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
    ///     reasoning_state: None,
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
        let explicit = request.reasoning_state.is_some();
        let mut compact_url = url::Url::parse(&responses_url)
            .map_err(|e| AgentLoopError::config(format!("Invalid compact endpoint URL: {e}")))?;
        if !explicit {
            compact_url.set_path(&format!(
                "{}/compact",
                compact_url.path().trim_end_matches('/')
            ));
        }
        let compact_url = compact_url.to_string();
        let mut body = serde_json::to_value(&request).map_err(|e| {
            AgentLoopError::llm(format!("failed to serialize compact request: {e}"))
        })?;
        if let Some(state) = &request.reasoning_state {
            if !self.native_phases
                || !crate::reasoning_updates::supports_configuration_updates(&request.model)
                || !state.is_supported()
            {
                return Err(AgentLoopError::Configuration(
                    "configuration updates require native Astra Responses".into(),
                ));
            }
            // Explicit compaction accepts updates; /responses/compact does not.
            // Do not inherit generation max_tokens: the API requires >=20,000
            // when a trigger request supplies max_output_tokens.
            let input = body["input"].as_array_mut().ok_or_else(|| {
                AgentLoopError::Configuration("explicit compaction needs full input".into())
            })?;
            input.push(serde_json::json!({"type": "compaction_trigger"}));
            body["stream"] = serde_json::json!(false);
            body["store"] = serde_json::json!(false);
            body["max_output_tokens"] = serde_json::json!(20_000);
            body["include"] = serde_json::json!(["reasoning.encrypted_content"]);
            if let Some(effort) = state.baseline {
                body["reasoning"] = serde_json::json!({"effort": effort});
            }
        }
        let body = serde_json::to_vec(&body).map_err(|e| {
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
        let value: Value = response
            .json()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to parse compact response: {}", e)))?;

        if explicit && value["status"] != "completed" {
            return Err(AgentLoopError::llm("explicit compaction did not complete"));
        }
        let explicit_output = explicit.then(|| value["output"].clone());
        let mut compact_response: CompactResponse = serde_json::from_value(value)
            .map_err(|e| AgentLoopError::llm(format!("Failed to parse compact response: {e}")))?;
        if explicit {
            compact_response.output = explicit_output
                .unwrap()
                .as_array()
                .ok_or_else(|| AgentLoopError::llm("explicit compaction returned invalid output"))?
                .iter()
                .map(|item| {
                    if item["type"] == "compaction" {
                        serde_json::from_value(item.clone()).map_err(|e| {
                            AgentLoopError::llm(format!("invalid compaction item: {e}"))
                        })
                    } else {
                        Ok(CompactOutputItem::ProviderItem(item.clone()))
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            let boundary = compact_response
                .output
                .iter()
                .rposition(|item| matches!(item, CompactOutputItem::Compaction { .. }))
                .ok_or_else(|| {
                    AgentLoopError::llm("explicit compaction returned no compaction item")
                })?;
            // /responses output uses the latest compaction as its replacement
            // boundary. Retain every following item, including opaque reasoning.
            compact_response.output.drain(..boundary);
            // Responses usage counts generation, not the size of the resulting
            // compacted window. The engine compares serialized bytes instead.
            if let Some(usage) = compact_response.usage.as_mut() {
                usage.output_tokens = None;
            }
        }

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
            if supports_phases && let Some(effort) = msg.configuration_update {
                input_items.push(configuration_update_item(effort));
            }
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
                    // THREAT[TM-LLM-034]: do not replay foreign-provider opaque artifacts.
                    if item.provider != "openai" {
                        continue;
                    }
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
fn configuration_update_item(effort: crate::model::ReasoningEffort) -> ResponsesInputItem {
    ResponsesInputItem::ConfigurationUpdate {
        r#type: "configuration_update".into(),
        reasoning: crate::compact::ConfigurationReasoning { effort },
    }
}

fn coalesce_configuration_updates(items: Vec<ResponsesInputItem>) -> Vec<ResponsesInputItem> {
    let mut output = Vec::with_capacity(items.len());
    for item in items {
        if matches!(item, ResponsesInputItem::ConfigurationUpdate { .. })
            && matches!(
                output.last(),
                Some(ResponsesInputItem::ConfigurationUpdate { .. })
            )
        {
            output.pop();
        }
        output.push(item);
    }
    output
}

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
    coalesce_configuration_updates(if previous_response_id.is_some() {
        compute_delta_input_items(input_items)
    } else {
        repair_unpaired_function_call_items(input_items)
    })
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
        || message.contains("previous_response_not_found")
        || (message.contains("previous response") && message.contains("not found"))
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

        let (instructions, mut transcript_input_items) =
            Self::build_input(&messages, supports_phases);
        let update_state = config.reasoning_state.as_ref().filter(|_| {
            supports_phases
                && crate::reasoning_updates::supports_configuration_updates(&config.model)
        });
        if let Some(state) = update_state {
            if !state.is_supported() {
                return Err(AgentLoopError::Configuration(
                    "unsupported Astra configuration effort".into(),
                ));
            }
            if let Some(effort) = state.pending {
                // Insert before the fresh input, not before the preceding
                // assistant output, so delta trimming and fallback agree.
                let delta_len = compute_delta_input_items(transcript_input_items.clone()).len();
                let boundary = transcript_input_items.len() - delta_len;
                transcript_input_items.insert(boundary, configuration_update_item(effort));
            }
            transcript_input_items = coalesce_configuration_updates(transcript_input_items);
        } else {
            transcript_input_items
                .retain(|item| !matches!(item, ResponsesInputItem::ConfigurationUpdate { .. }));
        }
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
                reasoning_state,
            }) => {
                previous_response_id = None;
                let mut input_items: Vec<_> = output.iter().map(ResponsesInputItem::from).collect();
                if update_state.is_some()
                    && let Some(effort) = reasoning_state.as_ref().and_then(|state| state.effective)
                {
                    input_items.push(configuration_update_item(effort));
                }
                input_items.extend(transcript_input_items);
                coalesce_configuration_updates(input_items)
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
        let include = (reasoning.is_some() || update_state.is_some())
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
                request.input = coalesce_configuration_updates(
                    repair_unpaired_function_call_items(full_replay_input_items),
                );
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
                                            return Ok(complete_tool_call(
                                                &accumulated_tool_calls,
                                                &finish_reason,
                                                item.get("id").and_then(Value::as_str).unwrap_or(""),
                                                item.get("call_id").and_then(Value::as_str),
                                                item.get("name").and_then(Value::as_str),
                                                item.get("arguments").and_then(Value::as_str),
                                            ));
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
                                            response_id: response_obj
                                                .get("id")
                                                .and_then(Value::as_str)
                                                .map(str::to_owned),
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

// ToolCalls are cumulative snapshots: consumers replace their current call set.
// Terminal item arguments are authoritative even when deltas were absent or partial.
fn complete_tool_call(
    accumulated: &Mutex<Vec<ToolCallAccumulator>>,
    finish_reason: &Mutex<Option<String>>,
    id: &str,
    call_id: Option<&str>,
    name: Option<&str>,
    arguments: Option<&str>,
) -> LlmStreamEvent {
    let mut calls = accumulated.lock().unwrap();
    if !id.is_empty() {
        let index = calls
            .iter()
            .position(|call| call.id == id)
            .unwrap_or_else(|| {
                calls.push(ToolCallAccumulator {
                    id: id.into(),
                    ..Default::default()
                });
                calls.len() - 1
            });
        let call = &mut calls[index];
        if let Some(call_id) = call_id {
            call.call_id = call_id.into();
        }
        if let Some(name) = name {
            call.name = name.into();
        }
        if let Some(arguments) = arguments {
            call.arguments = arguments.into();
        }
    }
    let snapshot: Vec<_> = calls
        .iter()
        .filter(|call| !call.name.is_empty())
        .map(|call| ToolCall {
            id: call.call_id.clone(),
            name: call.name.clone(),
            arguments: serde_json::from_str(&call.arguments).unwrap_or(json!({})),
        })
        .collect();
    if snapshot.is_empty() {
        LlmStreamEvent::TextDelta(String::new())
    } else {
        *finish_reason.lock().unwrap() = Some("tool_calls".into());
        LlmStreamEvent::ToolCalls(snapshot)
    }
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
                Some(types::OutputItem::FunctionCall {
                    id,
                    call_id,
                    name,
                    arguments,
                    ..
                }) => complete_tool_call(
                    accumulated_tool_calls,
                    finish_reason,
                    &id,
                    Some(&call_id),
                    Some(&name),
                    Some(&arguments),
                ),
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
    ConfigurationUpdate {
        r#type: String,
        reasoning: crate::compact::ConfigurationReasoning,
    },
    ProviderItem(Value),
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
            CompactOutputItem::ProviderItem(item) => Self::ProviderItem(item.clone()),
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

    #[tokio::test]
    async fn rejected_stateful_continuation_replays_repaired_transcript_once() {
        use crate::tool_types::ToolCall;
        use futures::StreamExt;
        use serde_json::json;
        use wiremock::matchers::{body_partial_json, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::builder().start().await;
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
                configuration_update: None,
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("[package]".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                phase: None,
                reasoning: Vec::new(),
                configuration_update: None,
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
            reasoning_state: None,
        };

        let mut stream = driver
            .chat_completion_stream(endpoint.endpoint(), messages, &config)
            .await
            .expect("continuation should recover");
        let mut completion = None;
        while let Some(event) = stream.next().await {
            if let LlmStreamEvent::Done(metadata) = event.expect("valid recovered event") {
                assert!(
                    completion.replace(metadata).is_none(),
                    "exactly one completion"
                );
            }
        }
        let completion = completion.expect("recovery completed");
        assert_eq!(completion.response_id.as_deref(), Some("resp_recovered"));
        assert_eq!(completion.prompt_tokens, Some(4));
        assert_eq!(completion.completion_tokens, Some(1));

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 2);
        let first: serde_json::Value = requests[0].body_json().expect("first body");
        let second: serde_json::Value = requests[1].body_json().expect("second body");
        assert_eq!(
            first,
            json!({"model":"gpt-5.4","stream":true,"previous_response_id":"resp_tool_turn","input":[{"type":"function_call_output","call_id":"call_1","output":"[package]"}]})
        );
        assert_eq!(
            second,
            json!({"model":"gpt-5.4","stream":true,"input":[
                {"type":"message","role":"user","content":"inspect the project"},
                {"type":"function_call","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"Cargo.toml\"}"},
                {"type":"function_call_output","call_id":"call_1","output":"[package]"}
            ]})
        );
    }

    #[tokio::test]
    async fn openrouter_provider_does_not_send_hosted_tool_search() {
        use crate::tool_types::DeferrablePolicy;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(successful_auth_stream())
            .expect(1)
            .mount(&server)
            .await;
        let endpoint = crate::runtime_provider::RuntimeProvider::new(
            "gateway",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(server.uri());
        let driver =
            OpenResponsesProtocolChatDriver::new().with_retry_config(LlmRetryConfig::no_retry());
        let mut config = auth_test_config();
        config.tools = ["first", "second"]
            .into_iter()
            .map(|name| make_tool(name, Some("General"), DeferrablePolicy::Automatic))
            .collect();
        config.tool_search = Some(crate::driver_registry::ToolSearchConfig {
            enabled: true,
            threshold: 1,
        });
        assert_authenticated_stream(
            driver
                .chat_completion_stream(
                    endpoint.endpoint(),
                    vec![LlmMessage::text(LlmMessageRole::User, "question")],
                    &config,
                )
                .await
                .unwrap(),
        )
        .await;
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].body_json::<Value>().unwrap(),
            json!({
                "model":"gpt-5.4","stream":true,"input":[{"type":"message","role":"user","content":"question"}],
                "tools":[
                    {"type":"function","name":"first","description":"first description","parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false},"strict":true},
                    {"type":"function","name":"second","description":"second description","parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false},"strict":true}
                ]
            })
        );
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
        let server = MockServer::builder().start().await;
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
            reasoning_state: None,
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
    async fn streamed_tool_calls_use_terminal_arguments_in_typed_and_fallback_events() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for typed in [true, false] {
            let mut events = vec![
                json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"function_call","id":"fc-a","call_id":"call-a","name":"first","arguments":"","status":"in_progress"}}),
                json!({"type":"response.function_call_arguments.delta","sequence_number":2,"output_index":0,"item_id":"fc-a","delta":"{\"a\":"}),
                json!({"type":"response.output_item.done","sequence_number":4,"output_index":0,"item":{"type":"function_call","id":"fc-a","call_id":"call-a","name":"first","arguments":"{\"a\":1}","status":"completed"}}),
                json!({"type":"response.output_item.done","sequence_number":5,"output_index":1,"item":{"type":"function_call","id":"fc-b","call_id":"call-b","name":"second","arguments":"{\"b\":2}","status":"completed"}}),
                json!({"type":"response.completed","sequence_number":6,"response":{"id":"resp-tools","object":"response","created_at":1,"status":"completed","model":"gpt-5.4","output":[],"usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}}),
            ];
            for event in &mut events {
                if !typed {
                    event.as_object_mut().unwrap().remove("sequence_number");
                }
                assert_eq!(
                    serde_json::from_value::<StreamingEvent>(event.clone()).is_ok(),
                    typed,
                    "{event}"
                );
            }
            let mut wire = events
                .iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            wire.push_str("data: [DONE]\n\n");
            let server = MockServer::builder().start().await;
            Mock::given(method("POST"))
                .and(path("/v1/responses"))
                .and(header("authorization", "Bearer synthetic-key"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "text/event-stream")
                        .set_body_string(wire),
                )
                .expect(1)
                .mount(&server)
                .await;
            let provider = crate::runtime_provider::RuntimeProvider::new(
                "test",
                OpenResponsesProtocolChatDriver::new(),
            )
            .base_url(format!("{}/v1", server.uri()))
            .auth(crate::runtime_provider::BearerAuth::new("synthetic-key"));
            let mut config = auth_test_config();
            for (name, argument) in [("first", "a"), ("second", "b")] {
                let mut tool = make_tool(name, None, crate::tool_types::DeferrablePolicy::Never);
                let ToolDefinition::Builtin(definition) = &mut tool else {
                    unreachable!()
                };
                definition.parameters = json!({"type":"object","properties":{argument:{"type":"integer"}},"required":[argument],"additionalProperties":false});
                config.tools.push(tool);
            }
            let response = OpenResponsesProtocolChatDriver::new()
                .with_retry_config(LlmRetryConfig::no_retry())
                .chat_completion(
                    provider.endpoint(),
                    vec![LlmMessage::text(LlmMessageRole::User, "run both")],
                    &config,
                )
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(response.tool_calls.unwrap()).unwrap(),
                json!([
                    {"id":"call-a","name":"first","arguments":{"a":1}},
                    {"id":"call-b","name":"second","arguments":{"b":2}},
                ]),
                "typed={typed}"
            );
            assert_eq!(
                response.metadata.finish_reason.as_deref(),
                Some("tool_calls")
            );
            assert_eq!(response.metadata.response_id.as_deref(), Some("resp-tools"));
            assert_eq!(
                (
                    response.metadata.prompt_tokens,
                    response.metadata.completion_tokens,
                    response.metadata.total_tokens
                ),
                (Some(4), Some(2), Some(6))
            );
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].body_json::<Value>().unwrap(),
                json!({"model":"gpt-5.4","input":[{"type":"message","role":"user","content":"run both"}],"stream":true,"tools":[
                    {"type":"function","name":"first","description":"first description","parameters":{"type":"object","properties":{"a":{"type":"integer"}},"required":["a"],"additionalProperties":false},"strict":true},
                    {"type":"function","name":"second","description":"second description","parameters":{"type":"object","properties":{"b":{"type":"integer"}},"required":["b"],"additionalProperties":false},"strict":true},
                ]})
            );
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
        assert_eq!(
            error.message,
            "An error occurred while processing your request."
        );
        assert!(crate::llm_retry::is_transient_stream_error(&error));
    }

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
            reasoning_state: None,
        }
    }

    struct SignedRequest {
        method: String,
        url: String,
        body: Vec<u8>,
    }

    struct RecordingAuth {
        requests: Arc<Mutex<Vec<SignedRequest>>>,
        fail_on: Option<usize>,
    }

    #[async_trait::async_trait]
    impl crate::runtime_provider::ProviderAuth for RecordingAuth {
        async fn headers(
            &self,
            request: crate::runtime_provider::ProviderAuthRequest<'_>,
        ) -> Result<Vec<(String, String)>> {
            let mut requests = self.requests.lock().unwrap();
            requests.push(SignedRequest {
                method: request.method.into(),
                url: request.url.into(),
                body: request.body.to_vec(),
            });
            let attempt = requests.len();
            if self.fail_on == Some(attempt) {
                return Err(AgentLoopError::llm_kind(
                    LlmErrorKind::Authentication,
                    "token refresh refused",
                ));
            }
            Ok(vec![(
                "Authorization".into(),
                format!("Bearer token-{attempt}"),
            )])
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    struct HeaderInjectingExtension;

    impl OpenResponsesRequestExtension for HeaderInjectingExtension {
        fn decorate(&self, body: &mut Value, _config: &LlmCallConfig) -> Result<()> {
            body["routing_marker"] = json!("decorated");
            Ok(())
        }

        fn decorate_headers(&self, headers: &mut HeaderMap, _config: &LlmCallConfig) -> Result<()> {
            headers.insert(
                "x-route",
                reqwest::header::HeaderValue::from_static("fallback"),
            );
            headers.insert(
                "authorization",
                reqwest::header::HeaderValue::from_static("Bearer decoration"),
            );
            Ok(())
        }
    }

    fn successful_auth_stream() -> wiremock::ResponseTemplate {
        wiremock::ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"authenticated\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-auth\",\"status\":\"completed\",\"output\":[]}}\n\n"
            ))
    }

    async fn assert_authenticated_stream(mut stream: LlmResponseStream) {
        let mut text = String::new();
        let mut finishes = Vec::new();
        while let Some(event) = stream.next().await {
            match event.expect("stream transport succeeds") {
                LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
                LlmStreamEvent::Done(metadata) => finishes.push(metadata.finish_reason),
                other => panic!("unexpected auth response event: {other:?}"),
            }
        }
        assert_eq!(text, "authenticated");
        assert_eq!(finishes, vec![Some("stop".into())]);
    }

    #[tokio::test]
    async fn auth_headers_reach_wire_with_explicit_precedence_and_successful_response() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};
        for (static_header, caller_override) in [(false, false), (true, false), (false, true)] {
            let server = MockServer::builder().start().await;
            Mock::given(method("POST"))
                .and(path("/v1/responses"))
                .respond_with(successful_auth_stream())
                .expect(1)
                .mount(&server)
                .await;
            let provider = crate::runtime_provider::RuntimeProvider::new(
                "auth-test",
                OpenResponsesProtocolChatDriver::new(),
            )
            .base_url(format!("{}/v1", server.uri()));
            let provider = if static_header {
                provider.auth(crate::runtime_provider::StaticHeaderAuth::new(
                    "API-Key",
                    "static-key",
                ))
            } else {
                provider.auth(crate::runtime_provider::BearerAuth::new("wire-key"))
            };
            let mut config = auth_test_config();
            if caller_override {
                config.extra_headers = vec![
                    ("AUTHORIZATION".into(), "Bearer caller".into()),
                    ("X-Route".into(), "caller-route".into()),
                ];
            }
            let driver = OpenResponsesProtocolChatDriver::new()
                .with_retry_config(LlmRetryConfig::no_retry())
                .with_request_extension(Arc::new(HeaderInjectingExtension));
            let stream = driver
                .chat_completion_stream(
                    provider.endpoint(),
                    vec![LlmMessage::text(LlmMessageRole::User, "hi")],
                    &config,
                )
                .await
                .unwrap();
            assert_authenticated_stream(stream).await;
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            let headers = &requests[0].headers;
            let expected = if caller_override {
                "Bearer caller"
            } else if static_header {
                "Bearer decoration"
            } else {
                "Bearer wire-key"
            };
            assert_eq!(
                headers
                    .get_all("authorization")
                    .iter()
                    .map(|h| h.to_str().unwrap())
                    .collect::<Vec<_>>(),
                vec![expected]
            );
            assert_eq!(
                headers.get("api-key").map(|h| h.to_str().unwrap()),
                static_header.then_some("static-key")
            );
            assert_eq!(
                headers["x-route"],
                if caller_override {
                    "caller-route"
                } else {
                    "fallback"
                }
            );
            assert_eq!(headers["content-type"], "application/json");
            let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(body["routing_marker"], "decorated");
            assert_eq!(body["model"], "gpt-5.4");
        }
    }

    fn auth_retry_config() -> LlmRetryConfig {
        LlmRetryConfig {
            max_retries: 1,
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(1),
            backoff_multiplier: 1.0,
            jitter_factor: 0.0,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn refreshed_tokens_and_signed_payload_reach_each_retry_attempt() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer token-1"))
            .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer token-2"))
            .respond_with(successful_auth_stream())
            .expect(1)
            .mount(&server)
            .await;
        let signed = Arc::new(Mutex::new(Vec::new()));
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "auth-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(RecordingAuth {
            requests: signed.clone(),
            fail_on: None,
        });
        let driver = OpenResponsesProtocolChatDriver::new()
            .with_retry_config(auth_retry_config())
            .with_request_extension(Arc::new(HeaderInjectingExtension));
        let stream = driver
            .chat_completion_stream(
                provider.endpoint(),
                vec![LlmMessage::text(LlmMessageRole::User, "hi")],
                &auth_test_config(),
            )
            .await
            .unwrap();
        assert_authenticated_stream(stream).await;
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].headers["authorization"], "Bearer token-1");
        assert_eq!(requests[1].headers["authorization"], "Bearer token-2");
        assert_eq!(requests[0].body, requests[1].body);
        let signed = signed.lock().unwrap();
        assert_eq!(signed.len(), 2);
        for (attempt, request) in signed.iter().zip(&requests) {
            assert_eq!(attempt.method, "POST");
            assert_eq!(attempt.url, format!("{}/v1/responses", server.uri()));
            assert_eq!(attempt.body, request.body);
            let body: Value = serde_json::from_slice(&attempt.body).unwrap();
            assert_eq!(body["routing_marker"], "decorated");
        }
    }

    #[tokio::test]
    async fn auth_failure_aborts_before_sending_or_reusing_an_expired_token() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for fail_on in [1, 2] {
            let server = MockServer::builder().start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
                .expect((fail_on - 1) as u64)
                .mount(&server)
                .await;
            let signed = Arc::new(Mutex::new(Vec::new()));
            let provider = crate::runtime_provider::RuntimeProvider::new(
                "auth-test",
                OpenResponsesProtocolChatDriver::new(),
            )
            .base_url(format!("{}/v1", server.uri()))
            .auth(RecordingAuth {
                requests: signed.clone(),
                fail_on: Some(fail_on),
            });
            let driver =
                OpenResponsesProtocolChatDriver::new().with_retry_config(auth_retry_config());
            let result = driver
                .chat_completion_stream(
                    provider.endpoint(),
                    vec![LlmMessage::text(LlmMessageRole::User, "hi")],
                    &auth_test_config(),
                )
                .await;
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("auth failure must abort"),
            };
            assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::Authentication));
            assert!(error.to_string().contains("token refresh refused"));
            assert_eq!(signed.lock().unwrap().len(), fail_on);
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), fail_on - 1);
            if fail_on == 2 {
                assert_eq!(requests[0].headers["authorization"], "Bearer token-1");
            }
        }
    }

    #[tokio::test]
    async fn compact_request_preserves_endpoint_query_and_complete_contract() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::builder().start().await;
        Mock::given(method("POST")).and(path("/v1/responses/compact"))
            .and(query_param("api-version", "preview"))
            .and(header("authorization", "Bearer compact-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output":[{"type":"message","role":"user","content":"keep me"},{"type":"compaction","encrypted_content":"opaque"}],
                "usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120,"cost":0.03}
            }))).expect(1).mount(&server).await;
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "compact-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1/responses?api-version=preview", server.uri()))
        .auth(crate::runtime_provider::BearerAuth::new("compact-key"));
        let driver =
            OpenResponsesProtocolChatDriver::new().with_retry_config(LlmRetryConfig::no_retry());
        let result = ChatDriver::compact(
            &driver,
            provider.endpoint(),
            CompactRequest {
                reasoning_state: None,
                model: "model-compact".into(),
                input: vec![CompactInputItem::Message {
                    role: "user".into(),
                    content: CompactContent::Text("keep me".into()),
                }],
                previous_response_id: None,
                instructions: Some("preserve facts".into()),
            },
        )
        .await
        .unwrap()
        .expect("advertised compact capability must return output");
        assert!(ChatDriver::supports_compact(&driver));
        assert_eq!(
            serde_json::to_value(&result.output).unwrap(),
            json!([
                {"type":"message","role":"user","content":"keep me"},
                {"type":"compaction","encrypted_content":"opaque"}
            ])
        );
        let usage = result.usage.unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens,
                usage.cost
            ),
            (Some(100), Some(20), Some(120), Some(0.03))
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body,
            json!({"model":"model-compact","input":[{"type":"message","role":"user","content":"keep me"}],"instructions":"preserve facts"})
        );
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
        let server = MockServer::builder().start().await;
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
    #[tokio::test]
    async fn request_controls_reach_wire_with_exact_omission_and_reasoning_semantics() {
        use crate::model::ReasoningEffort;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};
        for (effort, expected_reasoning, parallel, tier, verbosity) in [
            (None, None, None, None, None),
            (
                Some(ReasoningEffort::None),
                None,
                Some(false),
                Some("flex"),
                Some("low"),
            ),
            (
                Some(ReasoningEffort::Low),
                Some("low"),
                Some(true),
                Some("priority"),
                Some("high"),
            ),
            (
                Some(ReasoningEffort::High),
                Some("high"),
                Some(false),
                Some("default"),
                Some("medium"),
            ),
        ] {
            let server = MockServer::builder().start().await;
            Mock::given(method("POST"))
                .and(path("/responses"))
                .respond_with(successful_auth_stream())
                .expect(1)
                .mount(&server)
                .await;
            let provider = crate::runtime_provider::RuntimeProvider::new(
                "controls",
                OpenResponsesProtocolChatDriver::new(),
            )
            .base_url(server.uri());
            let driver = OpenResponsesProtocolChatDriver::new()
                .with_retry_config(LlmRetryConfig::no_retry());
            let mut config = auth_test_config();
            config.reasoning_effort = effort;
            config.parallel_tool_calls = parallel;
            config.speed = tier.map(str::to_owned);
            config.verbosity = verbosity.map(str::to_owned);
            config.openrouter_routing = Some(
                crate::driver_registry::OpenRouterRoutingConfig::fallback_models(["ignored-model"]),
            );
            let mut expected = json!({"model":"gpt-5.4","input":[{"type":"message","role":"user","content":"question"}],"instructions":"rules","stream":true});
            if let Some(parallel) = parallel {
                config.temperature = Some(0.25);
                config.max_tokens = Some(64);
                config
                    .metadata
                    .insert("session_id".into(), "session-controls".into());
                expected["temperature"] = json!(0.25);
                expected["max_output_tokens"] = json!(64);
                expected["metadata"] = json!({"session_id":"session-controls"});
                expected["parallel_tool_calls"] = json!(parallel);
                expected["service_tier"] = json!(tier.unwrap());
                expected["text"] = json!({"verbosity":verbosity.unwrap()});
            }
            if let Some(reasoning) = expected_reasoning {
                expected["reasoning"] = json!({"effort":reasoning,"summary":"detailed"});
                expected["include"] = json!(["reasoning.encrypted_content"]);
            }
            let stream = driver
                .chat_completion_stream(
                    provider.endpoint(),
                    vec![
                        LlmMessage::text(LlmMessageRole::System, "rules"),
                        LlmMessage::text(LlmMessageRole::User, "question"),
                    ],
                    &config,
                )
                .await
                .unwrap();
            assert_authenticated_stream(stream).await;
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].body_json::<Value>().unwrap(),
                expected,
                "effort={effort:?}"
            );
        }
    }
    #[test]
    fn input_conversion_preserves_ordered_content_and_replayable_reasoning() {
        use crate::execution_phase::ExecutionPhase;
        use crate::reasoning::{ReasoningContentPart, ReasoningText};
        let mut user = LlmMessage::parts(
            LlmMessageRole::User,
            vec![
                LlmContentPart::text("look α"),
                LlmContentPart::image("data:image/png;base64,aA=="),
                LlmContentPart::Audio {
                    url: "audio-data".into(),
                },
            ],
        );
        user.phase = Some(ExecutionPhase::FinalAnswer);
        let mut assistant = LlmMessage::text(LlmMessageRole::Assistant, "checking");
        assistant.phase = Some(ExecutionPhase::Commentary);
        assistant.configuration_update = Some(crate::model::ReasoningEffort::High);
        assistant.reasoning = vec![
            ReasoningContentPart::opaque("openai")
                .with_item_id("rs-first")
                .with_encrypted("enc-first")
                .with_text(ReasoningText::Summary {
                    parts: vec!["one".into(), "two".into()],
                }),
            ReasoningContentPart::opaque("openai")
                .with_item_id("rs-second")
                .with_encrypted("enc-second"),
            ReasoningContentPart::opaque("openai").with_item_id("no-payload"),
            ReasoningContentPart::opaque("openai").with_encrypted("no-id"),
            ReasoningContentPart::opaque("anthropic")
                .with_item_id("foreign-id")
                .with_encrypted("foreign-secret"),
        ];
        assistant.tool_calls = Some(vec![
            ToolCall {
                id: "call-a".into(),
                name: "first".into(),
                arguments: json!({"a":1}),
            },
            ToolCall {
                id: "call-b".into(),
                name: "second".into(),
                arguments: json!({"b":2}),
            },
        ]);
        let mut output_a = LlmMessage::parts(
            LlmMessageRole::Tool,
            vec![
                LlmContentPart::text("first"),
                LlmContentPart::image("ignored-image"),
                LlmContentPart::text(" result"),
            ],
        );
        output_a.tool_call_id = Some("call-a".into());
        let mut output_b = LlmMessage::text(LlmMessageRole::Tool, "second result");
        output_b.tool_call_id = Some("call-b".into());
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "rules"),
            user,
            LlmMessage::text(LlmMessageRole::System, "notice"),
            assistant,
            output_a,
            output_b,
            LlmMessage::text(LlmMessageRole::Assistant, "answer"),
        ];
        for phases in [false, true] {
            let (instructions, input) =
                OpenResponsesProtocolChatDriver::build_input(&messages, phases);
            assert_eq!(instructions.as_deref(), Some("rules\n\nnotice"));
            let mut expected = vec![json!({"type":"message","role":"user","content":[
                {"type":"input_text","text":"look α"},{"type":"input_image","image_url":"data:image/png;base64,aA=="},{"type":"input_audio","input_audio":{"data":"audio-data","format":"wav"}}
            ]})];
            if phases {
                expected.push(json!({"type":"configuration_update","reasoning":{"effort":"high"}}));
            }
            expected.extend([
                json!({"type":"reasoning","id":"rs-first","encrypted_content":"enc-first","summary":[{"type":"summary_text","text":"one"},{"type":"summary_text","text":"two"}]}),
                json!({"type":"reasoning","id":"rs-second","encrypted_content":"enc-second","summary":[]}),
            ]);
            let mut message = json!({"type":"message","role":"assistant","content":"checking"});
            if phases {
                message["phase"] = json!("commentary");
            }
            expected.push(message);
            expected.extend([
                json!({"type":"function_call","call_id":"call-a","name":"first","arguments":"{\"a\":1}"}),
                json!({"type":"function_call","call_id":"call-b","name":"second","arguments":"{\"b\":2}"}),
                json!({"type":"function_call_output","call_id":"call-a","output":"first result"}),
                json!({"type":"function_call_output","call_id":"call-b","output":"second result"}),
                json!({"type":"message","role":"assistant","content":"answer"}),
            ]);
            assert_eq!(serde_json::to_value(input).unwrap(), json!(expected));
        }
        let mut call_only = messages[3].clone();
        call_only.content = LlmMessageContent::Text(String::new());
        call_only.reasoning.clear();
        call_only.configuration_update = None;
        let (instructions, input) =
            OpenResponsesProtocolChatDriver::build_input(&[call_only], false);
        assert_eq!(instructions, None);
        assert_eq!(
            serde_json::to_value(input).unwrap(),
            json!([
                {"type":"function_call","call_id":"call-a","name":"first","arguments":"{\"a\":1}"},
                {"type":"function_call","call_id":"call-b","name":"second","arguments":"{\"b\":2}"},
            ])
        );
    }
    #[test]
    fn finalization_preserves_exact_delta_and_repairs_only_broken_pairs() {
        use crate::model::ReasoningEffort;
        let user = user_message("question");
        let assistant = ResponsesInputItem::Message {
            r#type: "message".into(),
            role: "assistant".into(),
            content: ResponsesContent::Text("answer".into()),
            phase: None,
        };
        let reasoning = ResponsesInputItem::Reasoning {
            r#type: "reasoning".into(),
            id: "rs-last".into(),
            encrypted_content: "opaque".into(),
            summary: vec![],
        };
        let call = function_call("a", "first");
        let output = function_call_output("a");
        let second_call = function_call("b", "second");
        let second_output = function_call_output("b");
        let low = configuration_update_item(ReasoningEffort::Low);
        let high = configuration_update_item(ReasoningEffort::High);
        let u = json!({"type":"message","role":"user","content":"question"});
        let a = json!({"type":"message","role":"assistant","content":"answer"});
        let c = json!({"type":"function_call","call_id":"a","name":"first","arguments":"{}"});
        let o = json!({"type":"function_call_output","call_id":"a","output":"result"});
        let c2 = json!({"type":"function_call","call_id":"b","name":"second","arguments":"{}"});
        let o2 = json!({"type":"function_call_output","call_id":"b","output":"result"});
        let l = json!({"type":"configuration_update","reasoning":{"effort":"low"}});
        let h = json!({"type":"configuration_update","reasoning":{"effort":"high"}});
        for (case, input, full, delta) in [
            ("empty", vec![], json!([]), json!([])),
            (
                "fresh users",
                vec![user.clone(), user.clone()],
                json!([u, u]),
                json!([u, u]),
            ),
            (
                "assistant boundary",
                vec![user.clone(), assistant.clone(), user.clone()],
                json!([u, a, u]),
                json!([u]),
            ),
            (
                "assistant last",
                vec![user.clone(), assistant.clone()],
                json!([u, a]),
                json!([]),
            ),
            (
                "parallel results",
                vec![
                    user.clone(),
                    assistant.clone(),
                    call.clone(),
                    second_call.clone(),
                    output.clone(),
                    second_output.clone(),
                    user.clone(),
                ],
                json!([u, a, c, c2, o, o2, u]),
                json!([o, o2, u]),
            ),
            (
                "reasoning boundary",
                vec![user.clone(), reasoning, output.clone(), user.clone()],
                json!([u,{"type":"reasoning","id":"rs-last","encrypted_content":"opaque","summary":[]},u]),
                json!([o, u]),
            ),
            (
                "server-side output",
                vec![output.clone(), user.clone()],
                json!([u]),
                json!([o, u]),
            ),
            (
                "dangling call",
                vec![user.clone(), call.clone()],
                json!([u]),
                json!([]),
            ),
            (
                "mixed broken pairs",
                vec![
                    user.clone(),
                    function_call("old", "old"),
                    call.clone(),
                    output.clone(),
                    function_call_output("orphan"),
                    second_call,
                    second_output,
                ],
                json!([u, c, o, c2, o2]),
                json!([o2]),
            ),
            (
                "adjacent updates",
                vec![low.clone(), high.clone(), user.clone(), low.clone()],
                json!([h, u, l]),
                json!([h, u, l]),
            ),
            (
                "updates after boundary",
                vec![low.clone(), assistant, low, high, user],
                json!([l, a, h, u]),
                json!([h, u]),
            ),
        ] {
            for (previous, expected) in [(None, full), (Some("resp-prior".to_owned()), delta)] {
                assert_eq!(
                    serde_json::to_value(finalize_input_for_request(input.clone(), &previous))
                        .unwrap(),
                    expected,
                    "{case}, previous={previous:?}"
                );
            }
        }
    }
    #[tokio::test]
    async fn continuation_wire_uses_provider_state_or_complete_checkpoint_replay() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};
        for stateful in [false, true] {
            for has_previous in [false, true] {
                for checkpoint in [false, true] {
                    let server = MockServer::builder().start().await;
                    Mock::given(method("POST"))
                        .and(path("/responses"))
                        .respond_with(successful_auth_stream())
                        .expect(1)
                        .mount(&server)
                        .await;
                    let endpoint = crate::runtime_provider::RuntimeProvider::new(
                        "continuation",
                        OpenResponsesProtocolChatDriver::new(),
                    )
                    .base_url(server.uri());
                    let driver = if stateful {
                        OpenResponsesProtocolChatDriver::new().with_stateful_responses(true)
                    } else {
                        OpenResponsesProtocolChatDriver::new()
                    };
                    let driver = driver.with_retry_config(LlmRetryConfig::no_retry());
                    let mut assistant = LlmMessage::text(LlmMessageRole::Assistant, "checking");
                    assistant.tool_calls = Some(vec![ToolCall {
                        id: "call-a".into(),
                        name: "lookup".into(),
                        arguments: json!({"key":"value"}),
                    }]);
                    let mut output = LlmMessage::text(LlmMessageRole::Tool, "found");
                    output.tool_call_id = Some("call-a".into());
                    let messages = vec![
                        LlmMessage::text(LlmMessageRole::System, "rules"),
                        LlmMessage::text(LlmMessageRole::User, "question"),
                        assistant,
                        output,
                    ];
                    let mut config = auth_test_config();
                    if has_previous {
                        config.previous_response_id = Some("resp-prior".into());
                    }
                    if checkpoint {
                        config.provider_opaque_context = Some(
                            crate::driver_registry::ProviderOpaqueContext::OpenResponsesCompact {
                                output: vec![
                                    CompactOutputItem::Compaction {
                                        encrypted_content: "opaque-checkpoint".into(),
                                    },
                                    CompactOutputItem::ProviderItem(
                                        json!({"type":"reasoning","id":"rs-checkpoint","encrypted_content":"opaque-reasoning","summary":[]}),
                                    ),
                                ],
                                reasoning_state: None,
                            },
                        );
                    }
                    let full = vec![
                        json!({"type":"message","role":"user","content":"question"}),
                        json!({"type":"message","role":"assistant","content":"checking"}),
                        json!({"type":"function_call","call_id":"call-a","name":"lookup","arguments":"{\"key\":\"value\"}"}),
                        json!({"type":"function_call_output","call_id":"call-a","output":"found"}),
                    ];
                    let mut expected = json!({"model":"gpt-5.4","instructions":"rules","stream":true,"input":full});
                    if checkpoint {
                        let mut input = vec![
                            json!({"type":"compaction","encrypted_content":"opaque-checkpoint"}),
                            json!({"type":"reasoning","id":"rs-checkpoint","encrypted_content":"opaque-reasoning","summary":[]}),
                        ];
                        input.extend(full);
                        expected["input"] = json!(input);
                    } else if stateful && has_previous {
                        expected["previous_response_id"] = json!("resp-prior");
                        expected["input"] = json!([{"type":"function_call_output","call_id":"call-a","output":"found"}]);
                    }
                    assert_authenticated_stream(
                        driver
                            .chat_completion_stream(endpoint.endpoint(), messages, &config)
                            .await
                            .unwrap(),
                    )
                    .await;
                    let requests = server.received_requests().await.unwrap();
                    assert_eq!(requests.len(), 1);
                    assert_eq!(
                        requests[0].body_json::<Value>().unwrap(),
                        expected,
                        "stateful={stateful}, previous={has_previous}, checkpoint={checkpoint}"
                    );
                }
            }
        }
    }
    fn isolated_stream_event(event: Value) -> LlmStreamEvent {
        handle_streaming_event(
            serde_json::from_value(event).unwrap(),
            &Mutex::new(0),
            &Mutex::new(0),
            &Mutex::new(None),
            &Mutex::new(vec![]),
            &Mutex::new(None),
            "gpt-5".into(),
            None,
        )
    }

    #[test]
    fn reasoning_artifacts_preserve_only_opaque_payload_and_curated_summaries() {
        use crate::reasoning::{ReasoningContentPart, ReasoningText};
        for encrypted in [None, Some("opaque")] {
            for summaries in [vec![], vec!["first", "second"]] {
                let mut summary: Vec<_> = summaries
                    .iter()
                    .map(|text| json!({"type":"summary_text","text":text}))
                    .collect();
                summary.push(json!({"type":"reasoning_text","text":"private summary entry"}));
                let event = json!({"type":"response.output_item.done","sequence_number":5,"output_index":0,"item":{"type":"reasoning","id":"rs-original","summary":summary,"content":[{"type":"reasoning_text","text":"private plaintext"}],"encrypted_content":encrypted}});
                let mut expected =
                    ReasoningContentPart::opaque("openai").with_item_id("rs-original");
                if let Some(payload) = encrypted {
                    expected = expected.with_encrypted(payload);
                }
                if !summaries.is_empty() {
                    expected = expected.with_text(ReasoningText::Summary {
                        parts: summaries.iter().map(|s| s.to_string()).collect(),
                    });
                }
                let LlmStreamEvent::ReasoningItem(actual) = isolated_stream_event(event) else {
                    panic!("expected reasoning artifact")
                };
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn reasoning_deltas_preserve_text_and_distinguish_summary_channel() {
        for (kind, index, expected_summary) in [
            ("response.reasoning_text.delta", "content_index", false),
            (
                "response.reasoning_summary_text.delta",
                "summary_index",
                true,
            ),
        ] {
            let mut event = json!({"type":kind,"sequence_number":3,"item_id":"rs-original","output_index":0,"delta":"reason α"});
            event[index] = json!(0);
            match isolated_stream_event(event) {
                LlmStreamEvent::ReasoningDelta { delta, summary } => {
                    assert_eq!(delta, "reason α");
                    assert_eq!(summary, expected_summary);
                }
                other => panic!("expected reasoning delta, got {other:?}"),
            }
        }
    }

    #[test]
    fn phase_hints_preserve_known_values_and_ignore_missing_or_unknown_values() {
        use crate::execution_phase::ExecutionPhase;
        for (phase, expected) in [
            (Some("commentary"), Some(ExecutionPhase::Commentary)),
            (Some("final_answer"), Some(ExecutionPhase::FinalAnswer)),
            (None, None),
            (Some("future-phase"), None),
        ] {
            let mut event = json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"message","id":"msg-first","status":"in_progress","role":"assistant","content":[]}});
            if let Some(phase) = phase {
                event["item"]["phase"] = json!(phase);
            }
            match (isolated_stream_event(event), expected) {
                (LlmStreamEvent::MessagePhase(actual), Some(expected)) => {
                    assert_eq!(actual, expected)
                }
                (LlmStreamEvent::TextDelta(text), None) => assert_eq!(text, ""),
                other => panic!("unexpected phase hint: {other:?}"),
            }
        }
    }
    #[test]
    fn tool_conversion_preserves_schema_and_selects_strict_or_sanitized_fallback() {
        let email = r"^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$";
        let safe_email = r"^[A-Za-z0-9_'+\-](?:[A-Za-z0-9_'+\-]|\.[A-Za-z0-9_'+\-])*@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$";
        for (schema, expected, strict) in [
            (
                json!({"type":"object","additionalProperties":false}),
                json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),
                true,
            ),
            (
                json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}),
                json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}),
                true,
            ),
            (
                json!({"type":"object","properties":{"query":{"type":"string"}}}),
                json!({"type":"object","properties":{"query":{"type":["string","null"]}},"required":["query"],"additionalProperties":false}),
                true,
            ),
            (json!({"type":"string"}), json!({"type":"string"}), false),
            (
                json!({"type":"object","allOf":[{"type":"object"}]}),
                json!({"type":"object","properties":{},"allOf":[{"type":"object"}]}),
                false,
            ),
            (
                json!({"type":"object","properties":{"email":{"type":"string","pattern":email}}}),
                json!({"type":"object","properties":{"email":{"type":"string","pattern":safe_email}}}),
                false,
            ),
        ] {
            let mut tool = make_tool("lookup", None, crate::tool_types::DeferrablePolicy::Never);
            let ToolDefinition::Builtin(definition) = &mut tool else {
                unreachable!()
            };
            definition.parameters = schema.clone();
            let mut expected_tool = json!({"type":"function","name":"lookup","description":"lookup description","parameters":expected});
            if strict {
                expected_tool["strict"] = json!(true);
            }
            assert_eq!(
                serde_json::to_value(OpenResponsesProtocolChatDriver::convert_tools(&[tool]))
                    .unwrap(),
                json!([expected_tool]),
                "schema={schema}"
            );
        }
    }
    #[test]
    fn completion_metadata_preserves_usage_cost_phase_and_terminal_reason() {
        for (status, details, reason) in [
            ("completed", None, "stop"),
            ("incomplete", Some("max_output_tokens"), "length"),
            ("incomplete", Some("max_tokens"), "length"),
            ("incomplete", Some("content_filter"), "content_filter"),
            ("cancelled", None, "cancelled"),
            ("failed", None, "error"),
        ] {
            let kind = if status == "incomplete" {
                "response.incomplete"
            } else {
                "response.completed"
            };
            let mut event = json!({"type":kind,"sequence_number":9,"response":{"id":"resp-complete","object":"response","created_at":1,"status":status,"model":"gpt-5","output":[{"type":"message","id":"msg-first","role":"assistant","status":"completed","content":[],"phase":"commentary"},{"type":"message","id":"msg-last","role":"assistant","status":"completed","content":[],"phase":"final_answer"}],"usage":{"input_tokens":1000,"output_tokens":20,"total_tokens":1020,"input_tokens_details":{"cached_tokens":800},"cost":0.125}}});
            if let Some(details) = details {
                event["response"]["incomplete_details"] = json!({"reason":details});
            }
            let LlmStreamEvent::Done(actual) = isolated_stream_event(event) else {
                panic!("expected terminal metadata")
            };
            assert_eq!(
                (
                    actual.prompt_tokens,
                    actual.completion_tokens,
                    actual.total_tokens,
                    actual.cache_read_tokens
                ),
                (Some(200), Some(20), Some(1020), Some(800)),
                "status={status}"
            );
            assert_eq!(actual.provider_cost_usd, Some(0.125));
            assert_eq!(actual.model.as_deref(), Some("gpt-5"));
            assert_eq!(actual.response_id.as_deref(), Some("resp-complete"));
            assert_eq!(actual.phase.as_deref(), Some("final_answer"));
            assert_eq!(actual.finish_reason.as_deref(), Some(reason));
            assert!(actual.cache_creation_tokens.is_none());
            assert!(actual.retry_metadata.is_none());
            assert!(actual.cache_diagnostics.is_none());
        }
    }
}
