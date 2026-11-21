//! Interactions API transport (`POST /v1beta/interactions`).
//!
//! Decision: interactive chat calls on Google's own endpoint go through the
//! Interactions API, Google's recommended interface since June 2026, so
//! Gemini keeps the conversation on its side the way OpenAI Responses does.
//! Every interaction is stored (`store` defaults to true; Google keeps it 55
//! days on paid projects) and its id becomes the response id. The next turn
//! sends `previous_interaction_id` plus only what happened since the model
//! last answered (tool results, new user messages). Thought signatures and
//! the earlier transcript stay on Google's side and are not resent, and
//! Google applies its implicit cache to the stored prefix.
//!
//! `system_instruction`, `tools` and `generation_config` are scoped to one
//! interaction, so they are sent on every request.
//!
//! When the stored interaction is gone (deleted, expired, a free-tier project
//! keeps it one day) Google answers 404, or 400 for an id it cannot parse. The
//! request then goes again once with the whole transcript and no
//! `previous_interaction_id`, so a lost chain costs one request, not the turn.
//!
//! `generateContent` stays for what Interactions does not offer yet: the Batch
//! API, explicit cached content, long decodes past 65,536 output tokens (its
//! `CONTINUATION` checkpoints), and any provider with a custom base URL, which
//! may be a proxy that only speaks `generateContent`.

use super::*;
use crate::gemini::computer_use::{self, GeminiComputer};
use everruns_contracts::native_computer::COMPUTER_TOOL_NAME;
use everruns_contracts::tool_types::ToolCall;
use std::collections::HashSet;

/// One Interactions request body.
#[derive(Debug, Clone, Serialize)]
pub(super) struct InteractionRequest {
    model: String,
    input: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_interaction_id: Option<String>,
    stream: bool,
}

impl GeminiChatDriver {
    /// Whether this call goes through the Interactions API.
    pub(super) fn uses_interactions(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        config: &LlmCallConfig,
    ) -> bool {
        let on_google = self
            .interactions
            .unwrap_or_else(|| endpoint.base_url() == Some(DEFAULT_BASE_URL));
        on_google
            && config.max_tokens.is_none_or(|tokens| tokens <= 65_536)
            && !config
                .prompt_cache
                .as_ref()
                .is_some_and(|cache| cache.enabled && cache.gemini_cached_content.is_some())
    }

    pub(super) fn interaction_request(
        &self,
        messages: &[Message],
        config: &LlmCallConfig,
        stream: bool,
        continue_from: Option<&str>,
    ) -> Result<(InteractionRequest, bool)> {
        // Reuse the generateContent encoder for tools and generation settings,
        // so both transports decide them in one place.
        let (legacy, grounded) = self.prepare_request(messages, config, true)?;
        let computer = GeminiComputer::for_call(config);
        let mut tools = Vec::new();
        for tool in legacy.tools.into_iter().flatten() {
            if tool.google_search.is_some() {
                tools.push(json!({ "type": "google_search" }));
            }
            for function in tool.function_declarations {
                // Gemini's built-in tool stands in for the function.
                if computer.is_some() && function.name == COMPUTER_TOOL_NAME {
                    continue;
                }
                tools.push(json!({
                    "type": "function",
                    "name": function.name,
                    "description": function.description,
                    "parameters": function.parameters,
                }));
            }
        }
        if let Some(computer) = &computer {
            tools.push(computer.tool_entry());
        }
        let generation_config = legacy.generation_config.map(|cfg| {
            let mut out = serde_json::Map::new();
            if let Some(temperature) = cfg.temperature {
                out.insert("temperature".into(), json!(temperature));
            }
            if let Some(tokens) = cfg.max_output_tokens {
                out.insert("max_output_tokens".into(), json!(tokens));
            }
            if let Some(thinking) = cfg.thinking_config {
                // Interactions takes levels for every model; Gemini 2.x
                // budgets map onto the nearest level.
                let level = thinking.thinking_level.unwrap_or(
                    match thinking.thinking_budget.unwrap_or(0) {
                        0..=1_024 => "low",
                        1_025..=8_192 => "medium",
                        _ => "high",
                    },
                );
                out.insert("thinking_level".into(), json!(level));
                out.insert("thinking_summaries".into(), json!("auto"));
            }
            Value::Object(out)
        });
        let delta = match continue_from {
            // The stored interaction holds everything up to the model's last
            // answer; send only what came after it.
            Some(_) => {
                let last_answer = messages
                    .iter()
                    .rposition(|message| message.role == MessageRole::Assistant);
                &messages[last_answer.map_or(0, |index| index + 1)..]
            }
            None => messages,
        };
        Ok((
            InteractionRequest {
                model: config.model.clone(),
                input: interaction_steps(delta, messages, computer.is_some()),
                system_instruction: fold_system_messages(messages),
                tools,
                generation_config,
                previous_interaction_id: continue_from.map(str::to_string),
                stream,
            },
            grounded,
        ))
    }

