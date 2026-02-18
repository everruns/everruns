// Session schedule service
//
// Provides both the SessionScheduleStore implementation (for tools)
// and the SessionSchedulePoller (for triggering due schedules).
//
// Two implementations:
// - InMemorySessionScheduleStore: for dev mode (no database)
// - DbSessionScheduleStore: for production (PostgreSQL)

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use everruns_core::session_schedule::{
    CreateSessionSchedule, SessionSchedule, SessionScheduleStatus,
};
use everruns_core::traits::SessionScheduleStore;
use everruns_core::typed_id::{MessageId, SessionId};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// In-Memory Implementation (dev mode)
// ============================================================================

/// In-memory session schedule store for dev mode
#[derive(Default)]
pub struct InMemorySessionScheduleStore {
    schedules: RwLock<HashMap<Uuid, SessionSchedule>>,
}

impl InMemorySessionScheduleStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionScheduleStore for InMemorySessionScheduleStore {
    async fn create_schedule(
        &self,
        input: CreateSessionSchedule,
    ) -> everruns_core::error::Result<SessionSchedule> {
        let schedule = SessionSchedule {
            id: Uuid::now_v7(),
            session_id: input.session_id,
            organization_id: input.organization_id,
            harness_id: input.harness_id,
            agent_id: input.agent_id,
            description: input.description,
            scheduled_at: input.scheduled_at,
            status: SessionScheduleStatus::Pending,
            triggered_at: None,
            message_id: None,
            error: None,
            created_at: Utc::now(),
        };
        self.schedules.write().insert(schedule.id, schedule.clone());
        Ok(schedule)
    }

    async fn cancel_schedule(
        &self,
        schedule_id: Uuid,
    ) -> everruns_core::error::Result<Option<SessionSchedule>> {
        let mut schedules = self.schedules.write();
        if let Some(schedule) = schedules.get_mut(&schedule_id)
            && schedule.status == SessionScheduleStatus::Pending
        {
            schedule.status = SessionScheduleStatus::Cancelled;
            return Ok(Some(schedule.clone()));
        }
        Ok(None)
    }

    async fn list_schedules(
        &self,
        session_id: SessionId,
    ) -> everruns_core::error::Result<Vec<SessionSchedule>> {
        let schedules = self.schedules.read();
        let mut result: Vec<SessionSchedule> = schedules
            .values()
            .filter(|s| s.session_id == session_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }

    async fn claim_due_schedules(
        &self,
        limit: u32,
    ) -> everruns_core::error::Result<Vec<SessionSchedule>> {
        let now = Utc::now();
        let schedules = self.schedules.read();
        let mut due: Vec<SessionSchedule> = schedules
            .values()
            .filter(|s| s.status == SessionScheduleStatus::Pending && s.scheduled_at <= now)
            .cloned()
            .collect();
        due.sort_by(|a, b| a.scheduled_at.cmp(&b.scheduled_at));
        due.truncate(limit as usize);
        Ok(due)
    }

    async fn mark_triggered(
        &self,
        schedule_id: Uuid,
        message_id: MessageId,
    ) -> everruns_core::error::Result<()> {
        let mut schedules = self.schedules.write();
        if let Some(schedule) = schedules.get_mut(&schedule_id) {
            schedule.status = SessionScheduleStatus::Triggered;
            schedule.triggered_at = Some(Utc::now());
            schedule.message_id = Some(message_id);
        }
        Ok(())
    }

    async fn mark_failed(
        &self,
        schedule_id: Uuid,
        error: &str,
    ) -> everruns_core::error::Result<()> {
        let mut schedules = self.schedules.write();
        if let Some(schedule) = schedules.get_mut(&schedule_id) {
            schedule.status = SessionScheduleStatus::Failed;
            schedule.error = Some(error.to_string());
        }
        Ok(())
    }
}

// ============================================================================
// PostgreSQL Implementation
// ============================================================================

/// PostgreSQL-backed session schedule store
pub struct DbSessionScheduleStore {
    pool: sqlx::PgPool,
}

impl DbSessionScheduleStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionScheduleStore for DbSessionScheduleStore {
    async fn create_schedule(
        &self,
        input: CreateSessionSchedule,
    ) -> everruns_core::error::Result<SessionSchedule> {
        let id = Uuid::now_v7();
        let now = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO session_schedules (id, session_id, organization_id, harness_id, agent_id, description, scheduled_at, status, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8)
            "#,
        )
        .bind(id)
        .bind(input.session_id.uuid())
        .bind(&input.organization_id)
        .bind(input.harness_id.to_string())
        .bind(input.agent_id.map(|a| a.to_string()))
        .bind(&input.description)
        .bind(input.scheduled_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| everruns_core::error::AgentLoopError::store(e.to_string()))?;

