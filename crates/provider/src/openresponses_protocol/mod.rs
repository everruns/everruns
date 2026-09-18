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

use reqwest::{Client, header::HeaderMap};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

pub use crate::compact::{
    CompactContent, CompactContentPart, CompactInputItem, CompactOutputItem, CompactRequest,
    CompactResponse, CompactUsage, messages_to_compact_input,
};
use crate::driver_registry::{
    LlmCallConfig, LlmContentPart, LlmMessage, LlmMessageContent, LlmMessageRole,
    fold_system_messages,
};
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::llm_retry::{
    LlmRetryConfig, RateLimitInfo, RetryDecision, RetryMetadata, SendOutcome, is_rate_limit_status,
    retry_request, send_error_message,
};
use crate::openai_protocol::{is_openai_model_not_found, is_openai_request_too_large};
use crate::openresponses_types::{self as types};
use crate::tool_types::ToolDefinition;
use crate::user_facing_error::is_provider_quota_message;

// Split out of one 6000-line file. Every item keeps its visibility, so the
// module's public surface is unchanged.
mod chat_driver;
mod input;
mod streaming;
mod wire;

pub(crate) use input::*;
pub(crate) use streaming::*;
pub(crate) use wire::*;

#[cfg(test)]
mod tests_replay;
#[cfg(test)]
mod tests_request;
#[cfg(test)]
mod tests_support;
#[cfg(test)]
mod tests_tools;

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
/// onto the outgoing JSON or HTTP headers and classify provider-specific error
/// bodies via this seam, so the core driver stays free of provider branching.
/// `decorate` and `decorate_headers` run once per request, after the base body
/// is serialized and before it is sent; either may return an error to abort the
/// request (e.g. failed routing validation).
pub trait OpenResponsesRequestExtension: Send + Sync {
    /// Whether a rejected stateful continuation may be retried as a repaired
    /// stateless transcript. Extensions with provider-owned pending work must
    /// opt out so the fallback cannot discard or repeat that work.
    fn allow_stateless_recovery(&self) -> bool {
        true
    }

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

    /// Classify a provider-specific terminal HTTP error while its structured
    /// response fields are still available.
    fn classify_error(
        &self,
        _status: u16,
        _headers: &HeaderMap,
        _error_body: &str,
    ) -> Option<LlmErrorKind> {
        None
    }
}

#[derive(Clone)]
pub struct OpenResponsesProtocolChatDriver {
    /// Retry configuration for rate limit errors
    retry_config: LlmRetryConfig,
    /// Optional provider-specific request-body decorator (see
    /// [`OpenResponsesRequestExtension`]). `None` for vanilla OpenAI/Azure.
    request_extension: Option<Arc<dyn OpenResponsesRequestExtension>>,
    /// Explicit stateful-continuation support supplied by the service provider.
    stateful_responses: Option<bool>,
    native_phases: bool,
    hosted_tool_search: bool,
    native_prompt_cache_options: bool,
}

impl OpenResponsesProtocolChatDriver {
    /// Create a wire-only Open Responses protocol driver.
    pub fn new() -> Self {
        // EVE-924: choose the rustls backend on the startup path. The shared
        // client installs it as well, but that now happens on the first
        // request, and products expect the process-wide choice to be settled
        // while providers are being constructed.
        crate::install_default_crypto_provider();
        Self {
            retry_config: LlmRetryConfig::default(),
            request_extension: None,
            stateful_responses: None,
            native_phases: false,
            hosted_tool_search: false,
            native_prompt_cache_options: false,
        }
    }

    /// Enable optional protocol extensions implemented by this endpoint.
    pub fn with_native_features(mut self, phases: bool, hosted_tool_search: bool) -> Self {
        self.native_phases = phases;
        self.hosted_tool_search = hosted_tool_search;
        self
    }

    /// Enable OpenAI's explicit cache controls on an endpoint that supports them.
    pub fn with_prompt_cache_options(mut self, enabled: bool) -> Self {
        self.native_prompt_cache_options = enabled;
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

                self.client()
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
                    let response_headers = response.headers().clone();
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
                    let kind = self
                        .request_extension
                        .as_ref()
                        .and_then(|extension| {
                            extension.classify_error(
                                status.as_u16(),
                                &response_headers,
                                &error_text,
                            )
                        })
                        .unwrap_or_else(|| {
                            LlmErrorKind::from_provider_status(status.as_u16(), &error_text)
                        });

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
                    RetryDecision::Terminal(AgentLoopError::llm_http_kind(
                        kind,
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

    /// The process-wide streaming HTTP client, resolved per request rather than
    /// held as a field. Building it loads the platform trust store (~1.3 ms),
    /// which would otherwise land on the agent startup path; after the first
    /// request this is a `OnceLock` read and an `Arc` clone.
    ///
    /// Returned by value for subclass access; a `reqwest::Client` is an `Arc`
    /// handle, so cloning it shares the same connection pool.
    ///
    /// The shared client is SSRF-hardened (redirects disabled + DNS-pinned
    /// resolver). The api_url is org-configurable, so a bare `Client::new()`
    /// would leave this provider open to DNS-rebind / redirect SSRF
    /// (TM-API-013, EVE-623).
    pub fn client(&self) -> Client {
        crate::driver_helpers::shared_streaming_http_client()
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
                    has_images = parts.iter().any(|p| {
                        matches!(
                            p,
                            LlmContentPart::Image { .. } | LlmContentPart::File { .. }
                        )
                    });
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
                    "OpenResponses API does not support images/files in tool results; attachments dropped"
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
                        LlmContentPart::File { url, filename } => ResponsesContentPart::InputFile {
                            r#type: "input_file".to_string(),
                            input_file: ResponsesInputFile {
                                file_data: Some(url.clone()),
                                file_url: None,
                                filename: filename.clone(),
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
                let mut builder = self.client().post(&resolved.url);
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
                    // Reasoning replay tokens are provider-specific. Never send
                    // another provider's artifact to the Responses API.
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
