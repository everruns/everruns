// Session service for business logic (M2)

use crate::storage::{
    models::{CreateSessionRow, UpdateSession},
    Database,
};
use anyhow::Result;
use everruns_core::{Session, SessionStatus};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::sessions::{CreateSessionRequest, UpdateSessionRequest};

pub struct SessionService {
    db: Arc<Database>,
}

impl SessionService {
    pub fn new(db: Arc<Database>) -> Self {
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

    pub async fn list(&self, agent_id: Uuid) -> Result<Vec<Session>> {
        let rows = self.db.list_sessions(agent_id).await?;
        Ok(rows.into_iter().map(Self::row_to_session).collect())
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
        Session {
            id: row.id,
            agent_id: row.agent_id,
            title: row.title,
            tags: row.tags,
            model_id: row.model_id,
            status: SessionStatus::from(row.status.as_str()),
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}
