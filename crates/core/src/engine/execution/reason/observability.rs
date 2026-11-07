use serde_json::json;
use std::collections::{BTreeSet, HashMap};

use crate::engine::capabilities::CapabilityRegistry;
use crate::engine::event_emitter::EventEmitter;
use crate::engine::events::{
    CapabilityUsageData, CapabilityUsageKind, CapabilityUsageRecord, EventContext, EventRequest,
    LlmPromptCacheInfo, LlmRequestOptions, LlmToolSearchInfo,
};
use crate::engine::execution::ExecutionContext;
use crate::engine::tool_types::ToolDefinition;
use crate::engine::typed_id::SessionId;

/// Claude on OpenRouter with the auto strategy: the request carries a
/// top-level `cache_control` (see the OpenRouter request extension).
fn openrouter_claude_auto(
    config: &crate::engine::driver_registry::LlmCallConfig,
    cfg: &crate::engine::driver_registry::PromptCacheConfig,
) -> bool {
    config.model.starts_with("anthropic/")
        && cfg.strategy == everruns_contracts::driver_registry::PromptCacheStrategy::Auto
}

pub(super) fn build_request_options(
    config: &crate::engine::driver_registry::LlmCallConfig,
    provider: &str,
) -> Option<LlmRequestOptions> {
    let prompt_cache = config
        .prompt_cache
        .as_ref()
        .filter(|cfg| cfg.enabled)
        .map(|cfg| LlmPromptCacheInfo {
            enabled: true,
            strategy: cfg.strategy,
            provider_mode: match provider {
                "openai" => Some(
                    if everruns_contracts::openai_compat::supports_cache_options(&config.model) {
                        match cfg.strategy {
                            everruns_contracts::driver_registry::PromptCacheStrategy::Auto => {
                                "implicit"
                            }
                            everruns_contracts::driver_registry::PromptCacheStrategy::Explicit => {
                                "explicit"
                            }
                        }
                    } else {
                        "prompt_cache_key"
                    }
                    .to_string(),
                ),
                "anthropic" => Some("cache_control".to_string()),
                "mistral" => Some("prompt_cache_key".to_string()),
                // Automatic caching via top-level `cache_control` (auto only).
                "openrouter" if openrouter_claude_auto(config, cfg) => {
                    Some("cache_control".to_string())
                }
                "gemini" => Some(
                    if cfg.gemini_cached_content.is_some() {
                        "cached_content"
                    } else {
                        "implicit"
                    }
                    .to_string(),
                ),
                _ => None,
            },
            retention: match provider {
                "anthropic" => Some(cfg.retention.anthropic_ttl().unwrap_or("5m").to_string()),
                "openrouter" if openrouter_claude_auto(config, cfg) => {
                    Some(cfg.retention.anthropic_ttl().unwrap_or("5m").to_string())
                }
                "openai"
                    if everruns_contracts::openai_compat::supports_cache_options(&config.model) =>
                {
                    Some(cfg.retention.openai_responses_ttl().to_string())
                }
                _ => None,
            },
        });

    let tool_search = config
        .tool_search
        .as_ref()
        .filter(|cfg| cfg.enabled)
        .map(|cfg| LlmToolSearchInfo {
            enabled: true,
            threshold: cfg.threshold,
        });

    let mut provider_options = HashMap::new();
    let explicit_openai_cache = provider == "openai"
        && everruns_contracts::openai_compat::supports_cache_options(&config.model)
        && config.prompt_cache.as_ref().is_some_and(|cache| {
            cache.enabled
                && cache.strategy
                    == everruns_contracts::driver_registry::PromptCacheStrategy::Explicit
        });
    if provider == "openai" && config.previous_response_id.is_some() && !explicit_openai_cache {
        provider_options.insert(
            "openai".to_string(),
            json!({ "previous_response_id": true }),
        );
    }
    if let Some(state) = &config.reasoning_state {
        let options = provider_options
            .entry("openai".to_string())
            .or_insert_with(|| json!({}));
        options["reasoning_baseline"] = json!(state.baseline);
        options["effective_reasoning_effort"] = json!(state.effective);
    }
    if provider == "gemini"
        && config
            .prompt_cache
            .as_ref()
            .filter(|cfg| cfg.enabled)
            .and_then(|cfg| cfg.gemini_cached_content.as_ref())
            .is_some()
    {
        provider_options.insert("gemini".to_string(), json!({ "cached_content": true }));
    }
    if provider == "anthropic" {
        let mut anthropic = json!({});
        if let Some(option) = config.driver_options.get("anthropic/server_compaction") {
            anthropic["reduction_mode"] = json!("server_compaction");
            anthropic["trigger_tokens"] = option
                .get("trigger_tokens")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }
        if let Some(option) = config
            .driver_options
            .get("everruns/provider_managed_reduction")
        {
            anthropic["replay_source"] = option
                .get("replay_source")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }
        if let Some(option) = config
            .driver_options
            .get("everruns/provider_managed_reduction_fallback")
        {
            anthropic["reduction_mode"] = json!("legacy");
            anthropic["fallback_reason"] = option
                .get("reason")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }
        if config
            .driver_options
            .get("everruns/provider_managed_compaction_observed")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            anthropic["compaction_block_observed"] = json!(true);
        }
        if anthropic
            .as_object()
            .is_some_and(|options| !options.is_empty())
        {
            provider_options.insert("anthropic".to_string(), anthropic);
        }
    }

    let request_options = LlmRequestOptions {
        temperature: config.temperature,
        max_tokens: config.max_tokens,
        reasoning_effort: config
            .reasoning_state
            .as_ref()
            .and_then(|state| state.effective)
            .or(config.reasoning_effort)
            .map(|effort| effort.as_str().to_string()),
        // The reason atom always drives the provider through its streaming
        // endpoint (`chat_completion_stream`).
        stream: Some(true),
        prompt_cache,
        tool_search,
        provider_options,
        metadata: config.metadata.clone(),
    };

    (!request_options.is_empty()).then_some(request_options)
}

