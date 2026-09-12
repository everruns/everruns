//! OpenAI model constraints checked before a request reaches the network.

#[cfg(feature = "http")]
use crate::driver_registry::LlmCallConfig;
#[cfg(feature = "http")]
use crate::error::{AgentLoopError, Result};
#[cfg(feature = "http")]
use crate::model_profiles::get_model_profile;
use crate::provider::DriverId;
#[cfg(feature = "http")]
use crate::runtime_provider::ProviderEndpoint;
#[cfg(feature = "http")]
use serde_json::Value;

pub fn supports_cache_options(model: &str) -> bool {
    crate::model_profiles::get_model_profile_key(&DriverId::OpenAI, model).is_some_and(|key| {
        matches!(
            key.as_str(),
            "openai/gpt-6-astra"
                | "openai/gpt-5.6-sol"
                | "openai/gpt-5.6-terra"
                | "openai/gpt-5.6-luna"
        )
    })
}

#[cfg(feature = "http")]
pub(crate) fn validate_config(config: &LlmCallConfig) -> Result<()> {
    if let Some(profile) = get_model_profile(&DriverId::OpenAI, &config.model) {
        if let (Some(effort), Some(allowed)) = (config.reasoning_effort, profile.reasoning_effort)
            && !allowed.values.iter().any(|option| option.value == effort)
        {
            return Err(AgentLoopError::Configuration(format!(
                "Reasoning effort '{}' is unsupported by {}",
                effort.as_str(),
                config.model
            )));
        }
        if config.temperature.is_some() && profile.family == "gpt-6-astra" {
            return Err(AgentLoopError::Configuration(format!(
                "temperature is unsupported by {}",
                config.model
            )));
        }
    }
    Ok(())
}

#[cfg(feature = "http")]
pub(crate) fn validate_body(
    body: &Value,
    endpoint: &ProviderEndpoint,
    responses: bool,
) -> Result<()> {
    let astra = body
        .get("model")
        .and_then(Value::as_str)
        .and_then(|model| crate::model_profiles::get_model_profile_key(&DriverId::OpenAI, model))
        .is_some_and(|key| key == "openai/gpt-6-astra");
    if !astra {
        return Ok(());
    }
    let invalid = |message: &str| AgentLoopError::Configuration(message.to_string());
    for key in ["temperature", "top_p", "top_logprobs"] {
        if body.get(key).is_some_and(|v| !v.is_null()) {
            return Err(invalid(&format!("GPT-6 Astra does not support {key}")));
        }
    }
    if !responses {
        let has_tools = body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty());
        let has_tool_history =
            body.get("messages")
                .and_then(Value::as_array)
                .is_some_and(|messages| {
                    messages.iter().any(|m| {
                        m.get("role").and_then(Value::as_str) == Some("tool")
                            || m.get("tool_calls").is_some_and(|v| !v.is_null())
                    })
                });
        if has_tools || has_tool_history {
            return Err(invalid(
                "GPT-6 Astra tool calling requires the Responses API",
            ));
        }
        if body.get("logprobs").is_some_and(|v| !v.is_null()) {
            return Err(invalid("GPT-6 Astra does not support logprobs"));
        }
    } else if body
        .get("include")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|v| v == "message.output_text.logprobs"))
    {
        return Err(invalid(
            "GPT-6 Astra does not support message.output_text.logprobs",
        ));
    }
    // Residency follows the configured processing endpoint, never a model name,
    // user locale, or a substring in an unrelated gateway URL.
    let eu = endpoint
        .base_url()
        .and_then(|u| url::Url::parse(u).ok())
        .is_some_and(|u| {
            u.host_str().is_some_and(|host| {
                host.trim_end_matches('.')
                    .eq_ignore_ascii_case("eu.api.openai.com")
            })
        });
    if eu
        && matches!(
            body.get("service_tier").and_then(Value::as_str),
            Some("fast" | "priority")
        )
    {
        return Err(invalid(
            "GPT-6 Astra requires Standard processing with EU data residency",
        ));
    }
    Ok(())
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use super::*;
    use crate::{OpenAIProtocolChatDriver, Provider, ReasoningEffort};
    use serde_json::json;

    #[test]
    fn astra_compatibility_checks_endpoint_and_wire_shape() {
        let eu = Provider::new("openai", OpenAIProtocolChatDriver::new())
            .base_url("https://eu.api.openai.com/v1");
        let eu_absolute = Provider::new("openai", OpenAIProtocolChatDriver::new())
            .base_url("https://eu.api.openai.com./v1");
        let global = Provider::new("openai", OpenAIProtocolChatDriver::new())
            .base_url("https://api.openai.com/v1");
        let unrelated = Provider::new("custom", OpenAIProtocolChatDriver::new())
            .base_url("https://eu.api.openai.com.example/v1");
        for tier in ["fast", "priority"] {
            let body = json!({"model":"gpt-6-astra", "service_tier":tier});
            assert!(validate_body(&body, eu.endpoint(), true).is_err());
            assert!(validate_body(&body, eu_absolute.endpoint(), true).is_err());
            assert!(validate_body(&body, global.endpoint(), true).is_ok());
            assert!(validate_body(&body, unrelated.endpoint(), true).is_ok());
        }
        for body in [
            json!({"model":"gpt-6-astra","tools":[{"type":"function"}]}),
            json!({"model":"gpt-6-astra","messages":[{"role":"tool","content":"done"}]}),
        ] {
            assert!(validate_body(&body, global.endpoint(), false).is_err());
            assert!(validate_body(&body, global.endpoint(), true).is_ok());
        }
        for field in ["temperature", "top_p", "top_logprobs", "logprobs"] {
            let mut body = json!({"model":"gpt-6-astra"});
            body[field] = json!(0);
            assert!(validate_body(&body, global.endpoint(), false).is_err());
        }
        assert!(
            validate_body(
                &json!({"model":"gpt-6-astra","include":["message.output_text.logprobs"]}),
                global.endpoint(),
                true
            )
            .is_err()
        );
        assert!(
            validate_body(
                &json!({"model":"gpt-6-astra","service_tier":"default"}),
                eu.endpoint(),
                true
            )
            .is_ok()
        );
        assert!(
            validate_body(
                &json!({"model":"gpt-5.5","service_tier":"priority","tools":[{}]}),
                eu.endpoint(),
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn reasoning_effort_is_validated_before_none_is_filtered() {
        let mut config = LlmCallConfig {
            reasoning_state: None,
            speed: None,
            verbosity: None,
            model: "gpt-6-astra".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            driver_options: Default::default(),
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };
        for effort in [ReasoningEffort::None, ReasoningEffort::Minimal] {
            config.reasoning_effort = Some(effort);
            assert!(validate_config(&config).is_err());
        }
        for effort in [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
            ReasoningEffort::Max,
        ] {
            config.reasoning_effort = Some(effort);
            assert!(validate_config(&config).is_ok());
        }
        config.temperature = Some(1.0);
        assert!(validate_config(&config).is_err());
        config.model = "gpt-5.5".into();
        config.reasoning_effort = Some(ReasoningEffort::None);
        config.temperature = None;
        assert!(validate_config(&config).is_ok());
    }
}
