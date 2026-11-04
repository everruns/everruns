//! Explicit opt-in to Anthropic's short-lived, non-ZDR conversation transport.
use super::{Capability, CapabilityLocalization, SystemPromptContext};
use async_trait::async_trait;
use serde_json::{Value, json};

/// Capability ID for optional Anthropic Messages thread transport.
pub const CLAUDE_MESSAGE_THREADS_CAPABILITY_ID: &str = "claude_message_threads";

/// Sends deltas on compatible Anthropic requests while keeping local history.
pub struct ClaudeMessageThreadsCapability;

#[async_trait]
impl Capability for ClaudeMessageThreadsCapability {
    fn id(&self) -> &str {
        CLAUDE_MESSAGE_THREADS_CAPABILITY_ID
    }
    fn name(&self) -> &str {
        "Claude Message Threads"
    }
    fn description(&self) -> &str {
        "Send only new messages on compatible Claude turns. Anthropic temporarily retains the conversation; not eligible for zero data retention or HIPAA. Full history remains in Everruns."
    }
    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }
    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Потоки повідомлень Claude",
            "Надсилає лише нові повідомлення в сумісних запитах Claude. Anthropic тимчасово зберігає розмову; не сумісно з нульовим зберіганням даних або HIPAA. Повна історія залишається в Everruns.",
        )]
    }
    fn driver_options(&self, _config: &Value) -> Vec<(String, Value)> {
        vec![("anthropic/message_threads".into(), json!(true))]
    }
    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opt_in_contributes_only_anthropic_transport_option() {
        let cap = ClaudeMessageThreadsCapability;
        assert_eq!(
            cap.driver_options(&Value::Null),
            vec![("anthropic/message_threads".into(), json!(true))]
        );
        assert!(cap.description().contains("zero data retention"));
    }
}
