//! Mistral's stateless Responses preview. Keep Chat Completions as the default
//! until callers explicitly choose the preview through driver options.

use std::sync::Arc;

use everruns_contracts::OpenResponsesProtocolChatDriver;
use everruns_contracts::driver_registry::{LlmCallConfig, LlmContentPart, Message, MessageContent};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::openresponses_protocol::OpenResponsesRequestExtension;
use serde_json::Value;

/// Set to `true` in `LlmCallConfig::driver_options` to use `/v1/responses`.
/// Absent or `false` preserves Chat Completions, including native JSON calls.
pub const MISTRAL_RESPONSES_OPTION: &str = "mistral/responses";

pub(super) fn enabled(config: &LlmCallConfig) -> Result<bool> {
    match config.driver_options.get(MISTRAL_RESPONSES_OPTION) {
        None | Some(Value::Bool(false)) => Ok(false),
        Some(Value::Bool(true)) => Ok(true),
        Some(_) => Err(AgentLoopError::Configuration(
            "mistral/responses must be a boolean".into(),
        )),
    }
}

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
        // The shared content contract stores both URLs and data URLs in `url`.
        // Mistral distinguishes file_url from file_data on the wire.
        if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
            for item in input {
                for key in ["content", "output"] {
                    if let Some(parts) = item.get_mut(key).and_then(Value::as_array_mut) {
                        for part in parts {
                            if part["type"] == "input_file"
                                && let Some(object) = part.as_object_mut()
                                && let Some(Value::Object(mut file)) = object.remove("input_file")
                            {
                                if let Some(Value::String(url)) = file.get("file_data")
                                    && url.starts_with("https://")
                                {
                                    let url = url.clone();
                                    file.remove("file_data");
                                    file.insert("file_url".into(), Value::String(url));
                                }
                                object.extend(file);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
