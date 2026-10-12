//! Private import namespace for the server's kernel-facing implementation.
//!
//! This is not a public compatibility surface. It keeps server modules focused
//! on control-plane behavior while preserving the actual ownership split:
//! execution contracts come from `everruns-core`, capability identity/config
//! and provider/model/ID types come from `everruns-contracts`.

pub(crate) use ::everruns_contracts::{
    CapabilityId, CapabilityRef as AgentCapabilityConfig, is_plugin_capability,
    parse_plugin_capability_id, plugin_capability_id,
};

#[cfg(test)]
pub(crate) use ::everruns_contracts::compact::CompactOutputItem;
#[cfg(test)]
pub(crate) use ::everruns_contracts::driver_registry::{
    LlmCompletionMetadata, LlmResponse, LlmResponseStream, ProviderOpaqueContext,
};
#[cfg(test)]
pub(crate) use ::everruns_contracts::error::{AgentLoopError, Result};
#[cfg(test)]
pub(crate) use ::everruns_contracts::provider::{DriverId, ProviderTraceConfig};
#[cfg(test)]
pub(crate) use ::everruns_contracts::tool_types::ToolCall;
#[cfg(test)]
pub(crate) use ::everruns_contracts::typed_id;
#[cfg(test)]
pub(crate) use ::everruns_contracts::typed_id::{
    HarnessId, MessageId, ModelId, PrincipalId, SessionId, TurnId, VirtualUserId,
};
pub(crate) use everruns_core::*;

pub(crate) mod contracts {
    // Persistence values are projected only at this private server boundary.
    pub(crate) mod model {
        pub(crate) use crate::domains::models::record::{Model, ModelSource, ModelWithProvider};
        pub(crate) use everruns_contracts::model::*;
    }
    pub(crate) mod provider {
        pub(crate) use crate::domains::providers::record::ProviderStatus;
        pub(crate) use everruns_contracts::provider::*;
    }

    pub(crate) use ::everruns_contracts::{
        driver_registry, error, model_profiles, model_spec, openresponses_types, tool_types,
        typed_id, url_validation, user_facing_error,
    };
}

pub(crate) use crate::domains::mcp_servers::record::{McpServer, McpServerStatus};
pub(crate) use crate::domains::skills::record::{Skill, SkillSourceType, SkillStatus, SkillUsage};
pub(crate) use crate::domains::virtual_users::record::{VirtualUser, VirtualUserStatus};