    pub(super) async fn interaction_response(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        messages: &[Message],
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let stream = !config.prefer_non_streaming || config.limits.needs_incremental_events();
        let url = endpoint
            .url("interactions")
            .ok_or_else(|| AgentLoopError::config("Gemini provider has no base URL"))?;
        let continue_from = config
            .previous_response_id
            .as_deref()
            .filter(|id| !id.is_empty());
        let (request, grounded) =
            self.interaction_request(messages, config, stream, continue_from)?;
        let (request, response, retries) = match self
            .send_interaction(endpoint, config, &url, &request)
            .await
        {
            Ok((response, retries)) => (request, response, retries),
            Err(error) if continue_from.is_some() && lost_interaction(&error) => {
                tracing::info!(
                    model = %config.model,
                    "GeminiDriver: stored interaction unavailable, resending the full transcript"
                );
                let (request, _) = self.interaction_request(messages, config, stream, None)?;
                let (response, retries) = self
                    .send_interaction(endpoint, config, &url, &request)
                    .await?;
                (request, response, retries)
            }
            Err(error) => return Err(error),
        };
        let mut state = InteractionState {
            model: config.model.clone(),
            grounded,
            computer: GeminiComputer::for_call(config),
            declared: computer_use::declared_functions(&request.tools),
            retry_metadata: retries.had_retries().then_some(retries),
            ..Default::default()
        };
        let captured = config
            .capture_request
            .then(|| serde_json::to_value(&request).ok())
            .flatten();
        if !stream {
            let interaction: Value = response.json().await.map_err(|error| {
                AgentLoopError::llm_kind(
                    LlmErrorKind::MalformedResponse,
                    format!("Invalid Gemini interaction: {error}"),
                )
            })?;
            for step in interaction
                .get("steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                state.step(step);
            }
            state.interaction(&interaction);
            state.finish();
            return Ok(driver_helpers::attach_request_body(
                Box::pin(futures::stream::iter(state.pending.into_iter().map(Ok))),
                captured,
            ));
        }
        let events = response.bytes_stream().eventsource();
        Ok(driver_helpers::attach_request_body(
            Box::pin(futures::stream::unfold(
                (events, state),
                |(mut events, mut state)| async move {
                    loop {
                        if let Some(event) = state.pending.pop_front() {
                            return Some((Ok(event), (events, state)));
                        }
                        if state.done {
                            return None;
                        }
                        match events.next().await {
                            Some(Ok(event)) if event.data == "[DONE]" => state.finish(),
                            Some(Ok(event)) => match serde_json::from_str::<Value>(&event.data) {
                                Ok(data) => state.event(&data),
                                Err(error) => tracing::debug!(
                                    %error,
                                    "GeminiDriver: failed to parse interaction event"
                                ),
                            },
                            Some(Err(error)) => {
                                state.fail(AgentLoopError::llm(format!("Stream error: {error}")))
                            }
                            None => state.finish(),
                        }
                    }
                },
            )),
            captured,
        ))
    }

