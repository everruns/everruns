//! Native checkpoint installation and its separately accounted provider call.

#[cfg(all(test, feature = "builtins"))]
#[path = "native_usage_tests.rs"]
mod native_usage_tests;

use super::materially_reduced;
use crate::engine::compact::{CompactRequest, messages_to_compact_input};
use crate::engine::driver_registry::{Message, MessageRole};
use crate::engine::error::{AgentLoopError, Result};
use crate::engine::event_emitter::EventEmitter;
use crate::engine::events::{
    EventContext, EventRequest, LlmGenerationData, LlmRequestOptions, TokenUsage,
};
use crate::engine::typed_id::SessionId;
use std::sync::Arc;
use uuid::Uuid;
use web_time::Instant;

#[derive(Debug)]
pub(super) struct AppliedNativeCompaction {
    pub(super) checkpoint_id: Option<String>,
    pub(super) output_items_after: usize,
    pub(super) tokens_before: Option<u64>,
    pub(super) tokens_after: Option<u64>,
    pub(super) bytes_before: Option<u64>,
    pub(super) bytes_after: Option<u64>,
    pub(super) duration_ms: u64,
    /// Provider-reported cost of the compaction call, when reported (EVE-895).
    pub(super) cost_usd: Option<f64>,
    pub(super) estimated_cost_usd: Option<f64>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn try_apply_native_compaction(
    chat_driver: &dyn crate::engine::ChatDriver,
    compaction_policy: &dyn crate::engine::compaction_policy::CompactionPolicy,
    checkpoint_store: Option<&Arc<dyn crate::engine::CompactionCheckpointStore>>,
    session_id: SessionId,
    source_sequence: Option<i64>,
    provider_type: &str,
    provider_id: &str,
    event_emitter: &dyn EventEmitter,
    event_context: &EventContext,
    model: &str,
    system_prompt: Option<&str>,
    stateful_response_continuation: bool,
    llm_messages: &mut Vec<Message>,
    llm_config: &mut crate::engine::driver_registry::LlmCallConfig,
) -> Result<Option<AppliedNativeCompaction>> {
    if !chat_driver.supports_compact() {
        return Ok(None);
    }

    let started = Instant::now();
    let has_system_prompt = system_prompt.is_some();
    let messages_to_compact = if has_system_prompt {
        &llm_messages[1..]
    } else {
        &llm_messages[..]
    };
    let (mut standalone_input, has_prior_opaque_context) =
        match llm_config.provider_opaque_context.as_ref() {
            Some(crate::engine::ProviderOpaqueContext::OpenResponsesCompact { output, .. }) => (
                output
                    .iter()
                    .map(crate::engine::CompactInputItem::from)
                    .collect(),
                true,
            ),
            Some(_) | None => (Vec::new(), false),
        };
    if llm_config.reasoning_state.is_some()
        && let Some(crate::engine::ProviderOpaqueContext::OpenResponsesCompact {
            reasoning_state,
            ..
        }) = &llm_config.provider_opaque_context
        && let Some(effort) = reasoning_state.as_ref().and_then(|state| state.effective)
    {
        standalone_input.push(crate::engine::CompactInputItem::ConfigurationUpdate {
            reasoning: everruns_contracts::compact::ConfigurationReasoning { effort },
        });
    }
    let prefix_len = standalone_input.len();
    standalone_input.extend(messages_to_compact_input(messages_to_compact));
    if let Some(effort) = llm_config
        .reasoning_state
        .as_ref()
        .and_then(|state| state.pending)
    {
        // The pending update belongs before fresh input, just as in normal
        // generation. Historical updates already travel with their messages.
        let boundary = prefix_len
            + standalone_input[prefix_len..]
                .iter()
                .rposition(crate::engine::CompactInputItem::is_assistant_item)
                .map_or(0, |index| index + 1);
        standalone_input.insert(
            boundary,
            crate::engine::CompactInputItem::ConfigurationUpdate {
                reasoning: everruns_contracts::compact::ConfigurationReasoning { effort },
            },
        );
    }
    // Coalesce only adjacent updates (possible at an empty checkpoint suffix).
    // Never move updates across a semantic item.
    let mut input: Vec<crate::engine::CompactInputItem> = Vec::new();
    for item in standalone_input {
        if matches!(
            item,
            crate::engine::CompactInputItem::ConfigurationUpdate { .. }
        ) && matches!(
            input.last(),
            Some(crate::engine::CompactInputItem::ConfigurationUpdate { .. })
        ) {
            input.pop();
        }
        input.push(item);
    }
    let standalone_input = input;
    let local_tokens_before = (!stateful_response_continuation && !has_prior_opaque_context)
        .then(|| compaction_policy.estimate_total_tokens(messages_to_compact) as u64);
    let bytes_before = (!stateful_response_continuation
        || has_prior_opaque_context
        || llm_config.reasoning_state.is_some())
    .then(|| {
        serde_json::to_vec(&standalone_input)
            .ok()
            .map(|value| value.len() as u64)
    })
    .flatten();
    // Reconstruct standalone input even for a stateful continuation. Compacting
    // only the previous response handle would omit the fresh request delta, then
    // clearing that handle for the retry would make the omission permanent.
    let (input, compact_previous_response_id) = (standalone_input, None);

    let compact_response = match chat_driver
        .compact(
            &crate::engine::ProviderEndpoint::default(),
            CompactRequest {
                reasoning_state: llm_config.reasoning_state.clone(),
                model: model.to_string(),
                input,
                previous_response_id: compact_previous_response_id,
                instructions: system_prompt.map(str::to_string),
            },
        )
        .await
    {
        Ok(Some(response)) => response,
        Ok(None) => return Ok(None),
        Err(error) => {
            if everruns_contracts::durable_retry::single_attempt()
                && (error.is_transient_llm_error()
                    || everruns_contracts::durable_retry::is_retryable_start(&error))
            {
                return Err(everruns_contracts::durable_retry::retryable_start(error));
            }
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "ReasonAtom: native compaction failed"
            );
            return Ok(None);
        }
    };

