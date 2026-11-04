//! GitHub source bindings, leases and generation fencing.
use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_server_macros::sql;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentSourceRow {
    pub agent_id: Uuid,
    pub org_id: i64,
    pub generation: Uuid,
    pub identity_id: Uuid,
    pub connection_id: Uuid,
    pub installation_id: Option<i64>,
    /// Which development GitHub account reads and writes the source:
    /// `personal`, `organization`, or `agent` for bindings made before the split.
    pub github_account: String,
    pub configured_by: Option<Uuid>,
    pub configured_role: String,
    pub repository_id: i64,
    pub repository: String,
    pub branch: String,
    pub path: String,
    pub mode: String,
    pub local_digest: Option<String>,
    pub local_package: Option<serde_json::Value>,
    pub local_dirty: bool,
    pub outbox: Option<serde_json::Value>,
    pub managed_channels: Vec<String>,
    pub applied_commit: Option<String>,
    pub package_digest: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub next_sync_at: DateTime<Utc>,
    pub lease_token: Option<Uuid>,
    pub lease_until: Option<DateTime<Utc>>,
}

impl Database {
    // All definition writes and source binding changes lock the same Agent row.
    // That closes the check/write race when a source is attached during an edit.
    pub async fn lock_agent_definition(&self, org: i64, agent: Uuid) -> Result<()> {
        sqlx::query("SELECT id FROM agents WHERE org_id = $1 AND id = $2 FOR UPDATE")
            .bind(org)
            .bind(agent)
            .fetch_optional(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_agent_source(&self, org: i64, agent: Uuid) -> Result<Option<AgentSourceRow>> {
        Ok(sqlx::query_as(sql!(
            "SELECT {AgentSourceRow} FROM agent_github_sources WHERE org_id = $1 AND agent_id = $2"
        ))
        .bind(org)
        .bind(agent)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn set_agent_source(&self, row: &AgentSourceRow) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_github_sources (
                agent_id, org_id, generation, identity_id, connection_id,
                configured_by, configured_role, repository_id, repository, branch, path,
                applied_commit, package_digest, managed_channels, installation_id, status,
                last_checked_at, last_synced_at, next_sync_at, mode, local_digest, local_package,
                github_account
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                'synced', NOW(), NOW(), NOW() + INTERVAL '60 seconds', $16, $17, $18, $19
            )
            ON CONFLICT (agent_id) DO UPDATE SET
                generation=$3, identity_id=$4, connection_id=$5, configured_by=$6,
                configured_role=$7, repository_id=$8, repository=$9, branch=$10, path=$11,
                applied_commit=$12, package_digest=$13, managed_channels=$14, installation_id=$15,
                mode=$16, local_digest=$17, local_package=$18, github_account=$19, local_dirty=FALSE, outbox=NULL,
                status='synced', error=NULL, last_checked_at=NOW(), last_synced_at=NOW(),
                next_sync_at=NOW() + INTERVAL '60 seconds', lease_token=NULL, lease_until=NULL
            WHERE agent_github_sources.org_id=$2
        "#,
        )
        .bind(row.agent_id)
        .bind(row.org_id)
        .bind(row.generation)
        .bind(row.identity_id)
        .bind(row.connection_id)
        .bind(row.configured_by)
        .bind(&row.configured_role)
        .bind(row.repository_id)
        .bind(&row.repository)
        .bind(&row.branch)
        .bind(&row.path)
        .bind(&row.applied_commit)
        .bind(&row.package_digest)
        .bind(&row.managed_channels)
        .bind(row.installation_id)
        .bind(&row.mode)
        .bind(&row.local_digest)
        .bind(&row.local_package)
        .bind(&row.github_account)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_agent_source_channels(
        &self,
        org: i64,
        agent: Uuid,
        names: &[String],
    ) -> Result<()> {
        sqlx::query("UPDATE agent_github_sources SET managed_channels=$3 WHERE org_id=$1 AND agent_id=$2 AND mode='two_way'")
            .bind(org).bind(agent).bind(names).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn mark_agent_source_dirty(&self, org: i64, agent: Uuid) -> Result<()> {
        sqlx::query("UPDATE agent_github_sources SET local_dirty=TRUE, next_sync_at=NOW(), status=CASE WHEN status='conflict' THEN status ELSE 'pending' END WHERE org_id=$1 AND agent_id=$2 AND mode='two_way'")
            .bind(org).bind(agent).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn stage_agent_source_outbox(
        &self,
        row: &AgentSourceRow,
        outbox: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query("UPDATE agent_github_sources SET outbox=$4 WHERE org_id=$1 AND agent_id=$2 AND generation=$3 AND lease_token=$5")
            .bind(row.org_id).bind(row.agent_id).bind(row.generation).bind(outbox).bind(row.lease_token)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn finish_agent_source_write(
        &self,
        row: &AgentSourceRow,
        local_digest: &str,
        package: &serde_json::Value,
        dirty: bool,
    ) -> Result<()> {
        sqlx::query("UPDATE agent_github_sources SET local_digest=$4, local_package=$5, local_dirty=$6, outbox=NULL, next_sync_at=CASE WHEN $6 THEN NOW() ELSE next_sync_at END WHERE org_id=$1 AND agent_id=$2 AND generation=$3 AND lease_token=$7")
            .bind(row.org_id).bind(row.agent_id).bind(row.generation).bind(local_digest).bind(package).bind(dirty).bind(row.lease_token)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn conflict_agent_source(&self, row: &AgentSourceRow, message: &str) -> Result<()> {
        sqlx::query("UPDATE agent_github_sources SET status='conflict', error=$4, lease_token=NULL, lease_until=NULL WHERE org_id=$1 AND agent_id=$2 AND generation=$3 AND lease_token=$5")
            .bind(row.org_id).bind(row.agent_id).bind(row.generation).bind(message).bind(row.lease_token)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_agent_source(&self, org: i64, agent: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM agent_github_sources WHERE org_id=$1 AND agent_id=$2")
            .bind(org)
            .bind(agent)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn request_agent_source_sync(&self, org: i64, agent: Uuid) -> Result<()> {
        sqlx::query("UPDATE agent_github_sources SET next_sync_at=NOW(), status=CASE WHEN lease_until > NOW() THEN status ELSE 'pending' END WHERE org_id=$1 AND agent_id=$2")
            .bind(org).bind(agent).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn request_agent_source_push(
        &self,
        org: i64,
        identities: &[Uuid],
        installation: Option<i64>,
        repo: i64,
        branch: &str,
    ) -> Result<()> {
        // Signed deliveries only wake bindings on the receiving installation's identities,
        // or, for an org link, bindings that read through that installation.
        // Replays coalesce into the same work; payload commits are never applied directly.
        sqlx::query("UPDATE agent_github_sources SET next_sync_at=LEAST(next_sync_at,NOW()) WHERE org_id=$1 AND (identity_id=ANY($2) OR (github_account='organization' AND installation_id=$5)) AND repository_id=$3 AND branch=$4")
            .bind(org).bind(identities).bind(repo).bind(branch).bind(installation).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn claim_agent_source_sync(&self) -> Result<Option<AgentSourceRow>> {
        Ok(sqlx::query_as(sql!(
            r#"
            WITH due AS (
                SELECT s.agent_id FROM agent_github_sources s
                JOIN agents a ON a.id=s.agent_id AND a.org_id=s.org_id
                WHERE a.status='active' AND s.next_sync_at <= NOW()
                    AND (s.lease_until IS NULL OR s.lease_until < NOW())
                ORDER BY s.next_sync_at FOR UPDATE OF s SKIP LOCKED LIMIT 1
            )
            UPDATE agent_github_sources s SET lease_token=$1,
                lease_until=NOW() + INTERVAL '5 minutes',
                next_sync_at=NOW() + INTERVAL '60 seconds',
                status='syncing', last_checked_at=NOW()
            FROM due WHERE s.agent_id=due.agent_id RETURNING {AgentSourceRow as s}
        "#
        ))
        .bind(Uuid::new_v4())
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn finish_agent_source_sync(
        &self,
        row: &AgentSourceRow,
        commit: Option<&str>,
        digest: Option<&str>,
        channels: Option<&[String]>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE agent_github_sources SET
                status=CASE WHEN $6::text IS NULL THEN CASE WHEN local_dirty THEN 'pending' ELSE 'synced' END ELSE 'error' END,
                error=$6, applied_commit=COALESCE($4,applied_commit),
                package_digest=COALESCE($5,package_digest),
                managed_channels=COALESCE($8,managed_channels),
                last_synced_at=CASE WHEN $6::text IS NULL THEN NOW() ELSE last_synced_at END,
                lease_token=NULL, lease_until=NULL
            WHERE org_id=$1 AND agent_id=$2 AND generation=$3 AND lease_token=$7
        "#,
        )
        .bind(row.org_id)
        .bind(row.agent_id)
        .bind(row.generation)
        .bind(commit)
        .bind(digest)
        .bind(error)
        .bind(row.lease_token)
        .bind(channels)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
