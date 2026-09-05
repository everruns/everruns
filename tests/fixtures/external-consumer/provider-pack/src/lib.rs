//! External provider package: implements the `ChatDriver` contract and driver
//! registration against the provider SPI (`everruns-provider`) only — no
//! `everruns`, `everruns-core`, or `everruns-host` imports (EVE-874).

use async_trait::async_trait;
use everruns_provider::driver_registry::{
    ChatDriver, DriverRegistry, LlmCallConfig, LlmCompletionMetadata, LlmMessage,
    LlmResponseStream, LlmStreamEvent,
};
use everruns_provider::runtime_provider::{Provider, ProviderEndpoint};

/// Canonical wire id of the fixture's external driver.
pub const ACME_DRIVER_ID: &str = "acme_llm";

/// A scripted downstream chat driver: echoes the last user message back as a
/// streamed completion. Stands in for a real vendor wire protocol.
pub struct AcmeChatDriver;

#[async_trait]
impl ChatDriver for AcmeChatDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<LlmMessage>,
        _config: &LlmCallConfig,
    ) -> everruns_provider::Result<LlmResponseStream> {
        let last = messages
            .last()
            .map(|message| message.content.to_text())
            .unwrap_or_default();
        let events = vec![
            Ok(LlmStreamEvent::TextDelta(format!("acme:{last}"))),
            Ok(LlmStreamEvent::Done(Box::new(
                LlmCompletionMetadata::default(),
            ))),
        ];
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

/// Register the external driver into a [`DriverRegistry`] through the open
/// `register_external` seam — the same descriptor machinery the official
/// provider crates use, with no vendor/service branching in the SPI.
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_external(ACME_DRIVER_ID, |config| {
        Provider::new(config.provider.clone(), AcmeChatDriver)
            .base_url(config.base_url.as_deref().unwrap_or("https://acme.test"))
            .into_boxed_driver()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::{DriverId, LlmMessageRole, ProviderConfig};

    fn call_config() -> LlmCallConfig {
        LlmCallConfig {
            model: "acme-1".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
            reasoning_state: None,
        }
    }

    #[tokio::test]
    async fn external_driver_registers_and_completes() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);

        let id = DriverId::external(ACME_DRIVER_ID);
        assert!(registry.has_driver(&id));

        let driver = registry
            .create_chat_driver(&ProviderConfig::new(id))
            .expect("external driver must be constructible without an api key");
        let response = driver
            .chat_completion(
                &ProviderEndpoint::default(),
                vec![LlmMessage::text(LlmMessageRole::User, "ping")],
                &call_config(),
            )
            .await
            .expect("scripted completion succeeds");
        assert_eq!(response.text, "acme:ping");
    }
}