    async fn send_interaction(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        config: &LlmCallConfig,
        url: &str,
        request: &InteractionRequest,
    ) -> Result<(reqwest::Response, RetryMetadata)> {
        self.send_generate_content_request(
            endpoint,
            request,
            url,
            &config.model,
            &config.extra_headers,
        )
        .await
    }
}

/// A continuation the provider no longer has (404) or cannot read (400).
/// A missing model reports the same 404, so the stateless retry fails the
/// same way when that was the cause.
fn lost_interaction(error: &AgentLoopError) -> bool {
    matches!(error.http_status(), Some(400 | 404))
        || error
            .to_string()
            .to_ascii_lowercase()
            .contains("not available")
}

/// Interaction steps for `delta`, resolving tool names against the whole
/// transcript (a result's call can sit behind the stored interaction).
///
/// With `computer` on, `computer` calls converted from Gemini actions go back
/// as the action Gemini called, and their results in its shape.
fn interaction_steps<'a>(
    delta: &'a [Message],
    transcript: &'a [Message],
    computer: bool,
) -> Vec<Value> {
    let calls: HashMap<&str, &'a ToolCall> = transcript
        .iter()
        .filter_map(|message| message.tool_calls.as_ref())
        .flatten()
        .map(|call| (call.id.as_str(), call))
        .collect();
    let native = |call: &'a ToolCall| {
        computer
            .then(|| computer_use::original_call(&call.arguments))
            .flatten()
    };
    let mut steps = Vec::new();
    for message in delta {
        match message.role {
            MessageRole::System => {}
            MessageRole::User => {
                let content = content_items(&message.content);
                if !content.is_empty() {
                    steps.push(json!({ "type": "user_input", "content": content }));
                }
            }
            MessageRole::Tool => {
                let Some(call_id) = message.tool_call_id.as_deref() else {
                    continue;
                };
                // THREAT[TM-TOOL-005]: a result whose call was trimmed away
                // has nothing to answer and is rejected by the provider.
                let Some(call) = calls.get(call_id) else {
                    continue;
                };
                let mut result = content_items(&message.content);
                let name = match native(call) {
                    Some((name, _)) => {
                        result = computer_use::result_items(result, &call.arguments);
                        name
                    }
                    None => call.name.as_str(),
                };
                if result.is_empty() {
                    result.push(json!({ "type": "text", "text": "" }));
                }
                steps.push(json!({
                    "type": "function_result",
                    "call_id": call_id,
                    "name": name,
                    "result": result,
                }));
            }
            MessageRole::Assistant => {
                for reasoning in message
                    .reasoning
                    .iter()
                    .filter(|r| r.provider == "google" && r.bound_tool_call_id.is_none())
                {
                    let mut step = json!({
                        "type": "thought",
                        "signature": reasoning.signature.clone().unwrap_or_default(),
                    });
                    if let Some(text) = reasoning.display_text().filter(|t| !t.is_empty()) {
                        step["summary"] = json!([{ "type": "text", "text": text }]);
                    }
                    steps.push(step);
                }
                let content = content_items(&message.content);
                if !content.is_empty() {
                    steps.push(json!({ "type": "model_output", "content": content }));
                }
                for call in message.tool_calls.iter().flatten() {
                    let arguments = if call.arguments.is_object() {
                        call.arguments.clone()
                    } else if let Some(text) = call.arguments.as_str() {
                        serde_json::from_str(text).unwrap_or(json!({}))
                    } else {
                        json!({})
                    };
                    let (name, arguments) = match native(call) {
                        Some((name, original)) => (name, original.clone()),
                        None => (call.name.as_str(), arguments),
                    };
                    let mut step = json!({
                        "type": "function_call",
                        "id": call.id,
                        "name": name,
                        "arguments": arguments,
                    });
                    if let Some(signature) = message
                        .reasoning
                        .iter()
                        .find(|r| {
                            r.provider == "google"
                                && r.bound_tool_call_id.as_deref() == Some(&call.id)
                        })
                        .and_then(|r| r.signature.clone())
                    {
                        step["signature"] = json!(signature);
                    }
                    steps.push(step);
                }
            }
        }
    }
    steps
}

