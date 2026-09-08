use serde_json::json;
use std::collections::{BTreeSet, HashMap};

use crate::capabilities::CapabilityRegistry;
use crate::events::{
    CapabilityUsageKind, CapabilityUsageRecord, LlmPromptCacheInfo, LlmRequestOptions,
    LlmToolSearchInfo,
};
use crate::tool_types::ToolDefinition;

pub(super) fn build_request_options(
    config: &crate::driver_registry::LlmCallConfig,
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
                "openai" => Some("prompt_cache_key".to_string()),
                "anthropic" => Some("cache_control".to_string()),
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
    if provider == "openai" && config.previous_response_id.is_some() {
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

fn capability_name_snapshot(registry: &CapabilityRegistry, capability_id: &str) -> Option<String> {
    registry
        .get(capability_id)
        .map(|capability| capability.name().to_string())
}

pub(super) fn capability_usage_snapshot_records(
    registry: &CapabilityRegistry,
    resolved_capability_configs: &[crate::CapabilityRef],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_conversions::llm_call_config_builder_from_agent;
    use crate::model::ReasoningEffort;
    use everruns_core::runtime_agent::RuntimeAgent;

    #[test]
    fn request_options_capture_sampling_and_streaming_intent() {
        let agent = RuntimeAgent {
            model: "claude-sonnet-4-5".to_string(),
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
