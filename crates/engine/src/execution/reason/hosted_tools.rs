//! Provider-executed tools on the request (OpenRouter server tools, OpenAI
//! hosted tools). The payload shapes belong to the driver crates; the engine
//! only sniffs the opaque `driver_options` so the `engine -> core/provider`
//! dependency direction holds.

use std::collections::HashMap;

use everruns_provider::openai_hosted_tools::{HOSTED_TOOLS_DRIVER_IDS, OPENAI_HOSTED_TOOLS_OPTION};
use serde_json::Value;

use crate::error::{AgentLoopError, Result};

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