/// Interactions content items: text, images and documents. Audio is
/// described, not sent, as on `generateContent`.
fn content_items(content: &MessageContent) -> Vec<Value> {
    GeminiChatDriver::convert_content(content)
        .into_iter()
        .filter_map(|part| match part {
            GeminiPart::Text { text, .. } => Some(json!({ "type": "text", "text": text })),
            GeminiPart::InlineData { inline_data } => Some(json!({
                "type": media_type(&inline_data.mime_type),
                "data": inline_data.data,
                "mime_type": inline_data.mime_type,
            })),
            GeminiPart::FileData { file_data } => Some(json!({
                "type": media_type(&file_data.mime_type),
                "uri": file_data.file_uri,
                "mime_type": file_data.mime_type,
            })),
            _ => None,
        })
        .collect()
}

fn media_type(mime: &str) -> &'static str {
    match mime.split('/').next() {
        Some("image") => "image",
        Some("audio") => "audio",
        Some("video") => "video",
        _ => "document",
    }
}

/// Folds interaction steps (whole or streamed) into stream events.
#[derive(Default)]
pub(super) struct InteractionState {
    model: String,
    grounded: bool,
    retry_metadata: Option<RetryMetadata>,
    /// Streamed steps by index: type plus what has arrived so far.
    open: HashMap<u64, Value>,
    calls: StreamToolCallAccumulator,
    call_counter: u32,
    answer: String,
    citations: Vec<everruns_contracts::web_citation::WebCitation>,
    searches: u32,
    interaction_id: Option<String>,
    status: Option<String>,
    usage: Option<Value>,
    pending: std::collections::VecDeque<LlmStreamEvent>,
    done: bool,
    /// Gemini's computer use tool, when this call declared it.
    computer: Option<GeminiComputer>,
    /// Function names the request declared; never rewritten as actions.
    declared: HashSet<String>,
    /// The response's first computer call, naming its batch.
    computer_batch: Option<String>,
}

