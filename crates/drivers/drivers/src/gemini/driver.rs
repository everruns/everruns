mod non_streaming;

// Google Gemini Chat Driver
//
// Implementation of ChatDriver for Google's Gemini API.
// Uses the generateContent API with streaming support.
//
// API docs: https://ai.google.dev/api/generate-content
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff. Retry metadata is included in the response for observability.
//
// Native web search (`everruns/web_search`, see
// `everruns_contracts::native_web_search`): Grounding with Google Search, the
// `google_search` tool, stands in for the `web_search` function tool when the
// model has it and the agent sets no domain filters (grounding takes none).
// The searches run inside the generation; their queries arrive in the
// candidate's `groundingMetadata.webSearchQueries` and are counted for cost.
//
// Decision: native search only when `web_search` is the call's only function
// tool. On `generateContent`, combining a built-in tool with function
// declarations needs Gemini 3, `toolConfig.includeServerSideToolInvocations`
// and `VALIDATED` function calling, and returns `toolCall` / `toolResponse`
// parts (with thought signatures and call ids) that every later request must
// replay exactly. This driver rebuilds model turns from text, reasoning and
// function calls rather than replaying raw parts, so it cannot meet that
// contract yet (the combination is also still a preview). Gemini 2.x rejects
// the combination outright. With other tools on the call the request declines
// and the function tool stays, for the next provider of the search plan.
//
// Citations: the grounding sources and supports are reported as web citations
// on `Done`, mapped against the streamed answer text (see `grounding`).

#[cfg(not(target_arch = "wasm32"))]
mod batch;
mod request;
mod streaming;
use streaming::{ContinuationRequest, convert_gemini_stream};

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use everruns_contracts::credential_schema::CredentialFormSchema;
use everruns_contracts::driver_helpers::{
    self, AUDIO_CONTENT_PLACEHOLDER, GEMINI_NOT_FOUND_PATTERNS, GEMINI_TOO_LARGE_PATTERNS,
    parse_data_url,
};
use everruns_contracts::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmCompletionMetadata, LlmContentPart, LlmResponseStream, LlmStreamEvent, Message,
    MessageContent, MessageRole, disjoint_prompt_tokens, fold_system_messages,
};
use everruns_contracts::error::{AgentLoopError, LlmErrorKind, Result};
use everruns_contracts::is_provider_quota_message;
use everruns_contracts::llm_retry::{
    LlmRetryConfig, RetryDecision, RetryMetadata, SendOutcome, retry_request, send_error_message,
};
use everruns_contracts::reasoning::{ReasoningContentPart, ReasoningText};
use everruns_contracts::stream_accumulator::StreamToolCallAccumulator;
use everruns_contracts::stream_reconnect::{ByteStream, connect_bytes_with_reconnect};
use everruns_contracts::tool_types::ToolDefinition;

use super::grounding::{self, GroundingChunk, GroundingSupport};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Ready-to-use Gemini provider assembly.
pub fn provider(
    id: impl Into<everruns_contracts::ProviderKey>,
    api_key: impl Into<String>,
) -> everruns_contracts::Provider {
    everruns_contracts::Provider::new(id, GeminiChatDriver::new())
        .base_url(DEFAULT_BASE_URL)
        .auth(everruns_contracts::StaticHeaderAuth::new(
            "x-goog-api-key",
            api_key,
        ))
}

/// Google Gemini Chat Driver
///
/// Implements `ChatDriver` for Google's Gemini API.
/// Supports streaming responses and tool calls.
#[derive(Clone)]
pub struct GeminiChatDriver {
    retry_config: LlmRetryConfig,
}

