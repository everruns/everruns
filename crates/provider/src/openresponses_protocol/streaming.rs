//! Streaming-event handling and tool-call accumulation.

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

use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use crate::driver_registry::{LlmCompletionMetadata, LlmStreamEvent, disjoint_prompt_tokens};
use crate::error::{AgentLoopError, Result};
use crate::llm_retry::RetryMetadata;
use crate::openresponses_types::{self as types, StreamingEvent};
use crate::tool_types::ToolCall;

/// Accumulator for tool call arguments during streaming
#[derive(Clone, Default)]
pub(crate) struct ToolCallAccumulator {
    /// Item ID in the stream
    pub(crate) id: String,
    /// Unique call ID for the function call
    pub(crate) call_id: String,
    /// Function name
    pub(crate) name: String,
    /// Accumulated JSON arguments
    pub(crate) arguments: String,
    /// Only terminal items are executable. Announced siblings can still carry
    /// partial arguments when another call finishes first.
    pub(crate) completed: bool,
}

impl ToolCallAccumulator {
    /// The identity and body this entry would be emitted with. Compared, not
    /// shown, so [`ToolCallStream`] can tell a repeat emission from a new one
    /// without requiring `PartialEq` on the public `ToolCall`.
    pub(crate) fn signature(&self) -> (String, String, String) {
        (
            self.call_id.clone(),
            self.name.clone(),
            self.arguments.clone(),
        )
    }
}

/// The tool calls one response has described so far, and what has already been
/// handed to the consumer.
///
/// A gateway is free to carry a call's identity on any of the frames that
/// describe it: `response.output_item.added` names it, the argument deltas
/// stream its body, `response.output_item.done` repeats both in full, and the
/// terminal `response` resource lists it once more. Only `.added` used to be
/// read for the name, so a stream that dropped or mis-shaped that one frame
/// left an entry that [`Self::snapshot`] filtered away: the model called a
/// tool and the agent saw a plain text answer instead, with nothing downstream
/// able to tell that apart from the model choosing to stop.
#[derive(Default)]
pub(crate) struct ToolCallStream {
    pub(crate) calls: Vec<ToolCallAccumulator>,
    /// Signatures of the set handed to the consumer by the last `ToolCalls`
    /// event, so reconciling at completion stays a no-op when the incremental
    /// frames already delivered everything.
    pub(crate) emitted: Vec<(String, String, String)>,
}

impl ToolCallStream {
    /// Append one streamed argument fragment to its call.
    pub(crate) fn observe_arguments_delta(&mut self, item_id: &str, delta: &str) {
        match self.calls.iter_mut().find(|tc| tc.id == item_id) {
            Some(entry) if !entry.completed => entry.arguments.push_str(delta),
            Some(_) => {}
            None => self.calls.push(ToolCallAccumulator {
                id: item_id.to_string(),
                arguments: delta.to_string(),
                ..Default::default()
            }),
        }
    }

    /// Fold a whole `function_call` item into the set.
    ///
    /// Every field is last-writer-wins over the frames that carry it, and an
    /// empty field means "this frame does not carry it" rather than "cleared",
    /// so a later identity-free frame cannot erase a known name.
    pub(crate) fn observe_item(&mut self, id: &str, call_id: &str, name: &str, arguments: &str) {
        // Match on either identifier: the frames do not all carry both, and a
        // gateway that omits `id` entirely would otherwise collapse every
        // parallel call in the response into one entry.
        let existing = self.calls.iter().position(|tc| {
            (!id.is_empty() && tc.id == id) || (!call_id.is_empty() && tc.call_id == call_id)
        });
        let entry = match existing {
            Some(index) => &mut self.calls[index],
            None => {
                self.calls.push(ToolCallAccumulator::default());
                self.calls.last_mut().expect("entry just pushed")
            }
        };
        if !id.is_empty() {
            entry.id = id.to_string();
        }
        if !call_id.is_empty() {
            entry.call_id = call_id.to_string();
        }
        if !name.is_empty() {
            entry.name = name.to_string();
        }
        // Whole-item frames carry the complete argument string, so they replace
        // the streamed fragments rather than appending to them.
        if !arguments.is_empty() {
            entry.arguments = arguments.to_string();
        }
    }

