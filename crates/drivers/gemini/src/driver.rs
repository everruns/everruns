// Google Gemini Chat Driver
//
// Implementation of ChatDriver for Google's Gemini API.
// Uses the generateContent API with streaming support.
//
// API docs: https://ai.google.dev/api/generate-content
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff. Retry metadata is included in the response for observability.

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use everruns_provider::credential_schema::CredentialFormSchema;
use everruns_provider::driver_helpers::{
    self, AUDIO_CONTENT_PLACEHOLDER, GEMINI_NOT_FOUND_PATTERNS, GEMINI_TOO_LARGE_PATTERNS,
    parse_data_url,
};
use everruns_provider::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmCompletionMetadata, LlmContentPart, LlmMessage, LlmMessageContent, LlmMessageRole,
    LlmResponseStream, LlmStreamEvent, disjoint_prompt_tokens, fold_system_messages,
};
use everruns_provider::error::{AgentLoopError, LlmErrorKind, Result};
use everruns_provider::is_provider_quota_message;
use everruns_provider::llm_retry::{
    LlmRetryConfig, RetryDecision, RetryMetadata, SendOutcome, retry_request, send_error_message,
};
use everruns_provider::reasoning::{ReasoningContentPart, ReasoningText};
use everruns_provider::stream_accumulator::StreamToolCallAccumulator;
use everruns_provider::stream_reconnect::{ByteStream, connect_bytes_with_reconnect};
use everruns_provider::tool_types::ToolDefinition;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Ready-to-use Gemini provider assembly.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    api_key: impl Into<String>,
) -> everruns_provider::Provider {
    everruns_provider::Provider::new(id, GeminiChatDriver::new())
        .base_url(DEFAULT_BASE_URL)
        .auth(everruns_provider::StaticHeaderAuth::new(
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
    client: Client,
    retry_config: LlmRetryConfig,
}

impl GeminiChatDriver {
    /// Create a new driver with the given API key
    pub fn new() -> Self {
        Self {
            client: everruns_provider::driver_helpers::shared_streaming_http_client(),
            retry_config: LlmRetryConfig::default(),
        }
    }

    fn convert_role(role: &LlmMessageRole) -> &'static str {
        match role {
            LlmMessageRole::System => "user", // System is handled separately
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "model",
            LlmMessageRole::Tool => "user", // Tool results sent as function responses
        }
    }

    fn convert_content(content: &LlmMessageContent) -> Vec<GeminiPart> {
        match content {
            LlmMessageContent::Text(text) => {
                if text.is_empty() {
                    vec![]
                } else {
                    vec![GeminiPart::text(text.clone())]
                }
            }
            LlmMessageContent::Parts(parts) => parts
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
                })
                .collect(),
        }
    }

    fn convert_messages(messages: &[LlmMessage]) -> (Option<GeminiContent>, Vec<GeminiContent>) {
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
                LlmMessageRole::System => {
                    // Folded above into `system_instruction`; never emit
                    // System-role content into the Gemini `contents` array.
                }
                LlmMessageRole::Tool => {
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

                        contents.push(GeminiContent {
                            role: Some("user".to_string()),
                            parts: vec![GeminiPart::FunctionResponse {
                                function_response: GeminiFunctionResponse {
                                    name: String::from(*name),
                                    response: response_value,
                                },
                            }],
                        });
                    }
                }
                LlmMessageRole::Assistant => {
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
        }])
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
        endpoint: &everruns_provider::ProviderEndpoint,
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
                let mut builder = self.client.post(&resolved.url);
                let mut headers = resolved.headers;
                headers.push(("Content-Type".to_string(), "application/json".to_string()));
                for (name, value) in
                    everruns_provider::driver_helpers::merge_request_headers(headers, extra_headers)
                {
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
                        *last_error.lock().unwrap() = Some(error_text);
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
}

