// MCP Server domain types — re-exports from existing locations.
//
// During migration, request types still live in api/mcp_servers.rs and storage
// row types in storage/models.rs. This module re-exports them so domain
// code has a single import path. Once all callers are migrated, types
// will move here.

pub use crate::api::mcp_servers::{CreateMcpServerRequest, UpdateMcpServerRequest};
pub use crate::storage::models::{CreateMcpServerRow, McpServerRow, UpdateMcpServer};
