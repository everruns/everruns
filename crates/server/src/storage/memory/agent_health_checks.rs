// In-memory agent health check storage. See specs/agent-checks.md.

use anyhow::Result;
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::models::*;

impl InMemoryDatabase {
    pub async fn create_agent_health_check_run(
        &self,
        org_id: i64,
        input: CreateAgentHealthCheckRunRow,
    ) -> Result<AgentHealthCheckRunRow> {
        let now = Self::now();
        let row = AgentHealthCheckRunRow {
            id: Uuid::now_v7(),
            org_id,
            public_id: input.public_id,
            agent_id: input.agent_id,
            config_hash: input.config_hash,
            model_id: input.model_id,
            status: "pending".to_string(),
            summary: None,
            results: None,
            error_message: None,
            started_at: None,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        self.agent_health_check_runs
            .write()
            .insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_agent_health_check_run(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        Ok(self
            .agent_health_check_runs
            .read()
            .values()
            .find(|r| r.org_id == org_id && r.public_id == public_id)
            .cloned())
    }

    pub async fn list_agent_health_check_runs(
        &self,
        org_id: i64,
        agent_id: Uuid,
        limit: i64,
    ) -> Result<Vec<AgentHealthCheckRunRow>> {
        let mut rows: Vec<AgentHealthCheckRunRow> = self
            .agent_health_check_runs
            .read()
            .values()
            .filter(|r| r.org_id == org_id && r.agent_id == Some(agent_id))
            .cloned()
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        rows.truncate(limit.max(0) as usize);
        Ok(rows)
    }

    pub async fn latest_agent_health_check_run(
        &self,
        org_id: i64,
        agent_id: Uuid,
        config_hash: &str,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        Ok(self
            .agent_health_check_runs
            .read()
            .values()
            .filter(|r| {
                r.org_id == org_id && r.agent_id == Some(agent_id) && r.config_hash == config_hash
            })
            .max_by(|a, b| a.created_at.cmp(&b.created_at))
            .cloned())
    }

    pub async fn update_agent_health_check_run(
        &self,
        id: Uuid,
        input: UpdateAgentHealthCheckRunRow,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        let now = Self::now();
        let mut guard = self.agent_health_check_runs.write();
        let Some(row) = guard.get_mut(&id) else {
            return Ok(None);
        };
        if let Some(status) = input.status {
            if status == "running" && row.started_at.is_none() {
                row.started_at = Some(now);
            }
            if matches!(status.as_str(), "completed" | "failed") && row.completed_at.is_none() {
                row.completed_at = Some(now);
            }
            row.status = status;
        }
        if input.summary.is_some() {
            row.summary = input.summary;
        }
        if input.results.is_some() {
            row.results = input.results;
        }
        if input.error_message.is_some() {
            row.error_message = input.error_message;
        }
        row.updated_at = now;
        Ok(Some(row.clone()))
    }
}