    let tokens_before = compact_response
        .usage
        .as_ref()
        .and_then(|usage| usage.input_tokens)
        .map(u64::from)
        .or(local_tokens_before);
    let tokens_after = compact_response
        .usage
        .as_ref()
        // Explicit Responses usage measures billed generation, not the retained
        // window. Preserve that usage on the response and compare bytes here.
        .filter(|_| llm_config.reasoning_state.is_none())
        .and_then(|usage| usage.output_tokens)
        .map(u64::from);
    let cost_usd = compact_response.usage.as_ref().and_then(|usage| usage.cost);
    let usage = compact_response.usage.as_ref().and_then(|usage| {
        match (usage.input_tokens, usage.output_tokens) {
            (Some(input), Some(output)) => Some(TokenUsage::new(input, output).with_cost(
                usage.cost,
                crate::engine::model_profiles::estimate_cost_usd_for_speed(
                    &everruns_contracts::DriverId::external(provider_type),
                    model,
                    input,
                    output,
                    0,
                    0,
                    None,
                ),
            )),
            // A provider can price a call without returning token counters.
            _ => usage
                .cost
                .map(|cost| TokenUsage::new(0, 0).with_cost(Some(cost), None)),
        }
    });
    let estimated_cost_usd = usage.as_ref().and_then(|usage| usage.estimated_cost_usd);
    // Account every completed provider call, even if its window or checkpoint
    // is subsequently rejected. Do not fold it into the next main generation.
    event_emitter
        .emit(EventRequest::new(
            session_id,
            event_context.clone(),
            LlmGenerationData::success(
                Vec::new(),
                Vec::new(),
                None,
                Vec::new(),
                model.to_string(),
                Some(provider_type.to_string()),
                usage,
                Some(started.elapsed().as_millis() as u64),
                None,
            )
            .with_provider_id(Some(provider_id.to_string()))
            .with_request_options(LlmRequestOptions {
                stream: Some(false),
                metadata: std::collections::HashMap::from([
                    ("purpose".into(), "context_compaction".into()),
                    ("strategy".into(), "native".into()),
                ]),
                ..Default::default()
            }),
        ))
        .await?;
    let bytes_after = serde_json::to_vec(&compact_response.output)
        .ok()
        .map(|value| value.len() as u64);
    let effective = match (tokens_before, tokens_after) {
        (Some(before), Some(after)) => materially_reduced(before, after),
        _ => match (bytes_before, bytes_after) {
            (Some(before), Some(after)) => materially_reduced(before, after),
            _ => false,
        },
    };
    if !effective {
        tracing::info!(
            session_id = %session_id,
            ?tokens_before,
            ?tokens_after,
            ?bytes_before,
            ?bytes_after,
            "ReasonAtom: native compaction produced no material reduction"
        );
        return Ok(None);
    }

    let output_items_after = compact_response.output.len();
    let opaque_context =
        crate::engine::driver_registry::ProviderOpaqueContext::OpenResponsesCompact {
            output: compact_response.output,
            reasoning_state: llm_config.reasoning_state.clone(),
        };
    let checkpoint_id =
        if let (Some(store), Some(source_sequence)) = (checkpoint_store, source_sequence) {
            let id = Uuid::now_v7();
            let installed = store
                .install(crate::engine::CompactionCheckpoint {
                    id,
                    session_id,
                    source_sequence,
                    provider_type: provider_type.to_string(),
                    model: model.to_string(),
                    format_version: crate::engine::COMPACTION_CHECKPOINT_FORMAT_VERSION,
                    payload: crate::engine::CompactionCheckpointPayload::ProviderOpaque {
                        context: opaque_context.clone(),
                    },
                })
                .await?;
            if !installed {
                return Err(AgentLoopError::store(
                    "a newer compaction checkpoint was installed concurrently",
                ));
            }
            Some(id.to_string())
        } else {
            None
        };

    // Apply only after the durable install succeeds, so checkpoint failures do
    // not partially mutate the retry request.
    llm_config.previous_response_id = None;
    llm_config.tool_baseline = None;
    llm_config.system_prompt_baseline = None;
    llm_config.provider_opaque_context = Some(opaque_context);
    llm_messages.retain(|message| message.role == MessageRole::System);
    if let Some(state) = llm_config.reasoning_state.as_mut() {
        // Explicit compaction resets configuration updates. Reassert the
        // effective effort even when it equals the original baseline.
        state.pending = state.effective;
    }

    Ok(Some(AppliedNativeCompaction {
        checkpoint_id,
        output_items_after,
        tokens_before,
        tokens_after,
        bytes_before,
        bytes_after,
        duration_ms: started.elapsed().as_millis() as u64,
        cost_usd,
        estimated_cost_usd,
    }))
}
