// Session schedule poller
//
// Background loop that claims due session schedules and triggers turns.
// Each triggered schedule injects a user message into the session with
// metadata.source = "schedule", then starts a turn workflow.

use crate::execution_metadata;
use crate::services::{EventService, SessionScheduleService};
use crate::storage::StorageBackend;
use chrono::Utc;
use everruns_core::events::{EventContext, EventRequest, InputMessageData};
use everruns_core::typed_id::{MessageId, SessionId};
use everruns_core::{ContentPart, Message, MessageRole, TextContentPart};
use everruns_worker::AgentRunner;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Spawn the session schedule poller as a background task.
///
/// Polls every `poll_interval` for due schedules, injects messages, and
/// triggers turns. Safe for concurrent execution via `FOR UPDATE SKIP LOCKED`.
pub fn spawn_session_scheduler(
    db: Arc<StorageBackend>,
    schedule_service: Arc<SessionScheduleService>,
    event_service: Arc<EventService>,
    runner: Arc<dyn AgentRunner>,
    poll_interval: Duration,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(poll_interval);
        // Skip the immediate first tick — let the server finish starting.
        interval.tick().await;

        tracing::info!(
            poll_interval_secs = poll_interval.as_secs(),
            "Started session schedule poller"
        );

        loop {
            interval.tick().await;
            if let Err(e) = poll_and_trigger(&db, &schedule_service, &event_service, &runner).await
            {
                tracing::error!(error = %e, "Session schedule poll failed");
            }
        }
    });
}

/// One iteration of the poll loop.
async fn poll_and_trigger(
    db: &Arc<StorageBackend>,
    schedule_service: &Arc<SessionScheduleService>,
    event_service: &Arc<EventService>,
    runner: &Arc<dyn AgentRunner>,
) -> anyhow::Result<()> {
    let due = db.claim_due_session_schedules(10).await?;
    if due.is_empty() {
        return Ok(());
    }

    tracing::info!(count = due.len(), "Claimed due session schedules");

    for row in due {
        let schedule_id = row.id;
        let session_id = row.session_id;
        let org_id = row.org_id;
        let description = row.description.clone();

        // Mark triggered (updates next_trigger, disables one-shot, etc.)
        if let Err(e) = schedule_service.mark_triggered(org_id, schedule_id).await {
            tracing::error!(
                schedule_id = %schedule_id,
                error = %e,
                "Failed to mark schedule as triggered"
            );
            continue;
        }

        // Look up session to get harness_id and agent_id for the runner.
        let session = match db.get_session(org_id, session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    "Session not found for schedule, skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::error!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    error = %e,
                    "Failed to fetch session for schedule"
                );
                continue;
            }
        };

        let harness_id = match session.harness_id {
            Some(h) => h,
            None => {
                tracing::warn!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    "Session has no harness_id, skipping"
                );
                continue;
            }
        };

        // Build injected message with metadata.source = "schedule"
        let message_id = Uuid::now_v7();
        let message_id_typed = MessageId::from_uuid(message_id);
        let now = Utc::now();

        let mut metadata = HashMap::new();
        metadata.insert(
            "source".to_string(),
            serde_json::Value::String("schedule".to_string()),
        );
        metadata.insert(
            "schedule_id".to_string(),
            serde_json::Value::String(schedule_id.to_string()),
        );

        let core_message = Message {
            id: message_id_typed,
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart::new(description))],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: Some(metadata),
            external_actor: None,
            created_at: now,
        };

        // Emit the input.message event
        let session_id_typed: SessionId = session_id;
        if let Err(e) = event_service
            .emit(
                EventRequest::new(
                    session_id_typed,
                    EventContext::empty(),
                    InputMessageData::new(core_message),
                )
                .with_metadata(execution_metadata::scheduled_run_metadata(
                    schedule_id,
                    session.agent_identity_id,
                )),
            )
            .await
        {
            tracing::error!(
                schedule_id = %schedule_id,
                session_id = %session_id,
                error = %e,
                "Failed to emit schedule message event"
            );
            continue;
        }

        // Trigger the turn workflow
        let runner = runner.clone();
        let agent_id = session.agent_id;
        tokio::spawn(async move {
            if let Err(e) = runner
                .start_run(
                    org_id,
                    session_id_typed,
                    harness_id,
                    agent_id,
                    message_id_typed,
                )
                .await
            {
                tracing::error!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    error = %e,
                    "Failed to start turn for scheduled message"
                );
            } else {
                tracing::info!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    "Schedule-triggered turn started"
                );
            }
        });
    }

    Ok(())
}
