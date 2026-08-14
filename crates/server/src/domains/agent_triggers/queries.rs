// Agent-triggers domain queries — shared read/mapping helpers.

use crate::domains::common::{CommandError, classify_anyhow};
use crate::errors::ResourceNotFoundError;
use crate::storage::StorageBackend;
use crate::storage::models::{AgentRow, AgentTriggerRow};
use everruns_platform::{AgentTrigger, AgentTriggerType};
use everruns_provider::typed_id::{AgentId, TriggerId};
use std::sync::Arc;

/// Map a storage row into the core [`AgentTrigger`].
///
/// `AgentTriggerRow.agent_id` is the internal agent FK; API DTOs must carry
/// the caller-facing agent public id.
pub fn row_to_trigger(row: AgentTriggerRow, agent_public_id: AgentId) -> AgentTrigger {
    AgentTrigger {
        id: row.id,
        agent_id: agent_public_id,
        trigger_type: AgentTriggerType::from(row.trigger_type.as_str()),
        config: row.config,
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

/// Fetch a trigger by id, scoped to the org.
pub async fn get_by_id(
    db: &Arc<StorageBackend>,
    org_id: i64,
    id: TriggerId,
) -> Result<Option<AgentTriggerRow>, CommandError> {
    db.get_agent_trigger(org_id, id)
        .await
        .map_err(classify_anyhow)
}

/// Validate that `agent_id` names an active agent in `org_id` and return the
/// row (callers need its harness + internal id). Mirrors `apps::validate_agent`.
pub async fn require_active_agent(
    db: &Arc<StorageBackend>,
    org_id: i64,
    agent_id: &AgentId,
) -> Result<AgentRow, CommandError> {
    let row = db
        .get_agent_by_public_id(org_id, &agent_id.to_string())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| classify_anyhow(ResourceNotFoundError::new("Agent").into()))?;
    if row.status != "active" {
        return Err(CommandError::bad_request(
            "Archived or deleted agents cannot own triggers",
        ));
    }
    Ok(row)
}
