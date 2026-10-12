// Generic notification service and event listener.
// Design:
// - Durable notifications are the source of truth for UI bell/toast state.
// - Every kind shares one shape: a title naming the thing, a body saying what
//   happened, a source naming who sent it (agent or system), and a target the
//   UI opens. New kinds (shared agents, integrations) add a kind string and a
//   producer, not a new client.
// - Delivery surfaces stay separate from creation logic.
// - Turn notifications resolve recipients via the input_message_id mapping
//   captured when a user sends a message, and fire only for the recipient's
//   own chat threads: a turn in an API, channel, or someone else's session is
//   not something this user is waiting on.

use async_trait::async_trait;
use everruns_contracts::typed_id::{MessageId, NotificationId, SessionId};
use everruns_core::events::TURN_FAILED;
use everruns_core::{Event, EventData, EventListener, TURN_COMPLETED};
use serde_json::json;
use std::sync::Arc;
use tracing::{error, instrument};
use uuid::Uuid;

use crate::storage::{
    CreateNotificationRow, CreateNotificationTurnRequestRow, NotificationRow,
    NotificationSourceRow, NotificationTurnRequestRow, SessionRow, StorageBackend,
};

const LONG_RUNNING_TURN_THRESHOLD_MS: u64 = 60_000;
const MAX_UNVIEWED_PER_KIND: u32 = 20;
/// Longest answer excerpt shown in a notification body.
const ANSWER_EXCERPT_CHARS: usize = 160;
pub const LONG_RUNNING_TURN_COMPLETED_KIND: &str = "turn.long_running_completed";
pub const LONG_RUNNING_TURN_FAILED_KIND: &str = "turn.long_running_failed";

/// How a long-running turn ended, as far as the notification cares.
enum TurnOutcome<'a> {
    Completed { answer: Option<&'a str> },
    Failed,
}

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

    async fn create_long_running_turn_notification(
        &self,
        turn_event: &Event,
        candidate: NotificationTurnRequestRow,
        duration_ms: u64,
        outcome: TurnOutcome<'_>,
    ) -> anyhow::Result<Option<NotificationRow>> {
        let kind = match outcome {
            TurnOutcome::Completed { .. } => LONG_RUNNING_TURN_COMPLETED_KIND,
            TurnOutcome::Failed => LONG_RUNNING_TURN_FAILED_KIND,
        };

        let Some(session) = self
            .db
            .get_session(candidate.org_id, candidate.session_id)
            .await?
        else {
            return Ok(None);
        };
        if !is_own_chat(&session, candidate.user_id) {
            return Ok(None);
        }

        let unviewed = self
            .db
            .count_unviewed_notifications_by_kind(candidate.org_id, candidate.user_id, kind)
            .await?;
        if unviewed >= MAX_UNVIEWED_PER_KIND {
            return Ok(None);
        }

        let agent = match session.agent_id {
            Some(agent_id) => self.db.get_agent(candidate.org_id, agent_id).await?,
            None => None,
        };
        let agent_name = agent.as_ref().map(|agent| {
            agent
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(agent.name.as_str())
                .to_string()
        });
        let source = agent.as_ref().map(|agent| NotificationSourceRow {
            source_type: "agent".to_string(),
            source_id: Some(agent.public_id.clone()),
            source_name: agent_name.clone(),
        });

        let title = chat_title(&session, agent_name.as_deref());
        let body = turn_body(&outcome, duration_ms);
        let input_message_id = candidate.input_message_id;

        let notification = self
            .db
            .create_notification(CreateNotificationRow {
                org_id: candidate.org_id,
                user_id: candidate.user_id,
                kind: kind.to_string(),
                title,
                body,
                target_type: Some("session".to_string()),
                target_id: Some(candidate.session_id.to_string()),
                // The chat view, not the operator's session inspector: this is
                // the conversation the user was waiting on.
                href: Some(format!("/chats/{}", candidate.session_id)),
                payload: json!({
                    "turn_id": turn_event.context.turn_id.as_ref().map(ToString::to_string),
                    "input_message_id": input_message_id.to_string(),
                    "duration_ms": duration_ms,
                    "session_id": candidate.session_id.to_string(),
                }),
                dedupe_key: Some(format!("turn-long-complete:{input_message_id}")),
                source,
            })
            .await?;

        Ok(Some(notification))
    }
}

