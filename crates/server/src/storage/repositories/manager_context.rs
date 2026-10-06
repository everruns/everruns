//! Manager context in PostgreSQL. See `crate::storage::manager_context`.

use anyhow::Result;
use everruns_server_macros::sql;

use super::Database;
use crate::storage::manager_context::{
    ManagerContextEdit, ManagerContextKey, ManagerContextRow, ManagerContextWriteError, plan_write,
};

impl Database {
    pub async fn get_manager_context(
        &self,
        key: &ManagerContextKey,
    ) -> Result<Option<ManagerContextRow>> {
        self.get_manager_context_locked(key, false).await
    }

    /// Reads the row, with `FOR SHARE` when `share` is set.
    pub async fn get_manager_context_locked(
        &self,
        key: &ManagerContextKey,
        share: bool,
    ) -> Result<Option<ManagerContextRow>> {
        const SELECT: &str = sql!(
            r#"
            SELECT {ManagerContextRow}
            FROM entity_manager_context
            WHERE org_id = $1 AND entity_kind = $2 AND entity_ref = $3
            "#
        );
        const SELECT_FOR_SHARE: &str = sql!(
            r#"
            SELECT {ManagerContextRow}
            FROM entity_manager_context
            WHERE org_id = $1 AND entity_kind = $2 AND entity_ref = $3
            FOR SHARE
            "#
        );
        let row =
            sqlx::query_as::<_, ManagerContextRow>(if share { SELECT_FOR_SHARE } else { SELECT })
                .bind(key.org_id)
                .bind(&key.entity_kind)
                .bind(&key.entity_ref)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row)
    }

    pub async fn write_manager_context(
        &self,
        key: &ManagerContextKey,
        edit: &ManagerContextEdit,
        expected_revision: Option<i64>,
        updated_by_user_id: Option<uuid::Uuid>,
    ) -> Result<ManagerContextRow, ManagerContextWriteError> {
        let mut tx = self.pool.begin().await.map_err(anyhow::Error::from)?;
        // Lock the row (when there is one) so the revision check and the write
        // see the same document. Two first writers race on the insert instead;
        // the primary key turns the loser's insert into a unique violation.
        let current = sqlx::query_as::<_, ManagerContextRow>(sql!(
            r#"
            SELECT {ManagerContextRow}
            FROM entity_manager_context
            WHERE org_id = $1 AND entity_kind = $2 AND entity_ref = $3
            FOR UPDATE
            "#
        ))
        .bind(key.org_id)
        .bind(&key.entity_kind)
        .bind(&key.entity_ref)
        .fetch_optional(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
        let (content, revision) = plan_write(current.as_ref(), edit, expected_revision)?;
        let row = sqlx::query_as::<_, ManagerContextRow>(sql!(
            r#"
            INSERT INTO entity_manager_context (
                org_id, entity_kind, entity_ref, content, revision, updated_by_user_id, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, now())
            ON CONFLICT (org_id, entity_kind, entity_ref) DO UPDATE SET
                content = EXCLUDED.content,
                revision = EXCLUDED.revision,
                updated_by_user_id = EXCLUDED.updated_by_user_id,
                updated_at = EXCLUDED.updated_at
            RETURNING {ManagerContextRow}
            "#
        ))
        .bind(key.org_id)
        .bind(&key.entity_kind)
        .bind(&key.entity_ref)
        .bind(&content)
        .bind(revision)
        .bind(updated_by_user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
        tx.commit().await.map_err(anyhow::Error::from)?;
        Ok(row)
    }

    pub async fn delete_manager_context(&self, key: &ManagerContextKey) -> Result<()> {
        sqlx::query(
            "DELETE FROM entity_manager_context \
             WHERE org_id = $1 AND entity_kind = $2 AND entity_ref = $3",
        )
        .bind(key.org_id)
        .bind(&key.entity_kind)
        .bind(&key.entity_ref)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