pub(super) fn with_completed_request_options(
    mut generation: crate::engine::events::LlmGenerationData,
    config: &crate::engine::driver_registry::LlmCallConfig,
    provider: &str,
    meta: Option<&crate::engine::driver_registry::LlmCompletionMetadata>,
) -> crate::engine::events::LlmGenerationData {
    let mut options = build_request_options(config, provider).unwrap_or_default();
    record_wire_cache_retention(
        &mut options,
        meta.and_then(|m| m.request_body.as_ref()),
        provider,
    );
    if !options.is_empty() {
        generation = generation.with_request_options(options);
    }
    generation
}

/// Endpoint and protocol gates can shorten or omit the requested retention.
/// Completed generations record the sent body rather than inferring it from a
/// provider name (which can also represent Azure or Chat Completions).
pub(super) fn record_wire_cache_retention(
    options: &mut LlmRequestOptions,
    body: Option<&serde_json::Value>,
    provider: &str,
) {
    if provider == "openai"
        && let Some(body) = body
    {
        let retention = body
            .pointer("/prompt_cache_options/ttl")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        if let Some(cache) = &mut options.prompt_cache {
            cache.retention = retention;
        } else if let Some(mode @ ("implicit" | "explicit")) = body
            .pointer("/prompt_cache_options/mode")
            .and_then(serde_json::Value::as_str)
        {
            // A native driver may enable caching without a capability. Only
            // captured wire options establish what the driver sent.
            options.prompt_cache = Some(LlmPromptCacheInfo {
                enabled: true,
                strategy: if mode == "explicit" {
                    everruns_contracts::driver_registry::PromptCacheStrategy::Explicit
                } else {
                    everruns_contracts::driver_registry::PromptCacheStrategy::Auto
                },
                provider_mode: Some(mode.to_owned()),
                retention,
            });
        }
    }
}

fn capability_name_snapshot(registry: &CapabilityRegistry, capability_id: &str) -> Option<String> {
    registry
        .get(capability_id)
        .map(|capability| capability.name().to_string())
}

