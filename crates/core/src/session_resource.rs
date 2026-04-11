// Session resource registry — unified view of all resources active in a session.
//
// Decision: SessionResource is a tagged enum that aggregates leased resources
// (sandboxes, browsers) and subagents under one API surface. Internal storage
// stays separate — leased resources in their own table, subagents in sessions.
// This type is the read-side projection for the API/UI.
// Decision: The "kind" tag drives UI rendering (different cards for different
// resource types). Each variant carries only the fields relevant to that kind.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::leased_resource::LeasedResource;
use crate::session::SubagentStatus;
use crate::typed_id::SessionId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Unified view of a session-scoped resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "kind")]
pub enum SessionResource {
    /// A lifecycle-managed external resource (sandbox, browser session, etc.)
    #[serde(rename = "leased")]
    Leased {
        #[serde(flatten)]
        resource: LeasedResource,
    },
    /// A child session performing delegated work.
    #[serde(rename = "subagent")]
    Subagent {
        /// Child session ID.
        #[cfg_attr(feature = "openapi", schema(value_type = String))]
        session_id: SessionId,
        /// Human-readable name ("Test Runner").
        name: String,
        /// Original task description.
        #[serde(skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        /// Lifecycle status.
        status: SubagentStatus,
        /// Blueprint ID if this subagent runs a blueprint.
        #[serde(skip_serializing_if = "Option::is_none")]
        blueprint_id: Option<String>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    },
}
