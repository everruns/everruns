//! Entity history in PostgreSQL. See `crate::storage::entity_changes`.

use anyhow::Result;

use super::Database;
use crate::storage::entity_changes::{
    EntityChangeQuery, EntityChangeRow, EntityRevisionKey, EntityRevisionRow,
    MAX_SNAPSHOTS_PER_ENTITY, NewEntityChange,
};

/// The columns of `EntityChangeRow`, as a literal so queries stay static.
macro_rules! columns {
    () => {
        "id, org_id, entity_kind, entity_ref, command, action, reason, changed_fields, \
         actor_kind, actor_user_id, via_session_id, via_agent_id, surface, request_id, \
         idempotency_key, revision, restored_from_revision, created_at"
    };
}

impl Database {
    /// Append one entry. An entry with a snapshot that differs from the
    /// entity's latest revision becomes the next revision; one that matches is
    /// a no-op change and keeps no snapshot. Revisions are numbered under a
    /// per-entity advisory lock, so concurrent changes cannot share one.
    pub async fn record_entity_change(&self, change: NewEntityChange) -> Result<EntityChangeRow> {
        let mut tx = self.pool.begin().await?;
        let mut revision: Option<i64> = None;
        let (mut snapshot, mut snapshot_hash) = (None, None);
        if let (Some(value), Some(hash)) = (&change.snapshot, &change.snapshot_hash) {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, $2))")
                .bind(format!("{}/{}", change.entity_kind, change.entity_ref))
                .bind(change.org_id)
                .execute(&mut *tx)
                .await?;
            let latest: Option<(i64, Option<String>)> = sqlx::query_as(
                "SELECT revision, snapshot_hash FROM entity_changes \
                 WHERE org_id = $1 AND entity_kind = $2 AND entity_ref = $3 \
                   AND revision IS NOT NULL \
                 ORDER BY revision DESC LIMIT 1",
            )
            .bind(change.org_id)
            .bind(&change.entity_kind)
            .bind(&change.entity_ref)
            .fetch_optional(&mut *tx)
            .await?;
            if latest
                .as_ref()
                .is_none_or(|(_, latest)| latest.as_ref() != Some(hash))
            {
                let next = latest.map_or(0, |(revision, _)| revision) + 1;
                sqlx::query(
                    "UPDATE entity_changes SET snapshot = NULL \
                     WHERE org_id = $1 AND entity_kind = $2 AND entity_ref = $3 \
                       AND revision <= $4 AND snapshot IS NOT NULL",
                )
                .bind(change.org_id)
                .bind(&change.entity_kind)
                .bind(&change.entity_ref)
                .bind(next - MAX_SNAPSHOTS_PER_ENTITY)
                .execute(&mut *tx)
                .await?;
                revision = Some(next);
                snapshot = Some(value.clone());
                snapshot_hash = Some(hash.clone());
            }
        }
        let row = sqlx::query_as::<_, EntityChangeRow>(concat!(
            r#"
            INSERT INTO entity_changes (
                id, org_id, entity_kind, entity_ref, command, action, reason,
                changed_fields, actor_kind, actor_user_id, via_session_id, via_agent_id,
                surface, request_id, idempotency_key, revision, snapshot, snapshot_hash,
                restored_from_revision
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18,
                $19)
            RETURNING "#,
            columns!()
        ))
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
        .bind(revision)
        .bind(snapshot)
        .bind(snapshot_hash)
        .bind(change.restored_from_revision)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn latest_entity_revision(
        &self,
        org_id: i64,
        entity_kind: &str,
        entity_ref: &str,
    ) -> Result<Option<i64>> {
        let revision: Option<i64> = sqlx::query_scalar(
            "SELECT max(revision) FROM entity_changes \
             WHERE org_id = $1 AND entity_kind = $2 AND entity_ref = $3",
        )
        .bind(org_id)
        .bind(entity_kind)
        .bind(entity_ref)
        .fetch_one(&self.pool)
        .await?;
        Ok(revision)
    }

    pub async fn get_entity_revision(
        &self,
        key: &EntityRevisionKey,
    ) -> Result<Option<EntityRevisionRow>> {
        let entry = sqlx::query_as::<_, EntityChangeRow>(concat!(
            "SELECT ",
            columns!(),
            " FROM entity_changes \
             WHERE org_id = $1 AND entity_kind = $2 AND entity_ref = $3 \
               AND revision IS NOT NULL AND ($4::bigint IS NULL OR revision = $4) \
             ORDER BY revision DESC LIMIT 1"
        ))
        .bind(key.org_id)
        .bind(&key.entity_kind)
        .bind(&key.entity_ref)
        .bind(key.revision)
        .fetch_optional(&self.pool)
        .await?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        let snapshot: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT snapshot FROM entity_changes WHERE id = $1")
                .bind(entry.id)
                .fetch_one(&self.pool)
                .await?;
        Ok(Some(EntityRevisionRow { entry, snapshot }))
    }

    pub async fn list_entity_changes(
        &self,
        query: &EntityChangeQuery,
    ) -> Result<Vec<EntityChangeRow>> {
        let rows = sqlx::query_as::<_, EntityChangeRow>(concat!(
            "SELECT ",
            columns!(),
            r#"
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
            "#
        ))
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
