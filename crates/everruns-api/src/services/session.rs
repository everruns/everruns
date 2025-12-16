// Session service for business logic (M2)

use anyhow::Result;
use everruns_contracts::Session;
use everruns_storage::{
    models::{CreateSession, UpdateSession},
    Database,
};
use std::sync::Arc;
use uuid::Uuid;

pub struct SessionService {
    db: Arc<Database>,
}

impl SessionService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateSession) -> Result<Session> {
        let row = self.db.create_session(input).await?;
        Ok(Self::row_to_session(row))
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Session>> {
        let row = self.db.get_session(id).await?;
        Ok(row.map(Self::row_to_session))
    }

    pub async fn list(&self, harness_id: Uuid) -> Result<Vec<Session>> {
        let rows = self.db.list_sessions(harness_id).await?;
        Ok(rows.into_iter().map(Self::row_to_session).collect())
    }

    pub async fn update(&self, id: Uuid, input: UpdateSession) -> Result<Option<Session>> {
        let row = self.db.update_session(id, input).await?;
        Ok(row.map(Self::row_to_session))
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool> {
        self.db.delete_session(id).await
    }

    fn row_to_session(row: everruns_storage::SessionRow) -> Session {
        Session {
            id: row.id,
            harness_id: row.harness_id,
            title: row.title,
            tags: row.tags,
            model_id: row.model_id,
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}
