use everruns_core::{Capability, CapabilityStatus};
use std::sync::Arc;

/// Turn-local view that preserves a capability's message filtering while
/// suppressing every model-visible contribution.
pub(super) struct MessageFilterOnlyCapability(pub(super) Arc<dyn Capability>);

impl Capability for MessageFilterOnlyCapability {
    fn id(&self) -> &str {
        self.0.id()
    }

    fn aliases(&self) -> Vec<&'static str> {
        self.0.aliases()
    }

    fn name(&self) -> &str {
        self.0.name()
    }

    fn description(&self) -> &str {
        self.0.description()
    }

    fn status(&self) -> CapabilityStatus {
        self.0.status()
    }

    fn message_filter_provider(
        &self,
    ) -> Option<Arc<dyn everruns_core::message_filter::MessageFilterProvider>> {
        self.0.message_filter_provider()
    }

    fn message_filter_config(
        &self,
        config: &serde_json::Value,
        compaction_enabled: bool,
        provider_managed_reduction: bool,
    ) -> serde_json::Value {
        self.0
            .message_filter_config(config, compaction_enabled, provider_managed_reduction)
    }
}
