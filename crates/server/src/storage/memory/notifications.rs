// In-memory storage: Notifications

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::{MessageId, NotificationId};
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Notifications
    // ============================================

    pub async fn create_notification_turn_request(
        &self,
        input: CreateNotificationTurnRequestRow,
    ) -> Result<()> {
        let row = NotificationTurnRequestRow {
            input_message_id: input.input_message_id,
            org_id: input.org_id,
            user_id: input.user_id,
            session_id: input.session_id,
            created_at: Self::now(),
        };
        self.notification_turn_requests
            .write()
            .insert(input.input_message_id, row);
        Ok(())
    }

    pub async fn get_notification_turn_request(
        &self,
        input_message_id: MessageId,
    ) -> Result<Option<NotificationTurnRequestRow>> {
        Ok(self
            .notification_turn_requests
            .read()
            .get(&input_message_id)
            .cloned())
    }

    pub async fn create_notification(
        &self,
        input: CreateNotificationRow,
    ) -> Result<NotificationRow> {
        let now = Self::now();
        let mut notifications = self.notifications.write();

        if let Some(existing_id) = notifications
            .values()
            .find(|row| {
                row.org_id == input.org_id
                    && row.user_id == input.user_id
                    && row.viewed_at.is_none()
                    && row.dedupe_key.is_some()
                    && row.dedupe_key == input.dedupe_key
            })
            .map(|row| row.id)
            && let Some(existing) = notifications.get_mut(&existing_id)
        {
            existing.title = input.title;
            existing.body = input.body;
            existing.target_type = input.target_type;
            existing.target_id = input.target_id;
            existing.href = input.href;
            existing.payload = input.payload;
            existing.occurrence_count += 1;
            existing.updated_at = now;
            return Ok(existing.clone());
        }

        let row = NotificationRow {
            id: NotificationId::new(),
            org_id: input.org_id,
            user_id: input.user_id,
            kind: input.kind,
            title: input.title,
            body: input.body,
            target_type: input.target_type,
            target_id: input.target_id,
            href: input.href,
            payload: input.payload,
            dedupe_key: input.dedupe_key,
            occurrence_count: 1,
            viewed_at: None,
            created_at: now,
            updated_at: now,
        };
        notifications.insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_notification(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        Ok(self.notifications.read().get(&id).and_then(|row| {
            if row.org_id == org_id && row.user_id == user_id {
                Some(row.clone())
            } else {
                None
            }
        }))
    }

    pub async fn list_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        let mut rows: Vec<_> = self
            .notifications
            .read()
            .values()
            .filter(|row| row.org_id == org_id && row.user_id == user_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.uuid().cmp(&a.id.uuid()))
        });
        rows.truncate(limit as usize);
        Ok(rows)
    }

    pub async fn list_notifications_updated_since(
        &self,
        org_id: i64,
        user_id: Uuid,
        updated_since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        let mut rows: Vec<_> = self
            .notifications
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && row.user_id == user_id
                    && updated_since.is_none_or(|ts| row.updated_at >= ts)
            })
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            a.updated_at
                .cmp(&b.updated_at)
                .then_with(|| a.id.uuid().cmp(&b.id.uuid()))
        });
        rows.truncate(limit as usize);
        Ok(rows)
    }

    pub async fn count_unviewed_notifications(&self, org_id: i64, user_id: Uuid) -> Result<u32> {
        Ok(self
            .notifications
            .read()
            .values()
            .filter(|row| row.org_id == org_id && row.user_id == user_id && row.viewed_at.is_none())
            .count() as u32)
    }

    pub async fn count_unviewed_notifications_by_kind(
        &self,
        org_id: i64,
        user_id: Uuid,
        kind: &str,
    ) -> Result<u32> {
        Ok(self
            .notifications
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && row.user_id == user_id
                    && row.kind == kind
                    && row.viewed_at.is_none()
            })
            .count() as u32)
    }

    pub async fn mark_notification_viewed(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        let now = Self::now();
        let mut notifications = self.notifications.write();
        let Some(row) = notifications.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id || row.user_id != user_id {
            return Ok(None);
        }
        if row.viewed_at.is_none() {
            row.viewed_at = Some(now);
            row.updated_at = now;
        }
        Ok(Some(row.clone()))
    }
}
