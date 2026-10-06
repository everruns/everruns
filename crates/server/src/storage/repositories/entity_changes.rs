//! Entity history in PostgreSQL. See `crate::storage::entity_changes`.

use anyhow::Result;

use super::Database;
use crate::storage::entity_changes::{EntityChangeQuery, EntityChangeRow, NewEntityChange};

impl Database {
    pub async fn record_entity_change(&self, change: NewEntityChange) -> Result<EntityChangeRow> {
        let row = sqlx::query_as::<_, EntityChangeRow>(
            r#"
            INSERT INTO entity_changes (
                id, org_id, entity_kind, entity_ref, command, action, reason,
                changed_fields, actor_kind, actor_user_id, via_session_id, via_agent_id,
                surface, request_id, idempotency_key
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            RETURNING id, org_id, entity_kind, entity_ref, command, action, reason,
                changed_fields, actor_kind, actor_user_id, via_session_id, via_agent_id, surface,
                request_id, idempotency_key, revision, created_at
            "#,
        )
        .bind(uuid::Uuid::now_v7())
        .bind(change.org_id)
        .bind(&change.entity_kind)
        .bind(&change.entity_ref)
        .bind(&change.command)
        .bind(&change.action)
        .bind(&change.reason)
        .bind(&change.changed_fields)
        .bind(&change.actor_kind)
        .bind(change.actor_user_id)
        .bind(change.via_session_id)
        .bind(&change.via_agent_id)
        .bind(&change.surface)
        .bind(&change.request_id)
        .bind(&change.idempotency_key)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_entity_changes(
        &self,
        query: &EntityChangeQuery,
    ) -> Result<Vec<EntityChangeRow>> {
        let rows = sqlx::query_as::<_, EntityChangeRow>(
            r#"
            SELECT id, org_id, entity_kind, entity_ref, command, action, reason,
                changed_fields, actor_kind, actor_user_id, via_session_id, via_agent_id, surface,
                request_id, idempotency_key, revision, created_at
            FROM entity_changes
            WHERE org_id = $1
              AND ($2::text IS NULL OR entity_kind = $2)
              AND ($3::text IS NULL OR entity_ref = $3)
              AND ($4::text IS NULL OR action = $4)
              AND ($5::uuid IS NULL OR actor_user_id = $5)
              AND ($6::text IS NULL OR via_agent_id = $6)
              AND ($7::timestamptz IS NULL OR created_at < $7)
              AND ($8::timestamptz IS NULL OR created_at >= $8)
            ORDER BY created_at DESC, id DESC
            LIMIT $9
            "#,
        )
        .bind(query.org_id)
        .bind(&query.entity_kind)
        .bind(&query.entity_ref)
        .bind(&query.action)
        .bind(query.actor_user_id)
        .bind(&query.via_agent_id)
        .bind(query.before)
        .bind(query.since)
        .bind(query.limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
