use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use crate::compact::{CompactRequest, messages_to_compact_input};
use crate::driver_registry::{LlmMessage, LlmMessageContent, LlmMessageRole};
use crate::error::{AgentLoopError, Result};
use crate::event_emitter::EventEmitter;
use crate::events::{
    CompactionReason, CompactionStepData, ContextCompactedData, ContextCompactingData,
    EventContext, EventRequest, LlmCompactionInfo, TokenUsage,
};
use crate::typed_id::SessionId;

pub(super) const CHECKPOINT_REARM_MIN_SUFFIX_MESSAGES: usize = 4;
pub(super) const PROACTIVE_RETRY_MIN_TOKEN_GROWTH: u64 = 4_096;
pub(super) const PROACTIVE_RETRY_GROWTH_DIVISOR: u64 = 20;

pub(super) fn proactive_source_fingerprint(
    provider_opaque_context: Option<&crate::ProviderOpaqueContext>,
    messages: &[LlmMessage],
) -> [u8; 32] {
    let mut input = match provider_opaque_context {
        Some(crate::ProviderOpaqueContext::OpenResponsesCompact { output }) => {
            output.iter().map(crate::CompactInputItem::from).collect()
        }
        None => Vec::new(),
    };
    input.extend(messages_to_compact_input(messages));
    let bytes = serde_json::to_vec(&input).unwrap_or_default();
    Sha256::digest(bytes).into()
}

#[derive(Debug)]
pub(super) struct AppliedNativeCompaction {
    pub(super) checkpoint_id: Option<String>,
    pub(super) input_items_before: usize,
    pub(super) output_items_after: usize,
    pub(super) tokens_before: Option<u64>,
    pub(super) tokens_after: Option<u64>,
    pub(super) bytes_before: Option<u64>,
    pub(super) bytes_after: Option<u64>,
    pub(super) duration_ms: u64,
    /// Provider-reported cost of the compaction call, when reported (EVE-895).
    pub(super) cost_usd: Option<f64>,
}

