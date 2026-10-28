//! Caller-tagged execution history, including owner-requested archived Work Chats.
use super::*;

impl Database {
    pub async fn list_sessions_by_tags(
        &self,
        org_id: i64,
        tags: &[String],
        activities: &[crate::domains::sessions::record::SessionActivity],
        updated_after: Option<DateTime<Utc>>,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: u32,
    ) -> Result<(Vec<SessionRow>, u32)> {
        self.list_sessions_by_tags_with_archive(
            org_id,
            tags,
            activities,
            updated_after,
            after,
            limit,
            false,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_sessions_by_tags_with_archive(
        &self,
        org_id: i64,
        tags: &[String],
        activities: &[crate::domains::sessions::record::SessionActivity],
        updated_after: Option<DateTime<Utc>>,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: u32,
        include_archived: bool,
    ) -> Result<(Vec<SessionRow>, u32)> {
        // THREAT[TM-API-001]: interpolated filters contain closed-enum
        // activity literals or the fixed archive predicate. Caller values bind as parameters.
        let activity_predicate = if activities.is_empty() {
            String::new()
        } else {
            format!(
                " AND ({ACTIVITY_SQL}) IN ({})",
                sql_string_list(activities.iter().map(|a| a.as_str()))
            )
        };
        let filter = format!(
            "WHERE org_id = $1 AND tags @> $2 {} \
             AND ($3::timestamptz IS NULL OR updated_at >= $3){activity_predicate}",
            if include_archived {
                ""
            } else {
                "AND archived_at IS NULL"
            }
        );
        let count_sql = format!("SELECT COUNT(*) FROM sessions {filter}");
        let total: (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(count_sql.as_str()))
            .bind(org_id)
            .bind(tags)
            .bind(updated_after)
            .fetch_one(&self.pool)
            .await?;
        let page_sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions {filter} \
             AND ($4::timestamptz IS NULL OR (updated_at, id) < ($4, $5)) \
             ORDER BY updated_at DESC, id DESC LIMIT $6"
        );
        let rows = sqlx::query_as::<_, SessionRow>(sqlx::AssertSqlSafe(page_sql.as_str()))
            .bind(org_id)
            .bind(tags)
            .bind(updated_after)
            .bind(after.map(|(at, _)| at))
            .bind(after.map(|(_, id)| id))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        Ok((rows, total.0 as u32))
    }
}
