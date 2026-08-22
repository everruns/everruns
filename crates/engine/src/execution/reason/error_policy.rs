use crate::capabilities::CapabilityRegistry;
use crate::message::{Message, MessageRole};
use crate::{ErrorDisclosure, user_facing_error_codes};

const ERROR_PLACEHOLDER_MESSAGES: &[&str] = &[
    "I encountered an error while processing your request. Please try again later.",
    "The AI provider is experiencing issues. Please try again shortly.",
    "Rate limited by the AI provider. Please wait a moment.",
    "The conversation has become too long for the model to process. Please start a new session or reduce the context size.",
    "There is a misconfiguration with the AI provider. Please contact support.",
    "No model is configured for this chat. Choose a model or configure a default model, then try again.",
    "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue.",
];

pub(super) fn is_error_placeholder_message(msg: &Message) -> bool {
    if msg.role != MessageRole::Agent {
        return false;
    }
    // Must have no tool calls (pure text-only error message)
    if msg.has_tool_calls() {
        return false;
    }
    if let Some(metadata) = &msg.metadata
        && let Some(serde_json::Value::String(code)) = metadata.get("error_code")
    {
        return matches!(
            code.as_str(),
            user_facing_error_codes::BUDGET_EXHAUSTED
                | user_facing_error_codes::BUDGET_PAUSED
                | user_facing_error_codes::MODEL_UNAVAILABLE
                | user_facing_error_codes::MODEL_NOT_CONFIGURED
                | user_facing_error_codes::REQUEST_TOO_LARGE
                | user_facing_error_codes::PROVIDER_RATE_LIMITED
                | user_facing_error_codes::PROVIDER_USAGE_LIMIT_REACHED
                | user_facing_error_codes::PROVIDER_MISCONFIGURED
                | user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED
                | user_facing_error_codes::PROVIDER_UNAVAILABLE
                | user_facing_error_codes::DEPENDENCY_UNAVAILABLE
                | user_facing_error_codes::PROCESSING_ERROR
        );
    }
    let text = msg.text().unwrap_or("");
    ERROR_PLACEHOLDER_MESSAGES.contains(&text) || is_dynamic_error_placeholder(text)
}

/// Per-message error-disclosure override from the most recent user message's
/// controls (mirrors how reasoning effort is resolved). The value is clamped
/// against the capability-configured ceiling in `resolve_error_disclosure`.
pub(super) fn error_disclosure_override(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::User)?
        .controls
        .as_ref()?
        .error_disclosure
        .clone()
}

/// Resolve the message-requested disclosure against the ceiling contributed
/// by the active capability set. The engine consumes the contribution through
/// the neutral `Capability` contract and does not know the implementation ID.
pub(super) fn resolve_error_disclosure(
    registry: &CapabilityRegistry,
    configs: &[crate::CapabilityRef],
    requested: Option<&str>,
) -> ErrorDisclosure {
    let ceiling = configs
        .iter()
        .find_map(|config| {
            registry
                .get(config.capability_id())?
                .error_disclosure(config.config_value())
        })
        .unwrap_or_default();

    // THREAT[TM-LLM-024]: client controls may narrow disclosure but never
    // widen it beyond the operator-selected capability ceiling.
    requested
        .and_then(ErrorDisclosure::parse)
        .map_or(ceiling, |requested| requested.min(ceiling))
}

pub(super) fn filter_response_text(
    registry: &CapabilityRegistry,
    configs: &[crate::CapabilityRef],
    mut text: String,
) -> String {
    for config in configs {
        if let Some(capability) = registry.get(config.capability_id()) {
            text = capability.filter_response_text(text, config.config_value());
        }
    }
    text
}

fn is_dynamic_error_placeholder(text: &str) -> bool {
    (text.starts_with("Budget exhausted.") && text.ends_with("Increase the budget to continue."))
        || (text.starts_with("Budget paused.")
            && text.ends_with("Increase or resume the budget to continue."))
        || (text.starts_with("Budget paused with ")
            && text.ends_with("Increase or resume the budget to continue."))
        || (text.starts_with("Soft limit reached.") && text.ends_with("soft limit."))
        || (text.starts_with("The model `") && text.ends_with("Please select a different model."))
}