/// The `capability.usage` records for the turn's resolved capabilities and
/// the tools they expose: the attribution every reason backend reports,
/// whichever loop runs the model.
pub fn capability_usage_snapshot_records(
    registry: &CapabilityRegistry,
    resolved_capability_configs: &[crate::engine::CapabilityRef],
    tool_definitions: &[ToolDefinition],
) -> Vec<CapabilityUsageRecord> {
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();

    for config in resolved_capability_configs {
        let capability_id = config.capability_id().to_string();
        if seen.insert((
            "resolved".to_string(),
            capability_id.clone(),
            None::<String>,
        )) {
            records.push(CapabilityUsageRecord {
                capability_name: capability_name_snapshot(registry, &capability_id),
                capability_id,
                usage_kind: CapabilityUsageKind::Resolved,
                tool_name: None,
                usage_count: Some(1),
                duration_ms: None,
            });
        }
    }

    for tool in tool_definitions {
        let Some((capability_id, capability_name)) = tool.capability_attribution() else {
            continue;
        };
        let capability_id = capability_id.to_string();
        let tool_name = tool.name().to_string();
        if seen.insert((
            "exposed".to_string(),
            capability_id.clone(),
            Some(tool_name.clone()),
        )) {
            records.push(CapabilityUsageRecord {
                capability_name: capability_name
                    .map(str::to_string)
                    .or_else(|| capability_name_snapshot(registry, &capability_id)),
                capability_id,
                usage_kind: CapabilityUsageKind::Exposed,
                tool_name: Some(tool_name),
                usage_count: Some(1),
                duration_ms: None,
            });
        }
    }

    records
}

/// Turns whose last snapshot this process remembers.
const SNAPSHOT_MEMO_TURNS: usize = 16_384;

/// The last snapshot this process emitted for each recent turn, by hash.
///
/// Decision: a turn's model calls almost always see the same capabilities and
/// tools, and reporting keeps one row per turn and record
/// (`reporting_capability_usage_key`), so a repeated snapshot only cost an
/// event of about 50 records per model call and a projection that inserts
/// nothing. A call emits when its snapshot differs from the last one emitted
/// for the turn here, which keeps tools exposed mid-turn. A turn this process
/// does not remember (a restart, a turn moved to another worker) emits again,
/// and the projection drops the repeat.
#[derive(Default)]
struct SnapshotMemo {
    order: std::collections::VecDeque<(SessionId, crate::engine::typed_id::TurnId)>,
    last: HashMap<(SessionId, crate::engine::typed_id::TurnId), u64>,
}

impl SnapshotMemo {
    /// Record `hash` for the turn; whether it differs from the last one.
    fn changed(&mut self, turn: (SessionId, crate::engine::typed_id::TurnId), hash: u64) -> bool {
        match self.last.insert(turn, hash) {
            Some(previous) => previous != hash,
            None => {
                self.order.push_back(turn);
                if self.order.len() > SNAPSHOT_MEMO_TURNS
                    && let Some(oldest) = self.order.pop_front()
                {
                    self.last.remove(&oldest);
                }
                true
            }
        }
    }
}

fn snapshot_changed(
    session_id: SessionId,
    turn_id: crate::engine::typed_id::TurnId,
    records: &[CapabilityUsageRecord],
) -> bool {
    use std::hash::{Hash, Hasher};
    static MEMO: std::sync::OnceLock<std::sync::Mutex<SnapshotMemo>> = std::sync::OnceLock::new();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(records)
        .unwrap_or_default()
        .hash(&mut hasher);
    let mut memo = MEMO
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    memo.changed((session_id, turn_id), hasher.finish())
}