impl GeminiChatDriver {
    /// Create a new driver with the given API key
    pub fn new() -> Self {
        // EVE-924: choose the rustls backend on the startup path. The shared
        // client installs it as well, but that now happens on the first
        // request, and products expect the process-wide choice to be settled
        // while providers are being constructed.
        everruns_contracts::install_default_crypto_provider();
        Self {
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// Configure retries for `429` and transient `5xx` responses.
    ///
    /// Retry time counts against any timeout the caller wraps around the call;
    /// pass [`LlmRetryConfig::no_retry`] to own retries in the host.
    ///
    /// ```
    /// use everruns_contracts::LlmRetryConfig;
    /// use everruns_drivers::gemini::GeminiChatDriver;
    ///
    /// let driver = GeminiChatDriver::new().with_retry_config(LlmRetryConfig::no_retry());
    /// # let _ = driver;
    /// ```
    pub fn with_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// The process-wide streaming HTTP client, resolved per request rather than
    /// held as a field. Building it loads the platform trust store (~1.3 ms),
    /// which would otherwise land on the agent startup path; after the first
    /// request this is a `OnceLock` read and an `Arc` clone.
    fn client(&self) -> Client {
        everruns_contracts::driver_helpers::shared_streaming_http_client()
    }

    fn convert_role(role: &MessageRole) -> &'static str {
        match role {
            MessageRole::System => "user", // System is handled separately
            MessageRole::User => "user",
            MessageRole::Assistant => "model",
            MessageRole::Tool => "user", // Tool results sent as function responses
        }
    }

    fn convert_content(content: &MessageContent) -> Vec<GeminiPart> {
        match content {
            MessageContent::Text(text) => {
                if text.is_empty() {
                    vec![]
                } else {
                    vec![GeminiPart::text(text.clone())]
                }
            }
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    LlmContentPart::Text { text } => {
                        if text.is_empty() {
                            None
                        } else {
                            Some(GeminiPart::text(text.clone()))
                        }
                    }
                    LlmContentPart::Image { url } => {
                        if let Some(parsed) = parse_data_url(url) {
                            Some(GeminiPart::InlineData {
                                inline_data: GeminiBlob {
                                    mime_type: parsed.media_type,
                                    data: parsed.data,
                                },
                            })
                        } else if url.starts_with("data:") {
                            // Malformed data URL
                            None
                        } else {
                            // HTTP URL - use file_data
                            Some(GeminiPart::FileData {
                                file_data: GeminiFileData {
                                    mime_type: "image/jpeg".to_string(),
                                    file_uri: url.clone(),
                                },
                            })
                        }
                    }
                    LlmContentPart::Audio { .. } => {
                        Some(GeminiPart::text(AUDIO_CONTENT_PLACEHOLDER))
                    }
                    LlmContentPart::File { url, .. } => {
                        if let Some(parsed) = parse_data_url(url) {
                            Some(GeminiPart::InlineData {
                                inline_data: GeminiBlob {
                                    mime_type: parsed.media_type,
                                    data: parsed.data,
                                },
                            })
                        } else if url.starts_with("data:") {
                            // Malformed data URL
                            None
                        } else {
                            // File URL - reference via file_data (PDF)
                            Some(GeminiPart::FileData {
                                file_data: GeminiFileData {
                                    mime_type: "application/pdf".to_string(),
                                    file_uri: url.clone(),
                                },
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

    fn convert_messages(messages: &[Message]) -> (Option<GeminiContent>, Vec<GeminiContent>) {
        // Accumulate all system messages into Gemini's separate
        // `system_instruction`. Overwriting on each System message would drop the
        // agent system prompt whenever a later notice/summary System message is
        // present (infinity_context / compaction). See `fold_system_messages`.
        let system_instruction = fold_system_messages(messages).map(|text| GeminiContent {
            role: None, // system_instruction has no role
            parts: vec![GeminiPart::text(text)],
        });
        let mut contents = Vec::new();
        // IDs may be reused on later turns; resolve each result against calls
        // already seen at its position in the transcript.
        let mut function_names = HashMap::new();

        for msg in messages {
            match msg.role {
                MessageRole::System => {
                    // Folded above into `system_instruction`; never emit
                    // System-role content into the Gemini `contents` array.
                }
                MessageRole::Tool => {
                    // Tool results in Gemini use functionResponse parts
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        // THREAT[TM-TOOL-005]: Gemini rejects functionResponse parts unless the matching
                        // functionCall is present in the visible request after trimming.
                        let Some(name) = function_names.get(tool_call_id.as_str()) else {
                            continue;
                        };

                        // Gemini requires an object response even when the tool
                        // returns a scalar or an array.
                        let text = msg.content.to_text();
                        let response_value =
                            serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
                        let response_value = if response_value.is_object() {
                            response_value
                        } else {
                            json!({"result": response_value})
                        };

                        let mut parts = vec![GeminiPart::FunctionResponse {
                            function_response: GeminiFunctionResponse {
                                name: String::from(*name),
                                response: response_value,
                            },
                        }];
                        // Images and PDFs the tool returned (a screenshot, an
                        // attachment `read_file` reopened) follow the response
                        // as inline data in the same turn, instead of dropping.
                        parts.extend(Self::convert_content(&msg.content).into_iter().filter(
                            |part| {
                                matches!(
                                    part,
                                    GeminiPart::InlineData { .. } | GeminiPart::FileData { .. }
                                )
                            },
                        ));
                        contents.push(GeminiContent {
                            role: Some("user".to_string()),
                            parts,
                        });
                    }
                }
                MessageRole::Assistant => {
                    // Thought parts lead the turn, matching the order Gemini
                    // emitted them. Signatures bound to a specific call are
                    // replayed on that call instead (below), never here.
                    let mut parts: Vec<GeminiPart> = msg
                        .reasoning
                        .iter()
                        .filter(|r| r.provider == "google" && r.bound_tool_call_id.is_none())
                        .map(|r| {
                            GeminiPart::thought(
                                r.display_text().unwrap_or_default(),
                                r.signature.clone(),
                            )
                        })
                        .collect();
                    parts.extend(Self::convert_content(&msg.content));

                    // Add function call parts if present
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            function_names.insert(tc.id.as_str(), tc.name.as_str());
                            let args = if tc.arguments.is_object() {
                                tc.arguments.clone()
                            } else if let Some(s) = tc.arguments.as_str() {
                                serde_json::from_str(s).unwrap_or(json!({}))
                            } else {
                                json!({})
                            };
                            parts.push(GeminiPart::FunctionCall {
                                function_call: GeminiFunctionCall {
                                    name: tc.name.clone(),
                                    args,
                                },
                                // Gemini binds a thought signature to a
                                // specific call; replay it on that call.
                                thought_signature: msg
                                    .reasoning
                                    .iter()
                                    .find(|r| {
                                        r.provider == "google"
                                            && r.bound_tool_call_id.as_deref() == Some(&tc.id)
                                    })
                                    .and_then(|r| r.signature.clone()),
                            });
                        }
                    }

                    if !parts.is_empty() {
                        contents.push(GeminiContent {
                            role: Some(Self::convert_role(&msg.role).to_string()),
                            parts,
                        });
                    }
                }
                _ => {
                    let parts = Self::convert_content(&msg.content);
                    if !parts.is_empty() {
                        contents.push(GeminiContent {
                            role: Some(Self::convert_role(&msg.role).to_string()),
                            parts,
                        });
                    }
                }
            }
        }

        (system_instruction, contents)
    }

    /// Recursively strip fields that Gemini doesn't accept in JSON Schema.
    ///
    /// Gemini's function-calling API uses an OpenAPI 3.0 subset that rejects
    /// `additionalProperties`. The field can appear at any depth — inside
    /// `properties`, `items`, `anyOf`, etc. — so we visit schema-valued keywords recursively
    /// while preserving literal annotation and extension data.
    fn clean_schema(mut value: Value) -> Value {
        Self::strip_unsupported(&mut value);
        value
    }

    // THREAT[TM-TOOL-038]: only schema nodes may be rewritten; literal payloads stay intact.
    fn strip_unsupported(value: &mut Value) {
        match value {
            Value::Object(obj) => {
                obj.remove("additionalProperties");
                // Literal defaults, enums, examples and extensions are data, not schemas.
                for (key, value) in obj.iter_mut() {
                    match key.as_str() {
                        "properties" | "patternProperties" | "definitions" | "$defs"
                        | "dependentSchemas" | "dependencies" => {
                            if let Some(schemas) = value.as_object_mut() {
                                for schema in schemas.values_mut() {
                                    Self::strip_unsupported(schema);
                                }
                            }
                        }
                        "items"
                        | "additionalItems"
                        | "contains"
                        | "propertyNames"
                        | "not"
                        | "if"
                        | "then"
                        | "else"
                        | "unevaluatedProperties"
                        | "unevaluatedItems"
                        | "contentSchema"
                        | "allOf"
                        | "anyOf"
                        | "oneOf"
                        | "prefixItems" => Self::strip_unsupported(value),
                        _ => {}
                    }
                }
            }
            Value::Array(arr) => {
                for v in arr.iter_mut() {
                    Self::strip_unsupported(v);
                }
            }
            _ => {}
        }
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Option<Vec<GeminiTool>> {
        if tools.is_empty() {
            return None;
        }

        let declarations: Vec<GeminiFunctionDeclaration> = tools
            .iter()
            .map(|tool| GeminiFunctionDeclaration {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: Self::clean_schema(tool.parameters().clone()),
            })
            .collect();

        Some(vec![GeminiTool {
            function_declarations: declarations,
            google_search: None,
        }])
    }

    /// The Grounding with Google Search tool in place of the `web_search`
    /// function tool, when the call asks for native search, the model has it,
    /// grounding can apply the agent's filters, and no other function tool is
    /// on the call (see the decision at the top of this file).
    fn native_web_search_tool(config: &LlmCallConfig) -> Option<GeminiTool> {
        use everruns_contracts::native_web_search::{
            NativeWebSearch, WEB_SEARCH_FUNCTION_TOOL, gemini_has_native_web_search,
        };
        if !gemini_has_native_web_search(&config.model)
            || config
                .tools
                .iter()
                .any(|tool| tool.name() != WEB_SEARCH_FUNCTION_TOOL)
        {
            return None;
        }
        let tool = NativeWebSearch::requested(config)?.gemini_tool()?;
        Some(GeminiTool {
            function_declarations: Vec::new(),
            google_search: tool.get("google_search").cloned(),
        })
    }

    /// Build the streaming URL for a model
    fn stream_path(model: &str) -> String {
        format!("models/{model}:streamGenerateContent?alt=sse")
    }

    /// Build the models list URL
    /// Send one streaming `streamGenerateContent` request, applying the shared
    /// header-phase retry loop (transient send failures, 429, and 5xx), and
    /// return the raw response plus its retry metadata.
    ///
    /// Invoked once per reconnect attempt by [`connect_bytes_with_reconnect`]. It
    /// re-sends the identical request and consumes no body bytes, so retrying is
    /// idempotent. Terminal classification and error messages are preserved
    /// exactly.
    async fn send_generate_content_request(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        request: &GeminiRequest,
        url: &str,
        model: &str,
        extra_headers: &[(String, String)],
    ) -> Result<(reqwest::Response, RetryMetadata)> {
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        retry_request(
            &self.retry_config,
            "GeminiDriver",
            || async {
                let body = serde_json::to_vec(request).map_err(|error| {
                    SendOutcome::Fatal(AgentLoopError::llm(format!(
                        "Failed to serialize Gemini request: {error}"
                    )))
                })?;
                let resolved = endpoint
                    .resolve("POST", url, &body)
                    .await
                    .map_err(SendOutcome::Fatal)?;
                let client = if is_argon(model) {
                    driver_helpers::shared_long_decode_http_client()
                } else {
                    self.client()
                };
                let mut builder = client.post(&resolved.url);
                let mut headers = resolved.headers;
                headers.push(("Content-Type".to_string(), "application/json".to_string()));
                for (name, value) in everruns_contracts::driver_helpers::merge_request_headers(
                    headers,
                    extra_headers,
                ) {
                    builder = builder.header(name, value);
                }
                builder.body(body).send().await.map_err(SendOutcome::Send)
            },
            |response, attempts, can_retry| {
                let model = model.to_string();
                let last_error = Arc::clone(&last_error);
                async move {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();

                    // Don't retry if this is a request-too-large error.
                    if is_gemini_request_too_large(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::request_too_large(
                            format!("Gemini API error ({}): {}", status, error_text),
                        ));
                    }

                    if can_retry {
                        // Exhausted billing quota is not transient — fail fast
                        // instead of burning retries.
                        if is_provider_quota_message(&error_text) {
                            return RetryDecision::Terminal(AgentLoopError::llm_kind(
                                LlmErrorKind::QuotaExhausted,
                                format!("Gemini API error ({}): {}", status, error_text),
                            ));
                        }

                        let wait = self.retry_config.calculate_backoff(attempts);
                        *last_error.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(error_text);
                        return RetryDecision::Retry {
                            wait,
                            rate_limit_info: None,
                        };
                    }

                    // Non-retryable error or max retries exceeded
                    let error_msg = format!("Gemini API error ({}): {}", status, error_text);

                    // Check if this is a model-not-found error
                    if is_gemini_model_not_found(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::model_not_available(model));
                    }

                    // Attach the semantic error kind while the HTTP status and
                    // body are still available (see LlmErrorKind).
                    let kind = LlmErrorKind::from_provider_status(status.as_u16(), &error_text);

                    let message = if attempts > 0 {
                        format!(
                            "{} (after {} retries, last error: {})",
                            error_msg,
                            attempts,
                            last_error
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .take()
                                .unwrap_or_default()
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
}

#[async_trait]
impl ChatDriver for GeminiChatDriver {
    #[cfg(not(target_arch = "wasm32"))]
    fn batch_driver(&self) -> Option<&dyn everruns_contracts::batch::BatchDriver> {
        Some(self)
    }

    fn supports_native_non_streaming(&self) -> bool {
        true
    }

    async fn chat_completion_non_streaming(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<everruns_contracts::driver_registry::LlmResponse> {
        let mut config = config.clone();
        config.prefer_non_streaming = true;
        self.chat_completion(endpoint, messages, &config).await
    }

    fn stream_stall_timeout(&self, config: &LlmCallConfig) -> Option<std::time::Duration> {
        (is_argon(&config.model) && config.max_tokens.is_some_and(|tokens| tokens > 65_536))
            .then_some(std::time::Duration::from_secs(1_200))
    }

    async fn chat_completion_stream(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let (request, grounded) = self.prepare_request(&messages, config, true)?;

        // Long decodes use stream-only continuation checkpoints. Whole JSON
        // responses use the same state fold for signatures, grounding and usage.
        if config.prefer_non_streaming
            && !config.limits.needs_incremental_events()
            && config.max_tokens.is_none_or(|tokens| tokens <= 65_536)
        {
            return self
                .native_response(endpoint, config, &request, grounded)
                .await;
        }

        // Establish the byte stream, transparently reconnecting on a transport
        // failure that lands before the first chunk (the "error decoding
        // response body" flake). Header-phase retries (429/5xx and transient
        // send failures) are handled inside the per-attempt send. Gemini parses
        // SSE by hand, so this uses the raw byte-stream reconnect variant.
        let url = endpoint
            .url(&Self::stream_path(&config.model))
            .ok_or_else(|| AgentLoopError::config("Gemini provider has no base URL"))?;
        let (byte_stream, retry_metadata) =
            connect_bytes_with_reconnect(&self.retry_config, "GeminiDriver", |_attempt| {
                self.send_generate_content_request(
                    endpoint,
                    &request,
                    &url,
                    &config.model,
                    &config.extra_headers,
                )
            })
            .await?;

        let state = GeminiStreamState {
            model: config.model.clone(),
            grounded,
            retry_metadata: retry_metadata.had_retries().then_some(retry_metadata),
            ..Default::default()
        };
        let captured_request = config
            .capture_request
            .then(|| serde_json::to_value(&request).ok())
            .flatten();
        Ok(everruns_contracts::driver_helpers::attach_request_body(
            convert_gemini_stream(
                byte_stream,
                state,
                config
                    .max_tokens
                    .filter(|&tokens| tokens > 65_536)
                    .map(|_| ContinuationRequest {
                        driver: self.clone(),
                        endpoint: endpoint.clone(),
                        request,
                        url,
                        model: config.model.clone(),
                        extra_headers: config.extra_headers.clone(),
                        hops: 0,
                    }),
            ),
            captured_request,
        ))
    }

    async fn list_models(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        if endpoint.base_url() != Some(DEFAULT_BASE_URL) {
            return Ok(None);
        }

        let url = endpoint
            .url("models")
            .ok_or_else(|| AgentLoopError::config("Gemini provider has no base URL"))?;
        let resolved = endpoint.resolve("GET", url, &[]).await?;
        let mut request = self.client().get(&resolved.url);
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

        let models_response: GeminiModelsResponse = response
            .json()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to parse models response: {}", e)))?;

        // Filter to generative models only (skip embedding, etc.)
        let discovered: Vec<DiscoveredModel> = models_response
            .models
            .into_iter()
            .filter(|m| {
                m.name.contains("gemini")
                    && m.supported_generation_methods
                        .as_ref()
                        .is_some_and(|methods| {
                            methods
                                .iter()
                                .any(|m| m == "generateContent" || m == "streamGenerateContent")
                        })
            })
            .map(|m| {
                // Strip "models/" prefix from name
                let model_id = m.name.strip_prefix("models/").unwrap_or(&m.name);
                DiscoveredModel {
                    capabilities: vec!["chat".to_string()],
                    model_id: model_id.to_string(),
                    display_name: Some(m.display_name),
                    created_at: None,
                    owned_by: Some("google".to_string()),
                    discovered_profile: None,
                }
            })
            .collect();

        Ok(Some(discovered))
    }
}

impl std::fmt::Debug for GeminiChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeminiChatDriver").finish_non_exhaustive()
    }
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the Gemini driver with the driver registry
/// This driver's descriptor: identity, services, and the credential schema
/// that declares its own environment variables.
/// Curated model recommendations for this provider. Refresh when the vendor
/// rotates flagships; keep every role default inside `recommended_models`.
pub const GEMINI_RECOMMENDED: everruns_contracts::recommended_models::ProviderRecommendedModels =
    everruns_contracts::recommended_models::ProviderRecommendedModels {
        provider: "gemini",
        recommended_models: &[
            "gemini-4-argon",
            "gemini-3.8-flash",
            "gemini-3.5-flash",
            "gemini-3.5-flash-lite",
        ],
        coding_model: "gemini-4-argon",
        research_model: "gemini-4-argon",
        chat_model: "gemini-3.8-flash",
        small_model: "gemini-3.5-flash-lite",
        decision_model: "gemini-3.5-flash-lite",
    };

pub fn descriptor() -> DriverDescriptor {
    DriverDescriptor {
        display_name: "Google Gemini".into(),
        // The google-genai SDK reads GEMINI_API_KEY and also honors the older
        // GOOGLE_API_KEY, preferring the former when both are set.
        credential_schema: CredentialFormSchema::api_key(
            "GEMINI_API_KEY",
            "Create an API key in [Google AI Studio](https://aistudio.google.com/apikey).",
        )
        .with_api_key_fallback_env("GOOGLE_API_KEY"),
        // Google defines no endpoint variable for the Gemini API, but
        // GEMINI_BASE_URL was recognized before these names moved onto the
        // drivers; declaring it keeps existing dev setups working.
        base_url_env: Some("GEMINI_BASE_URL".into()),
        recommended_models: Some(GEMINI_RECOMMENDED),
        ..DriverDescriptor::chat_only(DriverId::Gemini, |config| {
            let provider =
                everruns_contracts::Provider::new(config.provider.clone(), GeminiChatDriver::new())
                    .base_url(config.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL))
                    .auth(everruns_contracts::StaticHeaderAuth::new(
                        "x-goog-api-key",
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
    id: impl Into<everruns_contracts::ProviderKey>,
) -> std::result::Result<
    everruns_contracts::Provider,
    everruns_contracts::credential_provider::EnvCredentialError,
> {
    everruns_contracts::credential_provider::provider_from_env(&descriptor(), id)
}

impl Default for GeminiChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// SSE Parsing
// ============================================================================

fn is_argon(model: &str) -> bool {
    model.rsplit('/').next() == Some("gemini-4-argon")
}

/// Search metadata is cumulative within a request, but its source indices and
/// segment offsets belong to that request, not the combined continuation text.
#[derive(Default)]
struct GeminiGroundingHop {
    answer_range: std::ops::Range<usize>,
    search_queries: Vec<String>,
    chunks: Vec<GroundingChunk>,
    supports: Vec<GroundingSupport>,
}

#[derive(Default)]
struct GeminiStreamState {
    model: String,
    response_model: Option<String>,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: Option<u32>,
    thought_tokens: Option<u32>,
    billed_input: u32,
    billed_output: u32,
    billed_cache: Option<u32>,
    billed_thoughts: Option<u32>,
    continuation_token: Option<String>,
    can_continue: bool,
    output_since_checkpoint: bool,
    checkpoint_usage: Option<(u32, u32, Option<u32>, Option<u32>)>,
    calls: StreamToolCallAccumulator,
    call_counter: u32,
    finish_reason: Option<String>,
    provider_finish_reason: Option<String>,
    tool_calls_dropped: u32,
    retry_metadata: Option<RetryMetadata>,
    /// The request carried the Google Search tool.
    grounded: bool,
    search_calls: u32,
    answer: String,
    grounding: GeminiGroundingHop,
    completed_grounding: Vec<GeminiGroundingHop>,
    pending: std::collections::VecDeque<LlmStreamEvent>,
    done: bool,
}

impl GeminiStreamState {
    fn response(&mut self, response: GeminiStreamResponse) {
        if response.model_version.is_some() {
            self.response_model = response.model_version;
        }
        if let Some(usage) = response.usage_metadata {
            if let Some(tokens) = usage.prompt_token_count {
                self.input_tokens = tokens;
            }
            if let Some(tokens) = usage.candidates_token_count {
                self.output_tokens = tokens;
            }
            if let Some(tokens) = usage.cached_content_token_count {
                self.cached_tokens = Some(tokens);
            }
            if let Some(tokens) = usage.thoughts_token_count {
                self.thought_tokens = Some(tokens);
            }
        }
        // Grounding metadata may trail the terminal candidate; read it first.
        if self.grounded {
            for grounding in response
                .candidates
                .iter()
                .flatten()
                .filter_map(|candidate| candidate.grounding_metadata.as_ref())
            {
                self.record_searches(grounding);
            }
        }
        // Usage can arrive after the terminal candidate, but no later content
        // may reopen a completed or rejected generation.
        if self.finish_reason.is_some() {
            return;
        }
        for candidate in response.candidates.unwrap_or_default() {
            let checkpoint = candidate.continuation_token;
            if let Some(content) = candidate.content {
                for part in content.parts {
                    self.output_since_checkpoint = true;
                    match part {
                        GeminiResponsePart::Text {
                            text,
                            thought: Some(true),
                            thought_signature,
                        } => {
                            self.pending.push_back(match thought_signature {
                                Some(signature) => LlmStreamEvent::ReasoningItem(
                                    ReasoningContentPart::opaque("google")
                                        .with_signature(signature)
                                        .with_text(ReasoningText::Plain { text }),
                                ),
                                None => LlmStreamEvent::ReasoningDelta {
                                    delta: text,
                                    summary: false,
                                },
                            });
                        }
                        GeminiResponsePart::Text { text, .. } => {
                            self.answer.push_str(&text);
                            self.pending.push_back(LlmStreamEvent::TextDelta(text))
                        }
                        GeminiResponsePart::FunctionCall {
                            function_call,
                            thought_signature,
                        } => {
                            let id = format!("call_{}", self.call_counter);
                            self.call_counter += 1;
                            self.calls.push_complete(
                                id.clone(),
                                function_call.name,
                                function_call.args,
                            );
                            if let Some(signature) = thought_signature {
                                self.pending.push_back(LlmStreamEvent::ReasoningItem(
                                    ReasoningContentPart::opaque("google")
                                        .with_signature(signature)
                                        .with_bound_tool_call_id(id),
                                ));
                            }
                        }
                        GeminiResponsePart::Other(_) => {}
                    }
                }
            }
            if let Some(token) = checkpoint {
                self.continuation_token = Some(token);
                self.output_since_checkpoint = false;
                self.checkpoint_usage = Some((
                    self.input_tokens,
                    self.output_tokens,
                    self.cached_tokens,
                    self.thought_tokens,
                ));
            }
            if let Some(reason) = candidate
                .finish_reason
                .filter(|reason| reason != "FINISH_REASON_UNSPECIFIED")
            {
                self.finish_reason = Some(match reason.as_str() {
                    "STOP" => "stop".into(),
                    "MAX_TOKENS" | "CONTINUATION" => "length".into(),
                    "SAFETY" => "content_filter".into(),
                    _ => reason.to_ascii_lowercase(),
                });
                // THREAT[TM-TOOL-037]: only an accepted terminal candidate can release pending calls.
                // The whole frame is processed before yielding any of its events.
                // CONTINUATION ends a hop, not the logical response. Hold all
                // complete calls until that response is accepted or rejected.
                if reason != "CONTINUATION" || !self.can_continue {
                    let calls = self.calls.take_finalized();
                    if reason == "STOP" && !calls.is_empty() {
                        self.pending.push_back(LlmStreamEvent::ToolCalls(calls));
                    } else {
                        // Cut off or rejected: discarded, but counted.
                        self.tool_calls_dropped += u32::try_from(calls.len()).unwrap_or(u32::MAX);
                    }
                }
                self.provider_finish_reason = Some(reason);
                break;
            }
        }
    }

    /// Report each new grounding query as a finished hosted call. Gemini
    /// sends no search progress: the queries arrive with the grounded text,
    /// and a later chunk in the same request may repeat earlier ones. The same
    /// query in a continuation request is a separate search and charge.
    fn record_searches(&mut self, grounding: &GeminiGroundingMetadata) {
        if !grounding.grounding_supports.is_empty() {
            self.grounding.chunks = grounding.grounding_chunks.clone();
            self.grounding.supports = grounding.grounding_supports.clone();
        }
        for query in &grounding.web_search_queries {
            if query.trim().is_empty() || self.grounding.search_queries.contains(query) {
                continue;
            }
            self.grounding.search_queries.push(query.clone());
            self.search_calls = self.search_calls.saturating_add(1);
            self.pending.push_back(LlmStreamEvent::HostedToolCall(
                everruns_contracts::driver_registry::HostedToolCall {
                    id: format!("google_search_{}", self.search_calls),
                    tool: everruns_contracts::native_web_search::WEB_SEARCH_FUNCTION_TOOL
                        .to_string(),
                    status: everruns_contracts::driver_registry::HostedToolCallStatus::Completed,
                    summary: Some(query.chars().take(200).collect()),
                },
            ));
        }
    }

    /// Billed searches: Gemini 3 and later bill each query, Gemini 2.x each
    /// grounded prompt (see `hosted_call_price_usd`).
    fn billed_searches(&self) -> u32 {
        let per_query = self
            .model
            .rsplit('/')
            .next()
            .and_then(|model| model.strip_prefix("gemini-"))
            .and_then(|rest| rest.split(['.', '-']).next())
            .and_then(|major| major.parse::<u32>().ok())
            .is_some_and(|major| major >= 3);
        self.completed_grounding
            .iter()
            .chain(std::iter::once(&self.grounding))
            .map(|hop| {
                let queries = u32::try_from(hop.search_queries.len()).unwrap_or(u32::MAX);
                if per_query { queries } else { queries.min(1) }
            })
            .fold(0, u32::saturating_add)
    }

    fn finish(&mut self) {
        self.grounding.answer_range.end = self.answer.len();
        self.add_hop_usage();
        let calls = self.calls.take_finalized();
        if self.finish_reason.is_none() && !calls.is_empty() {
            self.pending.push_back(LlmStreamEvent::ToolCalls(calls));
        } else {
            self.tool_calls_dropped = self
                .tool_calls_dropped
                .saturating_add(u32::try_from(calls.len()).unwrap_or(u32::MAX));
        }
        self.pending.push_back(LlmStreamEvent::Done(Box::new({
            let mut metadata = LlmCompletionMetadata::default();
            metadata.total_tokens = Some(self.billed_input.saturating_add(self.billed_output));
            metadata.prompt_tokens =
                Some(disjoint_prompt_tokens(self.billed_input, self.billed_cache));
            metadata.completion_tokens = Some(self.billed_output);
            metadata.cache_read_tokens = self.billed_cache;
            metadata.reasoning_tokens = self.billed_thoughts;
            metadata.model = Some(self.model.clone());
            metadata.response_model = self.response_model.clone();
            // `None` when no candidate reported a finish reason: kept distinct
            // from an explicit stop.
            metadata.finish_reason = self.finish_reason.take();
            let raw = self.provider_finish_reason.take();
            let dropped = self.tool_calls_dropped;
            let stop = raw.as_deref().unwrap_or("none");
            everruns_contracts::llm_telemetry::warn_tool_calls_dropped(
                "gemini",
                &self.model,
                dropped,
                stop,
            );
            (metadata.provider_finish_reason, metadata.tool_calls_dropped) = (raw, dropped);
            metadata.retry_metadata = self.retry_metadata.take();
            metadata.web_citations = self
                .completed_grounding
                .iter()
                .chain(std::iter::once(&self.grounding))
                .flat_map(|hop| {
                    let answer = &self.answer[hop.answer_range.clone()];
                    let offset = self.answer[..hop.answer_range.start].chars().count();
                    let mut citations =
                        grounding::web_citations(answer, &hop.chunks, &hop.supports);
                    for citation in &mut citations {
                        if let Some(span) = &mut citation.span {
                            span.start += offset;
                            span.end += offset;
                        }
                    }
                    citations
                })
                .collect();
            let searches = self.billed_searches();
            if searches > 0 {
                metadata
                    .hosted_tool_calls
                    .insert("web_search_call".to_string(), searches);
            }
            metadata
        })));
        self.done = true;
    }

    fn add_hop_usage(&mut self) {
        self.billed_input = self.billed_input.saturating_add(self.input_tokens);
        self.billed_output = self
            .billed_output
            .saturating_add(self.output_tokens)
            .saturating_add(self.thought_tokens.unwrap_or(0));
        for (total, hop) in [
            (&mut self.billed_cache, self.cached_tokens),
            (&mut self.billed_thoughts, self.thought_tokens),
        ] {
            if let Some(tokens) = hop {
                *total = Some(total.unwrap_or(0).saturating_add(tokens));
            }
        }
    }

    fn next_hop(&mut self) {
        self.grounding.answer_range.end = self.answer.len();
        self.completed_grounding
            .push(std::mem::take(&mut self.grounding));
        self.grounding.answer_range = self.answer.len()..self.answer.len();
        self.add_hop_usage();
        self.input_tokens = 0;
        self.output_tokens = 0;
        self.cached_tokens = None;
        self.thought_tokens = None;
        self.finish_reason = None;
        self.provider_finish_reason = None;
        self.output_since_checkpoint = false;
        self.checkpoint_usage = None;
    }

    fn fail(&mut self, error: AgentLoopError) {
        self.pending.push_back(LlmStreamEvent::Error(
            everruns_contracts::driver_registry::LlmStreamError::provider(
                None::<String>,
                error.http_status(),
                error.to_string(),
            ),
        ));
        self.done = true;
    }
}

// ============================================================================
// Error Detection
// ============================================================================

fn is_gemini_model_not_found(status: reqwest::StatusCode, error_text: &str) -> bool {
    driver_helpers::is_model_not_found(status, error_text, GEMINI_NOT_FOUND_PATTERNS)
}

fn is_gemini_request_too_large(status: reqwest::StatusCode, error_text: &str) -> bool {
    driver_helpers::is_request_too_large(status, error_text, GEMINI_TOO_LARGE_PATTERNS)
}

// ============================================================================
// Gemini API Types
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cached_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    continuation_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum GeminiPart {
    Text {
        text: String,
        /// Marks this part as model reasoning rather than answer text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought: Option<bool>,
        /// Opaque signature over the reasoning this part belongs to. Gemini
        /// requires it returned on later turns or multi-turn tool use degrades.
        #[serde(
            rename = "thoughtSignature",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        thought_signature: Option<String>,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: GeminiBlob,
    },
    FileData {
        #[serde(rename = "fileData")]
        file_data: GeminiFileData,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
        /// Reasoning signature bound to this specific call.
        #[serde(
            rename = "thoughtSignature",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        thought_signature: Option<String>,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiBlob {
    mime_type: String,
    data: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFileData {
    mime_type: String,
    file_uri: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    args: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiFunctionResponse {
    name: String,
    response: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    /// Reasoning controls. Omitted entirely for non-thinking models and for
    /// effort `none`, since the field is rejected there.
    #[serde(rename = "thinkingConfig", skip_serializing_if = "Option::is_none")]
    thinking_config: Option<GeminiThinkingConfig>,
}

/// Gemini reasoning controls.
///
/// `include_thoughts` is what makes reasoning observable at all: without it the
/// model still thinks but returns no thought parts, so nothing reaches the
/// reasoning channel and no signature is available to replay.
#[derive(Debug, Serialize)]
struct GeminiThinkingConfig {
    #[serde(rename = "thinkingBudget", skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<u32>,
    #[serde(rename = "thinkingLevel", skip_serializing_if = "Option::is_none")]
    thinking_level: Option<&'static str>,
    #[serde(rename = "includeThoughts")]
    include_thoughts: bool,
}

// --- response side ---

/// One `tools` entry: function declarations, or the Google Search grounding
/// tool (`{"google_search": {}}`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiTool {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    function_declarations: Vec<GeminiFunctionDeclaration>,
    #[serde(rename = "google_search", skip_serializing_if = "Option::is_none")]
    google_search: Option<Value>,
}

#[derive(Debug, Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: Value,
}

// ============================================================================
// Streaming Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiStreamResponse {
    #[serde(default)]
    model_version: Option<String>,
    #[serde(default)]
    candidates: Option<Vec<GeminiCandidate>>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    continuation_token: Option<String>,
    #[serde(default)]
    content: Option<GeminiResponseContent>,
    #[serde(default)]
    finish_reason: Option<String>,
    /// Present on a grounded answer.
    #[serde(default)]
    grounding_metadata: Option<GeminiGroundingMetadata>,
}

/// The part of `groundingMetadata` the driver reads: the search queries the
/// model ran, and the sources with the answer segments they support.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGroundingMetadata {
    #[serde(default)]
    web_search_queries: Vec<String>,
    #[serde(default)]
    grounding_chunks: Vec<GroundingChunk>,
    #[serde(default)]
    grounding_supports: Vec<GroundingSupport>,
}

#[derive(Debug, Deserialize)]
struct GeminiResponseContent {
    parts: Vec<GeminiResponsePart>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GeminiResponsePart {
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
        #[serde(rename = "thoughtSignature", default)]
        thought_signature: Option<String>,
    },
    Text {
        text: String,
        #[serde(default)]
        thought: Option<bool>,
        #[serde(rename = "thoughtSignature", default)]
        thought_signature: Option<String>,
    },
    #[allow(dead_code)]
    Other(Value),
}

impl GeminiPart {
    fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            thought: None,
            thought_signature: None,
        }
    }

    /// A reasoning part replayed back to the model.
    fn thought(text: impl Into<String>, signature: Option<String>) -> Self {
        Self::Text {
            text: text.into(),
            thought: Some(true),
            thought_signature: signature,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    #[serde(default)]
    thoughts_token_count: Option<u32>,
    #[serde(default)]
    prompt_token_count: Option<u32>,
    #[serde(default)]
    candidates_token_count: Option<u32>,
    #[serde(default)]
    cached_content_token_count: Option<u32>,
}

// ============================================================================
// Models API Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct GeminiModelsResponse {
    #[serde(default)]
    models: Vec<GeminiModelInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiModelInfo {
    /// Model name (e.g., "models/gemini-2.5-pro")
    name: String,
    /// Display name (e.g., "Gemini 2.5 Pro")
    display_name: String,
    /// Supported generation methods
    #[serde(default)]
    supported_generation_methods: Option<Vec<String>>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
