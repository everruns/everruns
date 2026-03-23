// Google Gemini LLM Driver
//
// Implementation of LlmDriver for Google's Gemini API.
// Uses the generateContent API with streaming support.
//
// API docs: https://ai.google.dev/api/generate-content
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff. Retry metadata is included in the response for observability.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_helpers::{
    self, AUDIO_CONTENT_PLACEHOLDER, GEMINI_NOT_FOUND_PATTERNS, GEMINI_TOO_LARGE_PATTERNS,
    parse_data_url,
};
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverRegistry, LlmCallConfig, LlmCompletionMetadata,
    LlmContentPart, LlmDriver, LlmMessage, LlmMessageContent, LlmMessageRole, LlmResponseStream,
    LlmStreamEvent, ProviderType,
};
use everruns_core::llm_retry::{LlmRetryConfig, RetryMetadata, is_transient_error};
use everruns_core::tool_types::{ToolCall, ToolDefinition};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Google Gemini LLM Driver
///
/// Implements `LlmDriver` for Google's Gemini API.
/// Supports streaming responses and tool calls.
#[derive(Clone)]
pub struct GeminiLlmDriver {
    client: Client,
    api_key: String,
    base_url: String,
    uses_custom_url: bool,
    retry_config: LlmRetryConfig,
}