#[async_trait]
impl ChatDriver for GeminiChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let (system_instruction, contents) = Self::convert_messages(&messages);

        let tools = Self::convert_tools(&config.tools);

        // Reasoning is opt-in on Gemini and silent when absent: without a
        // thinkingConfig the model still reasons but returns no thought parts
        // and no signature, so nothing reaches the reasoning channel and
        // multi-turn tool use loses its thought continuity.
        let thinking_config = config
            .reasoning_effort
            .filter(everruns_provider::ReasoningEffort::requests_reasoning)
            .map(|effort| GeminiThinkingConfig {
                thinking_budget: driver_helpers::thinking_budget::from_effort(effort),
                include_thoughts: true,
            });

        let mut generation_config = GeminiGenerationConfig {
            temperature: config.temperature,
            max_output_tokens: config.max_tokens,
            thinking_config,
        };

        // If no max_tokens specified, use model's max output from profile, or 8192 fallback
        if generation_config.max_output_tokens.is_none() {
            generation_config.max_output_tokens = Some(
                everruns_provider::get_model_profile(
                    &everruns_provider::DriverId::Gemini,
                    &config.model,
                )
                .and_then(|p| {
                    p.limits.and_then(|l| {
                        u32::try_from(l.output)
                            .ok()
                            .and_then(|v| if v > 0 { Some(v) } else { None })
                    })
                })
                .unwrap_or(8_192),
            );
        }

        let request = GeminiRequest {
            contents,
            system_instruction,
            tools,
            generation_config: Some(generation_config),
            cached_content: config
                .prompt_cache
                .as_ref()
                .filter(|cfg| cfg.enabled)
                .and_then(|cfg| cfg.gemini_cached_content.clone()),
        };

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
            retry_metadata: retry_metadata.had_retries().then_some(retry_metadata),
            ..Default::default()
        };
        Ok(convert_gemini_stream(byte_stream, state))
    }

    async fn list_models(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        if endpoint.base_url() != Some(DEFAULT_BASE_URL) {
            return Ok(None);
        }

        let url = endpoint
            .url("models")
            .ok_or_else(|| AgentLoopError::config("Gemini provider has no base URL"))?;
        let resolved = endpoint.resolve("GET", url, &[]).await?;
        let mut request = self.client.get(&resolved.url);
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
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(DriverDescriptor {
        display_name: "Google Gemini".into(),
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key in [Google AI Studio](https://aistudio.google.com/apikey).",
        ),
        ..DriverDescriptor::chat_only(DriverId::Gemini, |config| {
            let provider =
                everruns_provider::Provider::new(config.provider.clone(), GeminiChatDriver::new())
                    .base_url(config.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL))
                    .auth(everruns_provider::StaticHeaderAuth::new(
                        "x-goog-api-key",
                        config.api_key.as_deref().unwrap_or(""),
                    ));
            provider.into_boxed_driver()
        })
    });
}

impl Default for GeminiChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// SSE Parsing
// ============================================================================

fn convert_gemini_stream(byte_stream: ByteStream, state: GeminiStreamState) -> LlmResponseStream {
    // THREAT[TM-TOOL-038]: decode UTF-8 and SSE framing before parsing provider JSON. Transport chunks
    // may split a character, line or event anywhere.
    let event_stream = byte_stream.eventsource();
    Box::pin(futures::stream::unfold(
        (event_stream, state),
        |(mut stream, mut state)| async move {
            loop {
                if let Some(event) = state.pending.pop_front() {
                    return Some((Ok(event), (stream, state)));
                }
                if state.done {
                    return None;
                }
                match stream.next().await {
                    Some(Ok(event)) if event.data == "[DONE]" => state.finish(),
                    Some(Ok(event)) => {
                        match serde_json::from_str::<GeminiStreamResponse>(&event.data) {
                            Ok(response) => state.response(response),
                            Err(error) => {
                                tracing::debug!(%error, "GeminiDriver: failed to parse SSE event")
                            }
                        }
                    }
                    Some(Err(error)) => {
                        state.pending.push_back(LlmStreamEvent::Error(
                            format!("Stream error: {error}").into(),
                        ));
                        state.done = true;
                    }
                    None => state.finish(),
                }
            }
        },
    ))
}