pub(super) fn materially_reduced(before: u64, after: u64) -> bool {
    const MIN_REDUCTION_UNITS: u64 = 32;
    let five_percent = before.div_ceil(20);
    let required_reduction = five_percent.max(MIN_REDUCTION_UNITS).min(before);
    after < before && before - after >= required_reduction
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn try_apply_native_compaction(
    chat_driver: &dyn crate::ChatDriver,
    compaction_policy: &dyn crate::compaction_policy::CompactionPolicy,
    checkpoint_store: Option<&Arc<dyn crate::CompactionCheckpointStore>>,
    session_id: SessionId,
    source_sequence: Option<i64>,
    provider_type: &str,
    model: &str,
    system_prompt: Option<&str>,
    stateful_response_continuation: bool,
    llm_messages: &mut Vec<LlmMessage>,
    llm_config: &mut crate::driver_registry::LlmCallConfig,
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
            Some(crate::ProviderOpaqueContext::OpenResponsesCompact { output }) => (
                output.iter().map(crate::CompactInputItem::from).collect(),
                true,
            ),
            None => (Vec::new(), false),
        };
    standalone_input.extend(messages_to_compact_input(messages_to_compact));
    let input_items_before = standalone_input.len();
    let local_tokens_before = (!stateful_response_continuation && !has_prior_opaque_context)
        .then(|| compaction_policy.estimate_total_tokens(messages_to_compact) as u64);
    let bytes_before = (!stateful_response_continuation || has_prior_opaque_context)
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
            &crate::ProviderEndpoint::default(),
            CompactRequest {
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
        .and_then(|usage| usage.output_tokens)
        .map(u64::from);
    let cost_usd = compact_response.usage.as_ref().and_then(|usage| usage.cost);
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
    let opaque_context = crate::driver_registry::ProviderOpaqueContext::OpenResponsesCompact {
        output: compact_response.output,
    };
    let checkpoint_id =
        if let (Some(store), Some(source_sequence)) = (checkpoint_store, source_sequence) {
            let id = Uuid::now_v7();
            let installed = store
                .install(crate::CompactionCheckpoint {
                    id,
                    session_id,
                    source_sequence,
                    provider_type: provider_type.to_string(),
                    model: model.to_string(),
                    format_version: crate::COMPACTION_CHECKPOINT_FORMAT_VERSION,
                    payload: crate::CompactionCheckpointPayload::ProviderOpaque {
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
    llm_config.provider_opaque_context = Some(opaque_context);
    llm_messages.retain(|message| message.role == LlmMessageRole::System);

    Ok(Some(AppliedNativeCompaction {
        checkpoint_id,
        input_items_before,
        output_items_after,
        tokens_before,
        tokens_after,
        bytes_before,
        bytes_after,
        duration_ms: started.elapsed().as_millis() as u64,
        cost_usd,
    }))
}

pub(super) struct ProactiveCompactionContext<'a> {
    pub(super) chat_driver: &'a dyn crate::ChatDriver,
    pub(super) policy: &'a dyn crate::compaction_policy::CompactionPolicy,
    pub(super) checkpoint_store: Option<&'a Arc<dyn crate::CompactionCheckpointStore>>,
    pub(super) event_emitter: &'a dyn EventEmitter,
    pub(super) event_context: &'a EventContext,
    pub(super) session_id: SessionId,
    pub(super) message_source_sequence: Option<i64>,
    pub(super) provider_type: &'a str,
    pub(super) model: &'a str,
    pub(super) system_prompt: Option<&'a str>,
    pub(super) stateful_response_continuation: bool,
    pub(super) checkpoint_restored: bool,
    pub(super) checkpoint_suffix_message_count: usize,
    pub(super) raw_tool_result_bytes: usize,
    pub(super) prior_usage: Option<&'a TokenUsage>,
}

pub(super) async fn apply_proactive_compaction(
    context: ProactiveCompactionContext<'_>,
    messages: &mut Vec<LlmMessage>,
    config: &mut crate::driver_registry::LlmCallConfig,
) -> Result<Option<LlmCompactionInfo>> {
    use crate::compaction_policy::CompactionStrategy;

    let settings = context.policy.settings();
    let context_window = context
        .chat_driver
        .effective_context_window(context.model)
        .or_else(|| {
            crate::model_profiles::get_model_profile(
                &everruns_provider::DriverId::external(context.provider_type),
                context.model,
            )
            .and_then(|profile| profile.limits.map(|limits| limits.context as usize))
        })
        .unwrap_or(128_000);
    let checkpoint_rearmed = !context.checkpoint_restored
        || context.checkpoint_suffix_message_count >= CHECKPOINT_REARM_MIN_SUFFIX_MESSAGES;
    let native_strategy = matches!(
        settings.strategy,
        CompactionStrategy::Auto | CompactionStrategy::Native
    );
    let durable_source = context
        .checkpoint_store
        .zip(context.message_source_sequence);
    let estimated_tokens_before = context.policy.estimate_total_tokens(messages) as u64;
    let native_attempt_rearmed = if let Some((store, source_sequence)) = durable_source {
        match store
            .get_proactive_attempt(context.session_id, context.provider_type, context.model)
            .await
        {
            Ok(attempt) => attempt.is_none_or(|attempt| {
                if source_sequence < attempt.source_sequence
                    || messages.len() < attempt.input_message_count
                {
                    return true;
                }
                let same_source_lineage = proactive_source_fingerprint(
                    config.provider_opaque_context.as_ref(),
                    &messages[..attempt.input_message_count],
                ) == attempt.source_fingerprint;
                if !same_source_lineage {
                    return true;
                }
                if source_sequence == attempt.source_sequence {
                    return false;
                }
                let required_growth = PROACTIVE_RETRY_MIN_TOKEN_GROWTH
                    .max(attempt.estimated_input_tokens / PROACTIVE_RETRY_GROWTH_DIVISOR);
                estimated_tokens_before.saturating_sub(attempt.estimated_input_tokens)
                    >= required_growth
            }),
            Err(error) => {
                tracing::warn!(
                    session_id = %context.session_id,
                    error = %error,
                    "ReasonAtom: proactive compaction attempt watermark lookup failed"
                );
                true
            }
        }
    } else {
        true
    };
    let window_pressure = context
        .policy
        .should_compact_proactively(messages, context_window);
    let cost_pressure = context.policy.should_compact_for_cost(
        estimated_tokens_before as usize,
        context.raw_tool_result_bytes,
        context.prior_usage,
    );
    let local_pressure = !context.stateful_response_continuation
        && checkpoint_rearmed
        && (window_pressure || cost_pressure);
    let should_attempt = native_strategy
        && context.chat_driver.supports_compact()
        && durable_source.is_some()
        && native_attempt_rearmed
        && local_pressure;

    let applied = if let (true, Some((store, source_sequence))) = (should_attempt, durable_source) {
        let messages_before = messages.len();
        let input_message_count = messages.len();
        let source_fingerprint =
            proactive_source_fingerprint(config.provider_opaque_context.as_ref(), messages);
        if let Err(error) = store
            .record_proactive_attempt(
                context.session_id,
                context.provider_type,
                context.model,
                crate::ProactiveCompactionAttempt {
                    source_sequence,
                    estimated_input_tokens: estimated_tokens_before,
                    input_message_count,
                    source_fingerprint,
                },
            )
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %error,
                "ReasonAtom: proactive compaction attempt watermark write failed"
            );
        }
        let _ = context
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                context.event_context.clone(),
                ContextCompactingData {
                    reason: CompactionReason::ProactiveBudget,
                    strategy: settings.strategy.to_string(),
                    messages_before,
                    tokens_before: Some(estimated_tokens_before),
                    bytes_before: None,
                },
            ))
            .await;

        let applied = try_apply_native_compaction(
            context.chat_driver,
            context.policy,
            context.checkpoint_store,
            context.session_id,
            context.message_source_sequence,
            context.provider_type,
            context.model,
            context.system_prompt,
            false,
            messages,
            config,
        )
        .await?;

        if let Some(applied) = applied.as_ref() {
            let steps = vec![CompactionStepData {
                strategy: "native".to_string(),
                messages_after: applied.output_items_after,
                duration_ms: applied.duration_ms,
            }];
            let _ = context
                .event_emitter
                .emit(EventRequest::new(
                    context.session_id,
                    context.event_context.clone(),
                    ContextCompactedData {
                        checkpoint_id: applied.checkpoint_id.clone(),
                        strategy_used: "native".to_string(),
                        messages_before,
                        messages_after: applied.output_items_after,
                        tokens_before: applied.tokens_before,
                        tokens_after: applied.tokens_after,
                        bytes_before: applied.bytes_before,
                        bytes_after: applied.bytes_after,
                        duration_ms: applied.duration_ms,
                        steps,
                    },
                ))
                .await;
        }
        applied
    } else {
        None
    };

    if local_pressure && applied.is_none() {
        if matches!(
            settings.strategy,
            CompactionStrategy::Auto | CompactionStrategy::ObservationMasking
        ) {
            let conversation = if context.system_prompt.is_some() {
                &messages[1..]
            } else {
                &messages[..]
            };
            let masked = context.policy.apply_observation_masking(conversation);
            if masked.masked_count > 0 {
                let mut model_view = Vec::new();
                if context.system_prompt.is_some() {
                    model_view.push(messages[0].clone());
                }
                model_view.extend(masked.messages);
                *messages = model_view;
            }
        }
        let budget_tokens = (context_window as f32 * settings.budget_percent) as usize;
        if context.policy.estimate_total_tokens(messages) > budget_tokens {
            *messages = context.policy.aggressive_trim(
                messages,
                budget_tokens,
                context.system_prompt.is_some(),
            );
        }
    }

    Ok(applied.map(|applied| {
        LlmCompactionInfo::new(
            Some(applied.input_items_before as u32),
            applied
                .tokens_after
                .and_then(|value| u32::try_from(value).ok()),
            Some(applied.duration_ms),
            applied.cost_usd,
        )
    }))
}