impl GeminiLlmDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            uses_custom_url: false,
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// Create a new driver from the GEMINI_API_KEY environment variable
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .map_err(|_| AgentLoopError::llm("GEMINI_API_KEY environment variable not set"))?;
        Ok(Self::new(api_key))
    }

    /// Create a new driver with a custom base URL
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            uses_custom_url: true,
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
                    vec![GeminiPart::Text { text: text.clone() }]
                }
            }
            LlmMessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    LlmContentPart::Text { text } => {
                        if text.is_empty() {
                            None
                        } else {
                            Some(GeminiPart::Text { text: text.clone() })
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
                    LlmContentPart::Audio { .. } => Some(GeminiPart::Text {
                        text: AUDIO_CONTENT_PLACEHOLDER.to_string(),
                    }),
                })
                .collect(),
        }
    }

    fn convert_messages(messages: &[LlmMessage]) -> (Option<GeminiContent>, Vec<GeminiContent>) {
        let mut system_instruction = None;
        let mut contents = Vec::new();

        for msg in messages {
            match msg.role {
                LlmMessageRole::System => {
                    // Extract system instruction (Gemini handles it separately)
                    system_instruction = Some(GeminiContent {
                        role: None, // system_instruction has no role
                        parts: vec![GeminiPart::Text {
                            text: msg.content.to_text(),
                        }],
                    });
                }
                LlmMessageRole::Tool => {
                    // Tool results in Gemini use functionResponse parts
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        // Try to parse as JSON, fall back to wrapping in object
                        let response_value = serde_json::from_str::<Value>(&msg.content.to_text())
                            .unwrap_or_else(|_| json!({"result": msg.content.to_text()}));

                        contents.push(GeminiContent {
                            role: Some("user".to_string()),
                            parts: vec![GeminiPart::FunctionResponse {
                                function_response: GeminiFunctionResponse {
                                    name: tool_call_id.clone(),
                                    response: response_value,
                                },
                            }],
                        });
                    }
                }
                LlmMessageRole::Assistant => {
                    let mut parts = Self::convert_content(&msg.content);

                    // Add function call parts if present
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
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

    /// Strip fields that Gemini doesn't accept in JSON Schema (e.g. `additionalProperties`).
    fn clean_schema(mut value: Value) -> Value {
        if let Some(obj) = value.as_object_mut() {
            obj.remove("additionalProperties");
            if let Some(props_obj) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) {
                for v in props_obj.values_mut() {
                    *v = Self::clean_schema(v.clone());
                }
            }
        }
        value
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
    fn stream_url(&self, model: &str) -> String {
        format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url, model, self.api_key
        )
    }

    /// Build the models list URL
    fn models_url(&self) -> String {
        format!("{}/models?key={}", self.base_url, self.api_key)
    }
}

#[async_trait]
impl LlmDriver for GeminiLlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let (system_instruction, contents) = Self::convert_messages(&messages);

        let tools = Self::convert_tools(&config.tools);

        let mut generation_config = GeminiGenerationConfig {
            temperature: config.temperature,
            max_output_tokens: config.max_tokens,
        };

        // If no max_tokens specified, default to 8192
        if generation_config.max_output_tokens.is_none() {
            generation_config.max_output_tokens = Some(8192);
        }

        let request = GeminiRequest {
            contents,
            system_instruction,
            tools,
            generation_config: Some(generation_config),
        };

        // Retry loop for rate limit (429) and transient errors
        let mut retry_metadata = RetryMetadata::default();
        let mut last_error: Option<String> = None;

        let url = self.stream_url(&config.model);

        let response = loop {
            let response = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .map_err(|e| AgentLoopError::llm(format!("Failed to send request: {}", e)))?;

            let status = response.status();

            if status.is_success() {
                break response;
            }

            // Check if this is a retryable error
            if is_transient_error(status) && retry_metadata.attempts < self.retry_config.max_retries
            {
                let error_text = response.text().await.unwrap_or_default();

                // Don't retry if this is a request-too-large error
                if is_gemini_request_too_large(status, &error_text) {
                    return Err(AgentLoopError::request_too_large(format!(
                        "Gemini API error ({}): {}",
                        status, error_text
                    )));
                }

                let wait_duration = self.retry_config.calculate_backoff(retry_metadata.attempts);

                tracing::warn!(
                    status = %status,
                    attempt = retry_metadata.attempts + 1,
                    max_retries = self.retry_config.max_retries,
                    wait_secs = wait_duration.as_secs_f64(),
                    "GeminiDriver: transient error, retrying"
                );

                retry_metadata.record_retry(wait_duration, None);
                last_error = Some(error_text);

                tokio::time::sleep(wait_duration).await;
                continue;
            }

            // Non-retryable error or max retries exceeded
            let error_text = response.text().await.unwrap_or_default();
            let error_msg = format!("Gemini API error ({}): {}", status, error_text);

            // Check if this is a model-not-found error
            if is_gemini_model_not_found(status, &error_text) {
                return Err(AgentLoopError::model_not_available(config.model.clone()));
            }

            if is_gemini_request_too_large(status, &error_text) {
                return Err(AgentLoopError::request_too_large(error_msg));
            }

            if retry_metadata.attempts > 0 {
                return Err(AgentLoopError::llm(format!(
                    "{} (after {} retries, last error: {})",
                    error_msg,
                    retry_metadata.attempts,
                    last_error.unwrap_or_default()
                )));
            }

            return Err(AgentLoopError::llm(error_msg));
        };

        if retry_metadata.had_retries() {
            tracing::info!(
                attempts = retry_metadata.attempts,
                total_wait_secs = retry_metadata.total_retry_wait.as_secs_f64(),
                "GeminiDriver: request succeeded after retries"
            );
        }

        // Gemini streams SSE events with JSON data, each containing a candidate
        let model = config.model.clone();
        let prompt_tokens = Arc::new(Mutex::new(0u32));
        let completion_tokens = Arc::new(Mutex::new(0u32));
        let cached_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));
        let tool_call_counter = Arc::new(Mutex::new(0u32));
        let shared_retry_metadata = if retry_metadata.had_retries() {
            Some(Arc::new(retry_metadata))
        } else {
            None
        };

        // Gemini SSE stream: each event has `data: {...}` lines
        let byte_stream = response.bytes_stream();

        // Use a buffered approach to handle SSE events
        let converted_stream: LlmResponseStream = Box::pin(futures::stream::unfold(
            (
                byte_stream,
                String::new(), // buffer for partial SSE data
                model,
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                accumulated_tool_calls,
                tool_call_counter,
                shared_retry_metadata,
                false, // done flag
            ),
            move |(
                mut stream,
                mut buffer,
                model,
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                accumulated_tool_calls,
                tool_call_counter,
                retry_metadata,
                done,
            )| async move {
                if done {
                    return None;
                }

                loop {
                    // Try to extract complete SSE events from buffer
                    if let Some(event) = extract_sse_event(&mut buffer) {
                        if event == "[DONE]" {
                            let in_tokens = *prompt_tokens.lock().unwrap();
                            let out_tokens = *completion_tokens.lock().unwrap();
                            let cached = *cached_tokens.lock().unwrap();

                            let result =
                                Ok(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                                    total_tokens: Some(in_tokens + out_tokens),
                                    prompt_tokens: Some(in_tokens),
                                    completion_tokens: Some(out_tokens),
                                    cache_read_tokens: cached,
                                    cache_creation_tokens: None,
                                    model: Some(model.clone()),
                                    finish_reason: Some("stop".to_string()),
                                    retry_metadata: retry_metadata
                                        .as_ref()
                                        .map(|arc| (**arc).clone()),
                                    response_id: None,
                                    phase: None,
                                })));
                            return Some((
                                result,
                                (
                                    stream,
                                    buffer,
                                    model,
                                    prompt_tokens,
                                    completion_tokens,
                                    cached_tokens,
                                    accumulated_tool_calls,
                                    tool_call_counter,
                                    retry_metadata,
                                    true,
                                ),
                            ));
                        }

                        // Parse the JSON data
                        match serde_json::from_str::<GeminiStreamResponse>(&event) {
                            Ok(response) => {
                                // Update usage metadata
                                if let Some(usage) = &response.usage_metadata {
                                    if let Some(pt) = usage.prompt_token_count {
                                        *prompt_tokens.lock().unwrap() = pt;
                                    }
                                    if let Some(ct) = usage.candidates_token_count {
                                        *completion_tokens.lock().unwrap() = ct;
                                    }
                                    if let Some(cache) = usage.cached_content_token_count {
                                        *cached_tokens.lock().unwrap() = Some(cache);
                                    }
                                }

                                if let Some(candidates) = &response.candidates {
                                    for candidate in candidates {
                                        if let Some(content) = &candidate.content {
                                            for part in &content.parts {
                                                match part {
                                                    GeminiResponsePart::Text { text } => {
                                                        let result = Ok(LlmStreamEvent::TextDelta(
                                                            text.clone(),
                                                        ));
                                                        return Some((
                                                            result,
                                                            (
                                                                stream,
                                                                buffer,
                                                                model,
                                                                prompt_tokens,
                                                                completion_tokens,
                                                                cached_tokens,
                                                                accumulated_tool_calls,
                                                                tool_call_counter,
                                                                retry_metadata,
                                                                false,
                                                            ),
                                                        ));
                                                    }
                                                    GeminiResponsePart::FunctionCall {
                                                        function_call,
                                                    } => {
                                                        let mut counter =
                                                            tool_call_counter.lock().unwrap();
                                                        let call_id = format!("call_{}", *counter);
                                                        *counter += 1;

                                                        accumulated_tool_calls
                                                            .lock()
                                                            .unwrap()
                                                            .push(ToolCall {
                                                                id: call_id,
                                                                name: function_call.name.clone(),
                                                                arguments: function_call
                                                                    .args
                                                                    .clone(),
                                                            });
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }

                                        // Check finish reason
                                        if let Some(reason) = &candidate.finish_reason
                                            && (reason == "STOP" || reason == "MAX_TOKENS")
                                        {
                                            let tool_calls: Vec<ToolCall> = std::mem::take(
                                                &mut *accumulated_tool_calls.lock().unwrap(),
                                            );
                                            if !tool_calls.is_empty() {
                                                let result =
                                                    Ok(LlmStreamEvent::ToolCalls(tool_calls));
                                                return Some((
                                                    result,
                                                    (
                                                        stream,
                                                        buffer,
                                                        model,
                                                        prompt_tokens,
                                                        completion_tokens,
                                                        cached_tokens,
                                                        accumulated_tool_calls,
                                                        tool_call_counter,
                                                        retry_metadata,
                                                        false,
                                                    ),
                                                ));
                                            }

                                            // Emit Done event
                                            let in_tokens = *prompt_tokens.lock().unwrap();
                                            let out_tokens = *completion_tokens.lock().unwrap();
                                            let cached = *cached_tokens.lock().unwrap();
                                            let finish = match reason.as_str() {
                                                "MAX_TOKENS" => "length",
                                                _ => "stop",
                                            };

                                            let result = Ok(LlmStreamEvent::Done(Box::new(
                                                LlmCompletionMetadata {
                                                    total_tokens: Some(in_tokens + out_tokens),
                                                    prompt_tokens: Some(in_tokens),
                                                    completion_tokens: Some(out_tokens),
                                                    cache_read_tokens: cached,
                                                    cache_creation_tokens: None,
                                                    model: Some(model.clone()),
                                                    finish_reason: Some(finish.to_string()),
                                                    retry_metadata: retry_metadata
                                                        .as_ref()
                                                        .map(|arc| (**arc).clone()),
                                                    response_id: None,
                                                    phase: None,
                                                },
                                            )));
                                            return Some((
                                                result,
                                                (
                                                    stream,
                                                    buffer,
                                                    model,
                                                    prompt_tokens,
                                                    completion_tokens,
                                                    cached_tokens,
                                                    accumulated_tool_calls,
                                                    tool_call_counter,
                                                    retry_metadata,
                                                    true,
                                                ),
                                            ));
                                        }
                                    }
                                }

                                // No actionable content in this chunk, continue
                                continue;
                            }
                            Err(e) => {
                                tracing::debug!(
                                    error = %e,
                                    data = %event,
                                    "GeminiDriver: failed to parse SSE event"
                                );
                                continue;
                            }
                        }
                    }

                    // Need more data from the stream
                    match stream.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(e)) => {
                            let result = Ok(LlmStreamEvent::Error(format!("Stream error: {}", e)));
                            return Some((
                                result,
                                (
                                    stream,
                                    buffer,
                                    model,
                                    prompt_tokens,
                                    completion_tokens,
                                    cached_tokens,
                                    accumulated_tool_calls,
                                    tool_call_counter,
                                    retry_metadata,
                                    true,
                                ),
                            ));
                        }
                        None => {
                            // Stream ended - emit Done if we haven't already
                            let tool_calls: Vec<ToolCall> =
                                std::mem::take(&mut *accumulated_tool_calls.lock().unwrap());
                            if !tool_calls.is_empty() {
                                let result = Ok(LlmStreamEvent::ToolCalls(tool_calls));
                                return Some((
                                    result,
                                    (
                                        stream,
                                        buffer,
                                        model,
                                        prompt_tokens,
                                        completion_tokens,
                                        cached_tokens,
                                        accumulated_tool_calls,
                                        tool_call_counter,
                                        retry_metadata,
                                        false,
                                    ),
                                ));
                            }

                            let in_tokens = *prompt_tokens.lock().unwrap();
                            let out_tokens = *completion_tokens.lock().unwrap();
                            let cached = *cached_tokens.lock().unwrap();

                            let result =
                                Ok(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                                    total_tokens: Some(in_tokens + out_tokens),
                                    prompt_tokens: Some(in_tokens),
                                    completion_tokens: Some(out_tokens),
                                    cache_read_tokens: cached,
                                    cache_creation_tokens: None,
                                    model: Some(model.clone()),
                                    finish_reason: Some("stop".to_string()),
                                    retry_metadata: retry_metadata
                                        .as_ref()
                                        .map(|arc| (**arc).clone()),
                                    response_id: None,
                                    phase: None,
                                })));
                            return Some((
                                result,
                                (
                                    stream,
                                    buffer,
                                    model,
                                    prompt_tokens,
                                    completion_tokens,
                                    cached_tokens,
                                    accumulated_tool_calls,
                                    tool_call_counter,
                                    retry_metadata,
                                    true,
                                ),
                            ));
                        }
                    }
                }
            },
        ));

        Ok(converted_stream)
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        if self.uses_custom_url {
            return Ok(None);
        }

        let response = self
            .client
            .get(self.models_url())
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