#[derive(Default)]
struct GeminiStreamState {
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: Option<u32>,
    calls: StreamToolCallAccumulator,
    call_counter: u32,
    finish_reason: Option<String>,
    retry_metadata: Option<RetryMetadata>,
    pending: std::collections::VecDeque<LlmStreamEvent>,
    done: bool,
}

impl GeminiStreamState {
    fn response(&mut self, response: GeminiStreamResponse) {
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
        }
        // Usage can arrive after the terminal candidate, but no later content
        // may reopen a completed or rejected generation.
        if self.finish_reason.is_some() {
            return;
        }
        for candidate in response.candidates.unwrap_or_default() {
            if let Some(content) = candidate.content {
                for part in content.parts {
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
            if let Some(reason) = candidate
                .finish_reason
                .filter(|reason| reason != "FINISH_REASON_UNSPECIFIED")
            {
                self.finish_reason = Some(match reason.as_str() {
                    "STOP" => "stop".into(),
                    "MAX_TOKENS" => "length".into(),
                    "SAFETY" => "content_filter".into(),
                    _ => reason.to_ascii_lowercase(),
                });
                // THREAT[TM-TOOL-037]: only an accepted terminal candidate can release pending calls.
                // The whole frame is processed before yielding any of its events.
                let calls = self.calls.take_finalized();
                if reason == "STOP" && !calls.is_empty() {
                    self.pending.push_back(LlmStreamEvent::ToolCalls(calls));
                }
                break;
            }
        }
    }

    fn finish(&mut self) {
        let calls = self.calls.take_finalized();
        if self.finish_reason.is_none() && !calls.is_empty() {
            self.pending.push_back(LlmStreamEvent::ToolCalls(calls));
        }
        self.pending
            .push_back(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                total_tokens: Some(self.input_tokens + self.output_tokens),
                prompt_tokens: Some(disjoint_prompt_tokens(
                    self.input_tokens,
                    self.cached_tokens,
                )),
                completion_tokens: Some(self.output_tokens),
                cache_read_tokens: self.cached_tokens,
                cache_creation_tokens: None,
                provider_cost_usd: None,
                model: Some(self.model.clone()),
                finish_reason: Some(self.finish_reason.take().unwrap_or_else(|| "stop".into())),
                retry_metadata: self.retry_metadata.take(),
                response_id: None,
                phase: None,
                cache_diagnostics: None,
            })));
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
    #[serde(rename = "includeThoughts")]
    include_thoughts: bool,
}

