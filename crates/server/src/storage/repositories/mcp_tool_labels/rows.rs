// Rows the MCP tool label repository reads and writes.

use chrono::{DateTime, Utc};
use everruns_server_macros::Columns;
use sqlx::FromRow;
use uuid::Uuid;

/// A saved risk label (and any suggestion) for one tool of an MCP server.
#[derive(Debug, Clone, FromRow, Columns)]
pub struct McpToolLabelRow {
    pub id: Uuid,
    pub org_id: i64,
    pub mcp_server_id: Uuid,
    pub tool_name: String,
    /// `read_only`, `changes`, or none.
    pub label: Option<String>,
    /// An automated suggestion a person has not confirmed. Nothing writes it yet.
    pub suggested_label: Option<String>,
    pub set_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