impl std::fmt::Debug for GeminiLlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeminiLlmDriver")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the Gemini driver with the driver registry
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register(ProviderType::Gemini, |api_key, base_url| {
        let driver = match base_url {
            Some(url) => GeminiLlmDriver::with_base_url(api_key, url),
            None => GeminiLlmDriver::new(api_key),
        };
        Box::new(driver) as BoxedLlmDriver
    });
}

// ============================================================================
// SSE Parsing
// ============================================================================

/// Extract a complete SSE event from the buffer, returning the data payload
fn extract_sse_event(buffer: &mut String) -> Option<String> {
    // Look for "data: " followed by a complete JSON object or "[DONE]"
    loop {
        let data_prefix = "data: ";
        let start = buffer.find(data_prefix)?;
        let data_start = start + data_prefix.len();

        // Find the end of this data line (next newline)
        let remaining = &buffer[data_start..];
        let end = remaining.find('\n')?;

        let data = remaining[..end].trim().to_string();

        // Remove consumed portion from buffer
        buffer.drain(..data_start + end + 1);

        // Skip empty data lines
        if data.is_empty() {
            continue;
        }

        return Some(data);
    }
}

// ============================================================================
// Error Detection
// ============================================================================

fn is_gemini_model_not_found(status: reqwest::StatusCode, error_text: &str) -> bool {
    llm_driver_helpers::is_model_not_found(status, error_text, GEMINI_NOT_FOUND_PATTERNS)
}

