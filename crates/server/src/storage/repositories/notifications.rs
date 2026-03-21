// PostgreSQL repository: Notifications

use super::super::models::*;
use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::typed_id::{MessageId, NotificationId};
use uuid::Uuid;

impl Database {
    // ============================================
    // Notifications
    // ============================================

    pub async fn create_notification_turn_request(
        &self,
        input: CreateNotificationTurnRequestRow,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO notification_turn_requests (input_message_id, org_id, user_id, session_id)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (input_message_id) DO UPDATE SET
                org_id = EXCLUDED.org_id,
                user_id = EXCLUDED.user_id,
                session_id = EXCLUDED.session_id
            "#,
        )
        .bind(input.input_message_id)
        .bind(input.org_id)
        .bind(input.user_id)
        .bind(input.session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_notification_turn_request(
        &self,
        input_message_id: MessageId,
    ) -> Result<Option<NotificationTurnRequestRow>> {
        sqlx::query_as::<_, NotificationTurnRequestRow>(
            r#"
            SELECT input_message_id, org_id, user_id, session_id, created_at
            FROM notification_turn_requests
            WHERE input_message_id = $1
            "#,
        )
        .bind(input_message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create_notification(
        &self,
        input: CreateNotificationRow,
    ) -> Result<NotificationRow> {
        let row = sqlx::query_as::<_, NotificationRow>(
            r#"
            INSERT INTO notifications (
                org_id, user_id, kind, title, body, target_type, target_id, href, payload, dedupe_key
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (org_id, user_id, dedupe_key)
                WHERE dedupe_key IS NOT NULL AND viewed_at IS NULL
            DO UPDATE SET
                title = EXCLUDED.title,
                body = EXCLUDED.body,
                target_type = EXCLUDED.target_type,
                target_id = EXCLUDED.target_id,
                href = EXCLUDED.href,
                payload = EXCLUDED.payload,
                occurrence_count = notifications.occurrence_count + 1,
                updated_at = NOW()
            RETURNING
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            "#,
        )
        .bind(input.org_id)
        .bind(input.user_id)
        .bind(&input.kind)
        .bind(&input.title)
        .bind(&input.body)
        .bind(&input.target_type)
        .bind(&input.target_id)
        .bind(&input.href)
        .bind(&input.payload)
        .bind(&input.dedupe_key)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_notification(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        sqlx::query_as::<_, NotificationRow>(
            r#"
            SELECT
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            FROM notifications
            WHERE org_id = $1 AND user_id = $2 AND id = $3
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        sqlx::query_as::<_, NotificationRow>(
            r#"
            SELECT
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            FROM notifications
            WHERE org_id = $1 AND user_id = $2
            ORDER BY created_at DESC, id DESC
            LIMIT $3
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_notifications_updated_since(
        &self,
        org_id: i64,
        user_id: Uuid,
        updated_since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        sqlx::query_as::<_, NotificationRow>(
            r#"
            SELECT
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            FROM notifications
            WHERE org_id = $1
              AND user_id = $2
              AND ($3::timestamptz IS NULL OR updated_at >= $3)
            ORDER BY updated_at ASC, id ASC
            LIMIT $4
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(updated_since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn count_unviewed_notifications(&self, org_id: i64, user_id: Uuid) -> Result<u32> {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::BIGINT
            FROM notifications
            WHERE org_id = $1 AND user_id = $2 AND viewed_at IS NULL
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count as u32)
    }

    pub async fn count_unviewed_notifications_by_kind(
        &self,
        org_id: i64,
        user_id: Uuid,
        kind: &str,
    ) -> Result<u32> {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::BIGINT
            FROM notifications
            WHERE org_id = $1 AND user_id = $2 AND kind = $3 AND viewed_at IS NULL
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(kind)
        .fetch_one(&self.pool)
        .await?;
        Ok(count as u32)
    }

    pub async fn mark_notification_viewed(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        sqlx::query_as::<_, NotificationRow>(
            r#"
            UPDATE notifications
            SET
                viewed_at = COALESCE(viewed_at, NOW()),
                updated_at = NOW()
            WHERE org_id = $1 AND user_id = $2 AND id = $3
            RETURNING
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }
}