        Ok(SessionSchedule {
            id,
            session_id: input.session_id,
            organization_id: input.organization_id,
            harness_id: input.harness_id,
            agent_id: input.agent_id,
            description: input.description,
            scheduled_at: input.scheduled_at,
            status: SessionScheduleStatus::Pending,
            triggered_at: None,
            message_id: None,
            error: None,
            created_at: now,
        })
    }

    async fn cancel_schedule(
        &self,
        schedule_id: Uuid,
    ) -> everruns_core::error::Result<Option<SessionSchedule>> {
        let result = sqlx::query_as::<_, ScheduleRow>(
            r#"
            UPDATE session_schedules
            SET status = 'cancelled'
            WHERE id = $1 AND status = 'pending'
            RETURNING *
            "#,
        )
        .bind(schedule_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| everruns_core::error::AgentLoopError::store(e.to_string()))?;

        Ok(result.map(|r| r.into_schedule()))
    }

    async fn list_schedules(
        &self,
        session_id: SessionId,
    ) -> everruns_core::error::Result<Vec<SessionSchedule>> {
        let rows = sqlx::query_as::<_, ScheduleRow>(
            r#"
            SELECT * FROM session_schedules
            WHERE session_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(session_id.uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| everruns_core::error::AgentLoopError::store(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_schedule()).collect())
    }

    async fn claim_due_schedules(
        &self,
        limit: u32,
    ) -> everruns_core::error::Result<Vec<SessionSchedule>> {
        let rows = sqlx::query_as::<_, ScheduleRow>(
            r#"
            UPDATE session_schedules
            SET status = 'triggered', triggered_at = NOW()
            WHERE id IN (
                SELECT id FROM session_schedules
                WHERE status = 'pending' AND scheduled_at <= NOW()
                ORDER BY scheduled_at
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            RETURNING *
            "#,
        )
        .bind(limit as i32)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| everruns_core::error::AgentLoopError::store(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_schedule()).collect())
    }

    async fn mark_triggered(
        &self,
        schedule_id: Uuid,
        message_id: MessageId,
    ) -> everruns_core::error::Result<()> {
        sqlx::query(
            r#"
            UPDATE session_schedules
            SET status = 'triggered', triggered_at = NOW(), message_id = $2
            WHERE id = $1
            "#,
        )
        .bind(schedule_id)
        .bind(message_id.uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| everruns_core::error::AgentLoopError::store(e.to_string()))?;

        Ok(())
    }

    async fn mark_failed(
        &self,
        schedule_id: Uuid,
        error: &str,
    ) -> everruns_core::error::Result<()> {
        sqlx::query(
            r#"
            UPDATE session_schedules
            SET status = 'failed', error = $2
            WHERE id = $1
            "#,
        )
        .bind(schedule_id)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| everruns_core::error::AgentLoopError::store(e.to_string()))?;

        Ok(())
    }
}

/// Internal row type for PostgreSQL queries
#[derive(sqlx::FromRow)]
struct ScheduleRow {
    id: Uuid,
    session_id: Uuid,
    organization_id: String,
    harness_id: String,
    agent_id: Option<String>,
    description: String,
    scheduled_at: chrono::DateTime<Utc>,
    status: String,
    triggered_at: Option<chrono::DateTime<Utc>>,
    message_id: Option<Uuid>,
    error: Option<String>,
    created_at: chrono::DateTime<Utc>,
}

impl ScheduleRow {
    fn into_schedule(self) -> SessionSchedule {
        use everruns_core::typed_id::{AgentId, HarnessId};

        SessionSchedule {
            id: self.id,
            session_id: SessionId::from_uuid(self.session_id),
            organization_id: self.organization_id,
            harness_id: self.harness_id.parse().unwrap_or_else(|_| HarnessId::new()),
            agent_id: self.agent_id.and_then(|s| s.parse::<AgentId>().ok()),
            description: self.description,
            scheduled_at: self.scheduled_at,
            status: SessionScheduleStatus::from(self.status.as_str()),
            triggered_at: self.triggered_at,
            message_id: self.message_id.map(MessageId::from_uuid),
            error: self.error,
            created_at: self.created_at,
        }
    }
}