fn is_gemini_request_too_large(status: reqwest::StatusCode, error_text: &str) -> bool {
    llm_driver_helpers::is_request_too_large(status, error_text, GEMINI_TOO_LARGE_PATTERNS)
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
}

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
    },
    Text {
        text: String,
    },
    #[allow(dead_code)]
    Other(Value),
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

    #[test]
    fn test_convert_content_text() {
        let content = LlmMessageContent::Text("Hello, world!".to_string());
        let parts = GeminiLlmDriver::convert_content(&content);
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn test_convert_content_empty_text() {
        let content = LlmMessageContent::Text(String::new());
        let parts = GeminiLlmDriver::convert_content(&content);
        assert!(parts.is_empty());
    }

    #[test]
    fn test_convert_messages_system_prompt() {
        let messages = vec![
            LlmMessage {
                role: LlmMessageRole::System,
                content: LlmMessageContent::Text("You are helpful".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
            LlmMessage {
                role: LlmMessageRole::User,
                content: LlmMessageContent::Text("Hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
        ];

        let (system, contents) = GeminiLlmDriver::convert_messages(&messages);

        assert!(system.is_some());
        assert_eq!(contents.len(), 1); // Only user message
    }

    #[test]
    fn test_convert_tools() {
        use everruns_core::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};
        let tools = vec![ToolDefinition::Builtin(BuiltinTool {
            name: "get_weather".to_string(),
            display_name: None,
            description: "Get the weather for a city".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "city": {"type": "string"}
                },
                "required": ["city"]
            }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: everruns_core::tool_types::ToolHints::default(),
        })];

        let gemini_tools = GeminiLlmDriver::convert_tools(&tools);
        assert!(gemini_tools.is_some());
        let tools = gemini_tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].function_declarations.len(), 1);
        assert_eq!(tools[0].function_declarations[0].name, "get_weather");
    }

    #[test]
    fn test_convert_tools_strips_additional_properties() {
        use everruns_core::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};
        let tools = vec![ToolDefinition::Builtin(BuiltinTool {
            name: "search".to_string(),
            display_name: None,
            description: "Search".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "additionalProperties": false}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: everruns_core::tool_types::ToolHints::default(),
        })];

        let gemini_tools = GeminiLlmDriver::convert_tools(&tools).unwrap();
        let params = &gemini_tools[0].function_declarations[0].parameters;
        assert!(params.get("additionalProperties").is_none());
        assert!(
            params["properties"]["query"]
                .get("additionalProperties")
                .is_none()
        );
    }

    #[test]
    fn test_convert_tools_empty() {
        let tools: Vec<ToolDefinition> = vec![];
        let gemini_tools = GeminiLlmDriver::convert_tools(&tools);
        assert!(gemini_tools.is_none());
    }

    #[test]
    fn test_is_gemini_request_too_large_413() {
        assert!(is_gemini_request_too_large(
            reqwest::StatusCode::PAYLOAD_TOO_LARGE,
            "Request too large"
        ));
    }

    #[test]
    fn test_is_gemini_request_too_large_payload_size() {
        assert!(is_gemini_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            "Request payload size exceeds the limit"
        ));
    }

    #[test]
    fn test_is_gemini_request_too_large_false_for_auth() {
        assert!(!is_gemini_request_too_large(
            reqwest::StatusCode::UNAUTHORIZED,
            "Invalid API key"
        ));
    }

    #[test]
    fn test_extract_sse_event() {
        let mut buffer = "data: {\"candidates\": []}\n\n".to_string();
        let event = extract_sse_event(&mut buffer);
        assert_eq!(event, Some("{\"candidates\": []}".to_string()));
    }

    #[test]
    fn test_extract_sse_event_incomplete() {
        let mut buffer = "data: {\"cand".to_string();
        let event = extract_sse_event(&mut buffer);
        assert!(event.is_none());
    }

    #[test]
    fn test_stream_url() {
        let driver = GeminiLlmDriver::new("test-key");
        let url = driver.stream_url("gemini-2.5-pro");
        assert!(url.contains("gemini-2.5-pro:streamGenerateContent"));
        assert!(url.contains("alt=sse"));
        assert!(url.contains("key=test-key"));
    }

    #[test]
    fn test_convert_messages_tool_result() {
        let messages = vec![LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("{\"temp\": 20}".to_string()),
            tool_calls: None,
            tool_call_id: Some("get_weather".to_string()),
            phase: None,
            thinking: None,
            thinking_signature: None,
        }];

        let (_, contents) = GeminiLlmDriver::convert_messages(&messages);

        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].role.as_deref(), Some("user"));
    }

    // ========================================================================
    // Model-not-found detection tests
    // ========================================================================

    #[test]
    fn test_is_gemini_model_not_found_404_not_found() {
        let error = r#"{"error":{"code":404,"message":"models/gemini-nonexistent is not found","status":"NOT_FOUND"}}"#;
        assert!(is_gemini_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_gemini_model_not_found_model_keyword() {
        let error = r#"{"error":{"code":404,"message":"Model does not exist"}}"#;
        assert!(is_gemini_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_gemini_model_not_found_false_for_non_404() {
        let error = r#"{"error":{"code":400,"status":"NOT_FOUND"}}"#;
        assert!(!is_gemini_model_not_found(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }
}
