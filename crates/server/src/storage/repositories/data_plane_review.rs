// PostgreSQL repository: data plane v2 review fixes (the `dp_review_fixes`
// migration): the org settings sync obligation, and the org reads its
// pushed settings need. Its call receipts table is no longer written.

use anyhow::Result;

use super::Database;

impl Database {
    /// Orgs whose settings changed since their last sync: `(org, rev)`.
    pub async fn dp_orgs_settings_behind(&self, limit: i64) -> Result<Vec<(i64, i64)>> {
        Ok(sqlx::query_as(
            "SELECT org_id, settings_rev FROM dp_orgs
              WHERE settings_rev > settings_synced_rev
              ORDER BY org_id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// `org_id`'s settings change count, when it is on a data plane.
    pub async fn dp_org_settings_rev(&self, org_id: i64) -> Result<Option<i64>> {
        Ok(
            sqlx::query_scalar("SELECT settings_rev FROM dp_orgs WHERE org_id = $1")
                .bind(org_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// `org_id`'s settings are synced up to change `rev`.
    pub async fn dp_settings_synced(&self, org_id: i64, rev: i64) -> Result<()> {
        sqlx::query(
            "UPDATE dp_orgs SET settings_synced_rev = GREATEST(settings_synced_rev, $2)
              WHERE org_id = $1",
        )
        .bind(org_id)
        .bind(rev)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The members of `org_id` who have their own setting `key` (a user
    /// preference), with its stored value: `(user, value)`.
    pub async fn dp_org_member_preferences(
        &self,
        org_id: i64,
        key: &str,
    ) -> Result<Vec<(uuid::Uuid, String)>> {
        Ok(sqlx::query_as(
            "SELECT up.user_id, up.value FROM user_preferences up
               JOIN organization_members om ON om.user_id = up.user_id
              WHERE om.org_id = $1 AND up.key = $2
              ORDER BY up.user_id",
        )
        .bind(org_id)
        .bind(key)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The org of v2 session `session_id`, if it is one.
    pub async fn dp_session_org(&self, session_id: &str) -> Result<Option<i64>> {
        Ok(
            sqlx::query_scalar("SELECT org_id FROM dp_session_summaries WHERE session_id = $1")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }
}
