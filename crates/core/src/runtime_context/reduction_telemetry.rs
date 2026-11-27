//! Runtime-only reduction observations. Never part of a provider call config.

/// Why the runtime used portable reduction instead of provider-managed reduction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReductionFallbackReason {
    /// The selected provider or model does not support managed reduction.
    ProviderOrModelIneligible,
    /// Current conversation context requires portable history handling.
    DynamicConversationContext,
    /// The provider rejected the requested managed capability.
    ProviderCapabilityRejected,
    /// Drivers are open extensions; retain their diagnostic without making it an option.
    Driver(String),
}

impl From<&str> for ReductionFallbackReason {
    fn from(reason: &str) -> Self {
        match reason {
            "provider_or_model_ineligible" => Self::ProviderOrModelIneligible,
            "dynamic_conversation_context" => Self::DynamicConversationContext,
            "provider_capability_rejected" => Self::ProviderCapabilityRejected,
            other => Self::Driver(other.to_owned()),
        }
    }
}

impl ReductionFallbackReason {
    /// Stable diagnostic spelling retained in runtime event telemetry.
    pub fn as_str(&self) -> &str {
        match self {
            Self::ProviderOrModelIneligible => "provider_or_model_ineligible",
            Self::DynamicConversationContext => "dynamic_conversation_context",
            Self::ProviderCapabilityRejected => "provider_capability_rejected",
            Self::Driver(reason) => reason,
        }
    }
}

/// History representation used by a provider-managed request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReductionReplaySource {
    /// Original conversation history.
    Raw,
    /// A previously persisted compaction checkpoint.
    Checkpoint,
}

/// Kept beside the assembled runtime context, outside `LlmCallConfig` and its
/// provider options. The compiler prevents accidentally passing this as config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReductionTelemetry {
    /// Why provider-managed reduction fell back, if it did.
    pub fallback_reason: Option<ReductionFallbackReason>,
    /// The history representation selected for this request.
    pub replay_source: Option<ReductionReplaySource>,
    /// Whether the provider emitted a compaction block during this generation.
    pub compaction_observed: bool,
}

impl ReductionTelemetry {
    #[cfg(feature = "engine")]
    pub(crate) fn prepare_request(&mut self, managed: bool, checkpoint_restored: bool) {
        if managed {
            self.replay_source = Some(if checkpoint_restored {
                ReductionReplaySource::Checkpoint
            } else {
                ReductionReplaySource::Raw
            });
        }
    }

    #[cfg(any(feature = "engine", test))]
    pub(crate) fn decorate(&self, provider: &str, options: &mut crate::events::LlmRequestOptions) {
        if *self == Self::default() {
            return;
        }
        let value = options
            .provider_options
            .entry(provider.to_owned())
            .or_insert_with(|| serde_json::json!({}));
        if let Some(reason) = &self.fallback_reason {
            value["reduction_mode"] = serde_json::json!("legacy");
            value["fallback_reason"] = serde_json::json!(reason.as_str());
        }
        if let Some(source) = self.replay_source {
            value["replay_source"] = serde_json::json!(match source {
                ReductionReplaySource::Raw => "raw",
                ReductionReplaySource::Checkpoint => "checkpoint",
            });
        }
        if self.compaction_observed {
            value["compaction_block_observed"] = serde_json::json!(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_observations_remain_attributable_for_builtin_and_custom_providers() {
        let telemetry = ReductionTelemetry {
            fallback_reason: Some(ReductionFallbackReason::ProviderCapabilityRejected),
            replay_source: Some(ReductionReplaySource::Checkpoint),
            compaction_observed: true,
        };
        for provider in [
            "anthropic",
            "openai",
            "openrouter",
            "gemini",
            "custom-framework-provider",
        ] {
            let mut options = crate::events::LlmRequestOptions::default();
            telemetry.decorate(provider, &mut options);
            assert_eq!(
                options.provider_options[provider]["fallback_reason"],
                "provider_capability_rejected"
            );
            assert_eq!(
                options.provider_options[provider]["replay_source"],
                "checkpoint"
            );
            assert_eq!(options.provider_options.len(), 1);
        }
    }
}
