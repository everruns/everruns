// Database-backed SessionStore implementation
//
// This module implements the core SessionStore trait for retrieving
// session configurations from the database.

use async_trait::async_trait;
use everruns_core::{
    AgentLoopError, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, HarnessId, Result, SessionId,
    TokenUsage,
    session::{Session, SessionStatus},
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
}

impl DbSessionStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SessionStore for DbSessionStore {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
        // TODO: Get org_id from context after Phase 3
        let session_row = self
            .db
            .get_session(DEFAULT_ORG_ID, session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

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
                    organization_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                    harness_id: row.harness_id.unwrap_or_else(|| HarnessId::from_seed(1)),
                    agent_id: row.agent_id,
                    title: row.title,
                    preview: None, // Preview populated separately when listing sessions
                    output_preview: None, // Output preview populated separately when listing sessions
                    tags: row.tags,
                    model_id: row.model_id,
                    capabilities,
                    tools: serde_json::from_value(row.tools).unwrap_or_default(),
                    status: SessionStatus::from(row.status.as_str()),
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    started_at: row.started_at,
                    finished_at: row.finished_at,
                    usage,
                    is_pinned: None,
                }))
            }
            None => Ok(None),
        }
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed session store
pub fn create_db_session_store(db: Database) -> DbSessionStore {
    DbSessionStore::new(db)
}
