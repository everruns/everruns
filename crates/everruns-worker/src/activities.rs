// Session activities for workflow execution (M2)

use anyhow::Result;
use everruns_contracts::events::AgUiEvent;
use everruns_storage::repositories::Database;
use tracing::info;
use uuid::Uuid;

/// Activity to persist AG-UI events to the session_events table
pub struct PersistEventActivity {
    db: Database,
}

impl PersistEventActivity {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Persist an event to the session_events table
    pub async fn persist_event(&self, session_id: Uuid, event: AgUiEvent) -> Result<()> {
        let event_type = match &event {
            AgUiEvent::RunStarted(_) => "session.started",
            AgUiEvent::RunFinished(_) => "session.finished",
            AgUiEvent::RunError(_) => "session.error",
            AgUiEvent::StepStarted(_) => "step.started",
            AgUiEvent::StepFinished(_) => "step.finished",
            AgUiEvent::TextMessageStart(_) => "text.start",
            AgUiEvent::TextMessageContent(_) => "text.delta",
            AgUiEvent::TextMessageEnd(_) => "text.end",
            AgUiEvent::ToolCallStart(_) => "tool.call.start",
            AgUiEvent::ToolCallArgs(_) => "tool.call.args",
            AgUiEvent::ToolCallEnd(_) => "tool.call.end",
            AgUiEvent::ToolCallResult(_) => "tool.result",
            AgUiEvent::StateSnapshot(_) => "state.snapshot",
            AgUiEvent::StateDelta(_) => "state.delta",
            AgUiEvent::MessagesSnapshot(_) => "messages.snapshot",
            AgUiEvent::Custom(_) => "custom",
        };

        let event_data = serde_json::to_value(&event)?;

        // Insert into events table with auto-incrementing sequence
        sqlx::query(
            r#"
            INSERT INTO events (session_id, sequence, event_type, data)
            VALUES ($1, COALESCE((SELECT MAX(sequence) + 1 FROM events WHERE session_id = $1), 1), $2, $3)
            "#,
        )
        .bind(session_id)
        .bind(event_type)
        .bind(event_data)
        .execute(self.db.pool())
        .await?;

        info!(
            session_id = %session_id,
            event_type = %event_type,
            "Persisted event"
        );

        Ok(())
    }
}
