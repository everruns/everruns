// Repository layer for database operations
// Note: Full implementations will be added incrementally across milestones

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::*;

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create database connection from URL
    pub async fn from_url(database_url: &str) -> Result<Self> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // Users (for future auth implementation)
    pub async fn create_user(&self, _input: CreateUser) -> Result<UserRow> {
        todo!("Implement when auth is added")
    }

    pub async fn get_user_by_email(&self, _email: &str) -> Result<Option<UserRow>> {
        todo!("Implement when auth is added")
    }

    pub async fn get_user(&self, _id: Uuid) -> Result<Option<UserRow>> {
        todo!("Implement when auth is added")
    }

    // Sessions (for future auth implementation)
    pub async fn create_session(&self, _input: CreateSession) -> Result<SessionRow> {
        todo!("Implement when auth is added")
    }

    pub async fn get_session_by_token(&self, _token: &str) -> Result<Option<SessionRow>> {
        todo!("Implement when auth is added")
    }

    pub async fn delete_session(&self, _token: &str) -> Result<()> {
        todo!("Implement when auth is added")
    }

    // Agents
    pub async fn create_agent(&self, input: CreateAgent) -> Result<AgentRow> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (name, description, default_model_id, status)
            VALUES ($1, $2, $3, 'active')
            RETURNING id, name, description, default_model_id, status, created_at, updated_at
            "#,
        )
        .bind(input.name)
        .bind(input.description)
        .bind(input.default_model_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent(&self, id: Uuid) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, name, description, default_model_id, status, created_at, updated_at
            FROM agents
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentRow>> {
        let rows = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, name, description, default_model_id, status, created_at, updated_at
            FROM agents
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_agent(&self, id: Uuid, input: UpdateAgent) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            UPDATE agents
            SET
                name = COALESCE($2, name),
                description = COALESCE($3, description),
                default_model_id = COALESCE($4, default_model_id),
                status = COALESCE($5, status),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, name, description, default_model_id, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(input.name)
        .bind(input.description)
        .bind(input.default_model_id)
        .bind(input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Agent versions
    pub async fn create_agent_version(&self, input: CreateAgentVersion) -> Result<AgentVersionRow> {
        // Get next version number
        let next_version: Option<i32> = sqlx::query_scalar(
            r#"
            SELECT COALESCE(MAX(version), 0) + 1
            FROM agent_versions
            WHERE agent_id = $1
            "#,
        )
        .bind(input.agent_id)
        .fetch_one(&self.pool)
        .await?;

        let next_version = next_version.unwrap_or(1);
        let definition_json = serde_json::to_value(&input.definition)?;

        let row = sqlx::query_as::<_, AgentVersionRow>(
            r#"
            INSERT INTO agent_versions (agent_id, version, definition)
            VALUES ($1, $2, $3)
            RETURNING agent_id, version, definition, created_at
            "#,
        )
        .bind(input.agent_id)
        .bind(next_version)
        .bind(definition_json)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent_version(
        &self,
        agent_id: Uuid,
        version: i32,
    ) -> Result<Option<AgentVersionRow>> {
        let row = sqlx::query_as::<_, AgentVersionRow>(
            r#"
            SELECT agent_id, version, definition, created_at
            FROM agent_versions
            WHERE agent_id = $1 AND version = $2
            "#,
        )
        .bind(agent_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agent_versions(&self, agent_id: Uuid) -> Result<Vec<AgentVersionRow>> {
        let rows = sqlx::query_as::<_, AgentVersionRow>(
            r#"
            SELECT agent_id, version, definition, created_at
            FROM agent_versions
            WHERE agent_id = $1
            ORDER BY version DESC
            "#,
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // Threads
    pub async fn create_thread(&self, _input: CreateThread) -> Result<ThreadRow> {
        let row = sqlx::query_as::<_, ThreadRow>(
            r#"
            INSERT INTO threads DEFAULT VALUES
            RETURNING id, created_at
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_thread(&self, id: Uuid) -> Result<Option<ThreadRow>> {
        let row = sqlx::query_as::<_, ThreadRow>(
            r#"
            SELECT id, created_at
            FROM threads
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Messages
    pub async fn create_message(&self, input: CreateMessage) -> Result<MessageRow> {
        let metadata_json = input.metadata.map(serde_json::to_value).transpose()?;

        let row = sqlx::query_as::<_, MessageRow>(
            r#"
            INSERT INTO messages (thread_id, role, content, metadata)
            VALUES ($1, $2, $3, $4)
            RETURNING id, thread_id, role, content, metadata, created_at
            "#,
        )
        .bind(input.thread_id)
        .bind(input.role)
        .bind(input.content)
        .bind(metadata_json)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_messages(&self, thread_id: Uuid) -> Result<Vec<MessageRow>> {
        let rows = sqlx::query_as::<_, MessageRow>(
            r#"
            SELECT id, thread_id, role, content, metadata, created_at
            FROM messages
            WHERE thread_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(thread_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // Runs
    pub async fn create_run(&self, input: CreateRun) -> Result<RunRow> {
        let row = sqlx::query_as::<_, RunRow>(
            r#"
            INSERT INTO runs (agent_id, agent_version, thread_id, status)
            VALUES ($1, $2, $3, 'pending')
            RETURNING id, agent_id, agent_version, thread_id, status, temporal_workflow_id, temporal_run_id, created_at, started_at, finished_at
            "#,
        )
        .bind(input.agent_id)
        .bind(input.agent_version)
        .bind(input.thread_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_run(&self, id: Uuid) -> Result<Option<RunRow>> {
        let row = sqlx::query_as::<_, RunRow>(
            r#"
            SELECT id, agent_id, agent_version, thread_id, status, temporal_workflow_id, temporal_run_id, created_at, started_at, finished_at
            FROM runs
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List runs with optional filtering
    pub async fn list_runs(
        &self,
        status: Option<&str>,
        agent_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<RunRow>> {
        let rows = sqlx::query_as::<_, RunRow>(
            r#"
            SELECT id, agent_id, agent_version, thread_id, status, temporal_workflow_id, temporal_run_id, created_at, started_at, finished_at
            FROM runs
            WHERE ($1::text IS NULL OR status = $1)
              AND ($2::uuid IS NULL OR agent_id = $2)
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(status)
        .bind(agent_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_run(&self, id: Uuid, input: UpdateRun) -> Result<Option<RunRow>> {
        let row = sqlx::query_as::<_, RunRow>(
            r#"
            UPDATE runs
            SET
                status = COALESCE($2, status),
                temporal_workflow_id = COALESCE($3, temporal_workflow_id),
                temporal_run_id = COALESCE($4, temporal_run_id),
                started_at = COALESCE($5, started_at),
                finished_at = COALESCE($6, finished_at)
            WHERE id = $1
            RETURNING id, agent_id, agent_version, thread_id, status, temporal_workflow_id, temporal_run_id, created_at, started_at, finished_at
            "#,
        )
        .bind(id)
        .bind(input.status)
        .bind(input.temporal_workflow_id)
        .bind(input.temporal_run_id)
        .bind(input.started_at)
        .bind(input.finished_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Actions
    pub async fn create_action(&self, _input: CreateAction) -> Result<ActionRow> {
        todo!("Implement in M7")
    }

    pub async fn list_actions(&self, _run_id: Uuid) -> Result<Vec<ActionRow>> {
        todo!("Implement in M7")
    }

    // Run events
    pub async fn create_run_event(&self, _input: CreateRunEvent) -> Result<RunEventRow> {
        todo!("Implement in M5")
    }

    /// List run events, optionally filtering by sequence number
    /// Returns events ordered by sequence_number ASC
    pub async fn list_run_events(
        &self,
        run_id: Uuid,
        since_sequence: Option<i64>,
    ) -> Result<Vec<RunEventRow>> {
        let rows = if let Some(seq) = since_sequence {
            sqlx::query_as::<_, RunEventRow>(
                r#"
                SELECT id, run_id, sequence_number, event_type, event_data, created_at
                FROM run_events
                WHERE run_id = $1 AND sequence_number > $2
                ORDER BY sequence_number ASC
                "#,
            )
            .bind(run_id)
            .bind(seq)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, RunEventRow>(
                r#"
                SELECT id, run_id, sequence_number, event_type, event_data, created_at
                FROM run_events
                WHERE run_id = $1
                ORDER BY sequence_number ASC
                "#,
            )
            .bind(run_id)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }
}
