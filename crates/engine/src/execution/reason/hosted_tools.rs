//! Provider-executed tools on the request (OpenRouter server tools, OpenAI
//! hosted tools). The payload shapes belong to the driver crates; the engine
//! only sniffs the opaque `driver_options` so the `engine -> core/provider`
//! dependency direction holds.

use std::collections::HashMap;

use everruns_provider::DriverId;
use everruns_provider::driver_registry::{HostedToolCall, HostedToolCallStatus};
use everruns_provider::openai_hosted_tools::{
    HOSTED_TOOLS_DRIVER_IDS, OPENAI_HOSTED_TOOLS_OPTION, hosted_calls_cost_usd,
};
use serde_json::Value;

use crate::driver_registry::LlmCompletionMetadata;
use crate::error::{AgentLoopError, Result};
use crate::events::{EventContext, EventRequest, HostedToolCallData, TokenUsage};
use crate::typed_id::{SessionId, TurnId};

/// The `tool.hosted_call` event for one hosted call state change.
pub(super) fn hosted_call_event(
    session_id: SessionId,
    context: &EventContext,
    turn_id: TurnId,
    call: HostedToolCall,
) -> EventRequest {
    let status = match call.status {
        HostedToolCallStatus::InProgress => "in_progress",
        HostedToolCallStatus::Completed => "completed",
        HostedToolCallStatus::Failed => "failed",
    };
    EventRequest::new(
        session_id,
        context.clone(),
        HostedToolCallData {
            turn_id,
            call_id: call.id,
            tool_name: call.tool,
            status: status.to_string(),
            summary: call.summary,
        },
    )
}

/// Usage for one generation. Cost is tracked as two values: the provider's
/// authoritative inline cost when present (OpenRouter `usage.cost`), and a
/// price-table estimate from the model profile, plus hosted tool calls, which
/// bill per call on top of tokens. Keeping both lets consumers prefer the
/// actual charge while reconciling estimate-vs-actual drift.
pub(super) fn completion_usage(
    meta: &LlmCompletionMetadata,
    provider_type: &DriverId,
    model: &str,
) -> Option<TokenUsage> {
    let (input, output) = (meta.prompt_tokens?, meta.completion_tokens?);
    let tokens = crate::model_profiles::estimate_cost_usd(
        provider_type,
        model,
        input,
        output,
        meta.cache_read_tokens.unwrap_or(0),
        meta.cache_creation_tokens.unwrap_or(0),
    );
    let hosted = hosted_calls_cost_usd(&meta.hosted_tool_calls, model);
    let estimated = match (tokens, hosted) {
        (Some(tokens), Some(hosted)) => Some(tokens + hosted),
        (tokens, hosted) => tokens.or(hosted),
    };
    Some(
        TokenUsage::with_cache(
            input,
            output,
            meta.cache_read_tokens,
            meta.cache_creation_tokens,
        )
        .with_cost(meta.provider_cost_usd, estimated),
    )
}

/// Whether the provider runs tools inside this request. Those tools never
/// surface as agent tool calls, so reissuing the request can duplicate their
/// side effects even when the stream emitted only reasoning.
pub(super) fn has_provider_executed_tools(options: &HashMap<String, Value>) -> bool {
    let openrouter = options
        .get("openrouter/routing")
        .and_then(|raw| raw.get("server_tools"))
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty());
    openrouter || options.contains_key(OPENAI_HOSTED_TOOLS_OPTION)
}

/// Reject OpenAI hosted tools on a provider that cannot run them (EVE-1115).
/// Dropping them would let an agent configured for web search answer without
/// it and never say so.
pub(super) fn ensure_hosted_tools_supported(
    options: &HashMap<String, Value>,
    provider_type: &str,
) -> Result<()> {
    if !options.contains_key(OPENAI_HOSTED_TOOLS_OPTION)
        || HOSTED_TOOLS_DRIVER_IDS.contains(&provider_type)
    {
        return Ok(());
    }
    Err(AgentLoopError::Configuration(format!(
        "The OpenAI Server Tools capability needs an OpenAI or Azure OpenAI model, \
         but this agent runs on `{provider_type}`. Switch the model or remove the capability."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hosted() -> HashMap<String, Value> {
        HashMap::from([(
            OPENAI_HOSTED_TOOLS_OPTION.to_string(),
            json!({ "web_search": {} }),
        )])
    }

    #[test]
    fn hosted_tools_are_provider_executed() {
        assert!(has_provider_executed_tools(&hosted()));
        assert!(has_provider_executed_tools(&HashMap::from([(
            "openrouter/routing".to_string(),
            json!({ "server_tools": [{ "type": "openrouter:web_search" }] }),
        )])));
        assert!(!has_provider_executed_tools(&HashMap::from([(
            "openrouter/routing".to_string(),
            json!({ "server_tools": [] }),
        )])));
        assert!(!has_provider_executed_tools(&HashMap::new()));
    }

    #[test]
    fn hosted_calls_add_to_the_estimated_cost() {
        let mut meta = LlmCompletionMetadata::default();
        meta.prompt_tokens = Some(1_000);
        meta.completion_tokens = Some(100);
        let model = "gpt-6.1-sol";
        let tokens_only = completion_usage(&meta, &DriverId::OpenAI, model)
            .unwrap()
            .estimated_cost_usd;

        meta.hosted_tool_calls.insert("web_search_call".into(), 2);
        let with_search = completion_usage(&meta, &DriverId::OpenAI, model)
            .unwrap()
            .estimated_cost_usd
            .unwrap();
        let expected = tokens_only.unwrap_or(0.0) + 0.02;
        assert!(
            (with_search - expected).abs() < 1e-9,
            "{with_search} vs {expected}"
        );

        // An unknown model still prices its searches.
        let unknown = completion_usage(&meta, &DriverId::OpenAI, "gpt-unknown")
            .unwrap()
            .estimated_cost_usd
            .unwrap();
        assert!((unknown - 0.02).abs() < 1e-9, "{unknown}");
    }

    #[test]
    fn usage_needs_token_counts() {
        assert!(
            completion_usage(
                &LlmCompletionMetadata::default(),
                &DriverId::OpenAI,
                "gpt-6.1-sol"
            )
            .is_none()
        );
    }

    #[test]
    fn hosted_tools_run_only_on_openai_providers() {
        assert!(ensure_hosted_tools_supported(&hosted(), "openai").is_ok());
        assert!(ensure_hosted_tools_supported(&hosted(), "azure_openai").is_ok());
        assert!(ensure_hosted_tools_supported(&HashMap::new(), "anthropic").is_ok());

        let error = ensure_hosted_tools_supported(&hosted(), "anthropic")
            .expect_err("anthropic cannot run OpenAI hosted tools");
        assert!(matches!(error, AgentLoopError::Configuration(_)));
        assert!(error.to_string().contains("`anthropic`"), "{error}");
    }
}