// ============================================================================
// Session Schedule Poller
// ============================================================================

use everruns_worker::AgentRunner;

/// Polls for due session schedules and triggers turns.
///
/// Runs as a background task alongside the server. On each tick:
/// 1. Claims due schedules from the store
/// 2. For each: creates an "app" role message, emits event, triggers turn
pub struct SessionSchedulePoller {
    schedule_store: Arc<dyn SessionScheduleStore>,
    event_service: crate::services::EventService,
    runner: Arc<dyn AgentRunner>,
    poll_interval: std::time::Duration,
}

impl SessionSchedulePoller {
    pub fn new(
        schedule_store: Arc<dyn SessionScheduleStore>,
        event_service: crate::services::EventService,
        runner: Arc<dyn AgentRunner>,
    ) -> Self {
        Self {
            schedule_store,
            event_service,
            runner,
            poll_interval: std::time::Duration::from_secs(1),
        }
    }

    /// Run the poller loop until the shutdown signal is received.
    ///
    /// `shutdown_rx`: when this watch channel receives `true`, the poller stops.
    pub async fn run(self, mut shutdown_rx: tokio::sync::watch::Receiver<bool>) {
        tracing::info!(
            "Session schedule poller started (interval: {:?})",
            self.poll_interval
        );

        let mut interval = tokio::time::interval(self.poll_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = shutdown_rx.changed() => {
                    if result.is_err() || *shutdown_rx.borrow() {
                        tracing::info!("Session schedule poller shutting down");
                        break;
                    }
                }
                _ = interval.tick() => {
                    if let Err(e) = self.process_due_schedules().await {
                        tracing::error!(error = %e, "Failed to process due session schedules");
                    }
                }
            }
        }
    }

    async fn process_due_schedules(&self) -> Result<()> {
        let due_schedules = self.schedule_store.claim_due_schedules(100).await?;

        if !due_schedules.is_empty() {
            tracing::info!(
                count = due_schedules.len(),
                "Processing due session schedules"
            );
        }

        for schedule in due_schedules {
            if let Err(e) = self.trigger_schedule(&schedule).await {
                tracing::error!(
                    schedule_id = %schedule.id,
                    session_id = %schedule.session_id,
                    error = %e,
                    "Failed to trigger session schedule"
                );
                let _ = self
                    .schedule_store
                    .mark_failed(schedule.id, &e.to_string())
                    .await;
            }
        }

        Ok(())
    }

    async fn trigger_schedule(&self, schedule: &SessionSchedule) -> Result<()> {
        use everruns_core::events::{EventContext, EventRequest, InputMessageData};

        // Create an "app" role message
        let message =
            everruns_core::Message::app(format!("Scheduled task: {}", schedule.description));
        let message_id = message.id;

        // Store as input.message event
        let event_request = EventRequest::new(
            schedule.session_id,
            EventContext::empty(),
            InputMessageData::new(message),
        );
        self.event_service.emit(event_request).await?;

        // Mark as triggered
        self.schedule_store
            .mark_triggered(schedule.id, message_id)
            .await?;

        // Start a turn
        let runner = self.runner.clone();
        let session_id = schedule.session_id;
        let harness_id = schedule.harness_id;
        let agent_id = schedule.agent_id;

        tokio::spawn(async move {
            // Parse org_id from organization_id string (e.g., "org_xxx" or numeric)
            // For now, use default org_id since schedule already has the org context
            let org_id = 1i64; // TODO: parse from schedule.organization_id

            if let Err(e) = runner
                .start_run(org_id, session_id, harness_id, agent_id, message_id)
                .await
            {
                tracing::error!(
                    session_id = %session_id,
                    message_id = %message_id,
                    error = %e,
                    "Failed to start turn for scheduled task"
                );
            } else {
                tracing::info!(
                    session_id = %session_id,
                    message_id = %message_id,
                    "Turn started for scheduled task"
                );
            }
        });

        Ok(())
    }
}