impl InteractionState {
    /// One SSE event.
    fn event(&mut self, data: &Value) {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        match data.get("event_type").and_then(Value::as_str) {
            Some("interaction.created" | "interaction.completed") => {
                if let Some(interaction) = data.get("interaction") {
                    self.interaction(interaction);
                }
            }
            Some("step.start") => {
                let step = data.get("step").cloned().unwrap_or_else(|| json!({}));
                self.open.insert(index, step);
            }
            Some("step.delta") => {
                let delta = data.get("delta").cloned().unwrap_or_default();
                let step = self.open.entry(index).or_insert_with(|| json!({}));
                match delta.get("type").and_then(Value::as_str) {
                    // Answer text streams straight through.
                    Some("text") if step["type"] == "model_output" => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            self.answer.push_str(text);
                            self.pending
                                .push_back(LlmStreamEvent::TextDelta(text.to_string()));
                        }
                        for annotation in delta
                            .get("annotations")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            let start = self.answer.len()
                                - delta
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .map_or(0, str::len);
                            self.citation(annotation, start);
                        }
                    }
                    Some("thought_signature") => {
                        step["signature"] = delta["signature"].clone();
                    }
                    _ => {
                        // Thought summaries and call arguments arrive whole
                        // or in pieces; keep the latest of each field.
                        if let Some(text) = delta.pointer("/content/text").and_then(Value::as_str) {
                            let summary = step["summary_text"].as_str().unwrap_or("").to_string();
                            step["summary_text"] = json!(summary + text);
                        }
                        if let Some(fields) = delta.as_object() {
                            for (key, value) in fields {
                                if !matches!(key.as_str(), "type" | "content") {
                                    step[key] = value.clone();
                                }
                            }
                        }
                    }
                }
            }
            Some("step.stop") => {
                if let Some(mut step) = self.open.remove(&index) {
                    if step["type"] == "model_output" {
                        return;
                    }
                    if let Some(text) = step["summary_text"].as_str().map(str::to_string) {
                        step["summary"] = json!([{ "type": "text", "text": text }]);
                    }
                    self.step(&step);
                }
            }
            Some("interaction.status_update") => {
                if let Some(status) = data.get("status").and_then(Value::as_str) {
                    self.status = Some(status.to_string());
                }
            }
            Some("error") => {
                let message = data
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Gemini interaction failed");
                self.fail(AgentLoopError::llm(message.to_string()));
            }
            _ => {}
        }
    }

    /// One whole step.
    fn step(&mut self, step: &Value) {
        let signature = step
            .get("signature")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        match step.get("type").and_then(Value::as_str) {
            Some("thought") => {
                let summary: Vec<String> = step
                    .get("summary")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect();
                let text = summary.join("");
                match signature {
                    Some(signature) => {
                        let mut part =
                            ReasoningContentPart::opaque("google").with_signature(signature);
                        if !text.is_empty() {
                            part = part.with_text(ReasoningText::Plain { text });
                        }
                        self.pending.push_back(LlmStreamEvent::ReasoningItem(part));
                    }
                    None if !text.is_empty() => {
                        self.pending.push_back(LlmStreamEvent::ReasoningDelta {
                            delta: text,
                            summary: true,
                        });
                    }
                    None => {}
                }
            }
            Some("model_output") => {
                for item in step
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        let start = self.answer.len();
                        self.answer.push_str(text);
                        self.pending
                            .push_back(LlmStreamEvent::TextDelta(text.to_string()));
                        for annotation in item
                            .get("annotations")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            self.citation(annotation, start);
                        }
                    }
                }
            }
            Some("function_call") => {
                let id = step
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{}", self.call_counter));
                self.call_counter += 1;
                let name = step.get("name").and_then(Value::as_str).unwrap_or_default();
                let arguments = match step.get("arguments") {
                    Some(Value::String(text)) => serde_json::from_str(text).unwrap_or(json!({})),
                    Some(value) if value.is_object() => value.clone(),
                    _ => json!({}),
                };
                let mut call = ToolCall {
                    id: id.clone(),
                    name: name.to_string(),
                    arguments,
                };
                if let Some(computer) = self
                    .computer
                    .filter(|_| GeminiComputer::is_action(name) && !self.declared.contains(name))
                {
                    computer.into_computer_call(&mut call, &mut self.computer_batch);
                }
                self.calls.push_complete(call.id, call.name, call.arguments);
                if let Some(signature) = signature {
                    self.pending.push_back(LlmStreamEvent::ReasoningItem(
                        ReasoningContentPart::opaque("google")
                            .with_signature(signature)
                            .with_bound_tool_call_id(id),
                    ));
                }
            }
            Some("google_search_call") if self.grounded => {
                for query in step
                    .pointer("/arguments/queries")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|query| !query.trim().is_empty())
                {
                    self.searches = self.searches.saturating_add(1);
                    self.pending.push_back(LlmStreamEvent::HostedToolCall(
                        everruns_contracts::driver_registry::HostedToolCall {
                            id: format!("google_search_{}", self.searches),
                            tool: everruns_contracts::native_web_search::WEB_SEARCH_FUNCTION_TOOL
                                .to_string(),
                            status:
                                everruns_contracts::driver_registry::HostedToolCallStatus::Completed,
                            summary: Some(query.chars().take(200).collect()),
                        },
                    ));
                }
            }
            _ => {}
        }
    }

    /// A `url_citation` annotation on answer text that starts at byte `start`.
    fn citation(&mut self, annotation: &Value, start: usize) {
        let Some(url) = annotation
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.trim().is_empty())
        else {
            return;
        };
        let range = |key: &str| {
            annotation
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
        };
        let span = match (range("start_index"), range("end_index")) {
            (Some(from), Some(to)) => everruns_contracts::web_citation::CitedSpan::from_bytes(
                &self.answer,
                start + from,
                start + to,
            ),
            _ => None,
        };
        self.citations
            .push(everruns_contracts::web_citation::WebCitation {
                url: url.to_string(),
                title: annotation
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .map(str::to_string),
                snippet: None,
                span,
            });
    }

    /// The interaction resource: id, status, usage.
    fn interaction(&mut self, interaction: &Value) {
        if let Some(id) = interaction.get("id").and_then(Value::as_str) {
            self.interaction_id = Some(id.to_string());
        }
        if let Some(status) = interaction.get("status").and_then(Value::as_str) {
            self.status = Some(status.to_string());
        }
        if let Some(usage) = interaction.get("usage") {
            self.usage = Some(usage.clone());
        }
        if self.status.as_deref() == Some("failed") {
            let message = interaction
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Gemini interaction failed");
            self.fail(AgentLoopError::llm(message.to_string()));
        }
    }

    fn finish(&mut self) {
        if self.done {
            return;
        }
        let calls = self.calls.take_finalized();
        let status = self.status.clone();
        let finish_reason = match status.as_deref() {
            Some("completed" | "requires_action") => Some("stop"),
            Some("incomplete") => Some("length"),
            Some("cancelled") => Some("cancelled"),
            _ => None,
        };
        let mut dropped = 0;
        if matches!(finish_reason, Some("stop") | None) && !calls.is_empty() {
            self.pending.push_back(LlmStreamEvent::ToolCalls(calls));
        } else {
            dropped = u32::try_from(calls.len()).unwrap_or(u32::MAX);
        }
        let count = |key: &str| {
            self.usage
                .as_ref()
                .and_then(|usage| usage.get(key))
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
        };
        let input = count("total_input_tokens")
            .unwrap_or(0)
            .saturating_add(count("total_tool_use_tokens").unwrap_or(0));
        let thoughts = count("total_thought_tokens");
        let output = count("total_output_tokens")
            .unwrap_or(0)
            .saturating_add(thoughts.unwrap_or(0));
        let cached = count("total_cached_tokens").filter(|&tokens| tokens > 0);
        let mut metadata = LlmCompletionMetadata::default();
        metadata.total_tokens = Some(input.saturating_add(output));
        metadata.prompt_tokens = Some(disjoint_prompt_tokens(input, cached));
        metadata.completion_tokens = Some(output);
        metadata.cache_read_tokens = cached;
        metadata.reasoning_tokens = thoughts;
        metadata.model = Some(self.model.clone());
        metadata.finish_reason = finish_reason.map(str::to_string);
        metadata.provider_finish_reason = status.clone();
        metadata.tool_calls_dropped = dropped;
        everruns_contracts::llm_telemetry::warn_tool_calls_dropped(
            "gemini",
            &self.model,
            dropped,
            status.as_deref().unwrap_or("none"),
        );
        metadata.retry_metadata = self.retry_metadata.take();
        metadata.response_id = self.interaction_id.take();
        metadata.web_citations = std::mem::take(&mut self.citations);
        if self.searches > 0 {
            metadata
                .hosted_tool_calls
                .insert("web_search_call".to_string(), self.searches);
        }
        self.pending
            .push_back(LlmStreamEvent::Done(Box::new(metadata)));
        self.done = true;
    }

    fn fail(&mut self, error: AgentLoopError) {
        if self.done {
            return;
        }
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

#[cfg(test)]
#[path = "interactions_tests.rs"]
mod tests;
