// Database-backed SessionStore implementation
//
// This module implements the core SessionStore trait for retrieving
// session configurations from the database.
//
// Decision: org_id and org_public_id are baked into the struct at
// construction time, matching the Grpc/Adapter store pattern.

use crate::org_init::BASE_HARNESS_ID;
use async_trait::async_trait;
use everruns_core::{
    HarnessId, Result, SessionId, StoreResultExt, TokenUsage,
    session::{Session, SessionStatus, SubagentStatus},
    traits::SessionStore,
};

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

#[async_trait]
impl SessionStore for DbSessionStore {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
        let session_row = self
            .db
            .get_session(self.org_id, session_id)
            .await
            .store_err()?;

        match session_row {
            Some(row) => {
                // Convert database usage columns to TokenUsage
                let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
                    Some(TokenUsage::with_cache(
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
                    ))
                } else {
                    None
                };

                // Parse capabilities from JSON
                let capabilities = serde_json::from_value(row.capabilities).unwrap_or_default();

                Ok(Some(Session {
                    id: row.id,
                    organization_id: self.org_public_id.clone(),
                    harness_id: row
                        .harness_id
                        .unwrap_or_else(|| HarnessId::from_uuid(BASE_HARNESS_ID)),
                    agent_id: row.agent_id,
                    agent_identity_id: row.agent_identity_id,
                    title: row.title,
                    locale: row.locale,
                    preview: None,
                    output_preview: None,
                    tags: row.tags,
                    model_id: row.model_id,
                    capabilities,
                    tools: serde_json::from_value(row.tools).unwrap_or_default(),
                    hints: row.hints.and_then(|v| serde_json::from_value(v).ok()),
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
                    subagent_name: row.subagent_name,
                    subagent_task: row.subagent_task,
                    subagent_status: row
                        .subagent_status
                        .map(|s| SubagentStatus::from(s.as_str())),
                }))
            }
            None => Ok(None),
        }
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed session store scoped to the given org
pub fn create_db_session_store(db: Database, org_id: i64, org_public_id: String) -> DbSessionStore {
    DbSessionStore::new(db, org_id, org_public_id)
}
