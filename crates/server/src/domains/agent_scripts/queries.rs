// Agent-scripts domain queries — shared read/mapping helpers.

use crate::domains::common::{CommandError, classify_anyhow};
use crate::errors::ResourceNotFoundError;
use crate::records::AgentScript;
use crate::storage::StorageBackend;
use crate::storage::models::{AgentRow, AgentScriptRow};
use everruns_contracts::typed_id::{AgentId, ScriptId};
use std::sync::Arc;

/// Map a storage row into the [`AgentScript`] record.
///
/// `AgentScriptRow.agent_id` is the internal agent FK; API DTOs carry the
/// caller-facing agent public id.
pub fn row_to_script(row: AgentScriptRow, agent_public_id: AgentId) -> AgentScript {
    AgentScript {
        id: row.id,
        agent_id: agent_public_id,
        name: row.name,
        description: row.description,
        input_schema: row.input_schema,
        body: row.body,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

/// Validate that `agent_id` names an active agent in `org_id` and return the
/// row (callers need its internal id).
pub async fn require_active_agent(
    db: &Arc<StorageBackend>,
    org_id: i64,
    agent_id: &AgentId,
) -> Result<AgentRow, CommandError> {
    let row = db
        .get_agent_by_public_id(org_id, &agent_id.to_string())
        .await?
        .ok_or_else(|| classify_anyhow(ResourceNotFoundError::new("Agent").into()))?;
    if row.status != "active" {
        return Err(CommandError::bad_request(
            "Archived or deleted agents cannot own scripts",
        ));
    }
    Ok(row)
}

pub fn parse_agent_id(raw: &str) -> Result<AgentId, CommandError> {
    raw.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))
}

fn parse_script_id(raw: &str) -> Result<ScriptId, CommandError> {
    raw.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid script ID: {e}")))
}

/// Resolve a script and confirm it belongs to the named agent (org-scoped).
pub async fn resolve_script_for_agent(
    db: &Arc<StorageBackend>,
    org_id: i64,
    agent_id: &str,
    script_id: &str,
) -> Result<(AgentRow, AgentScriptRow), CommandError> {
    let agent_public = parse_agent_id(agent_id)?;
    let agent = require_active_agent(db, org_id, &agent_public).await?;
    let script_id = parse_script_id(script_id)?;
    let script = db
        .get_agent_script(org_id, script_id)
        .await?
        .filter(|row| row.status != "deleted")
        .ok_or_else(|| CommandError::not_found("Agent script"))?;
    if script.agent_id != agent.id {
        return Err(CommandError::not_found("Agent script"));
    }
    Ok((agent, script))
}
