//! Mistral's stateless Responses preview. Keep Chat Completions as the default
//! until a provider connection explicitly chooses the preview.

use std::sync::Arc;

use everruns_contracts::OpenResponsesProtocolChatDriver;
use everruns_contracts::driver_registry::{LlmCallConfig, LlmContentPart, Message, MessageContent};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::openresponses_protocol::OpenResponsesRequestExtension;
use serde_json::Value;

pub(super) fn driver() -> OpenResponsesProtocolChatDriver {
    OpenResponsesProtocolChatDriver::new()
        .with_stateful_responses(false)
        .with_required_completion(true)
        .with_plaintext_reasoning("mistral")
        .with_request_extension(Arc::new(MistralResponsesExtension))
}

pub(super) fn validate_messages(messages: &[Message]) -> Result<()> {
    for message in messages {
        if let MessageContent::Parts(parts) = &message.content
            && parts.iter().any(|part| {
                matches!(
                    part,
                    LlmContentPart::Audio { .. } | LlmContentPart::ProviderOpaque(_)
                )
            })
        {
            return Err(AgentLoopError::Configuration(
                "Mistral Responses supports text, image and file inputs; audio and opaque provider content are unsupported".into(),
            ));
        }
    }
    Ok(())
}

struct MistralResponsesExtension;

impl OpenResponsesRequestExtension for MistralResponsesExtension {
    fn supports_native_non_streaming(&self) -> bool {
        // The preview is qualified through SSE, including collected calls.
        false
    }

    fn decorate(&self, body: &mut Value, config: &LlmCallConfig) -> Result<()> {
        if config.provider_opaque_context.is_some() {
            return Err(AgentLoopError::Configuration(
                "Mistral Responses does not support provider-managed compaction".into(),
            ));
        }
        let Some(body) = body.as_object_mut() else {
            return Err(AgentLoopError::Configuration(
                "invalid Responses request".into(),
            ));
        };
        // Mistral is stateless and exposes plaintext reasoning. OpenAI's
        // encrypted artifact opt-in and summary selector are not its protocol.
        body.remove("include");
        body.remove("previous_response_id");
        body.remove("service_tier");
        body.insert("store".into(), Value::Bool(false));
        if let Some(reasoning) = body.get_mut("reasoning").and_then(Value::as_object_mut) {
            reasoning.remove("summary");
        }
        if let Some(text) = body.get_mut("text").and_then(Value::as_object_mut) {
            text.remove("verbosity");
            if text.is_empty() {
                body.remove("text");
            }
        }
        Ok(())
    }
}
