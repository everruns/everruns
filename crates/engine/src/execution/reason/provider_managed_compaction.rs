use std::time::Instant;

use crate::error::Result;
use crate::event_emitter::EventEmitter;
use crate::events::{
    CompactionFailStage, CompactionReason, CompactionStepData, CompactionTrigger,
    ContextCompactedData, ContextCompactingData, ContextCompactionFailedData, EventContext,
    EventRequest,
};
use crate::tool_types::ToolDefinition;
use crate::typed_id::{AgentId, HarnessId, SessionId};

use super::super::provider_checkpoint::ProviderCheckpointInstall;
use super::error_policy::{error_disclosure_override, resolve_error_disclosure};
use super::{ReasonAtom, ReasonResult};

const STRATEGY: &str = "anthropic_server_compaction";

pub(super) fn decorate_request(
    config: &mut crate::driver_registry::LlmCallConfig,
    provider_managed: bool,
    checkpoint_restored: bool,
) {
    if !provider_managed {
        return;
    }
    config.driver_options.insert(
        "everruns/provider_managed_reduction".to_string(),
        serde_json::json!({
            "mode": STRATEGY,
            "replay_source": if checkpoint_restored { "checkpoint" } else { "raw" },
        }),
    );
}

pub(super) struct Lifecycle<'a> {
    emitter: &'a dyn EventEmitter,
    session_id: SessionId,
    context: &'a EventContext,
    model: &'a str,
    provider: &'a str,
    messages: usize,
    source_sequence: Option<i64>,
    trigger_tokens: Option<u64>,
}

