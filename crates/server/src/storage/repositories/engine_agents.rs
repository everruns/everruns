// PostgreSQL repository: platform agents as Engine definitions (data plane v2
// step 4). See `domains::agent_definitions` and the
// `engine_agent_definitions` migration.

use anyhow::Result;
use serde_json::Value;
use uuid::Uuid;

use super::Database;

/// What an Engine agent key stands for. Control plane only: never sent to
/// an executor.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct EngineAgentKeyRow {
    pub agent_key: String,
    pub org_id: i64,
    pub harness_id: Uuid,
    pub agent_id: Option<Uuid>,
}

/// A queued edit for the publisher.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EngineAgentPublishRow {
    pub id: i64,
    pub org_id: i64,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
}

impl Database {
    /// The key for sessions on `harness_id` answered by `agent_id`, minted on
    /// first use. Keys are random: nothing about them names the platform.
    pub async fn engine_agent_key(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
    ) -> Result<String> {
        let minted = format!("ak_{}", Uuid::new_v4().simple());
        // One statement either way: a concurrent first use keeps one key.
        let (key,): (String,) = sqlx::query_as(
            "WITH inserted AS (
                INSERT INTO engine_agent_keys (agent_key, org_id, harness_id, agent_id)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT DO NOTHING
                RETURNING agent_key
             )
             SELECT agent_key FROM inserted
             UNION ALL
             SELECT agent_key FROM engine_agent_keys
              WHERE org_id = $2 AND harness_id = $3
                AND agent_id IS NOT DISTINCT FROM $4
             LIMIT 1",
        )
        .bind(&minted)
        .bind(org_id)
        .bind(harness_id)
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(key)
    }

    /// What `agent_key` stands for.
    pub async fn engine_agent_key_target(
        &self,
        agent_key: &str,
    ) -> Result<Option<EngineAgentKeyRow>> {
        Ok(sqlx::query_as(
            "SELECT agent_key, org_id, harness_id, agent_id FROM engine_agent_keys WHERE agent_key = $1",
        )
        .bind(agent_key)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Keys a queued edit can change: one agent's, one harness chain's (any
    /// key in the org whose harness may inherit from it), or the whole org's.
    pub async fn engine_agent_keys_for_edit(
        &self,
        org_id: i64,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<EngineAgentKeyRow>> {
        // Decision: a harness edit republishes the whole organization's keys.
        // Children inherit from parents, so the affected set is a walk of the
        // chain; republishing an unchanged key is a digest comparison, which
        // is cheaper than keeping the walk right.
        Ok(sqlx::query_as(
            "SELECT agent_key, org_id, harness_id, agent_id FROM engine_agent_keys
              WHERE org_id = $1 AND ($2::uuid IS NULL OR agent_id = $2)
              ORDER BY agent_key",
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The current revision of `agent_key` and its digest.
    pub async fn engine_agent_current(&self, agent_key: &str) -> Result<Option<(String, String)>> {
        Ok(
            sqlx::query_as(
                "SELECT revision, digest FROM engine_agent_current WHERE agent_key = $1",
            )
            .bind(agent_key)
            .fetch_optional(&self.pool)
            .await?,
        )
    }

    /// Publish `runtime_agent` as the next revision of `agent_key` unless
    /// the current revision has the same digest. Returns the current
    /// revision after the call.
    pub async fn publish_engine_agent(
        &self,
        agent_key: &str,
        runtime_agent: &Value,
        digest: &str,
    ) -> Result<String> {
        let mut tx = self.pool.begin().await?;
        // Serializes publishers of one key; revisions stay dense.
        let current: Option<(String, String)> = sqlx::query_as(
            "SELECT c.revision, c.digest FROM engine_agent_keys k
               LEFT JOIN engine_agent_current c USING (agent_key)
              WHERE k.agent_key = $1 FOR UPDATE OF k",
        )
        .bind(agent_key)
        .fetch_optional(&mut *tx)
        .await?
        .and_then(|(revision, digest): (Option<String>, Option<String>)| revision.zip(digest));
        if let Some((revision, kept)) = &current
            && kept == digest
        {
            return Ok(revision.clone());
        }
        let next = match &current {
            Some((revision, _)) => {
                let n: u64 = revision.trim_start_matches('r').parse().unwrap_or(0);
                format!("r{}", n + 1)
            }
            None => "r1".to_string(),
        };
        sqlx::query(
            "INSERT INTO engine_agent_definitions (agent_key, revision, runtime_agent, digest)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(agent_key)
        .bind(&next)
        .bind(runtime_agent)
        .bind(digest)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO engine_agent_current (agent_key, revision, digest)
             VALUES ($1, $2, $3)
             ON CONFLICT (agent_key) DO UPDATE
               SET revision = excluded.revision, digest = excluded.digest, published_at = now()",
        )
        .bind(agent_key)
        .bind(&next)
        .bind(digest)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(next)
    }

    /// A published revision's declaration.
    pub async fn engine_agent_definition(
        &self,
        agent_key: &str,
        revision: &str,
    ) -> Result<Option<Value>> {
        Ok(sqlx::query_scalar(
            "SELECT runtime_agent FROM engine_agent_definitions WHERE agent_key = $1 AND revision = $2",
        )
        .bind(agent_key)
        .bind(revision)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Queue an edit for the publisher. `agent_id` and `harness_id` both
    /// `None` means every key of the organization.
    pub async fn enqueue_engine_agent_publish(
        &self,
        org_id: i64,
        agent_id: Option<Uuid>,
        harness_id: Option<Uuid>,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO engine_agent_publish_queue (org_id, agent_id, harness_id, reason)
             SELECT $1, $2, $3, $4
              WHERE EXISTS (SELECT 1 FROM engine_agent_keys WHERE org_id = $1)",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(harness_id)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Take up to `limit` queued edits. Rows are deleted as they are taken;
    /// a publisher that fails re-enqueues what it could not finish.
    pub async fn take_engine_agent_publishes(
        &self,
        limit: i64,
    ) -> Result<Vec<EngineAgentPublishRow>> {
        Ok(sqlx::query_as(
            "DELETE FROM engine_agent_publish_queue
              WHERE id IN (SELECT id FROM engine_agent_publish_queue
                            ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED)
             RETURNING id, org_id, agent_id, harness_id",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Organizations that have Engine agent keys.
    pub async fn engine_agent_orgs(&self) -> Result<Vec<i64>> {
        Ok(
            sqlx::query_scalar("SELECT DISTINCT org_id FROM engine_agent_keys")
                .fetch_all(&self.pool)
                .await?,
        )
    }
}
