use async_trait::async_trait;
use chrono::DateTime;
use futures::StreamExt;

use super::*;

#[async_trait]
impl ChatDriver for AnthropicChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Note: OTel instrumentation is handled via event listeners.
        // ReasonAtom emits llm.generation events, and OtelEventListener
        // creates gen-ai spans from those events.
        let prompt_cache_enabled = config.prompt_cache.as_ref().is_some_and(|cfg| cfg.enabled);
        let (wire_model, wants_million_context) = split_million_context(&config.model);
        crate::prefill::reject_trailing_assistant(wire_model, &messages)?;
        let profile = everruns_provider::get_model_profile(
            &everruns_provider::DriverId::Anthropic,
            wire_model,
        );
        let requested_server_compaction = config
            .driver_options
            .get(SERVER_COMPACTION_OPTION)
            .and_then(|option| option.get("trigger_tokens"))
            .and_then(Value::as_u64)
            .and_then(|tokens| usize::try_from(tokens).ok());
        let wants_server_compaction = requested_server_compaction.is_some()
            && endpoint.base_url() == Some(DEFAULT_BASE_URL)
            && !server_compaction::is_rejected(endpoint, &config.model)
            && profile
                .as_ref()
                .is_some_and(|profile| profile.supports_server_compaction);
        let mut prefix_messages = match &config.provider_opaque_context {
            Some(ProviderOpaqueContext::AnthropicMessagesPrefix { messages_json }) => {
                serde_json::from_str::<Vec<Value>>(messages_json).map_err(|error| {
                    AgentLoopError::store(format!(
                        "invalid Anthropic messages-prefix checkpoint: {error}"
                    ))
                })?
            }
            _ => Vec::new(),
        };
        let message_cache_enabled = prompt_cache_enabled && !wants_server_compaction;
        let (system_prompt, converted_messages) = Self::convert_messages_with_options(
            &layout::keep_later_system_messages_in_place(&messages, &config.model),
            message_cache_enabled,
            config.volatile_suffix_len,
            !prefix_messages.is_empty(),
        );
        let wants_clear_at = converted_messages
            .iter()
            .any(|message| message.clear_at.is_some());
        for message in converted_messages {
            prefix_messages.push(serde_json::to_value(message).map_err(|error| {
                AgentLoopError::llm(format!("failed to serialize Anthropic message: {error}"))
            })?);
        }
        let anthropic_messages = prefix_messages;
        let system = Self::system_prompt_for_request(system_prompt, prompt_cache_enabled);

        // `[1m]` model ids (e.g. `claude-opus-4-8[1m]`) are the gateway's
        // large-context twins of the 200K base models. Anthropic's wire `model`
        // field only accepts the bare id; the 1M window is requested via the
        // `context-1m` beta header (added in the retry loop below). Strip the
        // suffix for everything that reasons about the canonical model, and
        // keep the flag for the header.

        // Hosted tool_search (deferred tool loading) is gated on the Anthropic
        // model profile. When a hosted `ToolSearchConfig` is present and the
        // model supports it, defer tool schemas server-side via
        // `tool_search_tool_bm25_20251119` + per-tool `defer_loading`; otherwise
        // send full schemas. The config is provider-agnostic — set by the
        // `claude_tool_search` / `auto_tool_search` capability — and reaches here
        // on `config.tool_search`.
        let supports_tool_search = profile.as_ref().is_some_and(|p| p.tool_search);
        let tools = if config.tools.is_empty() {
            None
        } else if let Some(ref ts_config) = config.tool_search {
            if ts_config.enabled && supports_tool_search {
                Some(Self::convert_tools_with_search(
                    &config.tools,
                    ts_config.threshold,
                    prompt_cache_enabled,
                ))
            } else {
                Some(Self::convert_tools(&config.tools, prompt_cache_enabled))
            }
        } else {
            Some(Self::convert_tools(&config.tools, prompt_cache_enabled))
        };

        // Sampling parameters are removed on Fable 5.x and Opus 5.5/5/4.8/4.7 —
        // sending `temperature` returns 400 ("`temperature` is deprecated for
        // this model"). The model profile's `temperature` flag is the source
        // of truth; drop the parameter for models that reject it.
        let temperature = config.temperature.filter(|_| {
            let supported = profile.as_ref().is_none_or(|p| p.temperature);
            if !supported {
                tracing::warn!(
                    model = %config.model,
                    "AnthropicDriver: dropping temperature — not supported by this model"
                );
            }
            supported
        });

        // Build thinking config from reasoning effort.
        //
        // Recent Claude models (Fable 5.x, Opus 5.5/5/4.8/4.7, and the 4.6 family)
        // use adaptive thinking: `thinking: {type: "adaptive"}` plus
        // `output_config.effort`. On Fable 5.x and Opus 5.5/5/4.8/4.7 the budget-based
        // `thinking: {type: "enabled", budget_tokens}` form is removed and
        // returns 400, so this split is load-bearing, not stylistic.
        let (thinking, output_config) = match crate::effort::resolve(config, wire_model, &profile) {
            Some(effort) if uses_adaptive_thinking(wire_model) => {
                match adaptive_effort_level(effort) {
                    Some(level) => (
                        Some(AnthropicThinking::adaptive(
                            wire_model,
                            matches!(
                                config.reasoning_effort,
                                Some(effort) if effort != ReasoningEffort::None
                            ),
                        )),
                        Some(AnthropicOutputConfig {
                            effort: level.to_string(),
                        }),
                    ),
                    None => (None, None),
                }
            }
            Some(effort) => (AnthropicThinking::enabled_from_effort(effort), None),
            None => (None, None),
        };

        tracing::info!(
            model = %config.model,
            reasoning_effort = ?config.reasoning_effort,
            thinking = ?thinking,
            adaptive_effort = ?output_config.as_ref().map(|c| c.effort.as_str()),
            "AnthropicDriver: building request with thinking config"
        );

        // Caller's cap is the answer budget; thinking room goes on top.
        let max_tokens_from_profile = config.max_tokens.is_none();
        let budget = match thinking {
            Some(AnthropicThinking::Enabled { budget_tokens }) => Some(budget_tokens),
            _ => None,
        };
        let adaptive_effort = output_config.as_ref().map(|c| c.effort.as_str());
        let max_tokens =
            crate::effort::max_tokens(config.max_tokens, profile.as_ref(), budget, adaptive_effort);
        let context_management = if wants_server_compaction {
            let context_window = profile
                .as_ref()
                .and_then(|profile| profile.limits.as_ref())
                .map(|limits| limits.context.max(0) as usize)
                .unwrap_or_default();
            let maximum_trigger = context_window.saturating_sub(max_tokens as usize);
            if maximum_trigger < SERVER_COMPACTION_MIN_TOKENS {
                return Err(AgentLoopError::config(
                    "Anthropic server compaction has no valid trigger below the output budget",
                ));
            }
            let trigger_tokens = requested_server_compaction
                .unwrap_or(SERVER_COMPACTION_MIN_TOKENS)
                .max(SERVER_COMPACTION_MIN_TOKENS)
                .min(maximum_trigger);
            Some(AnthropicContextManagement {
                edits: vec![AnthropicContextEdit::Compact {
                    trigger: AnthropicCompactionTrigger {
                        r#type: "input_tokens",
                        value: trigger_tokens,
                    },
                    pause_after_compaction: false,
                }],
            })
        } else {
            None
        };

        // Budget-based thinking with tools needs the interleaved-thinking beta
        // header; adaptive thinking interleaves automatically (no header).
        let needs_interleaved_thinking =
            matches!(thinking, Some(AnthropicThinking::Enabled { .. })) && tools.is_some();

        // Map the request-level parallel preference (EVE-598) onto Anthropic's
        // `tool_choice.disable_parallel_tool_use`. `tool_choice` is only valid
        // when tools are present, so skip it for tool-less requests.
        let tool_choice = if tools.is_some() {
            AnthropicToolChoice::from_parallel_preference(
                config
                    .resolved_parallel_tool_calls(self.supports_parallel_tool_calls(&config.model)),
            )
        } else {
            None
        };

        // Prompt-cache diagnostics (`cache-diagnosis` beta): opt in per request
        // and, from the second turn on, name the response the API should
        // compare this request against.
        let cache_diagnostics = config
            .cache_diagnostics
            .as_ref()
            .filter(|diagnostics| diagnostics.enabled);
        let diagnostics = cache_diagnostics.map(|diagnostics| AnthropicDiagnosticsRequest {
            previous_message_id: diagnostics.previous_message_id.clone(),
        });
        let wants_cache_diagnostics = diagnostics.is_some();

        let request = AnthropicRequest {
            model: wire_model.to_string(),
            messages: anthropic_messages,
            max_tokens,
            temperature,
            system,
            stream: true,
            tools,
            tool_choice,
            thinking,
            output_config,
            diagnostics,
            context_management,
            cache_control: (wants_server_compaction && prompt_cache_enabled)
                .then(AnthropicCacheControl::ephemeral),
        };
        let request_messages = Arc::new(request.messages.clone());

        // Share the (possibly fallback-mutated) request across reconnect
        // attempts: the classify closure mutates it for the one-shot max_tokens
        // fallback, and reusing the Arc means a reconnect re-sends the corrected
        // request.
        let request = Arc::new(Mutex::new(request));

        // Establish the SSE stream, transparently reconnecting on a transport
        // failure that lands before the first event (the "error decoding
        // response body" flake). Header-phase retries (429/5xx, transient send
        // failures, and the max_tokens fallback) are handled inside the
        // per-attempt send.
        let (event_stream, retry_metadata) =
            connect_sse_with_reconnect(&self.retry_config, "AnthropicDriver", |attempts| {
                self.send_messages_request(
                    endpoint,
                    Arc::clone(&request),
                    SendMessagesOptions {
                        needs_interleaved_thinking,
                        wants_million_context,
                        wants_cache_diagnostics,
                        wants_clear_at,
                        wants_server_compaction,
                        max_tokens_from_profile,
                        model: &config.model,
                        extra_headers: &config.extra_headers,
                    },
                    attempts,
                )
            })
            .await?;
        if wants_server_compaction {
            server_compaction::record_confirmation(endpoint, &config.model);
        }

        let model = config.model.clone();
        let input_tokens = Arc::new(Mutex::new(0u32));
        let output_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let cache_creation_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let current_tool_call = Arc::new(Mutex::new(Option::<ToolCall>::None));
        let current_thinking = Arc::new(Mutex::new(Option::<OpenThinkingBlock>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));
        let response_content = Arc::new(Mutex::new(BTreeMap::<u32, Value>::new()));
        let input_json = Arc::new(Mutex::new(BTreeMap::<u32, String>::new()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));
        let response_id = Arc::new(Mutex::new(Option::<String>::None));
        let response_model = Arc::new(Mutex::new(Option::<String>::None));
        let diagnostics_payload = Arc::new(Mutex::new(Option::<serde_json::Value>::None));
        // Share retry metadata with stream closure (only set if retries occurred)
        let shared_retry_metadata = if retry_metadata.had_retries() {
            Some(Arc::new(retry_metadata))
        } else {
            None
        };

        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let request_messages = Arc::clone(&request_messages);
            let input_tokens = Arc::clone(&input_tokens);
            let output_tokens = Arc::clone(&output_tokens);
            let cache_read_tokens = Arc::clone(&cache_read_tokens);
            let cache_creation_tokens = Arc::clone(&cache_creation_tokens);
            let current_tool_call = Arc::clone(&current_tool_call);
            let current_thinking = Arc::clone(&current_thinking);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            let response_content = Arc::clone(&response_content);
            let input_json = Arc::clone(&input_json);
            let finish_reason = Arc::clone(&finish_reason);
            let response_id = Arc::clone(&response_id);
            let response_model = Arc::clone(&response_model);
            let diagnostics_payload = Arc::clone(&diagnostics_payload);
            let retry_metadata_for_done = shared_retry_metadata.clone();

            async move {
                match result {
                    Ok(event) => {
                        // Anthropic uses different event types
                        match event.event.as_str() {
                            "message_start" => {
                                // Parse response identity/model, usage, and prompt-cache diagnostics.
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicMessageStart>(&event.data)
                                {
                                    if let Some(id) = data.message.id {
                                        // Following requests use this for prompt-cache diagnostics.
                                        *response_id.lock().unwrap() = Some(id);
                                    }
                                    let transformations = &data.message.input_transformations;
                                    layout::log_input_transformations(data.message.model.as_deref(), transformations);
                                    *response_model.lock().unwrap() = data.message.model;
                                    if let Some(diagnostics) =
                                        data.message.diagnostics.or(data.diagnostics)
                                    {
                                        record_cache_diagnostics(
                                            &diagnostics_payload,
                                            diagnostics,
                                        );
                                    }
                                    if let Some(usage) = data.message.usage {
                                        *input_tokens.lock().unwrap() = usage.input_tokens;
                                        if let Some(cache_read) = usage.cache_read_input_tokens {
                                            *cache_read_tokens.lock().unwrap() = Some(cache_read);
                                        }
                                        if let Some(cache_creation) =
                                            usage.cache_creation_input_tokens
                                        {
                                            *cache_creation_tokens.lock().unwrap() =
                                                Some(cache_creation);
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_start" => {
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicContentBlockStart>(&event.data)
                                {
                                    response_content
                                        .lock()
                                        .unwrap()
                                        .insert(data.index, data.content_block.clone());
                                    let Ok(content_block) =
                                        serde_json::from_value(data.content_block)
                                    else {
                                        return Ok(LlmStreamEvent::TextDelta(String::new()));
                                    };
                                    match content_block {
                                        AnthropicContentBlockDelta::ToolUse { id, name } => {
                                            let mut current = current_tool_call.lock().unwrap();
                                            *current = Some(ToolCall {
                                                id,
                                                name,
                                                arguments: json!(""),
                                            });
                                        }
                                        AnthropicContentBlockDelta::Thinking { thinking } => {
                                            // Opens a block; text arrives as
                                            // thinking_delta and the signature
                                            // as signature_delta.
                                            *current_thinking.lock().unwrap() =
                                                Some(OpenThinkingBlock {
                                                    text: thinking,
                                                    ..Default::default()
                                                });
                                        }
                                        AnthropicContentBlockDelta::RedactedThinking { data } => {
                                            *current_thinking.lock().unwrap() =
                                                Some(OpenThinkingBlock {
                                                    redacted_payload: Some(data),
                                                    ..Default::default()
                                                });
                                        }
                                        AnthropicContentBlockDelta::Compaction => {
                                            return Ok(LlmStreamEvent::ProviderCompactionStarted);
                                        }
                                        AnthropicContentBlockDelta::Text { .. }
                                        | AnthropicContentBlockDelta::Unknown => {}
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_delta" => {
                                if let Ok(data) = serde_json::from_str::<
                                    AnthropicContentBlockDeltaEvent,
                                >(&event.data)
                                {
                                    match data.delta {
                                        AnthropicDelta::TextDelta { text } => {
                                            append_raw_block_field(
                                                &response_content,
                                                data.index,
                                                "text",
                                                &text,
                                            );
                                            // EVE-636: do not count deltas as tokens here —
                                            // deltas != tokens, and this took a mutex on every
                                            // token. Authoritative `output_tokens` is set from
                                            // the terminal `message_delta` usage event below.
                                            return Ok(LlmStreamEvent::TextDelta(text));
                                        }
                                        AnthropicDelta::InputJsonDelta { partial_json } => {
                                            input_json
                                                .lock()
                                                .unwrap()
                                                .entry(data.index)
                                                .or_default()
                                                .push_str(&partial_json);
                                            // EVE-636: accumulate tool-input JSON in place via
                                            // push_str (amortized O(total)) instead of
                                            // re-copying + re-boxing into a Value per delta
                                            // (O(n^2)). Parsed once at content_block_stop.
                                            let mut current = current_tool_call.lock().unwrap();
                                            if let Some(ref mut tc) = *current {
                                                append_tool_input_delta(tc, &partial_json);
                                            }
                                            return Ok(LlmStreamEvent::TextDelta(String::new()));
                                        }
                                        AnthropicDelta::ThinkingDelta { thinking } => {
                                            append_raw_block_field(
                                                &response_content,
                                                data.index,
                                                "thinking",
                                                &thinking,
                                            );
                                            let mut open = current_thinking.lock().unwrap();
                                            open.get_or_insert_with(OpenThinkingBlock::default)
                                                .text
                                                .push_str(&thinking);
                                            return Ok(LlmStreamEvent::ReasoningDelta {
                                                delta: thinking,
                                                summary: false,
                                            });
                                        }
                                        AnthropicDelta::SignatureDelta { signature } => {
                                            set_raw_block_field(
                                                &response_content,
                                                data.index,
                                                "signature",
                                                Value::String(signature.clone()),
                                            );
                                            // Signs the block currently open, and
                                            // only that block.
                                            tracing::debug!(
                                                signature_len = signature.len(),
                                                "AnthropicDriver: received signature_delta from API"
                                            );
                                            let mut open = current_thinking.lock().unwrap();
                                            open.get_or_insert_with(OpenThinkingBlock::default)
                                                .signature = Some(signature);
                                            return Ok(LlmStreamEvent::TextDelta(String::new()));
                                        }
                                        AnthropicDelta::CompactionDelta {
                                            content,
                                            encrypted_content,
                                        } => {
                                            if let Some(content) = content {
                                                append_nullable_raw_block_field(
                                                    &response_content,
                                                    data.index,
                                                    "content",
                                                    &content,
                                                );
                                            }
                                            if let Some(encrypted_content) = encrypted_content {
                                                set_raw_block_field(
                                                    &response_content,
                                                    data.index,
                                                    "encrypted_content",
                                                    Value::String(encrypted_content),
                                                );
                                            }
                                            return Ok(LlmStreamEvent::TextDelta(String::new()));
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_stop" => {
                                // Debug: log raw content_block_stop data
                                tracing::debug!(
                                    raw_data = %event.data,
                                    "AnthropicDriver: received content_block_stop event"
                                );

                                // Finalize current tool call if any
                                {
                                    let mut current = current_tool_call.lock().unwrap();
                                    if let Some(mut tc) = current.take() {
                                        // EVE-636: parse the accumulated JSON string exactly once.
                                        finalize_tool_arguments(&mut tc);
                                        accumulated_tool_calls.lock().unwrap().push(tc);
                                    }
                                }

                                let stop = serde_json::from_str::<AnthropicContentBlockStop>(
                                    &event.data,
                                )
                                .ok();
                                let completed = stop.as_ref().and_then(|data| {
                                    let index = data.index;
                                    if let Some(partial_json) =
                                        input_json.lock().unwrap().remove(&index)
                                    {
                                        match serde_json::from_str(&partial_json) {
                                            Ok(input) => set_raw_block_field(
                                                &response_content,
                                                index,
                                                "input",
                                                input,
                                            ),
                                            Err(error) => tracing::warn!(
                                                %error,
                                                index,
                                                "AnthropicDriver: invalid streamed tool input"
                                            ),
                                        }
                                    }
                                    if let Some(content_block) = &data.content_block {
                                        response_content
                                            .lock()
                                            .unwrap()
                                            .insert(index, content_block.clone());
                                    }
                                    response_content
                                        .lock()
                                        .unwrap()
                                        .get(&index)
                                        .cloned()
                                        .and_then(|value| serde_json::from_value(value).ok())
                                });

                                let mut open = current_thinking.lock().unwrap();
                                if let Some(mut block) = open.take() {
                                    match completed {
                                        Some(AnthropicCompletedContentBlock::Thinking {
                                            thinking,
                                            signature,
                                        }) => {
                                            if !thinking.is_empty() {
                                                block.text = thinking;
                                            }
                                            block.signature = Some(signature);
                                        }
                                        Some(AnthropicCompletedContentBlock::RedactedThinking {
                                            data,
                                        }) => {
                                            block.redacted_payload = Some(data);
                                        }
                                        _ => {}
                                    }
                                    // A block without a signature cannot be
                                    // replayed: Anthropic rejects thinking it
                                    // did not sign. Drop it rather than send
                                    // an artifact that will fail verification.
                                    if block.signature.is_none()
                                        && block.redacted_payload.is_none()
                                    {
                                        tracing::warn!(
                                            thinking_len = block.text.len(),
                                            "AnthropicDriver: thinking block closed without a signature; not replayable"
                                        );
                                        return Ok(LlmStreamEvent::TextDelta(String::new()));
                                    }
                                    return Ok(LlmStreamEvent::ReasoningItem(
                                        block.into_reasoning_part(),
                                    ));
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "message_delta" => {
                                // Check for stop_reason and output tokens
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicMessageDelta>(&event.data)
                                {
                                    if let Some(diagnostics) = data.diagnostics {
                                        record_cache_diagnostics(&diagnostics_payload, diagnostics);
                                    }
                                    if let Some(usage) = data.usage {
                                        *output_tokens.lock().unwrap() = usage.output_tokens;
                                        // Cache tokens may also appear in delta
                                        if usage.cache_read_input_tokens.is_some() {
                                            *cache_read_tokens.lock().unwrap() =
                                                usage.cache_read_input_tokens;
                                        }
                                        if usage.cache_creation_input_tokens.is_some() {
                                            *cache_creation_tokens.lock().unwrap() =
                                                usage.cache_creation_input_tokens;
                                        }
                                    }

                                    if let Some(stop_reason) = data.delta.stop_reason {
                                        let normalized = match stop_reason.as_str() {
                                            "max_tokens" => "length",
                                            "tool_use" => "tool_calls",
                                            "refusal" => "refusal",
                                            _ => "stop",
                                        };
                                        *finish_reason.lock().unwrap() =
                                            Some(normalized.to_string());

                                        if stop_reason == "tool_use" {
                                            let tool_calls =
                                                accumulated_tool_calls.lock().unwrap().clone();
                                            if !tool_calls.is_empty() {
                                                return Ok(LlmStreamEvent::ToolCalls(tool_calls));
                                            }
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "message_stop" => {
                                let in_tokens = *input_tokens.lock().unwrap();
                                let out_tokens = *output_tokens.lock().unwrap();
                                let cache_read = *cache_read_tokens.lock().unwrap();
                                let cache_creation = *cache_creation_tokens.lock().unwrap();
                                let content = response_content
                                    .lock()
                                    .unwrap()
                                    .values()
                                    .cloned()
                                    .collect::<Vec<_>>();
                                let checkpoint_candidate = anthropic_checkpoint_candidate(
                                    wants_server_compaction,
                                    request_messages.as_ref(),
                                    &content,
                                )?;

                                Ok(LlmStreamEvent::Done(Box::new({
                                    let mut metadata = LlmCompletionMetadata::default();
                                    metadata.total_tokens = Some(in_tokens + out_tokens);
                                    metadata.prompt_tokens = Some(in_tokens);
                                    metadata.completion_tokens = Some(out_tokens);
                                    metadata.cache_read_tokens = cache_read;
                                    metadata.cache_creation_tokens = cache_creation;
                                    metadata.model = Some(model);
                                    metadata.response_model = response_model.lock().unwrap().clone();
                                    // `None` when no `stop_reason` arrived: kept
                                    // distinct from an explicit stop.
                                    metadata.finish_reason =
                                        finish_reason.lock().unwrap().clone();
                                    metadata.retry_metadata = retry_metadata_for_done
                                        .map(|arc| (*arc).clone());
                                    metadata.response_id = response_id.lock().unwrap().clone();
                                    metadata.cache_diagnostics = diagnostics_payload
                                        .lock()
                                        .unwrap()
                                        .clone();
                                    if !content.is_empty() {
                                        metadata.provider_opaque_content =
                                            Some(ProviderOpaqueContent::new(
                                                "anthropic",
                                                Value::Array(content),
                                            ));
                                    }
                                    metadata.provider_checkpoint_candidate = checkpoint_candidate;
                                    metadata
                                })))
                            }
                            "error" => Ok(LlmStreamEvent::Error(
                                format!("Anthropic stream error: {}", event.data).into(),
                            )),
                            "ping" => {
                                // Keep-alive ping, ignore
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            _ => {
                                // Unknown event type, ignore
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
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

    /// Anthropic maps the preference onto `tool_choice.disable_parallel_tool_use`
    /// for every tool-capable Claude model.
    fn supports_parallel_tool_calls(&self, _model: &str) -> bool {
        true
    }

    fn provider_managed_reduction_option(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
        model: &str,
        budget_tokens: usize,
    ) -> Option<(String, Value)> {
        if endpoint.base_url() != Some(DEFAULT_BASE_URL) {
            return None;
        }
        if server_compaction::is_rejected(endpoint, model) {
            return None;
        }
        let (wire_model, _) = split_million_context(model);
        let profile = everruns_provider::get_model_profile(
            &everruns_provider::DriverId::Anthropic,
            wire_model,
        )?;
        if !profile.supports_server_compaction {
            return None;
        }
        let limits = profile.limits?;
        let maximum_trigger =
            (limits.context.max(0) as usize).saturating_sub(limits.output.max(0) as usize);
        if maximum_trigger < SERVER_COMPACTION_MIN_TOKENS {
            return None;
        }
        let trigger_tokens = budget_tokens
            .max(SERVER_COMPACTION_MIN_TOKENS)
            .min(maximum_trigger);
        Some((
            SERVER_COMPACTION_OPTION.to_string(),
            json!({ "trigger_tokens": trigger_tokens }),
        ))
    }

    fn provider_managed_reduction_fallback_reason(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
        config: &LlmCallConfig,
    ) -> Option<&'static str> {
        if endpoint.base_url() != Some(DEFAULT_BASE_URL)
            || server_compaction::is_rejected(endpoint, &config.model)
        {
            return None;
        }
        let (wire_model, _) = split_million_context(&config.model);
        let profile = everruns_provider::get_model_profile(
            &everruns_provider::DriverId::Anthropic,
            wire_model,
        )?;
        if !profile.supports_server_compaction {
            return None;
        }
        let context_window = profile.limits.as_ref()?.context.max(0) as usize;
        let effort = crate::effort::resolve(config, wire_model, &Some(profile.clone()));
        let (thinking_budget, adaptive_effort) = match effort {
            Some(effort) if uses_adaptive_thinking(wire_model) => {
                (None, adaptive_effort_level(effort))
            }
            Some(effort) => {
                let budget = match AnthropicThinking::enabled_from_effort(effort) {
                    Some(AnthropicThinking::Enabled { budget_tokens }) => Some(budget_tokens),
                    _ => None,
                };
                (budget, None)
            }
            None => (None, None),
        };
        let max_tokens = crate::effort::max_tokens(
            config.max_tokens,
            Some(&profile),
            thinking_budget,
            adaptive_effort,
        );
        (context_window.saturating_sub(max_tokens as usize) < SERVER_COMPACTION_MIN_TOKENS)
            .then_some("configured_output_budget")
    }

    fn validate_provider_opaque_context(&self, context: &ProviderOpaqueContext) -> bool {
        match context {
            ProviderOpaqueContext::AnthropicMessagesPrefix { messages_json } => {
                serde_json::from_str::<Vec<Value>>(messages_json).is_ok()
            }
            ProviderOpaqueContext::OpenResponsesCompact { .. } => true,
        }
    }

    async fn list_models(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs (proxies, self-hosted)
        if endpoint.base_url() != Some(DEFAULT_BASE_URL) {
            return Ok(None);
        }

        let url = endpoint
            .url("models")
            .ok_or_else(|| AgentLoopError::config("Anthropic provider has no base URL"))?;
        let resolved = endpoint.resolve("GET", url, &[]).await?;
        let mut request = self
            .client()
            .get(&resolved.url)
            .header("anthropic-version", ANTHROPIC_VERSION);
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
            // Classified at the boundary: a rejected key is not an outage.
            return Err(AgentLoopError::llm_http(
                status.as_u16(),
                &body,
                format!("Models API returned {}: {}", status, body),
            ));
        }

        let models_response: AnthropicModelsResponse = response
            .json()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to parse models response: {}", e)))?;

        // All Anthropic models are chat models, no filtering needed
        let discovered: Vec<DiscoveredModel> = models_response
            .data
            .into_iter()
            .map(|m| {
                let profile = Some(m.to_discovered_profile());
                DiscoveredModel {
                    capabilities: vec!["chat".to_string()],
                    model_id: m.id,
                    display_name: Some(m.display_name),
                    created_at: m
                        .created_at
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc)),
                    owned_by: Some("anthropic".to_string()),
                    discovered_profile: profile,
                }
            })
            .collect();

        Ok(Some(discovered))
    }
}