// --- response side ---

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiTool {
    function_declarations: Vec<GeminiFunctionDeclaration>,
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
    candidates: Option<Vec<GeminiCandidate>>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiResponseContent>,
    #[serde(default)]
    finish_reason: Option<String>,
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
mod tests {
    use super::*;
    use everruns_provider::driver_registry::ChatDriver;
    use everruns_provider::tool_types::ToolCall;

    // ========================================================================
    // Model-not-found detection tests
    // ========================================================================

    fn call_message(id: &str, name: &str, arguments: Value) -> LlmMessage {
        let mut message = LlmMessage::text(LlmMessageRole::Assistant, "");
        message.tool_calls = Some(vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments,
        }]);
        message
    }

    fn result_message(id: Option<&str>, text: &str) -> LlmMessage {
        let mut message = LlmMessage::text(LlmMessageRole::Tool, text);
        message.tool_call_id = id.map(str::to_string);
        message
    }

    #[test]
    fn transcript_preserves_system_order_and_resolves_tool_names_per_turn() {
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "A"),
            result_message(Some("call_0"), "future orphan"),
            LlmMessage::text(LlmMessageRole::User, "hi"),
            call_message("call_0", "get_weather", json!({"city":"Paris"})),
            result_message(Some("call_0"), r#"{"temp":20}"#),
            LlmMessage::text(LlmMessageRole::System, "B"),
            call_message("call_0", "get_time", json!({"zone":"UTC"})),
            result_message(Some("call_0"), "12:00"),
            result_message(Some("missing"), "orphan"),
            result_message(None, "missing id"),
        ];
        let (system, contents) = GeminiChatDriver::convert_messages(&messages);
        assert_eq!(
            serde_json::to_value(system).unwrap(),
            json!({"parts":[{"text":"A\n\nB"}]})
        );
        assert_eq!(
            serde_json::to_value(contents).unwrap(),
            json!([
                {"role":"user","parts":[{"text":"hi"}]},
                {"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]},
                {"role":"user","parts":[{"functionResponse":{"name":"get_weather","response":{"temp":20}}}]},
                {"role":"model","parts":[{"functionCall":{"name":"get_time","args":{"zone":"UTC"}}}]},
                {"role":"user","parts":[{"functionResponse":{"name":"get_time","response":{"result":"12:00"}}}]}
            ])
        );
        let (system, contents) = GeminiChatDriver::convert_messages(&[
            LlmMessage::text(LlmMessageRole::User, ""),
            result_message(Some("missing"), "orphan"),
        ]);
        assert!(system.is_none());
        assert!(contents.is_empty());
    }

    #[test]
    fn function_response_payloads_are_objects_without_losing_scalar_values() {
        for (text, expected) in [
            (r#"{"value":3}"#, json!({"value":3})),
            ("plain result", json!({"result":"plain result"})),
            ("[1,2]", json!({"result":[1,2]})),
            ("null", json!({"result":null})),
            ("true", json!({"result":true})),
            ("42", json!({"result":42})),
            (r#""quoted""#, json!({"result":"quoted"})),
        ] {
            let (_, contents) = GeminiChatDriver::convert_messages(&[
                call_message("call_7", "lookup", json!({})),
                result_message(Some("call_7"), text),
            ]);
            assert_eq!(
                serde_json::to_value(&contents[1]).unwrap(),
                json!({"role":"user","parts":[{"functionResponse":{"name":"lookup","response":expected}}]}),
                "{text}"
            );
        }
    }

    #[test]
    fn content_conversion_preserves_text_and_media_order_while_omitting_empty_parts() {
        let parts = LlmMessageContent::Parts(vec![
            LlmContentPart::text(""),
            LlmContentPart::text("before"),
            LlmContentPart::image("data:image/png;base64,aGVsbG8="),
            LlmContentPart::image("data:malformed"),
            LlmContentPart::image("https://images.example/photo.jpg"),
            LlmContentPart::Audio {
                url: "data:audio/wav;base64,YQ==".into(),
            },
            LlmContentPart::text("after"),
        ]);
        assert_eq!(
            serde_json::to_value(GeminiChatDriver::convert_content(&parts)).unwrap(),
            json!([
                {"text":"before"}, {"inlineData":{"mimeType":"image/png","data":"aGVsbG8="}},
                {"fileData":{"mimeType":"image/jpeg","fileUri":"https://images.example/photo.jpg"}},
                {"text":"[Audio content not supported]"}, {"text":"after"}
            ])
        );
        assert_eq!(
            serde_json::to_value(GeminiChatDriver::convert_content(&LlmMessageContent::Text(
                "hello".into()
            )))
            .unwrap(),
            json!([{"text":"hello"}])
        );
        assert_eq!(
            serde_json::to_value(GeminiChatDriver::convert_content(&LlmMessageContent::Text(
                String::new()
            )))
            .unwrap(),
            json!([])
        );
    }
    async fn collect_wire_chunks(chunks: Vec<Vec<u8>>) -> Vec<Value> {
        let bytes: ByteStream = Box::pin(futures::stream::iter(
            chunks.into_iter().map(|chunk| Ok(chunk.into())),
        ));
        let mut stream = convert_gemini_stream(
            bytes,
            GeminiStreamState {
                model: "model".into(),
                ..Default::default()
            },
        );
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(match event.unwrap() {
                LlmStreamEvent::TextDelta(text) => json!({"text":text}),
                LlmStreamEvent::ToolCalls(calls) => json!({"calls":calls}),
                LlmStreamEvent::Done(metadata) => json!({"done":{"model":metadata.model,"reason":metadata.finish_reason,"input":metadata.prompt_tokens,"output":metadata.completion_tokens,"total":metadata.total_tokens}}),
                other => panic!("unexpected event: {other:?}"),
            });
        }
        events
    }

    fn expected_unicode_events() -> Vec<Value> {
        vec![
            json!({"text":"hé🙂"}),
            json!({"calls":[{"id":"call_0","name":"lookup","arguments":{"path":"café/🙂"}}]}),
            json!({"done":{"model":"model","reason":"stop","input":3,"output":1,"total":4}}),
        ]
    }

    #[tokio::test]
    async fn stream_preserves_unicode_at_every_transport_boundary() {
        let wire = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hé🙂\"},{\"functionCall\":{\"name\":\"lookup\",\"args\":{\"path\":\"café/🙂\"}}}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":1}}\n\ndata: [DONE]\n\n".as_bytes();
        for split in 0..=wire.len() {
            assert_eq!(
                collect_wire_chunks(vec![wire[..split].to_vec(), wire[split..].to_vec()]).await,
                expected_unicode_events(),
                "byte split {split}"
            );
        }
        assert_eq!(
            collect_wire_chunks(wire.iter().map(|byte| vec![*byte]).collect()).await,
            expected_unicode_events()
        );
    }

    #[tokio::test]
    async fn stream_accepts_multiline_sse_data_and_optional_space() {
        let wire = ": heartbeat\r\nevent: message\r\nid: frame-one\r\ndata:{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hé🙂\"},{\"functionCall\":{\"name\":\"lookup\",\"args\":{\"path\":\"café/🙂\"}}}]},\"finishReason\":\"STOP\"}],\r\ndata: \"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":1}}\r\n\r\ndata:[DONE]\r\n\r\n".as_bytes();
        assert_eq!(
            collect_wire_chunks(wire.iter().map(|byte| vec![*byte]).collect()).await,
            expected_unicode_events()
        );
    }

    #[test]
    fn tool_schema_cleanup_preserves_complete_contract_and_literal_payloads() {
        use everruns_provider::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
        let parameters = json!({
            "type":"object","additionalProperties":false,"required":["items"],
            "properties":{
                "additionalProperties":{"type":"boolean","description":"user property"},
                "items":{"type":"array","items":{"type":"object","additionalProperties":false,"properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0,"default":0}},"required":["path"]}},
                "variant":{"anyOf":[{"type":"string"},{"type":"object","additionalProperties":true}]}
            },
            "default":{"additionalProperties":"default data"},
            "examples":[{"additionalProperties":false}],
            "enum":[{"additionalProperties":"enum data"}],
            "const":{"additionalProperties":"constant data"},
            "x-extension":{"additionalProperties":"extension data"}
        });
        let tools = vec![ToolDefinition::Builtin(BuiltinTool {
            name: "inspect".into(),
            display_name: None,
            description: "Inspect paths".into(),
            parameters,
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
            full_parameters: None,
        })];
        assert_eq!(
            serde_json::to_value(GeminiChatDriver::convert_tools(&tools).unwrap()).unwrap(),
            json!([{"functionDeclarations":[{"name":"inspect","description":"Inspect paths","parameters":{
                "type":"object","required":["items"],
                "properties":{
                    "additionalProperties":{"type":"boolean","description":"user property"},
                    "items":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0,"default":0}},"required":["path"]}},
                    "variant":{"anyOf":[{"type":"string"},{"type":"object"}]}
                },
                "default":{"additionalProperties":"default data"},"examples":[{"additionalProperties":false}],"enum":[{"additionalProperties":"enum data"}],"const":{"additionalProperties":"constant data"},"x-extension":{"additionalProperties":"extension data"}
            }}]}])
        );
        assert!(GeminiChatDriver::convert_tools(&[]).is_none());
    }
    #[test]
    fn size_classification_requires_status_and_provider_or_context_evidence() {
        for (status, message, expected) in [
            (413, "", true),
            (400, "Request payload size exceeds the limit", true),
            (400, "content too large", true),
            (400, "TOKEN LIMIT EXCEEDED", true),
            (400, "input is too long", true),
            (400, "request exceeds the maximum context", true),
            (400, "rate limit exceeded", false),
            (401, "Invalid API key", false),
            (500, "token limit exceeded", false),
            (200, "content too large", false),
        ] {
            assert_eq!(
                is_gemini_request_too_large(
                    reqwest::StatusCode::from_u16(status).unwrap(),
                    message
                ),
                expected,
                "{status}: {message}"
            );
        }
    }

    #[test]
    fn missing_model_classification_requires_404_and_gemini_evidence() {
        for (status, message, expected) in [
            (404, r#"{"error":{"status":"NOT_FOUND"}}"#, true),
            (404, "Model does not exist", true),
            (404, "Endpoint not found", false),
            (400, r#"{"error":{"status":"NOT_FOUND"}}"#, false),
            (401, "model not found", false),
            (500, "Model does not exist", false),
        ] {
            assert_eq!(
                is_gemini_model_not_found(reqwest::StatusCode::from_u16(status).unwrap(), message),
                expected,
                "{status}: {message}"
            );
        }
    }

    #[tokio::test]
    async fn request_limits_cache_gate_and_parallel_preference_reach_wire_contract() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for (model, limit, cache, parallel, expected_limit, expected_cache) in [
            (
                "gemini-3.1-pro-preview",
                None,
                Some((false, Some("cachedContents/disabled"))),
                Some(false),
                65536,
                None,
            ),
            (
                "unknown-model",
                None,
                Some((true, None)),
                Some(true),
                8192,
                None,
            ),
            (
                "gemini-3.1-pro-preview",
                Some(7),
                Some((true, Some("cachedContents/active"))),
                None,
                7,
                Some("cachedContents/active"),
            ),
            ("unknown-model", Some(9), None, Some(false), 9, None),
        ] {
            let server = MockServer::builder().start().await;
            Mock::given(method("POST")).and(path(format!("/v1beta/models/{model}:streamGenerateContent"))).and(query_param("alt", "sse")).and(header("x-goog-api-key", "synthetic-key")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]},\"finishReason\":\"STOP\"}]}\n\n")).expect(1).mount(&server).await;
            let service =
                provider("test", "synthetic-key").base_url(format!("{}/v1beta", server.uri()));
            let config = LlmCallConfig {
                model: model.into(),
                temperature: Some(0.25),
                max_tokens: limit,
                tools: vec![],
                reasoning_effort: None,
                speed: None,
                verbosity: None,
                metadata: Default::default(),
                previous_response_id: None,
                provider_opaque_context: None,
                tool_search: None,
                prompt_cache: cache.map(|(enabled, handle)| {
                    everruns_provider::driver_registry::PromptCacheConfig {
                        enabled,
                        strategy: Default::default(),
                        gemini_cached_content: handle.map(str::to_string),
                    }
                }),
                openrouter_routing: None,
                parallel_tool_calls: parallel,
                volatile_suffix_len: 0,
                extra_headers: vec![],
                cache_diagnostics: None,
            };
            let response = service
                .chat_completion(vec![LlmMessage::text(LlmMessageRole::User, "hi")], &config)
                .await
                .unwrap();
            assert_eq!(response.text, "ok");
            assert_eq!(response.metadata.finish_reason.as_deref(), Some("stop"));
            assert!(!GeminiChatDriver::new().supports_parallel_tool_calls(model));
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            let mut expected = json!({"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.25,"maxOutputTokens":expected_limit}});
            if let Some(handle) = expected_cache {
                expected["cachedContent"] = json!(handle);
            }
            assert_eq!(requests[0].body_json::<Value>().unwrap(), expected);
        }
    }
}
