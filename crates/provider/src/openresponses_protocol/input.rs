//! Input-item assembly: delta computation, orphan repair, and cache options.

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
use std::collections::HashSet;

use crate::driver_registry::LlmCallConfig;
use crate::error::{AgentLoopError, LlmErrorKind};

use super::*;

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
pub(crate) fn configuration_update_item(
    effort: crate::model::ReasoningEffort,
) -> ResponsesInputItem {
    ResponsesInputItem::ConfigurationUpdate {
        r#type: "configuration_update".into(),
        reasoning: crate::compact::ConfigurationReasoning { effort },
    }
}

pub(crate) fn coalesce_configuration_updates(
    items: Vec<ResponsesInputItem>,
) -> Vec<ResponsesInputItem> {
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

pub(crate) fn compute_delta_input_items(items: Vec<ResponsesInputItem>) -> Vec<ResponsesInputItem> {
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
pub(crate) fn finalize_input_for_request(
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
pub(crate) fn unpaired_function_call_ids(items: &[ResponsesInputItem]) -> Vec<String> {
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
pub(crate) fn repair_unpaired_function_call_items(
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

/// Breakpoints belong on content blocks, never the top-level instructions string.
/// Apply after serialization so ordinary input and compact-item replay stay lossless.
pub(crate) fn apply_cache_options(body: &mut Value, config: &LlmCallConfig, native_openai: bool) {
    if !native_openai || !crate::openai_compat::supports_cache_options(&config.model) {
        return;
    }
    let Some(cache) = config.prompt_cache.as_ref().filter(|c| c.enabled) else {
        return;
    };
    let explicit = cache.strategy == crate::driver_registry::PromptCacheStrategy::Explicit;
    body["prompt_cache_options"] =
        json!({"ttl": "30m", "mode": if explicit { "explicit" } else { "implicit" }});
    if explicit
        && let Some(instructions) = body
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::to_owned)
    {
        body.as_object_mut().unwrap().remove("instructions");
        body["input"].as_array_mut().unwrap().insert(
            0,
            json!({
                "type": "message", "role": "developer", "content": [{
                    "type": "input_text", "text": instructions,
                    "prompt_cache_breakpoint": {"mode": "explicit"}
                }]
            }),
        );
    }
}

pub(crate) fn is_missing_tool_output_continuation_error(error: &AgentLoopError) -> bool {
    if !matches!(error.llm_error_kind(), Some(LlmErrorKind::InvalidRequest)) {
        return false;
    }
    let message = error.to_string().to_ascii_lowercase();
    message.contains("no tool output found for function call")
        || message.contains("no tool call found for function call output")
        || message.contains("previous_response_not_found")
        || (message.contains("previous response") && message.contains("not found"))
}