impl<'a> Lifecycle<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        emitter: &'a dyn EventEmitter,
        session_id: SessionId,
        context: &'a EventContext,
        model: &'a str,
        provider: &'a str,
        messages: usize,
        source_sequence: Option<i64>,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Self {
        Self {
            emitter,
            session_id,
            context,
            model,
            provider,
            messages,
            source_sequence,
            trigger_tokens: config
                .driver_options
                .get("anthropic/server_compaction")
                .and_then(|option| option.get("trigger_tokens"))
                .and_then(serde_json::Value::as_u64),
        }
    }

    pub(super) async fn start(&self, started_at: &mut Option<Instant>) {
        if started_at.is_some() {
            return;
        }
        *started_at = Some(Instant::now());
        let _ = self
            .emitter
            .emit(EventRequest::new(
                self.session_id,
                self.context.clone(),
                ContextCompactingData {
                    reason: CompactionReason::ProactiveBudget,
                    strategy: STRATEGY.to_string(),
                    messages_before: self.messages,
                    tokens_before: self.trigger_tokens,
                    bytes_before: None,
                    trigger: CompactionTrigger::ContextBudget,
                    model: self.model.to_string(),
                    provider: Some(self.provider.to_string()),
                    driver: None,
                    budget_remaining_tokens: None,
                    source_sequence: self.source_sequence,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                },
            ))
            .await;
    }

    pub(super) async fn fail(&self) {
        let _ = self
            .emitter
            .emit(EventRequest::new(
                self.session_id,
                self.context.clone(),
                ContextCompactionFailedData {
                    reason: CompactionReason::ProactiveBudget,
                    trigger: CompactionTrigger::ContextBudget,
                    stage: CompactionFailStage::NativeCompaction,
                    error: "provider server compaction stream failed".to_string(),
                    strategy: STRATEGY.to_string(),
                    model: self.model.to_string(),
                    provider: Some(self.provider.to_string()),
                    driver: None,
                    tokens_before: self.trigger_tokens.unwrap_or_default(),
                    budget_remaining_tokens: None,
                    source_sequence: self.source_sequence,
                    messages_before: self.messages,
                    checkpoint_id: None,
                },
            ))
            .await;
    }
    pub(super) async fn fail_if(&self, condition: bool) {
        if condition {
            self.fail().await;
        }
    }

    pub(super) async fn fail_start<T>(
        &self,
        error: crate::error::AgentLoopError,
        provider_managed: bool,
    ) -> Result<T> {
        if provider_managed
            && !error.rejected_provider_capability(
                everruns_provider::RejectedProviderCapability::AnthropicServerCompaction,
            )
        {
            self.fail().await;
        }
        Err(error)
    }

    pub(super) async fn reject_incomplete(
        &self,
        started_at: Option<Instant>,
        termination: &super::StreamTermination,
    ) -> Result<()> {
        if started_at.is_none() || !matches!(termination, super::StreamTermination::Exhausted) {
            return Ok(());
        }
        self.fail().await;
        Err(crate::error::AgentLoopError::llm_kind(
            crate::error::LlmErrorKind::MalformedResponse,
            "provider compaction stream ended before its terminal event",
        ))
    }
    pub(super) fn record_observed(
        &self,
        config: &mut crate::driver_registry::LlmCallConfig,
        started_at: Option<Instant>,
    ) {
        if started_at.is_some() {
            config.driver_options.insert(
                "everruns/provider_managed_compaction_observed".to_string(),
                serde_json::Value::Bool(true),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn finish_after_output(
        &self,
        store: Option<&dyn crate::CompactionCheckpointStore>,
        source_sequence: Option<i32>,
        candidate: Option<crate::driver_registry::ProviderCheckpointCandidate>,
        started_at: Option<Instant>,
        checkpoint_restored: bool,
        metadata: Option<&crate::driver_registry::LlmCompletionMetadata>,
    ) {
        let candidate_received = candidate.is_some();
        let install = super::super::provider_checkpoint::install_candidate(
            store,
            source_sequence,
            candidate,
            self.session_id,
            self.provider,
            self.model,
        )
        .await;
        let Some(started_at) = started_at else {
            return;
        };
        if candidate_received {
            self.complete(
                started_at,
                if checkpoint_restored {
                    "checkpoint"
                } else {
                    "raw"
                },
                metadata,
                install,
            )
            .await;
        } else {
            self.fail().await;
        }
    }

    pub(super) async fn complete(
        &self,
        started_at: Instant,
        replay_source: &str,
        metadata: Option<&crate::driver_registry::LlmCompletionMetadata>,
        install: ProviderCheckpointInstall,
    ) {
        let duration_ms = started_at.elapsed().as_millis() as u64;
        let prompt_tokens = metadata
            .and_then(|metadata| metadata.prompt_tokens)
            .map(u64::from);
        let output_tokens = metadata
            .and_then(|metadata| metadata.completion_tokens)
            .map(u64::from);
        let (cache_read_tokens, cache_creation_tokens) = metadata
            .map(|metadata| (metadata.cache_read_tokens, metadata.cache_creation_tokens))
            .unwrap_or((None, None));
        let _ = self
            .emitter
            .emit(EventRequest::new(
                self.session_id,
                self.context.clone(),
                ContextCompactedData {
                    checkpoint_id: install.checkpoint_id,
                    checkpoint_bytes: install.checkpoint_bytes,
                    replay_source: Some(replay_source.to_string()),
                    strategy_used: STRATEGY.to_string(),
                    messages_before: self.messages,
                    messages_after: self.messages,
                    tokens_before: prompt_tokens,
                    tokens_after: output_tokens,
                    bytes_before: None,
                    bytes_after: install.checkpoint_bytes,
                    duration_ms,
                    steps: vec![CompactionStepData {
                        strategy: STRATEGY.to_string(),
                        messages_after: self.messages,
                        duration_ms,
                    }],
                    trigger: CompactionTrigger::ContextBudget,
                    model: self.model.to_string(),
                    provider: Some(self.provider.to_string()),
                    driver: None,
                    budget_remaining_tokens: None,
                    source_sequence: self.source_sequence,
                    cache_read_tokens,
                    cache_creation_tokens,
                },
            ))
            .await;
    }
}

pub(super) struct Call<'a> {
    pub(super) atom: &'a ReasonAtom,
    pub(super) session_id: SessionId,
    pub(super) harness_id: HarnessId,
    pub(super) agent_id: Option<AgentId>,
    pub(super) org_id: i64,
    pub(super) context: &'a crate::ExecutionContext,
    pub(super) trace_id: &'a str,
    pub(super) reason_span_id: &'a str,
    pub(super) previous_response_id: Option<String>,
    pub(super) iteration: u32,
    pub(super) mcp_tool_definitions: &'a [ToolDefinition],
    pub(super) assembled: crate::runtime_context::AssembledTurnContext,
}

pub(super) type ErrorHooks = Vec<(
    std::sync::Arc<dyn crate::llm_error_hook::LlmErrorHook>,
    serde_json::Value,
)>;

pub(super) struct CallOutcome {
    pub(super) disclosure: crate::ErrorDisclosure,
    pub(super) error_context: crate::UserFacingErrorContext,
    pub(super) error_hooks: ErrorHooks,
    pub(super) result: Result<ReasonResult>,
}

pub(super) async fn execute_with_fallback(call: Call<'_>) -> CallOutcome {
    let Call {
        atom,
        session_id,
        harness_id,
        agent_id,
        org_id,
        context,
        trace_id,
        reason_span_id,
        previous_response_id,
        iteration,
        mcp_tool_definitions,
        assembled,
    } = call;
    let mut disclosure = resolve_error_disclosure(
        &atom.capability_registry,
        &assembled.resolved_capability_configs,
        error_disclosure_override(&assembled.messages).as_deref(),
    );
    let mut error_hooks = atom.collect_llm_error_hooks(&assembled.resolved_capability_configs);
    let mut error_context = crate::UserFacingErrorContext::default()
        .with_provider(assembled.model.provider_type.to_string())
        .with_model_id(assembled.model.model.clone());
    let native_provider = assembled.model.provider_managed_reduction_option.is_some();
    let provider_type = assembled.model.provider_type.clone();
    let model = assembled.model.model.clone();
    let mut result = atom
        .execute_llm_call(
            session_id,
            harness_id,
            agent_id,
            org_id,
            context,
            trace_id,
            reason_span_id,
            previous_response_id.clone(),
            iteration,
            assembled,
        )
        .await;
    if !native_provider
        || !result.as_ref().is_err_and(|error| {
            error.rejected_provider_capability(
                everruns_provider::RejectedProviderCapability::AnthropicServerCompaction,
            )
        })
    {
        return CallOutcome {
            disclosure,
            error_context,
            error_hooks,
            result,
        };
    }

    let fallback = atom
        .context_resolver
        .resolve_turn_context(crate::runtime_context::TurnContextRequest {
            session_id,
            harness_id,
            agent_id,
            mcp_tool_definitions: mcp_tool_definitions.to_vec(),
        })
        .await;
    match fallback {
        Ok(mut fallback)
            if fallback.model.provider_type == provider_type
                && fallback.model.model == model
                && fallback.model.provider_managed_reduction_option.is_none() =>
        {
            tracing::warn!(
                session_id = %session_id,
                turn_id = %context.turn_id,
                provider = %provider_type,
                model,
                "ReasonAtom: retrying provider capability rejection with legacy history"
            );
            fallback.runtime_agent.driver_options.insert(
                "everruns/provider_managed_reduction_fallback".to_string(),
                serde_json::json!({"reason":"provider_capability_rejected"}),
            );
            disclosure = resolve_error_disclosure(
                &atom.capability_registry,
                &fallback.resolved_capability_configs,
                error_disclosure_override(&fallback.messages).as_deref(),
            );
            error_hooks = atom.collect_llm_error_hooks(&fallback.resolved_capability_configs);
            error_context = crate::UserFacingErrorContext::default()
                .with_provider(fallback.model.provider_type.to_string())
                .with_model_id(fallback.model.model.clone());
            result = atom
                .execute_llm_call(
                    session_id,
                    harness_id,
                    agent_id,
                    org_id,
                    context,
                    trace_id,
                    reason_span_id,
                    previous_response_id,
                    iteration,
                    fallback,
                )
                .await;
        }
        Ok(_) => tracing::warn!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            "ReasonAtom: fresh context remained native after capability rejection"
        ),
        Err(error) => result = Err(error),
    }
    CallOutcome {
        disclosure,
        error_context,
        error_hooks,
        result,
    }
}
