// App service for business logic

use crate::storage::{
    AppRow, StorageBackend,
    models::{CreateAppRow, UpdateApp},
};
use anyhow::Result;
use chrono::Utc;
use everruns_core::typed_id::{AgentId, HarnessId};
use everruns_core::{App, AppId, AppStatus, ChannelType};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::apps::{CreateAppRequest, UpdateAppRequest};

pub struct AppService {
    db: Arc<StorageBackend>,
}

impl AppService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn create(&self, org_id: i64, req: CreateAppRequest) -> Result<App> {
        let internal_uuid = Uuid::now_v7();
        let public_id = AppId::from_uuid(internal_uuid);

        // Validate harness and agent exist
        let harness_row = self
            .db
            .get_harness(org_id, req.harness_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Harness not found"))?;
        let agent_row = self
            .db
            .get_agent_by_public_id(org_id, &req.agent_id.to_string())
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

        let input = CreateAppRow {
            public_id: public_id.to_string(),
            name: req.name,
            description: req.description,
            harness_id: harness_row.id.uuid(),
            agent_id: agent_row.id.uuid(),
            channel_type: req.channel_type.to_string(),
            channel_config: req.channel_config.unwrap_or_default(),
        };

        let row = self.db.create_app(org_id, input).await?;
        Ok(self.row_to_app(row, org_id).await)
    }

    pub async fn get_by_public_id(&self, org_id: i64, public_id: &str) -> Result<Option<App>> {
        let row = self.db.get_app_by_public_id(org_id, public_id).await?;
        match row {
            Some(row) => Ok(Some(self.row_to_app(row, org_id).await)),
            None => Ok(None),
        }
    }

    /// Lookup app by public_id without org scoping (for unauthenticated webhooks).
    /// The org_id is derived from the row itself.
    pub async fn get_by_public_id_unscoped(&self, public_id: &str) -> Result<Option<App>> {
        let row = self.db.get_app_by_public_id_unscoped(public_id).await?;
        match row {
            Some(row) => {
                let org_id = row.org_id;
                Ok(Some(self.row_to_app(row, org_id).await))
            }
            None => Ok(None),
        }
    }

    pub async fn list(&self, org_id: i64) -> Result<Vec<App>> {
        let rows = self.db.list_apps(org_id).await?;
        let mut apps = Vec::with_capacity(rows.len());
        for row in rows {
            apps.push(self.row_to_app(row, org_id).await);
        }
        Ok(apps)
    }

    pub async fn update(
        &self,
        org_id: i64,
        public_id: &str,
        req: UpdateAppRequest,
    ) -> Result<Option<App>> {
        let existing = self.db.get_app_by_public_id(org_id, public_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        // Resolve harness/agent IDs if provided
        let harness_id = if let Some(hid) = req.harness_id {
            let h = self
                .db
                .get_harness(org_id, hid)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Harness not found"))?;
            Some(h.id.uuid())
        } else {
            None
        };

        let agent_id = if let Some(aid) = req.agent_id {
            let a = self
                .db
                .get_agent_by_public_id(org_id, &aid.to_string())
                .await?
                .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;
            Some(a.id.uuid())
        } else {
            None
        };

        let input = UpdateApp {
            name: req.name,
            description: req.description,
            harness_id,
            agent_id,
            channel_type: req.channel_type.map(|ct| ct.to_string()),
            channel_config: req.channel_config,
            status: req.status.map(|s| s.to_string()),
            published_at: None,
        };

        let row = self.db.update_app(org_id, existing.id, input).await?;
        match row {
            Some(row) => Ok(Some(self.row_to_app(row, org_id).await)),
            None => Ok(None),
        }
    }

    pub async fn delete(&self, org_id: i64, public_id: &str) -> Result<bool> {
        let existing = self.db.get_app_by_public_id(org_id, public_id).await?;
        let Some(existing) = existing else {
            return Ok(false);
        };
        self.db.delete_app(org_id, existing.id).await
    }

    pub async fn publish(&self, org_id: i64, public_id: &str) -> Result<Option<App>> {
        let existing = self.db.get_app_by_public_id(org_id, public_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        let input = UpdateApp {
            status: Some("published".to_string()),
            published_at: Some(Some(Utc::now())),
            ..Default::default()
        };

        let row = self.db.update_app(org_id, existing.id, input).await?;
        match row {
            Some(row) => Ok(Some(self.row_to_app(row, org_id).await)),
            None => Ok(None),
        }
    }

    pub async fn unpublish(&self, org_id: i64, public_id: &str) -> Result<Option<App>> {
        let existing = self.db.get_app_by_public_id(org_id, public_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        let input = UpdateApp {
            status: Some("draft".to_string()),
            ..Default::default()
        };

        let row = self.db.update_app(org_id, existing.id, input).await?;
        match row {
            Some(row) => Ok(Some(self.row_to_app(row, org_id).await)),
            None => Ok(None),
        }
    }

    async fn row_to_app(&self, row: AppRow, org_id: i64) -> App {
        // Harness uses HarnessId directly (no dual-ID pattern)
        let harness_id = HarnessId::from_uuid(row.harness_id);

        // Agent uses dual-ID pattern — resolve internal UUID to public_id
        let agent_id = self
            .db
            .get_agent_public_id(org_id, AgentId::from_uuid(row.agent_id))
            .await
            .ok()
            .flatten()
            .and_then(|pid| pid.parse::<AgentId>().ok())
            .unwrap_or_else(|| AgentId::from_uuid(row.agent_id));

        let public_id: AppId = row
            .public_id
            .parse()
            .unwrap_or_else(|_| AppId::from_uuid(row.id));

        App {
            public_id,
            internal_id: row.id,
            name: row.name,
            description: row.description,
            harness_id,
            agent_id,
            channel_type: ChannelType::from_str_opt(&row.channel_type)
                .unwrap_or(ChannelType::Slack),
            channel_config: row.channel_config,
            status: AppStatus::from(row.status.as_str()),
            published_at: row.published_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
