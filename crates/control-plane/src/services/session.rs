// Session service for business logic (M2)

use crate::api::common::Pagination;
use crate::storage::{
    StorageBackend,
    models::{CreateSessionRow, UpdateSession},
};
use anyhow::Result;
use everruns_core::{Session, SessionStatus, TokenUsage};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::sessions::{CreateSessionRequest, UpdateSessionRequest};

pub struct SessionService {
    db: Arc<StorageBackend>,
}

impl SessionService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn create(&self, agent_id: Uuid, req: CreateSessionRequest) -> Result<Session> {
        // If model_id not provided, use the agent's default_model_id
        let model_id = match req.model_id {
            Some(id) => Some(id),
            None => {
                // Look up the agent to get its default_model_id
                let agent = self.db.get_agent(agent_id).await?;
                agent.and_then(|a| a.default_model_id)
            }
        };

        let input = CreateSessionRow {
            agent_id,
            title: req.title,
            tags: req.tags,
            model_id,
        };
        let row = self.db.create_session(input).await?;
        Ok(Self::row_to_session(row))
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Session>> {
        let row = self.db.get_session(id).await?;
        Ok(row.map(Self::row_to_session))
    }

    /// List sessions for an agent with pagination.
    /// Returns (sessions, total_count).
    /// Sessions include preview text from the first user message.
    pub async fn list(
        &self,
        agent_id: Uuid,
        pagination: Pagination,
    ) -> Result<(Vec<Session>, u32)> {
        let (rows, total) = self.db.list_sessions(agent_id, pagination).await?;
        let mut sessions: Vec<Session> = rows.into_iter().map(Self::row_to_session).collect();

        // Fetch previews for all sessions in a single query
        let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();
        let previews = self.db.get_session_previews(&session_ids).await?;

        // Populate preview for each session
        for session in &mut sessions {
            if let Some(preview) = previews.get(&session.id) {
                session.preview = Some(preview.clone());
            }
        }

        Ok((sessions, total))
    }

    pub async fn update(&self, id: Uuid, req: UpdateSessionRequest) -> Result<Option<Session>> {
        let input = UpdateSession {
            title: req.title,
            tags: req.tags,
            ..Default::default()
        };
        let row = self.db.update_session(id, input).await?;
        Ok(row.map(Self::row_to_session))
    }

    /// Update session status (used by worker via gRPC)
    pub async fn update_status(&self, id: Uuid, status: String) -> Result<Option<Session>> {
        let input = UpdateSession {
            status: Some(status),
            ..Default::default()
        };
        let row = self.db.update_session(id, input).await?;
        Ok(row.map(Self::row_to_session))
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool> {
        self.db.delete_session(id).await
    }

    fn row_to_session(row: crate::storage::SessionRow) -> Session {
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

        Session {
            id: row.id,
            agent_id: row.agent_id,
            title: row.title,
            preview: None, // Populated separately in list()
            tags: row.tags,
            model_id: row.model_id,
            status: SessionStatus::from(row.status.as_str()),
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            usage,
        }
    }
}
