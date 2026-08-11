// Database-backed SessionStore implementation
//
// This module implements the core SessionStore trait for retrieving
// session configurations from the database.
//
// Decision: org_id and org_public_id are baked into the struct at
// construction time, matching the Grpc/Adapter store pattern.

use crate::max_iterations;
use async_trait::async_trait;
use everruns_core::{
    AgentLoopError, ExecutionSession, Result, SessionId, StoreResultExt, TokenUsage,
    traits::SessionStore,
};
use everruns_platform::{Session, SessionActivity, SessionSource, SessionStatus};

use super::repositories::Database;

// ============================================================================
// DbSessionStore - Retrieves sessions from the database
// ============================================================================

/// Database-backed session store
///
/// Retrieves session configurations from the database.
/// Used by ReasonAtom to load session data during workflow execution.
#[derive(Clone)]
pub struct DbSessionStore {
    db: Database,
    org_id: i64,
    org_public_id: String,
}

impl DbSessionStore {
    pub fn new(db: Database, org_id: i64, org_public_id: String) -> Self {
        Self {
            db,
            org_id,
            org_public_id,
        }
    }
}

impl DbSessionStore {
    /// Load the stored platform Session record (server-internal; EVE-882).
    ///
    /// The core `SessionStore` seam below projects this into the portable
    /// [`ExecutionSession`] — host execution never sees the stored record.
    pub async fn get_stored_session(&self, session_id: SessionId) -> Result<Option<Session>> {
        let session_row = self
            .db
            .get_session(self.org_id, session_id)
            .await
            .store_err()?;

        match session_row {
            Some(row) => {
                // Only resolve the built-in base harness when the row has no
                // explicit harness_id — avoids an unnecessary DB query on
                // every session fetch and prevents get_session from failing
                // when the base harness isn't provisioned for an org whose
                // session already carries an explicit harness.
                let harness_id = match row.harness_id {
                    Some(id) => id,
                    None => self
                        .db
                        .get_harness_by_name(self.org_id, "base")
                        .await
                        .store_err()?
                        .filter(|h| h.is_built_in)
                        .map(|h| h.id)
                        .ok_or_else(|| {
                            AgentLoopError::store(format!(
                                "base harness not provisioned for org {}",
                                self.org_id
                            ))
                        })?,
                };
                // Convert database usage columns to TokenUsage
                let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
                    // Actual and estimated cost totals are tracked separately; the
                    // aggregate carries each so consumers can prefer actual and
                    // reconcile drift.
                    Some(
                        TokenUsage::with_cache(
                            row.total_input_tokens as u32,
                            row.total_output_tokens as u32,
                            if row.total_cache_read_tokens > 0 {
                                Some(row.total_cache_read_tokens as u32)
                            } else {
                                None
                            },
                            if row.total_cache_creation_tokens > 0 {
                                Some(row.total_cache_creation_tokens as u32)
                            } else {
                                None
                            },
                        )
                        .with_cost(
                            (row.total_actual_cost_usd > 0.0).then_some(row.total_actual_cost_usd),
                            (row.total_estimated_cost_usd > 0.0)
                                .then_some(row.total_estimated_cost_usd),
                        )
                        .with_effective_cost(
                            (row.total_cost_usd > 0.0).then_some(row.total_cost_usd),
                        ),
                    )
                } else {
                    None
                };

                // Parse capabilities from JSON
                let capabilities = serde_json::from_value(row.capabilities).unwrap_or_default();

                Ok(Some(Session {
                    source: SessionSource::from(row.source.as_str()),
                    activity: SessionActivity::derive(
                        &SessionStatus::from(row.status.as_str()),
                        row.last_turn_status.as_deref(),
                    ),
                    id: row.id,
                    workspace_id: everruns_core::WorkspaceId::from_uuid(row.workspace_id),
                    organization_id: self.org_public_id.clone(),
                    harness_id,
                    agent_id: row.agent_id,
                    agent_version_id: row.agent_version_id,
                    agent_identity_id: row.agent_identity_id,
                    owner_principal_id: row.owner_principal_id,
                    resolved_owner_user_id: row.resolved_owner_user_id,
                    owner: None,
                    effective_owner: None,
                    title: row.title,
                    goal: row.goal,
                    locale: row.locale,
                    preview: None,
                    output_preview: None,
                    tags: row.tags,
                    model_id: row.model_id,
                    capabilities,
                    tools: serde_json::from_value(row.tools).unwrap_or_default(),
                    mcp_servers: serde_json::from_value(row.mcp_servers).unwrap_or_default(),
                    system_prompt: row.system_prompt,
                    initial_files: serde_json::from_value(row.initial_files).unwrap_or_default(),
                    network_access: row
                        .network_access
                        .and_then(|v| serde_json::from_value(v).ok()),
                    hints: row.hints.and_then(|v| serde_json::from_value(v).ok()),
                    max_iterations: max_iterations::from_db(row.max_iterations),
                    parallel_tool_calls: row.parallel_tool_calls,
                    status: SessionStatus::from(row.status.as_str()),
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    started_at: row.started_at,
                    finished_at: row.finished_at,
                    usage,
                    is_pinned: None,
                    active_schedule_count: None,
                    features: vec![],
                    parent_session_id: row.parent_session_id,
                    forked_from_session_id: row.forked_from_session_id,
                    forked_from_sequence: row.forked_from_sequence,
                    blueprint_id: row.blueprint_id,
                    blueprint_config: row.blueprint_config,
                }))
            }
            None => Ok(None),
        }
    }
}

#[async_trait]
impl SessionStore for DbSessionStore {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        Ok(self
            .get_stored_session(session_id)
            .await?
            .map(|session| session.execution_session()))
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed session store scoped to the given org
pub fn create_db_session_store(db: Database, org_id: i64, org_public_id: String) -> DbSessionStore {
    DbSessionStore::new(db, org_id, org_public_id)
}