/// A chat thread the recipient owns. Turn notifications fire only for these.
fn is_own_chat(session: &SessionRow, user_id: Uuid) -> bool {
    session.source == "chat" && session.resolved_owner_user_id == Some(user_id)
}

/// The chat's name as the user sees it in the sidebar.
fn chat_title(session: &SessionRow, agent_name: Option<&str>) -> String {
    if let Some(title) = session
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        return title.to_string();
    }
    match agent_name {
        Some(agent) => format!("Chat with {agent}"),
        None => "Untitled chat".to_string(),
    }
}

/// What happened, in a sentence: how long it took and how it ended, followed
/// by the start of the answer so the user can tell whether to open the chat.
fn turn_body(outcome: &TurnOutcome<'_>, duration_ms: u64) -> String {
    let duration = format_duration(duration_ms);
    match outcome {
        TurnOutcome::Completed { answer } => match answer.and_then(answer_excerpt) {
            Some(excerpt) => format!("Replied after {duration}: {excerpt}"),
            None => format!("Finished after {duration}."),
        },
        TurnOutcome::Failed => {
            format!("Stopped with an error after {duration}. Open the chat to see what happened.")
        }
    }
}

/// First words of an answer as plain text: markdown markers dropped,
/// whitespace collapsed, cut on a word boundary.
fn answer_excerpt(answer: &str) -> Option<String> {
    let plain = answer
        .lines()
        .map(|line| line.trim().trim_start_matches(['#', '>', '-', '*']).trim())
        .filter(|line| !line.starts_with("```"))
        .collect::<Vec<_>>()
        .join(" ")
        .replace("**", "")
        .replace('`', "");
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if plain.is_empty() {
        return None;
    }
    if plain.chars().count() <= ANSWER_EXCERPT_CHARS {
        return Some(plain);
    }
    let cut: String = plain.chars().take(ANSWER_EXCERPT_CHARS).collect();
    let cut = match cut.rfind(' ') {
        Some(at) if at > ANSWER_EXCERPT_CHARS / 2 => &cut[..at],
        _ => cut.as_str(),
    };
    Some(format!(
        "{}…",
        cut.trim_end_matches([',', '.', ';', ':', ' '])
    ))
}

pub struct NotificationEventListener {
    service: Arc<NotificationService>,
}

impl NotificationEventListener {
    pub fn new(service: Arc<NotificationService>) -> Self {
        Self { service }
    }