pub(super) struct ReactiveCompactionContext<'a> {
    pub(super) chat_driver: &'a dyn crate::ChatDriver,
    pub(super) policy: &'a dyn crate::compaction_policy::CompactionPolicy,
    pub(super) checkpoint_store: Option<&'a Arc<dyn crate::CompactionCheckpointStore>>,
    pub(super) event_emitter: &'a dyn EventEmitter,
    pub(super) event_context: &'a EventContext,
    pub(super) session_id: SessionId,
    pub(super) message_source_sequence: Option<i64>,
    pub(super) provider_type: &'a str,
    pub(super) model: &'a str,
    pub(super) summarization_model_fallback: &'a str,
    pub(super) system_prompt: Option<&'a str>,
    pub(super) stateful_response_continuation: bool,
}

pub(super) struct ReactiveCompactionResult {
    pub(super) generation_info: Option<LlmCompactionInfo>,
}

/// Apply the request-too-large recovery cascade as one state transition.
///
/// `Ok(None)` means the cascade could not materially reduce the request and
/// the original provider error must remain authoritative.
pub(super) async fn apply_reactive_compaction(
    context: ReactiveCompactionContext<'_>,
    messages: &mut Vec<LlmMessage>,
    config: &mut crate::driver_registry::LlmCallConfig,
) -> Result<Option<ReactiveCompactionResult>> {
    use crate::compaction_policy::CompactionStrategy;

    let settings = context.policy.settings();
    let messages_before = messages.len();
    tracing::info!(
        session_id = %context.session_id,
        strategy = %settings.strategy,
        messages = messages_before,
        "ReasonAtom: context too large, attempting compaction"
    );

    let tokens_before = Some(context.policy.estimate_total_tokens(messages) as u64);
    let _ = context
        .event_emitter
        .emit(EventRequest::new(
            context.session_id,
            context.event_context.clone(),
            ContextCompactingData {
                reason: CompactionReason::RequestTooLarge,
                strategy: settings.strategy.to_string(),
                messages_before,
                tokens_before,
                bytes_before: None,
            },
        ))
        .await;

    let cascade_start = Instant::now();
    let mut steps = Vec::new();
    let mut strategies_used = Vec::new();
    let mut checkpoint_id = None;
    let mut generation_info = None;
    let mut measured_tokens_before = tokens_before;
    let mut tokens_after = None;
    let mut bytes_before = None;
    let mut bytes_after = None;
    let has_system_prompt = context.system_prompt.is_some();

    let run_masking = matches!(
        settings.strategy,
        CompactionStrategy::Auto | CompactionStrategy::ObservationMasking
    );
    let run_native = matches!(
        settings.strategy,
        CompactionStrategy::Auto | CompactionStrategy::Native
    ) && context.chat_driver.supports_compact();
    let run_summarization = matches!(
        settings.strategy,
        CompactionStrategy::Auto | CompactionStrategy::Summarization
    );

    if run_masking {
        let step_start = Instant::now();
        let conversation = if has_system_prompt {
            &messages[1..]
        } else {
            &messages[..]
        };
        let masked = context.policy.apply_observation_masking(conversation);
        if masked.masked_count > 0 {
            let mut model_view = Vec::new();
            if has_system_prompt {
                model_view.push(messages[0].clone());
            }
            model_view.extend(masked.messages);
            *messages = model_view;

            let duration_ms = step_start.elapsed().as_millis() as u64;
            strategies_used.push("observation_masking".to_string());
            steps.push(CompactionStepData {
                strategy: "observation_masking".to_string(),
                messages_after: messages.len(),
                duration_ms,
            });
            tracing::info!(
                session_id = %context.session_id,
                masked_count = masked.masked_count,
                duration_ms,
                "ReasonAtom: observation masking applied"
            );
        }
    }

    if run_native
        && let Some(applied) = try_apply_native_compaction(
            context.chat_driver,
            context.policy,
            context.checkpoint_store,
            context.session_id,
            context.message_source_sequence,
            context.provider_type,
            context.model,
            context.system_prompt,
            context.stateful_response_continuation,
            messages,
            config,
        )
        .await?
    {
        generation_info = Some(LlmCompactionInfo::new(
            Some(applied.input_items_before as u32),
            applied
                .tokens_after
                .and_then(|value| u32::try_from(value).ok()),
            Some(applied.duration_ms),
            applied.cost_usd,
        ));
        checkpoint_id = applied.checkpoint_id;
        measured_tokens_before = applied.tokens_before;
        tokens_after = applied.tokens_after;
        bytes_before = applied.bytes_before;
        bytes_after = applied.bytes_after;
        strategies_used.push("native".to_string());
        steps.push(CompactionStepData {
            strategy: "native".to_string(),
            messages_after: applied.output_items_after,
            duration_ms: applied.duration_ms,
        });
    }

    if run_summarization && !strategies_used.iter().any(|strategy| strategy == "native") {
        let step_start = Instant::now();
        let conversation = if has_system_prompt {
            &messages[1..]
        } else {
            &messages[..]
        };
        let keep_recent = 10.min(conversation.len());
        let to_summarize = &conversation[..conversation.len() - keep_recent];
        let recent = &conversation[conversation.len() - keep_recent..];

        if !to_summarize.is_empty() {
            let summary_messages = vec![
                LlmMessage {
                    role: LlmMessageRole::System,
                    content: LlmMessageContent::Text(context.policy.summarization_prompt()),
                    tool_calls: None,
                    tool_call_id: None,
                    phase: None,
                    thinking: None,
                    thinking_signature: None,
                },
                LlmMessage {
                    role: LlmMessageRole::User,
                    content: LlmMessageContent::Text(
                        context
                            .policy
                            .format_messages_for_summarization(to_summarize),
                    ),
                    tool_calls: None,
                    tool_call_id: None,
                    phase: None,
                    thinking: None,
                    thinking_signature: None,
                },
            ];
            let summary_config = crate::driver_registry::LlmCallConfig {
                speed: None,
                verbosity: None,
                model: settings
                    .summarization_model
                    .clone()
                    .unwrap_or_else(|| context.summarization_model_fallback.to_string()),
                temperature: Some(0.0),
                max_tokens: Some(2000),
                tools: vec![],
                reasoning_effort: None,
                metadata: HashMap::new(),
                previous_response_id: None,
                provider_opaque_context: None,
                tool_search: None,
                prompt_cache: None,
                openrouter_routing: None,
                parallel_tool_calls: None,
                volatile_suffix_len: 0,
                extra_headers: Vec::new(),
                cache_diagnostics: None,
            };

            match context
                .chat_driver
                .chat_completion(
                    &crate::ProviderEndpoint::default(),
                    summary_messages,
                    &summary_config,
                )
                .await
            {
                Ok(response) => {
                    let system_message = has_system_prompt.then(|| messages[0].clone());
                    *messages = context.policy.compose_summary_with_recent(
                        system_message,
                        &response.text,
                        recent,
                    );
                    let duration_ms = step_start.elapsed().as_millis() as u64;
                    strategies_used.push("summarization".to_string());
                    steps.push(CompactionStepData {
                        strategy: "summarization".to_string(),
                        messages_after: messages.len(),
                        duration_ms,
                    });
                    tracing::info!(
                        session_id = %context.session_id,
                        duration_ms,
                        messages_after = messages.len(),
                        "ReasonAtom: summarization applied"
                    );
                }
                Err(error) => tracing::warn!(
                    session_id = %context.session_id,
                    error = %error,
                    "ReasonAtom: summarization failed, continuing"
                ),
            }
        }
    }

    if strategies_used.is_empty() || messages.len() > messages_before / 2 {
        let step_start = Instant::now();
        let target = context.policy.estimate_total_tokens(messages) / 2;
        let trimmed = context
            .policy
            .aggressive_trim(messages, target, has_system_prompt);
        if trimmed.len() < messages.len() {
            *messages = trimmed;
            let duration_ms = step_start.elapsed().as_millis() as u64;
            strategies_used.push("aggressive_trim".to_string());
            steps.push(CompactionStepData {
                strategy: "aggressive_trim".to_string(),
                messages_after: messages.len(),
                duration_ms,
            });
            tracing::info!(
                session_id = %context.session_id,
                messages_after = messages.len(),
                "ReasonAtom: aggressive trim applied (last resort)"
            );
        }
    }

    let duration_ms = cascade_start.elapsed().as_millis() as u64;
    let messages_after = messages.len();
    if tokens_after.is_none() {
        tokens_after = Some(context.policy.estimate_total_tokens(messages) as u64);
    }
    let effective = match (measured_tokens_before, tokens_after) {
        (Some(before), Some(after)) => materially_reduced(before, after),
        _ => match (bytes_before, bytes_after) {
            (Some(before), Some(after)) => materially_reduced(before, after),
            _ => false,
        },
    };
    if !effective {
        tracing::warn!(
            session_id = %context.session_id,
            ?measured_tokens_before,
            ?tokens_after,
            "ReasonAtom: compaction cascade made no material reduction"
        );
        return Ok(None);
    }

    let strategy_used = strategies_used.join("+");
    if strategies_used
        .iter()
        .any(|strategy| strategy != "observation_masking")
    {
        let _ = context
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                context.event_context.clone(),
                ContextCompactedData {
                    checkpoint_id,
                    strategy_used: strategy_used.clone(),
                    messages_before,
                    messages_after,
                    tokens_before: measured_tokens_before,
                    tokens_after,
                    bytes_before,
                    bytes_after,
                    duration_ms,
                    steps,
                },
            ))
            .await;
    }

    tracing::info!(
        session_id = %context.session_id,
        strategy = %strategy_used,
        messages_before,
        messages_after,
        duration_ms,
        "ReasonAtom: compaction cascade completed, retrying LLM call"
    );
    Ok(Some(ReactiveCompactionResult { generation_info }))
}