pub(super) async fn emit_capability_usage_snapshot(
    emitter: &dyn EventEmitter,
    registry: &CapabilityRegistry,
    session_id: SessionId,
    context: &ExecutionContext,
    resolved_capability_configs: &[crate::engine::CapabilityRef],
    tool_definitions: &[ToolDefinition],
) {
    let records =
        capability_usage_snapshot_records(registry, resolved_capability_configs, tool_definitions);
    if records.is_empty() || !snapshot_changed(session_id, context.turn_id, &records) {
        return;
    }
    if let Err(error) = emitter
        .emit(EventRequest::new(
            session_id,
            EventContext::from_execution_context(context),
            CapabilityUsageData { records },
        ))
        .await
    {
        tracing::warn!(
            session_id = %session_id,
            error = %error,
            "ReasonAtom: failed to emit capability.usage event"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_emits_its_snapshot_again_only_when_it_changes() {
        let mut memo = SnapshotMemo::default();
        let turn = (SessionId::new(), crate::engine::typed_id::TurnId::new());
        let other = (SessionId::new(), crate::engine::typed_id::TurnId::new());
        assert!(memo.changed(turn, 1), "the turn's first call emits");
        assert!(!memo.changed(turn, 1), "an unchanged snapshot does not");
        assert!(memo.changed(turn, 2), "a tool exposed mid-turn does");
        assert!(memo.changed(other, 2), "another turn emits its own");
    }

    #[test]
    fn the_snapshot_memo_forgets_the_oldest_turns() {
        let mut memo = SnapshotMemo::default();
        let first = (SessionId::new(), crate::engine::typed_id::TurnId::new());
        memo.changed(first, 1);
        for _ in 0..SNAPSHOT_MEMO_TURNS {
            memo.changed(
                (SessionId::new(), crate::engine::typed_id::TurnId::new()),
                1,
            );
        }
        assert_eq!(memo.last.len(), SNAPSHOT_MEMO_TURNS);
        assert!(memo.changed(first, 1), "a forgotten turn emits again");
    }
    use crate::engine::llm_conversions::llm_call_config_builder_from_agent;
    use crate::runtime_agent::RuntimeAgent;
    use everruns_contracts::model::ReasoningEffort;

    #[test]
    fn wire_retention_overrides_model_intent() {
        let mut config = crate::engine::driver_registry::LlmCallConfig::new("gpt-6.1-sol");
        config.prompt_cache = Some(crate::engine::driver_registry::PromptCacheConfig {
            enabled: true,
            retention: crate::engine::driver_registry::PromptCacheRetention::TwelveHours,
            ..Default::default()
        });
        for (body, expected) in [
            (json!({"prompt_cache_options":{"ttl":"12h"}}), Some("12h")),
            (json!({"prompt_cache_options":{"ttl":"30m"}}), Some("30m")),
            (json!({}), None),
        ] {
            let mut options = build_request_options(&config, "openai").unwrap();
            record_wire_cache_retention(&mut options, Some(&body), "openai");
            assert_eq!(options.prompt_cache.unwrap().retention.as_deref(), expected);
        }
    }

    #[test]
    fn wire_cache_defaults_are_recorded_without_a_capability() {
        let config = crate::engine::driver_registry::LlmCallConfig::new("gpt-6.1-sol");
        let mut options = build_request_options(&config, "openai").unwrap_or_default();
        assert!(options.prompt_cache.is_none());
        record_wire_cache_retention(&mut options, Some(&json!({})), "openai");
        assert!(options.prompt_cache.is_none());
        let body = json!({"prompt_cache_options":{"ttl":"12h", "mode":"implicit"}});
        record_wire_cache_retention(&mut options, Some(&body), "openai");
        let cache = options.prompt_cache.unwrap();
        assert!(cache.enabled);
        assert_eq!(cache.retention.as_deref(), Some("12h"));
        assert_eq!(cache.provider_mode.as_deref(), Some("implicit"));
        assert_eq!(
            cache.strategy,
            everruns_contracts::driver_registry::PromptCacheStrategy::Auto
        );
    }

    #[test]
    fn request_options_capture_sampling_and_streaming_intent() {
        let agent = RuntimeAgent {
            model: "claude-sonnet-4-6".to_string(),
            temperature: Some(0.2),
            max_tokens: Some(1024),
            ..RuntimeAgent::default()
        };
        let config = llm_call_config_builder_from_agent(&agent)
            .reasoning_effort(ReasoningEffort::High)
            .build();

        let options = build_request_options(&config, "anthropic").expect("options are recorded");
        assert_eq!(options.temperature, Some(0.2));
        assert_eq!(options.max_tokens, Some(1024));
        assert_eq!(options.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(options.stream, Some(true));
    }
}