    async fn handle(&self, event: &Event) -> anyhow::Result<()> {
        let Some(input_message_id) = event.context.input_message_id else {
            return Ok(());
        };
        match &event.data {
            EventData::TurnCompleted(data) => {
                let Some(duration_ms) = data.duration_ms else {
                    return Ok(());
                };
                if duration_ms < LONG_RUNNING_TURN_THRESHOLD_MS {
                    return Ok(());
                }
                let Some(candidate) = self
                    .service
                    .long_running_turn_candidate(input_message_id)
                    .await?
                else {
                    return Ok(());
                };
                let outcome = TurnOutcome::Completed {
                    answer: data.final_answer_preview.as_deref(),
                };
                self.service
                    .create_long_running_turn_notification(event, candidate, duration_ms, outcome)
                    .await?;
            }
            EventData::TurnFailed(_) => {
                let Some(candidate) = self
                    .service
                    .long_running_turn_candidate(input_message_id)
                    .await?
                else {
                    return Ok(());
                };
                // turn.failed carries no duration; the request row was written
                // when the user sent the message, which is when they started
                // waiting.
                let duration_ms =
                    (event.ts - candidate.created_at).num_milliseconds().max(0) as u64;
                if duration_ms < LONG_RUNNING_TURN_THRESHOLD_MS {
                    return Ok(());
                }
                self.service
                    .create_long_running_turn_notification(
                        event,
                        candidate,
                        duration_ms,
                        TurnOutcome::Failed,
                    )
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[async_trait]
impl EventListener for NotificationEventListener {
    #[instrument(skip(self, event), fields(event_id = %event.id, session_id = %event.session_id))]
    async fn on_event(&self, event: &Event) {
        if let Err(e) = self.handle(event).await {
            error!(error = %e, "Failed to create turn notification");
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![TURN_COMPLETED, TURN_FAILED])
    }

    fn name(&self) -> &'static str {
        "NotificationEventListener"
    }
}

fn format_duration(duration_ms: u64) -> String {
    let total_seconds = duration_ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes == 0 {
        format!("{seconds}s")
    } else {
        format!("{minutes}m {seconds:02}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::sessions::record::SessionSource;
    use crate::storage::StorageBackend;
    use crate::storage::test_database::test_session_row;
    use everruns_contracts::typed_id::{AgentId, TurnId};
    use everruns_core::Event;
    use everruns_core::events::{EventContext, TurnCompletedData, TurnFailedData};

    const ORG: i64 = 1;

    fn turn_completed_event(
        session_id: SessionId,
        input_message_id: MessageId,
        duration_ms: u64,
        answer: Option<&str>,
    ) -> Event {
        let turn_id = TurnId::new();
        Event {
            id: everruns_contracts::typed_id::EventId::new(),
            event_type: TURN_COMPLETED.to_string(),
            ts: chrono::Utc::now(),
            session_id,
            context: EventContext::turn(turn_id, input_message_id),
            data: EventData::TurnCompleted(TurnCompletedData {
                turn_id,
                iterations: 1,
                duration_ms: Some(duration_ms),
                final_answer_preview: answer.map(str::to_string),
                status: Some("completed".to_string()),
                ..Default::default()
            }),
            metadata: None,
            tags: None,
            sequence: Some(1),
        }
    }

    struct Fixture {
        db: Arc<StorageBackend>,
        service: Arc<NotificationService>,
        listener: NotificationEventListener,
        user_id: Uuid,
    }

    impl Fixture {
        async fn new() -> Self {
            let db = Arc::new(StorageBackend::test_database());
            let service = Arc::new(NotificationService::new(db.clone()));
            let listener = NotificationEventListener::new(service.clone());
            let user_id = db.create_test_user(Uuid::now_v7()).await;
            Self {
                db,
                service,
                listener,
                user_id,
            }
        }

        async fn session(
            &self,
            source: SessionSource,
            owner: Option<Uuid>,
            title: Option<&str>,
        ) -> SessionId {
            let agent = self.db.create_test_agent(ORG, Uuid::now_v7()).await;
            sqlx::query("UPDATE agents SET display_name = 'Research Assistant' WHERE id = $1")
                .bind(agent)
                .execute(self.db.database().pool())
                .await
                .unwrap();
            let mut row = test_session_row(ORG);
            row.source = source;
            row.resolved_owner_user_id = owner;
            row.title = title.map(str::to_string);
            row.agent_id = Some(AgentId::from_uuid(agent));
            self.db.create_session(row).await.unwrap().id
        }

        /// The user sends a message in `session_id`, and the turn it starts
        /// completes after `duration_ms`.
        async fn complete_turn(
            &self,
            session_id: SessionId,
            duration_ms: u64,
            answer: Option<&str>,
        ) -> MessageId {
            let input_message_id = MessageId::new();
            self.service
                .create_turn_request(ORG, self.user_id, session_id, input_message_id)
                .await
                .unwrap();
            self.listener
                .on_event(&turn_completed_event(
                    session_id,
                    input_message_id,
                    duration_ms,
                    answer,
                ))
                .await;
            input_message_id
        }

        async fn notifications(&self) -> Vec<NotificationRow> {
            self.service.list(ORG, self.user_id, 10).await.unwrap()
        }
    }

    #[tokio::test]
    async fn long_running_turn_in_own_chat_says_what_happened_and_opens_the_chat() {
        let f = Fixture::new().await;
        let session_id = f
            .session(
                SessionSource::Chat,
                Some(f.user_id),
                Some("Q3 marketing brief"),
            )
            .await;

        f.complete_turn(
            session_id,
            1_228_000,
            Some("## Plan\n\nHere is the **Q3 plan** covering the three pillars."),
        )
        .await;

        let notifications = f.notifications().await;
        assert_eq!(notifications.len(), 1);
        let n = &notifications[0];
        assert_eq!(n.kind, LONG_RUNNING_TURN_COMPLETED_KIND);
        assert_eq!(n.title, "Q3 marketing brief");
        assert_eq!(
            n.body,
            "Replied after 20m 28s: Plan Here is the Q3 plan covering the three pillars."
        );
        assert_eq!(
            n.href.as_deref(),
            Some(format!("/chats/{session_id}").as_str())
        );
        assert_eq!(n.source_type.as_deref(), Some("agent"));
        assert_eq!(n.source_name.as_deref(), Some("Research Assistant"));
        assert!(n.source_id.as_deref().unwrap().starts_with("agent_"));
        assert_eq!(f.service.count_unviewed(ORG, f.user_id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn untitled_chat_is_named_after_its_agent() {
        let f = Fixture::new().await;
        let session_id = f.session(SessionSource::Chat, Some(f.user_id), None).await;

        f.complete_turn(session_id, 72_000, None).await;

        let n = &f.notifications().await[0];
        assert_eq!(n.title, "Chat with Research Assistant");
        assert_eq!(n.body, "Finished after 1m 12s.");
    }

    #[tokio::test]
    async fn skips_sessions_that_are_not_the_users_own_chats() {
        let f = Fixture::new().await;
        let other_user = f.db.create_test_user(Uuid::now_v7()).await;
        let api_session = f.session(SessionSource::Api, Some(f.user_id), None).await;
        let someone_elses_chat = f.session(SessionSource::Chat, Some(other_user), None).await;
        let unowned_chat = f.session(SessionSource::Chat, None, None).await;

        for session_id in [api_session, someone_elses_chat, unowned_chat] {
            f.complete_turn(session_id, 120_000, Some("done")).await;
        }

        assert!(f.notifications().await.is_empty());
    }

    #[tokio::test]
    async fn skips_short_turns() {
        let f = Fixture::new().await;
        let session_id = f.session(SessionSource::Chat, Some(f.user_id), None).await;

        f.complete_turn(session_id, 59_000, Some("quick")).await;

        assert!(f.notifications().await.is_empty());
    }

    #[tokio::test]
    async fn long_running_failure_is_reported() {
        let f = Fixture::new().await;
        let session_id = f
            .session(SessionSource::Chat, Some(f.user_id), Some("Migration"))
            .await;
        let input_message_id = MessageId::new();
        f.service
            .create_turn_request(ORG, f.user_id, session_id, input_message_id)
            .await
            .unwrap();
        let turn_id = TurnId::new();
        let failed = |after: chrono::Duration| Event {
            id: everruns_contracts::typed_id::EventId::new(),
            event_type: TURN_FAILED.to_string(),
            ts: chrono::Utc::now() + after,
            session_id,
            context: EventContext::turn(turn_id, input_message_id),
            data: EventData::TurnFailed(TurnFailedData {
                turn_id,
                error: "provider unavailable".to_string(),
                error_code: None,
                error_fields: None,
                error_disclosure: None,
            }),
            metadata: None,
            tags: None,
            sequence: Some(2),
        };

        f.listener.on_event(&failed(chrono::Duration::zero())).await;
        assert!(
            f.notifications().await.is_empty(),
            "a quick failure is not news"
        );

        f.listener
            .on_event(&failed(chrono::Duration::minutes(5)))
            .await;
        let notifications = f.notifications().await;
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].kind, LONG_RUNNING_TURN_FAILED_KIND);
        assert!(
            notifications[0]
                .body
                .starts_with("Stopped with an error after 5m"),
            "{}",
            notifications[0].body
        );
    }

    #[tokio::test]
    async fn dedupes_same_turn_notification_and_marks_viewed() {
        let f = Fixture::new().await;
        let session_id = f.session(SessionSource::Chat, Some(f.user_id), None).await;
        let input_message_id = MessageId::new();
        f.service
            .create_turn_request(ORG, f.user_id, session_id, input_message_id)
            .await
            .unwrap();

        let event = turn_completed_event(session_id, input_message_id, 65_000, None);
        f.listener.on_event(&event).await;
        f.listener.on_event(&event).await;

        let notifications = f.notifications().await;
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].occurrence_count, 2);

        let viewed = f
            .service
            .mark_viewed(ORG, f.user_id, notifications[0].id)
            .await
            .unwrap()
            .unwrap();
        assert!(viewed.viewed_at.is_some());
        assert_eq!(f.service.count_unviewed(ORG, f.user_id).await.unwrap(), 0);
    }

    #[test]
    fn answer_excerpt_cuts_long_answers_on_a_word() {
        let long = "word ".repeat(100);
        let excerpt = answer_excerpt(&long).unwrap();
        assert!(excerpt.ends_with('…'));
        assert!(excerpt.chars().count() <= ANSWER_EXCERPT_CHARS + 1);
        assert!(!excerpt.contains("  "));
        assert_eq!(answer_excerpt("```\n\n```"), None);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(45_000), "45s");
        assert_eq!(format_duration(1_228_000), "20m 28s");
        assert_eq!(format_duration(3_900_000), "1h 05m");
    }
}
