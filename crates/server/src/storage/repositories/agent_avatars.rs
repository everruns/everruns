use anyhow::Result;
use uuid::Uuid;

use super::Database;
use crate::storage::{AgentAvatarVariantRow, SetAgentAvatar};

impl Database {
    /// Store a new avatar and make it the agent's current one, in one
    /// transaction. The previous avatar and its variants are deleted, so its
    /// URLs stop resolving. Returns `None` when the agent is not in the org.
    pub async fn set_agent_avatar(&self, input: SetAgentAvatar) -> Result<Option<Uuid>> {
        let mut tx = self.pool.begin().await?;
        let found: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM agents WHERE org_id = $1 AND id = $2 AND status <> 'deleted' FOR UPDATE",
        )
        .bind(input.org_id)
        .bind(input.agent_id)
        .fetch_optional(&mut *tx)
        .await?;
        if found.is_none() {
            return Ok(None);
        }
        let avatar_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO agent_avatars (id, org_id, agent_id, source) VALUES ($1, $2, $3, $4)",
        )
        .bind(avatar_id)
        .bind(input.org_id)
        .bind(input.agent_id)
        .bind(&input.source)
        .execute(&mut *tx)
        .await?;
        for variant in &input.variants {
            sqlx::query(
                "INSERT INTO agent_avatar_variants (avatar_id, variant, content_type, data) VALUES ($1, $2, $3, $4)",
            )
            .bind(avatar_id)
            .bind(&variant.variant)
            .bind(&variant.content_type)
            .bind(&variant.data)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE agents SET avatar_id = $1, updated_at = NOW() WHERE id = $2")
            .bind(avatar_id)
            .bind(input.agent_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM agent_avatars WHERE agent_id = $1 AND id <> $2")
            .bind(input.agent_id)
            .bind(avatar_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(avatar_id))
    }

    pub async fn get_agent_avatar_source(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT source FROM agent_avatars WHERE org_id = $1 AND agent_id = $2",
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Remove the agent's avatar. Returns whether one was removed.
    pub async fn clear_agent_avatar(&self, org_id: i64, agent_id: Uuid) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE agents SET avatar_id = NULL, updated_at = NOW() WHERE org_id = $1 AND id = $2 AND avatar_id IS NOT NULL",
        )
        .bind(org_id)
        .bind(agent_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("DELETE FROM agent_avatars WHERE org_id = $1 AND agent_id = $2")
            .bind(org_id)
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(updated > 0)
    }

    /// One variant of an avatar, by its unguessable id. Not org-scoped: avatar
    /// URLs are public so Slack, A2A clients and email can fetch them.
    pub async fn get_agent_avatar_variant(
        &self,
        avatar_id: Uuid,
        variant: &str,
    ) -> Result<Option<AgentAvatarVariantRow>> {
        Ok(sqlx::query_as(
            "SELECT content_type, data FROM agent_avatar_variants WHERE avatar_id = $1 AND variant = $2",
        )
        .bind(avatar_id)
        .bind(variant)
        .fetch_optional(&self.pool)
        .await?)
    }
}
