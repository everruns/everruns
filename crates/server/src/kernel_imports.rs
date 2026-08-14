//! Private import namespace for the server's kernel-facing implementation.
//!
//! This is not a public compatibility surface. It keeps server modules focused
//! on control-plane behavior while preserving the actual ownership split:
//! execution contracts come from `everruns-core`, capability identity/config
//! comes from `everruns-capability`, and provider/model/ID types come from
//! `everruns-provider`.

pub(crate) use everruns_capability::{
    CapabilityId, CapabilityRef as AgentCapabilityConfig, is_plugin_capability,
    parse_plugin_capability_id, plugin_capability_id,
};

#[cfg(test)]
pub(crate) use ::everruns_provider::compact::CompactOutputItem;
#[cfg(test)]
pub(crate) use ::everruns_provider::driver_registry::{
    LlmCompletionMetadata, LlmResponse, LlmResponseStream, ProviderOpaqueContext,
};
#[cfg(test)]
pub(crate) use ::everruns_provider::error::{AgentLoopError, Result};
#[cfg(test)]
pub(crate) use ::everruns_provider::provider::{DriverId, ProviderTraceConfig};
#[cfg(test)]
pub(crate) use ::everruns_provider::tool_types::ToolCall;
#[cfg(test)]
pub(crate) use ::everruns_provider::typed_id;
#[cfg(test)]
pub(crate) use ::everruns_provider::typed_id::{
    AgentIdentityId, HarnessId, MessageId, ModelId, PrincipalId, SessionId, TurnId,
};
pub(crate) use everruns_core::*;

pub(crate) mod everruns_provider {
    pub(crate) use ::everruns_provider::{
        driver_registry, error, model, model_profiles, model_spec, openresponses_types, provider,
        tool_types, typed_id, url_validation, user_facing_error,
    };
}
