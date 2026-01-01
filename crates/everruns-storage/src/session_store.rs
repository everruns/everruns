// Database-backed SessionStore implementation
//
// This module implements the core SessionStore trait for retrieving
// session configurations from the database.

use async_trait::async_trait;
use everruns_core::{
    session::{Session, SessionStatus},
    traits::SessionStore,
    AgentLoopError, Result,
};
use uuid::Uuid;

use crate::repositories::Database;

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
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>> {
        let session_row = self
            .db
            .get_session(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        match session_row {
            Some(row) => Ok(Some(Session {
                id: row.id,
                agent_id: row.agent_id,
                title: row.title,
                tags: row.tags,
                model_id: row.model_id,
                status: SessionStatus::from(row.status.as_str()),
                created_at: row.created_at,
                started_at: row.started_at,
                finished_at: row.finished_at,
            })),
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
