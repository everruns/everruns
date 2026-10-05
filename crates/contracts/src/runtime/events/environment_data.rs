//! Payloads for managed Environment recovery events.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Non-secret lifecycle details for a managed Environment incarnation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EnvironmentLifecycleData {
    /// Durable logical Environment identifier, when hosted persistence is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    /// Compute provider selected by the Environment profile.
    pub provider: String,
    /// Physical provider resource that was lost or replaced.
    pub previous_instance_id: String,
    /// Replacement physical resource. Present only on `environment.recovered`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_instance_id: Option<String>,
    /// Current incarnation fence, when hosted persistence is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<i64>,
    /// Provider process state is ephemeral and is not restored across replacement.
    pub process_state_lost: bool,
}