    /// Fold every `function_call` item of a terminal `response` resource in.
    pub(crate) fn observe_response(&mut self, output: &[types::OutputItem]) {
        for item in output {
            if let types::OutputItem::FunctionCall {
                id,
                call_id,
                name,
                arguments,
                ..
            } = item
            {
                self.observe_item(id, call_id, name, arguments);
                self.mark_complete(id, call_id);
            }
        }
    }

    /// The JSON-fallback twin of [`Self::observe_response`].
    pub(crate) fn observe_response_json(&mut self, response: &Value) {
        let Some(output) = response.get("output").and_then(|o| o.as_array()) else {
            return;
        };
        for item in output {
            if item.get("type").and_then(|t| t.as_str()) != Some("function_call") {
                continue;
            }
            let field = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or("");
            self.observe_item(
                field("id"),
                field("call_id"),
                field("name"),
                field("arguments"),
            );
            self.mark_complete(field("id"), field("call_id"));
        }
    }

    pub(crate) fn mark_complete(&mut self, id: &str, call_id: &str) {
        if let Some(entry) = self.calls.iter_mut().find(|tc| {
            (!id.is_empty() && tc.id == id) || (!call_id.is_empty() && tc.call_id == call_id)
        }) {
            entry.completed = true;
        }
    }

    /// The complete tool-call set observed so far, or `None` when it is empty
    /// or identical to the set already emitted.
    ///
    /// Always the full set, never a delta: the engine's stream reader
    /// *overwrites* its tool-call list on every `ToolCalls` event, so an event
    /// carrying only the newest call would drop the earlier ones.
    pub(crate) fn take_unemitted(&mut self) -> Option<Vec<ToolCall>> {
        let signature: Vec<(String, String, String)> = self
            .calls
            .iter()
            .filter(|tc| tc.completed && !tc.name.is_empty())
            .map(ToolCallAccumulator::signature)
            .collect();
        if signature.is_empty() || signature == self.emitted {
            return None;
        }
        self.emitted = signature;
        Some(self.snapshot())
    }

    pub(crate) fn snapshot(&self) -> Vec<ToolCall> {
        self.calls
            .iter()
            .filter(|tc| tc.completed && !tc.name.is_empty())
            .map(|tc| {
                let arguments: Value =
                    serde_json::from_str(&tc.arguments).unwrap_or_else(|error| {
                        // An empty string is the ordinary shape of a no-argument
                        // call. Anything else that fails to parse is a truncated
                        // or corrupt body, and silently substituting `{}` would
                        // run the tool with the wrong inputs, so say so.
                        if !tc.arguments.trim().is_empty() {
                            tracing::warn!(
                                tool = %tc.name,
                                call_id = %tc.call_id,
                                %error,
                                "OpenResponses: unparseable tool-call arguments, \
                                 falling back to empty arguments"
                            );
                        }
                        json!({})
                    });
                ToolCall {
                    id: tc.call_id.clone(),
                    name: tc.name.clone(),
                    arguments,
                }
            })
            .collect()
    }
}

/// Decode a completed native call only after its terminal item is available.
/// This keeps custom-call metadata intact while preserving the stream's normal
/// full-snapshot behavior for synchronous function calls.
pub(crate) fn completed_tool_call_event(
    item: &Value,
    accumulated: &Mutex<ToolCallStream>,
    finish_reason: &Mutex<Option<String>>,
) -> Result<LlmStreamEvent> {
    let mut complete = item.clone();
    if item.get("type").and_then(Value::as_str) == Some("function_call") {
        let acc = accumulated.lock().unwrap();
        if let Some(tc) = acc
            .calls
            .iter()
            .find(|tc| item.get("id").and_then(Value::as_str) == Some(tc.id.as_str()))
        {
            for (field, value) in [
                ("arguments", &tc.arguments),
                ("name", &tc.name),
                ("call_id", &tc.call_id),
            ] {
                if complete.get(field).is_none() {
                    complete[field] = Value::String(value.clone());
                }
            }
        }
    }

    let call: crate::native_async::NativeToolCall = serde_json::from_value(complete)
        .map_err(|_| AgentLoopError::llm("invalid completed tool call"))?;
    *finish_reason.lock().unwrap() = Some("tool_calls".to_string());
    if call.is_async() || matches!(call, crate::native_async::NativeToolCall::Custom { .. }) {
        // Native calls are checkpointed verbatim, so they must be well formed.
        call.validate()?;
        return Ok(LlmStreamEvent::NativeToolCall(call));
    }
    // A synchronous call joins the lenient accumulator below: arguments that
    // are not JSON become `{}` with a warning (see `ToolCallStream::snapshot`)
    // rather than failing the turn over one malformed call.

    let crate::native_async::NativeToolCall::Function {
        call_id,
        name,
        arguments,
        ..
    } = call
    else {
        unreachable!()
    };
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(&call_id)
        .to_string();
    let mut acc = accumulated.lock().unwrap();
    acc.observe_item(&id, &call_id, &name, &arguments);
    acc.mark_complete(&id, &call_id);
    Ok(acc
        .take_unemitted()
        .map(LlmStreamEvent::ToolCalls)
        .unwrap_or_else(|| LlmStreamEvent::TextDelta(String::new())))
}

