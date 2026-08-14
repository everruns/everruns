use async_trait::async_trait;
use everruns::SessionId;
use everruns_core::events::{Event, EventRequest};
use everruns_host::{
    EnvironmentBindingError, EnvironmentBindingStore, EventCursor, EventDurability, EventLog,
    EventLogError, EventPage, EventReadRequest, EventReader, WorkspaceBinding, WorkspaceHeadAccess,
};
use everruns_provider::typed_id::EventId;
use sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct PostgresEventLog {
    pool: PgPool,
}

impl PostgresEventLog {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventReader for PostgresEventLog {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        let session_id = request.session_id();
        let current_high: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) FROM everruns_scale_events WHERE session_id = $1",
        )
        .bind(session_id.uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(event_backend)?;

        let (after, snapshot) = match request.cursor() {
            Some(cursor) => {
                if cursor.session_id() != session_id {
                    return Err(EventLogError::CrossSessionCursor {
                        detail: "cursor belongs to another session".into(),
                    });
                }
                let snapshot = cursor.snapshot_high_watermark().unwrap_or(current_high);
                if cursor.after_sequence() > snapshot {
                    return Err(EventLogError::IncompatibleCursor {
                        detail: "cursor position exceeds its snapshot".into(),
                    });
                }
                if snapshot > current_high {
                    return Err(EventLogError::ExpiredCursor {
                        detail: "cursor snapshot is not available in this log".into(),
                    });
                }
                (cursor.after_sequence(), snapshot)
            }
            None => (0, current_high),
        };

        if snapshot == 0 {
            return EventPage::new(Vec::new(), None, 0);
        }
        let fetch_limit = i64::try_from(request.limit().get() + 1).unwrap_or(i64::MAX);
        let rows: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT envelope FROM everruns_scale_events \
             WHERE session_id = $1 AND sequence > $2 AND sequence <= $3 \
             ORDER BY sequence ASC LIMIT $4",
        )
        .bind(session_id.uuid())
        .bind(after)
        .bind(snapshot)
        .bind(fetch_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(event_backend)?;
        let mut events = rows
            .into_iter()
            .map(|value| {
                serde_json::from_value::<Event>(value).map_err(|error| EventLogError::Corruption {
                    detail: error.to_string(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = events.len() > request.limit().get();
        if has_more {
            events.pop();
        }
        let next_cursor = has_more
            .then(|| {
                EventCursor::continuation(
                    session_id,
                    events
                        .last()
                        .and_then(|event| event.sequence)
                        .expect("a page with more events returned at least one event"),
                    snapshot,
                )
            })
            .transpose()?;
        EventPage::new(events, next_cursor, snapshot)
    }
}

#[async_trait]
impl EventLog for PostgresEventLog {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        if request.is_ephemeral() {
            return Err(EventLogError::InvalidAppend {
                detail: format!(
                    "ephemeral event {} must be routed sink-only",
                    request.event_type
                ),
            });
        }
        let session_id = request.session_id;
        let mut transaction = self.pool.begin().await.map_err(event_backend)?;
        // THREAT[TM-DURABLE-009]: one transaction-scoped lock makes sequence
        // assignment and append a single canonical authority per session.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(session_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(event_backend)?;
        let sequence: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM everruns_scale_events WHERE session_id = $1",
        )
        .bind(session_id.uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(event_backend)?;
        let event = request.into_event(EventId::new(), sequence);
        let envelope =
            serde_json::to_value(&event).map_err(|error| EventLogError::InvalidAppend {
                detail: error.to_string(),
            })?;
        sqlx::query(
            "INSERT INTO everruns_scale_events (session_id, sequence, event_id, envelope) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(session_id.uuid())
        .bind(sequence)
        .bind(event.id.uuid())
        .bind(envelope)
        .execute(&mut *transaction)
        .await
        .map_err(event_backend)?;
        transaction.commit().await.map_err(event_backend)?;
        Ok(event)
    }

    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}

fn event_backend(error: sqlx::Error) -> EventLogError {
    EventLogError::Backend {
        detail: error.to_string(),
    }
}

#[derive(Clone)]
pub(crate) struct PostgresEnvironmentBindingStore {
    pool: PgPool,
}

impl PostgresEnvironmentBindingStore {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EnvironmentBindingStore for PostgresEnvironmentBindingStore {
    async fn load(
        &self,
        session_id: SessionId,
    ) -> Result<Option<WorkspaceBinding>, EnvironmentBindingError> {
        let value: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT binding FROM everruns_scale_environment_bindings WHERE session_id = $1",
        )
        .bind(session_id.uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| EnvironmentBindingError::Unavailable)?;
        value
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| EnvironmentBindingError::Corrupt)
    }

    async fn bind(
        &self,
        session_id: SessionId,
        binding: &WorkspaceBinding,
    ) -> Result<(), EnvironmentBindingError> {
        binding
            .validate()
            .map_err(|_| EnvironmentBindingError::Corrupt)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| EnvironmentBindingError::Unavailable)?;
        let head_key = format!(
            "{}:{}:{}",
            binding.provider_id, binding.workspace_id, binding.head_id
        );
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(head_key)
            .execute(&mut *transaction)
            .await
            .map_err(|_| EnvironmentBindingError::Unavailable)?;

        let existing: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT binding FROM everruns_scale_environment_bindings WHERE session_id = $1",
        )
        .bind(session_id.uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| EnvironmentBindingError::Unavailable)?;
        if let Some(existing) = existing {
            let existing: WorkspaceBinding =
                serde_json::from_value(existing).map_err(|_| EnvironmentBindingError::Corrupt)?;
            return if &existing == binding {
                Ok(())
            } else {
                Err(EnvironmentBindingError::Conflict)
            };
        }

        let isolated = matches!(binding.access, WorkspaceHeadAccess::Isolated);
        let incompatible: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM everruns_scale_environment_bindings \
             WHERE provider_id = $1 AND workspace_id = $2 AND head_id = $3 \
             AND session_id <> $4 AND (isolated OR $5))",
        )
        .bind(binding.provider_id.to_string())
        .bind(binding.workspace_id.to_string())
        .bind(binding.head_id.to_string())
        .bind(session_id.uuid())
        .bind(isolated)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| EnvironmentBindingError::Unavailable)?;
        if incompatible {
            return Err(EnvironmentBindingError::Conflict);
        }

        let value = serde_json::to_value(binding).map_err(|_| EnvironmentBindingError::Corrupt)?;
        sqlx::query(
            "INSERT INTO everruns_scale_environment_bindings \
             (session_id, provider_id, workspace_id, head_id, isolated, binding) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session_id.uuid())
        .bind(binding.provider_id.to_string())
        .bind(binding.workspace_id.to_string())
        .bind(binding.head_id.to_string())
        .bind(isolated)
        .bind(value)
        .execute(&mut *transaction)
        .await
        .map_err(|_| EnvironmentBindingError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| EnvironmentBindingError::Unavailable)
    }
}
