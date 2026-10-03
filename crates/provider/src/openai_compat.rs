//! OpenAI model constraints checked before a request reaches the network.

#[cfg(feature = "http")]
use crate::driver_registry::LlmCallConfig;
#[cfg(feature = "http")]
use crate::error::{AgentLoopError, Result};
#[cfg(feature = "http")]
use crate::model_profiles::get_model_profile;
use crate::openai_protocol::{is_azure_openai_api_url, is_openai_api_url};
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
                | "openai/gpt-6-sol"
                | "openai/gpt-6-luna"
                | "openai/gpt-6.1-sol"
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
        // Tiers are model-gated at OpenAI; fail with a clear error instead of
        // a provider 400. `fast` and `priority` name the same tier; `default`
        // is always accepted.
        if let Some(speed) = config.speed.as_deref().filter(|speed| *speed != "default") {
            let offered = profile.speed.as_ref().is_some_and(|tiers| {
                tiers
                    .values
                    .iter()
                    .any(|tier| tier.value.matches_tier(speed))
            });
            if !offered {
                return Err(AgentLoopError::Configuration(format!(
                    "Speed '{speed}' is unsupported by {}",
                    config.model
                )));
            }
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
            Some("fast" | "priority" | "ultrafast")
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
        for tier in ["fast", "priority", "ultrafast"] {
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
            model: "gpt-6-astra".to_string(),
            ..Default::default()
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

    #[test]
    fn speed_is_gated_by_the_model_profile() {
        let check = |model: &str, speed: &str| {
            validate_config(&LlmCallConfig {
                model: model.to_string(),
                speed: Some(speed.to_string()),
                ..Default::default()
            })
        };
        for speed in ["flex", "default", "priority", "fast", "ultrafast"] {
            assert!(check("gpt-6-astra", speed).is_ok(), "{speed}");
        }
        for speed in ["flex", "default", "priority", "fast"] {
            assert!(check("gpt-6.1-sol", speed).is_ok(), "{speed}");
        }
        let err = check("gpt-6.1-sol", "ultrafast").unwrap_err().to_string();
        assert!(
            err.contains("Speed 'ultrafast' is unsupported by gpt-6.1-sol"),
            "{err}"
        );
        assert!(check("gpt-5.5-pro", "fast").is_err());
        // No tier rows: only the provider default is valid.
        assert!(check("gpt-5-nano", "default").is_ok());
        assert!(check("gpt-5-nano", "flex").is_err());
        // Models outside the registry are left to the provider.
        assert!(check("some-gateway-model", "ultrafast").is_ok());
    }
}

/// The Chat Completions URL for `endpoint`, or a configuration error naming the
/// missing base URL.
///
/// Both the streaming and non-streaming paths need this before they build a
/// request, because the host decides which output-cap field name the request
/// carries (see [`max_output_fields`]).
#[cfg(feature = "http")]
pub fn chat_completions_url(endpoint: &ProviderEndpoint) -> Result<String> {
    endpoint.url("chat/completions").ok_or_else(|| {
        AgentLoopError::Configuration(
            "OpenAI Chat Completions provider has no base URL".to_string(),
        )
    })
}

/// Split an output-token cap into the field name the endpoint accepts, as
/// `(max_tokens, max_completion_tokens)`.
///
/// OpenAI deprecated `max_tokens` on Chat Completions and current models reject
/// it outright: `gpt-6-luna` answers HTTP 400 "Unsupported parameter:
/// 'max_tokens' is not supported with this model. Use 'max_completion_tokens'
/// instead." Azure serves the same models through the same parameter rules.
///
/// Everything else on this protocol keeps `max_tokens`. Measured against live
/// accounts, Fireworks and Cloudflare accept *either* name and honour both
/// (a cap of 16 returns `finish_reason: length` at 16 completion tokens), so
/// switching them would buy nothing — while the self-hosted OpenAI-compatible
/// servers this driver also serves may only implement the original name.
/// Narrowing the change to the hosts that reject `max_tokens` keeps the blast
/// radius at exactly the endpoints that need it.
pub fn max_output_fields(api_url: &str, max_tokens: Option<u32>) -> (Option<u32>, Option<u32>) {
    if is_openai_api_url(api_url) || is_azure_openai_api_url(api_url) {
        (None, max_tokens)
    } else {
        (max_tokens, None)
    }
}

#[cfg(test)]
mod output_cap_tests {
    use super::max_output_fields;

    /// OpenAI and Azure reject `max_tokens` on current models; everything else
    /// on this protocol still takes it. Exactly one field is ever set, so a
    /// request never carries both names.
    #[test]
    fn the_output_cap_uses_the_field_name_the_endpoint_accepts() {
        for url in [
            "https://api.openai.com/v1/chat/completions",
            "https://my-resource.openai.azure.com/openai/v1/chat/completions",
            "https://my-resource.services.ai.azure.com/openai/v1/chat/completions",
        ] {
            assert_eq!(
                max_output_fields(url, Some(128)),
                (None, Some(128)),
                "OpenAI-family host should use max_completion_tokens: {url}"
            );
        }

        for url in [
            // Measured live: both accept either name and honour both, so the
            // original name stays.
            "https://api.fireworks.ai/inference/v1/chat/completions",
            "https://api.cloudflare.com/client/v4/accounts/acct/ai/v1/chat/completions",
            // A self-hosted OpenAI-compatible server may only implement
            // `max_tokens`; it is not ours to break.
            "http://localhost:8000/v1/chat/completions",
        ] {
            assert_eq!(
                max_output_fields(url, Some(128)),
                (Some(128), None),
                "non-OpenAI host should keep max_tokens: {url}"
            );
        }
    }

    /// No cap set means neither field is sent, on either branch.
    #[test]
    fn an_absent_cap_sends_neither_field() {
        assert_eq!(
            max_output_fields("https://api.openai.com/v1/chat/completions", None),
            (None, None)
        );
        assert_eq!(
            max_output_fields(
                "https://api.fireworks.ai/inference/v1/chat/completions",
                None
            ),
            (None, None)
        );
    }
}
