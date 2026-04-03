// Generic notification service and event listener.
// Design:
// - Durable notifications are the source of truth for UI bell/toast state.
// - Delivery surfaces stay separate from creation logic.
// - Turn completion notifications resolve recipients via input_message_id mapping
//   captured when a user sends a message.

use async_trait::async_trait;
use everruns_core::typed_id::{MessageId, NotificationId, SessionId};
use everruns_core::{Event, EventData, EventListener, TURN_COMPLETED};
use serde_json::json;
use std::sync::Arc;
use tracing::{error, instrument};
use uuid::Uuid;

use crate::storage::{
    CreateNotificationRow, CreateNotificationTurnRequestRow, NotificationRow,
    NotificationTurnRequestRow, StorageBackend,
};

const LONG_RUNNING_TURN_THRESHOLD_MS: u64 = 60_000;
const MAX_UNVIEWED_PER_KIND: u32 = 20;
pub const LONG_RUNNING_TURN_COMPLETED_KIND: &str = "turn.long_running_completed";

pub struct NotificationService {
    db: Arc<StorageBackend>,
}

impl NotificationService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn create_turn_request(
        &self,
        org_id: i64,
        user_id: Uuid,
        session_id: SessionId,
        input_message_id: MessageId,
    ) -> anyhow::Result<()> {
        self.db
            .create_notification_turn_request(CreateNotificationTurnRequestRow {
                input_message_id,
                org_id,
                user_id,
                session_id,
            })
            .await
    }

    pub async fn list(
        &self,
        org_id: i64,
        user_id: Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<NotificationRow>> {
        self.db.list_notifications(org_id, user_id, limit).await
    }

    pub async fn list_updated_since(
        &self,
        org_id: i64,
        user_id: Uuid,
        updated_since: Option<chrono::DateTime<chrono::Utc>>,
        limit: i64,
    ) -> anyhow::Result<Vec<NotificationRow>> {
        self.db
            .list_notifications_updated_since(org_id, user_id, updated_since, limit)
            .await
    }

    pub async fn count_unviewed(&self, org_id: i64, user_id: Uuid) -> anyhow::Result<u32> {
        self.db.count_unviewed_notifications(org_id, user_id).await
    }

    pub async fn mark_viewed(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> anyhow::Result<Option<NotificationRow>> {
        self.db.mark_notification_viewed(org_id, user_id, id).await
    }

    async fn long_running_turn_candidate(
        &self,
        input_message_id: MessageId,
    ) -> anyhow::Result<Option<NotificationTurnRequestRow>> {
        self.db
            .get_notification_turn_request(input_message_id)
            .await
    }

    async fn create_long_running_turn_completed(
        &self,
        turn_event: &Event,
        input_message_id: MessageId,
        duration_ms: u64,
    ) -> anyhow::Result<Option<NotificationRow>> {
        let Some(candidate) = self.long_running_turn_candidate(input_message_id).await? else {
            return Ok(None);
        };

        let unviewed = self
            .db
            .count_unviewed_notifications_by_kind(
                candidate.org_id,
                candidate.user_id,
                LONG_RUNNING_TURN_COMPLETED_KIND,
            )
            .await?;
        if unviewed >= MAX_UNVIEWED_PER_KIND {
            return Ok(None);
        }

        let session = self
            .db
            .get_session(candidate.org_id, candidate.session_id)
            .await?;
        let session_label = session
            .and_then(|row| row.title)
            .unwrap_or_else(|| format!("Session {}", candidate.session_id));
        let href = format!("/sessions/{}/chat", candidate.session_id);
        let duration_label = format_duration(duration_ms);

        let notification = self
            .db
            .create_notification(CreateNotificationRow {
                org_id: candidate.org_id,
                user_id: candidate.user_id,
                kind: LONG_RUNNING_TURN_COMPLETED_KIND.to_string(),
                title: "Long-running turn completed".to_string(),
                body: format!("{session_label} finished after {duration_label}."),
                target_type: Some("session".to_string()),
                target_id: Some(candidate.session_id.to_string()),
                href: Some(href),
                payload: json!({
                    "turn_id": turn_event.context.turn_id.as_ref().map(ToString::to_string),
                    "input_message_id": input_message_id.to_string(),
                    "duration_ms": duration_ms,
                    "session_id": candidate.session_id.to_string(),
                }),
                dedupe_key: Some(format!("turn-long-complete:{input_message_id}")),
            })
            .await?;

        Ok(Some(notification))
    }
}

pub struct NotificationEventListener {
    service: Arc<NotificationService>,
}

impl NotificationEventListener {
    pub fn new(service: Arc<NotificationService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl EventListener for NotificationEventListener {
    #[instrument(skip(self, event), fields(event_id = %event.id, session_id = %event.session_id))]
    async fn on_event(&self, event: &Event) {
        let EventData::TurnCompleted(data) = &event.data else {
            return;
        };

        let Some(duration_ms) = data.duration_ms else {
            return;
        };
        if duration_ms < LONG_RUNNING_TURN_THRESHOLD_MS {
            return;
        }

        let Some(input_message_id) = event.context.input_message_id else {
            return;
        };

        if let Err(e) = self
            .service
            .create_long_running_turn_completed(event, input_message_id, duration_ms)
            .await
        {
            error!(error = %e, "Failed to create turn completion notification");
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![TURN_COMPLETED])
    }

    fn name(&self) -> &'static str {
        "NotificationEventListener"
    }
}

fn format_duration(duration_ms: u64) -> String {
    let total_seconds = duration_ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else {
        format!("{minutes}m {seconds:02}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateSessionRow, StorageBackend};
    use everruns_core::events::{EventContext, TurnCompletedData};
    use everruns_core::{Event, typed_id::TurnId};

    fn turn_completed_event(
        session_id: SessionId,
        turn_id: TurnId,
        input_message_id: MessageId,
        duration_ms: u64,
    ) -> Event {
        Event {
            id: everruns_core::EventId::new(),
            event_type: TURN_COMPLETED.to_string(),
            ts: chrono::Utc::now(),
            session_id,
            context: EventContext::turn(turn_id, input_message_id),
            data: EventData::TurnCompleted(TurnCompletedData {
                turn_id,
                iterations: 1,
                duration_ms: Some(duration_ms),
                usage: None,
                input_content: None,
            }),
            metadata: None,
            tags: None,
            sequence: Some(1),
        }
    }

    #[tokio::test]
    async fn creates_notification_for_long_running_turn() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = Arc::new(NotificationService::new(db.clone()));
        let listener = NotificationEventListener::new(service.clone());

        let user = db
            .create_user(crate::storage::CreateUserRow {
                email: "test@example.com".to_string(),
                name: "Test User".to_string(),
                avatar_url: None,
                roles: vec!["member".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: 1,
                harness_id: None,
                agent_id: None,
                agent_identity_id: None,
                title: Some("Inbox".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: json!([]),
                tools: json!([]),
                system_prompt: None,
                initial_files: serde_json::Value::Array(vec![]),
                hints: None,
                max_iterations: None,
                blueprint_id: None,
                blueprint_config: None,
                network_access: None,
            })
            .await
            .unwrap();

        let input_message_id = MessageId::new();
        service
            .create_turn_request(1, user.id, session.id, input_message_id)
            .await
            .unwrap();

        listener
            .on_event(&turn_completed_event(
                session.id,
                TurnId::new(),
                input_message_id,
                61_000,
            ))
            .await;

        let notifications = service.list(1, user.id, 10).await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].kind, LONG_RUNNING_TURN_COMPLETED_KIND);
        assert_eq!(notifications[0].target_type.as_deref(), Some("session"));
        assert_eq!(
            notifications[0].target_id.as_deref(),
            Some(session.id.to_string().as_str())
        );
        assert_eq!(service.count_unviewed(1, user.id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn skips_short_turns() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = Arc::new(NotificationService::new(db.clone()));
        let listener = NotificationEventListener::new(service.clone());
        let user_id = Uuid::now_v7();
        let session_id = SessionId::new();
        let input_message_id = MessageId::new();

        service
            .create_turn_request(1, user_id, session_id, input_message_id)
            .await
            .unwrap();

        listener
            .on_event(&turn_completed_event(
                session_id,
                TurnId::new(),
                input_message_id,
                59_000,
            ))
            .await;

        assert!(service.list(1, user_id, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn dedupes_same_turn_notification_and_marks_viewed() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = Arc::new(NotificationService::new(db.clone()));
        let listener = NotificationEventListener::new(service.clone());
        let user_id = Uuid::now_v7();
        let session_id = SessionId::new();
        let input_message_id = MessageId::new();

        service
            .create_turn_request(1, user_id, session_id, input_message_id)
            .await
            .unwrap();

        let event = turn_completed_event(session_id, TurnId::new(), input_message_id, 65_000);
        listener.on_event(&event).await;
        listener.on_event(&event).await;

        let notifications = service.list(1, user_id, 10).await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].occurrence_count, 2);

        let viewed = service
            .mark_viewed(1, user_id, notifications[0].id)
            .await
            .unwrap()
            .unwrap();
        assert!(viewed.viewed_at.is_some());
        assert_eq!(service.count_unviewed(1, user_id).await.unwrap(), 0);
    }
}
