//! The `ChatDriver` implementation for the OpenResponses wire protocol.

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
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::sync::{Arc, Mutex};

use crate::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream, LlmStreamEvent, Message,
    disjoint_prompt_tokens,
};
use crate::error::{AgentLoopError, Result};
use crate::openresponses_types::StreamingEvent;
use crate::stream_reconnect::connect_sse_with_reconnect;

use super::*;

#[async_trait]
impl ChatDriver for OpenResponsesProtocolChatDriver {
    fn supports_stateful_responses(&self) -> bool {
        self.stateful_responses.unwrap_or(false)
    }

    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        crate::openai_compat::validate_config(config)?;
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

        // Explicit breakpoints move instructions into input. Replaying the full
        // transcript avoids appending that developer prefix again on every
        // previous_response_id continuation (instructions normally do not persist).
        if self.native_prompt_cache_options
            && crate::openai_compat::supports_cache_options(&config.model)
            && config.prompt_cache.as_ref().is_some_and(|cache| {
                cache.enabled
                    && cache.strategy == crate::driver_registry::PromptCacheStrategy::Explicit
            })
        {
            previous_response_id = None;
        }

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
        // their encrypted payload, and it only does so on request. Stateful
        // continuations already retain that state server-side; Meta rejects
        // this include when paired with `previous_response_id`.
        let include = previous_response_id
            .is_none()
            .then_some(reasoning.is_some() || update_state.is_some())
            .filter(|include| *include)
            .map(|_| vec!["reasoning.encrypted_content".to_string()]);

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
        apply_cache_options(&mut request_body, config, self.native_prompt_cache_options);
        crate::openai_compat::validate_body(&request_body, endpoint, true)?;
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
                    && self
                        .request_extension
                        .as_ref()
                        .is_none_or(|extension| extension.allow_stateless_recovery())
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
                apply_cache_options(&mut request_body, config, self.native_prompt_cache_options);
                crate::openai_compat::validate_body(&request_body, endpoint, true)?;
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
        let accumulated_tool_calls = Arc::new(Mutex::new(ToolCallStream::default()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));
        // Events a single SSE frame needs to emit *before* the one it maps to.
        // Only the terminal frame uses it: reconciling the response's own
        // function-call list has to reach the consumer ahead of `Done`, which
        // ends the stream for it.
        let deferred_events = Arc::new(Mutex::new(Vec::<LlmStreamEvent>::new()));
        // Share retry metadata with stream closure (only set if retries occurred)
        let shared_retry_metadata = if retry_metadata.had_retries() {
            Some(Arc::new(retry_metadata))
        } else {
            None
        };

        let frame_deferred_events = Arc::clone(&deferred_events);
        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let input_tokens = Arc::clone(&input_tokens);
            let output_tokens = Arc::clone(&output_tokens);
            let cache_read_tokens = Arc::clone(&cache_read_tokens);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            let finish_reason = Arc::clone(&finish_reason);
            let deferred_events = Arc::clone(&frame_deferred_events);
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

