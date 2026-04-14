// Agent identity domain types — re-exports from existing locations.
//
// During migration, request types still live in api/agent_identities.rs and
// storage row types in storage/models.rs. This module re-exports them so
// domain code has a single import path. Once all callers are migrated,
// types will move here.

pub use crate::api::agent_identities::{
    CreateAgentIdentityRequest, ListAgentIdentitiesQuery, UpdateAgentIdentityRequest,
};
pub use crate::storage::models::{AgentIdentityRow, CreateAgentIdentityRow, UpdateAgentIdentity};
