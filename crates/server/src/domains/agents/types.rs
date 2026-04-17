// Agent domain types — re-exports from existing locations.
//
// During migration, request types still live in api/agents.rs and storage
// row types in storage/models.rs. This module re-exports them so domain
// code has a single import path. Once all callers are migrated, types
// will move here.

pub use crate::api::agents::{
    CheckAgentNameQuery, CheckAgentNameResponse, CreateAgentRequest, UpdateAgentRequest,
};
pub use crate::storage::models::{AgentRow, CreateAgentRow, UpdateAgent};
