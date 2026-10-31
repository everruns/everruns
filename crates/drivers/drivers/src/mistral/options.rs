//! Typed per-call Mistral configuration, kept in the provider crate.

use everruns_contracts::driver_registry::{LlmCallConfig, LlmCallConfigBuilder};

use super::MISTRAL_RESPONSES_OPTION;

/// Mistral wire API used for a model call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MistralApi {
    /// Chat Completions, the default when no API is selected.
    #[default]
    ChatCompletions,
    /// Stateless Responses preview, with full conversation replay.
    Responses,
}

/// Typed Mistral options for a call config or its builder.
///
/// ```
/// use everruns_contracts::LlmCallConfig;
/// use everruns_drivers::mistral::{MistralApi, MistralConfigExt};
///
/// let config = LlmCallConfig::new("mistral-large-4")
///     .with_mistral_api(MistralApi::Responses);
/// ```
///
/// Only the Mistral driver interprets this selection. It does not change the
/// provider, endpoint, or credentials attached to the call.
pub trait MistralConfigExt: Sized {
    /// Select the API, replacing any previous Mistral API selection.
    fn with_mistral_api(self, api: MistralApi) -> Self;
}

impl MistralConfigExt for LlmCallConfig {
    fn with_mistral_api(mut self, api: MistralApi) -> Self {
        self.driver_options.insert(
            MISTRAL_RESPONSES_OPTION.into(),
            (api == MistralApi::Responses).into(),
        );
        self
    }
}

impl MistralConfigExt for LlmCallConfigBuilder {
    fn with_mistral_api(self, api: MistralApi) -> Self {
        self.driver_option(
            MISTRAL_RESPONSES_OPTION,
            (api == MistralApi::Responses).into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_selection_replaces_previous_value_and_preserves_other_options() {
        let mut config = LlmCallConfig::new("test-model");
        config
            .driver_options
            .insert("other/option".into(), 7.into());
        config
            .driver_options
            .insert(MISTRAL_RESPONSES_OPTION.into(), "invalid".into());
        let config = config.with_mistral_api(MistralApi::Responses);
        assert!(super::super::responses::enabled(&config).unwrap());
        let config = config.with_mistral_api(MistralApi::ChatCompletions);
        assert!(!super::super::responses::enabled(&config).unwrap());
        assert_eq!(config.driver_options["other/option"], 7);
        assert_eq!(config.model, "test-model");
    }

    #[test]
    fn builder_supports_both_apis_and_preserves_call_settings() {
        for api in [MistralApi::default(), MistralApi::Responses] {
            let config = LlmCallConfigBuilder::from_config(LlmCallConfig::new("test-model"))
                .max_tokens(123)
                .with_mistral_api(api)
                .build();
            assert_eq!(
                super::super::responses::enabled(&config).unwrap(),
                api == MistralApi::Responses
            );
            assert_eq!(config.max_tokens, Some(123));
        }
    }
}