                        // A custom tool call is not represented by the typed
                        // Responses event enum. Decode completed calls before
                        // that enum so its native metadata reaches the runtime.
                        if let Ok(json) = serde_json::from_str::<Value>(event_data)
                            && json.get("type").and_then(Value::as_str)
                                == Some("response.output_item.done")
                            && let Some(item) = json.get("item")
                            && matches!(
                                item.get("type").and_then(Value::as_str),
                                Some("function_call" | "custom_tool_call")
                            )
                        {
                            return completed_tool_call_event(
                                item,
                                &accumulated_tool_calls,
                                &finish_reason,
                            );
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
                                &deferred_events,
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
                                            accumulated_tool_calls
                                                .lock()
                                                .unwrap()
                                                .observe_arguments_delta(item_id, delta);
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
                                            let field = |key: &str| {
                                                item.get(key).and_then(|v| v.as_str()).unwrap_or("")
                                            };
                                            accumulated_tool_calls.lock().unwrap().observe_item(
                                                field("id"),
                                                field("call_id"),
                                                field("name"),
                                                field("arguments"),
                                            );
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
                                            // The done frame describes the
                                            // finished call in full, so it is the
                                            // authoritative record of it; the
                                            // accumulator only fills in what
                                            // streamed earlier.
                                            let field = |key: &str| {
                                                item.get(key).and_then(|v| v.as_str()).unwrap_or("")
                                            };
                                            let mut acc = accumulated_tool_calls.lock().unwrap();
                                            acc.observe_item(
                                                field("id"),
                                                field("call_id"),
                                                field("name"),
                                                field("arguments"),
                                            );
                                            if let Some(tool_calls) = acc.take_unemitted() {
                                                *finish_reason.lock().unwrap() =
                                                    Some("tool_calls".to_string());
                                                return Ok(LlmStreamEvent::ToolCalls(tool_calls));
                                            }
                                        }
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }

                                    Some("response.completed")
                                    | Some("response.incomplete")
                                    | Some("response.done") => {
                                        // Response completed - extract usage
                                        let response_obj = json.get("response").unwrap_or(&json);

                                        // Reconcile against the response's own output list before ending
                                        // the stream. Every incremental frame is best-effort: one that is
                                        // dropped, reordered, or shaped differently by a gateway would
                                        // otherwise lose the call silently, and the finish reason below is
                                        // derived from what this driver emitted, so nothing downstream
                                        // could tell that apart from the model choosing to stop.
                                        {
                                            let mut acc =
                                                accumulated_tool_calls.lock().unwrap();
                                            acc.observe_response_json(response_obj);
                                            if let Some(tool_calls) = acc.take_unemitted() {
                                                *finish_reason.lock().unwrap() =
                                                    Some("tool_calls".to_string());
                                                deferred_events
                                                    .lock()
                                                    .unwrap()
                                                    .push(LlmStreamEvent::ToolCalls(tool_calls));
                                            }
                                        }

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
                                        let written = response_obj.pointer("/usage/input_tokens_details/cache_write_tokens")
                                            .and_then(Value::as_u64).map(|n| n.min(u32::MAX as u64) as u32);
                                        let reasoning_used = response_obj.pointer("/usage/output_tokens_details/reasoning_tokens")
                                            .and_then(Value::as_u64).map(|n| n.min(u32::MAX as u64) as u32);

                                        Ok(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                                            // `input` is OpenAI's cache-inclusive prompt count;
                                            // normalize to non-cached input (disjoint convention).
                                            total_tokens: Some(input + output),
                                            prompt_tokens: Some(disjoint_prompt_tokens(input, cached).saturating_sub(written.unwrap_or(0))),
                                            completion_tokens: Some(output),
                                            cache_read_tokens: cached,
                                            cache_creation_tokens: written,
                                            reasoning_tokens: reasoning_used,
                                            provider_cost_usd,
                                            model: Some(model),
                                            response_model: response_obj
                                                .get("model")
                                                .and_then(Value::as_str)
                                                .map(str::to_owned),
                                            finish_reason: Some(reason),
                                            retry_metadata: retry_metadata_for_done
                                                .map(|arc| (*arc).clone()),
                                            response_id: response_obj
                                                .get("id")
                                                .and_then(Value::as_str)
                                                .map(str::to_owned),
                                            phase,
                                            request_body: None,
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

        // Flush whatever the frame queued ahead of the event it mapped to. The
        // per-frame closure is 1:1 by construction, so this is the only place a
        // frame can widen into several events.
        let converted_stream: LlmResponseStream =
            Box::pin(converted_stream.flat_map(move |item| {
                let queued = std::mem::take(&mut *deferred_events.lock().unwrap());
                let mut batch: Vec<Result<LlmStreamEvent>> = queued.into_iter().map(Ok).collect();
                batch.push(item);
                futures::stream::iter(batch)
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
