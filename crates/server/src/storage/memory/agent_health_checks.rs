// In-memory agent health check storage. See knowledge/evaluation/agent-checks.md.

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

    /// In-memory parity for the Postgres reaper: mark every non-terminal run
    /// (`pending`/`running`) as `failed`. See knowledge/evaluation/agent-checks.md and EVE-586.
    pub async fn reap_running_agent_health_check_runs(&self) -> Result<u64> {
        let now = Self::now();
        let mut guard = self.agent_health_check_runs.write();
        let mut count = 0u64;
        for row in guard.values_mut() {
            if matches!(row.status.as_str(), "pending" | "running") {
                row.status = "failed".to_string();
                if row.error_message.is_none() {
                    row.error_message = Some("Run interrupted by server restart".to_string());
                }
                if row.completed_at.is_none() {
                    row.completed_at = Some(now);
                }
                row.updated_at = now;
                count += 1;
            }
        }
        Ok(count)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_input(pid: u128) -> CreateAgentHealthCheckRunRow {
        CreateAgentHealthCheckRunRow {
            public_id: format!("healthcheck_{pid:032x}"),
            agent_id: None,
            config_hash: "cfg".to_string(),
            model_id: None,
        }
    }

    #[tokio::test]
    async fn reap_marks_running_failed_and_leaves_terminal_runs() {
        let db = InMemoryDatabase::new();

        // An orphaned run still marked `running` (simulating a crashed process).
        let running = db
            .create_agent_health_check_run(1, create_input(1))
            .await
            .unwrap();
        db.update_agent_health_check_run(
            running.id,
            UpdateAgentHealthCheckRunRow {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // A run that finished normally must be left untouched.
        let done = db
            .create_agent_health_check_run(1, create_input(2))
            .await
            .unwrap();
        db.update_agent_health_check_run(
            done.id,
            UpdateAgentHealthCheckRunRow {
                status: Some("completed".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let reaped = db.reap_running_agent_health_check_runs().await.unwrap();
        assert_eq!(reaped, 1);

        let running_after = db
            .get_agent_health_check_run(1, &running.public_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(running_after.status, "failed");
        assert!(running_after.error_message.is_some());
        assert!(running_after.completed_at.is_some());

        let done_after = db
            .get_agent_health_check_run(1, &done.public_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done_after.status, "completed");

        // Idempotent: a second pass finds nothing left to reap.
        assert_eq!(db.reap_running_agent_health_check_runs().await.unwrap(), 0);
    }
}