/// Handle typed streaming events from the OpenResponses API
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_streaming_event(
    event: StreamingEvent,
    input_tokens: &Mutex<u32>,
    output_tokens: &Mutex<u32>,
    cache_read_tokens: &Mutex<Option<u32>>,
    accumulated_tool_calls: &Mutex<ToolCallStream>,
    finish_reason: &Mutex<Option<String>>,
    deferred_events: &Mutex<Vec<LlmStreamEvent>>,
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
            accumulated_tool_calls
                .lock()
                .unwrap()
                .observe_arguments_delta(&item_id, &delta);
            LlmStreamEvent::TextDelta(String::new())
        }

        StreamingEvent::OutputItemAdded { item, .. } => {
            match item {
                Some(types::OutputItem::FunctionCall {
                    id,
                    call_id,
                    name,
                    arguments,
                    ..
                }) => {
                    accumulated_tool_calls
                        .lock()
                        .unwrap()
                        .observe_item(&id, &call_id, &name, &arguments);
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
                }) => {
                    // The done frame describes the finished call in full, so it
                    // is the authoritative record of it; the accumulator only
                    // fills in what streamed earlier.
                    let mut acc = accumulated_tool_calls.lock().unwrap();
                    acc.observe_item(&id, &call_id, &name, &arguments);
                    if let Some(tool_calls) = acc.take_unemitted() {
                        *finish_reason.lock().unwrap() = Some("tool_calls".to_string());
                        return LlmStreamEvent::ToolCalls(tool_calls);
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
            // Reconcile against the response's own output list before ending
            // the stream. Every incremental frame is best-effort: one that is
            // dropped, reordered, or shaped differently by a gateway would
            // otherwise lose the call silently, and the finish reason below is
            // derived from what this driver emitted, so nothing downstream
            // could tell that apart from the model choosing to stop.
            {
                let mut acc = accumulated_tool_calls.lock().unwrap();
                acc.observe_response(&response.output);
                if let Some(tool_calls) = acc.take_unemitted() {
                    *finish_reason.lock().unwrap() = Some("tool_calls".to_string());
                    // The consumer overwrites its tool-call list on each event,
                    // so re-emitting the full set is a no-op when the
                    // incremental frames already delivered it.
                    deferred_events
                        .lock()
                        .unwrap()
                        .push(LlmStreamEvent::ToolCalls(tool_calls));
                }
            }

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
            let written = response
                .usage
                .as_ref()
                .and_then(|u| u.input_tokens_details.as_ref())
                .and_then(|d| d.cache_write_tokens);
            let reasoning_used = response
                .usage
                .as_ref()
                .and_then(|u| u.output_tokens_details.as_ref())
                .map(|d| d.reasoning_tokens);
            let provider_cost_usd = response.usage.as_ref().and_then(|u| u.cost);

            LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                // `input` is OpenAI's cache-inclusive prompt count; normalize to
                // non-cached input (disjoint convention).
                total_tokens: Some(input + output),
                prompt_tokens: Some(
                    disjoint_prompt_tokens(input, cached).saturating_sub(written.unwrap_or(0)),
                ),
                completion_tokens: Some(output),
                cache_read_tokens: cached,
                cache_creation_tokens: written,
                reasoning_tokens: reasoning_used,
                provider_cost_usd,
                model: Some(model),
                response_model: Some(response.model),
                finish_reason: Some(reason),
                retry_metadata: retry_metadata.map(|arc| (*arc).clone()),
                response_id: Some(response.id),
                phase,
                request_body: None,
                cache_diagnostics: None,
                provider_opaque_content: None,
                provider_checkpoint_candidate: None,
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
