//! Session Schedule Capability
//!
//! Provides tools for scheduling future tasks within a session:
//! - `schedule_session_task`: Schedule work at a specific time
//! - `cancel_session_schedule`: Cancel a pending schedule
//! - `list_session_schedules`: List all schedules for the current session

use super::{Capability, CapabilityStatus};
use crate::session_schedule::CreateSessionSchedule;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Session schedule capability - schedule future tasks within a session.
pub struct SessionScheduleCapability;

impl Capability for SessionScheduleCapability {
    fn id(&self) -> &str {
        "session_schedule"
    }

    fn name(&self) -> &str {
        "Session Schedule"
    }

    fn description(&self) -> &str {
        "Schedule future tasks within the current session. At the scheduled time, a message is injected and triggers a new turn."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("calendar-clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Session")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You can schedule future tasks within this session:

- `schedule_session_task`: Schedule work to be done at a specific time. Provide a description of the task and an ISO 8601 datetime. At the scheduled time, a message will appear in the session and you will execute the task.
- `cancel_session_schedule`: Cancel a pending scheduled task by its ID.
- `list_session_schedules`: List all scheduled tasks for this session."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ScheduleSessionTaskTool),
            Box::new(CancelSessionScheduleTool),
            Box::new(ListSessionSchedulesTool),
        ]
    }
}

// ============================================================================
// Tool: schedule_session_task
// ============================================================================

pub struct ScheduleSessionTaskTool;

#[async_trait]
impl Tool for ScheduleSessionTaskTool {
    fn name(&self) -> &str {
        "schedule_session_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Schedule Session Task")
    }

    fn description(&self) -> &str {
        "Schedule a task to be executed at a specific time within this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "What to do when the scheduled time arrives"
                },
                "scheduled_at": {
                    "type": "string",
                    "description": "ISO 8601 datetime for when to execute (e.g., 2024-01-15T03:00:00Z)"
                }
            },
            "required": ["description", "scheduled_at"],
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "schedule_session_task requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let description = match arguments.get("description").and_then(|v| v.as_str()) {
            Some(d) if !d.trim().is_empty() => d.trim().to_string(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: description"),
        };

        let scheduled_at_str = match arguments.get("scheduled_at").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: scheduled_at");
            }
        };

        let scheduled_at = match chrono::DateTime::parse_from_rfc3339(scheduled_at_str) {
            Ok(dt) => dt.with_timezone(&chrono::Utc),
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid scheduled_at format. Use ISO 8601 (e.g., 2024-01-15T03:00:00Z): {}",
                    e
                ));
            }
        };

        // Validate that scheduled time is in the future
        if scheduled_at <= chrono::Utc::now() {
            return ToolExecutionResult::tool_error("scheduled_at must be in the future");
        }

        let Some(schedule_store) = &context.schedule_store else {
            return ToolExecutionResult::tool_error("Schedule store not available in this context");
        };

        // We need session info to populate org/harness/agent fields.
        // These are obtained from the session store.
        let Some(session_store) = &context.session_store else {
            return ToolExecutionResult::tool_error("Session store not available in this context");
        };

        let session = match session_store.get_session(context.session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => return ToolExecutionResult::tool_error("Session not found"),
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        let input = CreateSessionSchedule {
            session_id: context.session_id,
            organization_id: session.organization_id.clone(),
            harness_id: session.harness_id,
            agent_id: session.agent_id,
            description: description.clone(),
            scheduled_at,
        };

        match schedule_store.create_schedule(input).await {
            Ok(schedule) => ToolExecutionResult::success(json!({
                "schedule_id": schedule.id.to_string(),
                "description": schedule.description,
                "scheduled_at": schedule.scheduled_at.to_rfc3339(),
                "status": schedule.status.to_string(),
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

// ============================================================================
// Tool: cancel_session_schedule
// ============================================================================

pub struct CancelSessionScheduleTool;

#[async_trait]
impl Tool for CancelSessionScheduleTool {
    fn name(&self) -> &str {
        "cancel_session_schedule"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Cancel Session Schedule")
    }

    fn description(&self) -> &str {
        "Cancel a pending scheduled task."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "schedule_id": {
                    "type": "string",
                    "description": "ID of the schedule to cancel"
                }
            },
            "required": ["schedule_id"],
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("cancel_session_schedule requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let schedule_id_str = match arguments.get("schedule_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: schedule_id");
            }
        };

        let schedule_id = match uuid::Uuid::parse_str(schedule_id_str) {
            Ok(id) => id,
            Err(e) => {
                return ToolExecutionResult::tool_error(format!("Invalid schedule_id: {}", e));
            }
        };

        let Some(schedule_store) = &context.schedule_store else {
            return ToolExecutionResult::tool_error("Schedule store not available in this context");
        };

        match schedule_store.cancel_schedule(schedule_id).await {
            Ok(Some(schedule)) => ToolExecutionResult::success(json!({
                "schedule_id": schedule.id.to_string(),
                "status": schedule.status.to_string(),
                "cancelled": true,
            })),
            Ok(None) => ToolExecutionResult::tool_error("Schedule not found or already completed"),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

// ============================================================================
// Tool: list_session_schedules
// ============================================================================

pub struct ListSessionSchedulesTool;

#[async_trait]
impl Tool for ListSessionSchedulesTool {
    fn name(&self) -> &str {
        "list_session_schedules"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Session Schedules")
    }

    fn description(&self) -> &str {
        "List all scheduled tasks for the current session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_session_schedules requires context.")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(schedule_store) = &context.schedule_store else {
            return ToolExecutionResult::tool_error("Schedule store not available in this context");
        };

        match schedule_store.list_schedules(context.session_id).await {
            Ok(schedules) => {
                let items: Vec<Value> = schedules
                    .iter()
                    .map(|s| {
                        json!({
                            "schedule_id": s.id.to_string(),
                            "description": s.description,
                            "scheduled_at": s.scheduled_at.to_rfc3339(),
                            "status": s.status.to_string(),
                            "created_at": s.created_at.to_rfc3339(),
                        })
                    })
                    .collect();
                ToolExecutionResult::success(json!({
                    "schedules": items,
                    "count": items.len(),
                }))
            }
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_schedule::{SessionSchedule, SessionScheduleStatus};
    use crate::traits::SessionScheduleStore;
    use crate::typed_id::{HarnessId, MessageId, SessionId};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MockScheduleStore {
        schedules: Arc<Mutex<Vec<SessionSchedule>>>,
    }

    #[async_trait]
    impl SessionScheduleStore for MockScheduleStore {
        async fn create_schedule(
            &self,
            input: CreateSessionSchedule,
        ) -> crate::error::Result<SessionSchedule> {
            let schedule = SessionSchedule {
                id: uuid::Uuid::now_v7(),
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
                created_at: chrono::Utc::now(),
            };
            self.schedules
                .lock()
                .expect("poisoned")
                .push(schedule.clone());
            Ok(schedule)
        }

        async fn cancel_schedule(
            &self,
            schedule_id: uuid::Uuid,
        ) -> crate::error::Result<Option<SessionSchedule>> {
            let mut schedules = self.schedules.lock().expect("poisoned");
            if let Some(s) = schedules.iter_mut().find(|s| s.id == schedule_id) {
                s.status = SessionScheduleStatus::Cancelled;
                Ok(Some(s.clone()))
            } else {
                Ok(None)
            }
        }

        async fn list_schedules(
            &self,
            session_id: SessionId,
        ) -> crate::error::Result<Vec<SessionSchedule>> {
            let schedules = self.schedules.lock().expect("poisoned");
            Ok(schedules
                .iter()
                .filter(|s| s.session_id == session_id)
                .cloned()
                .collect())
        }

        async fn claim_due_schedules(
            &self,
            _limit: u32,
        ) -> crate::error::Result<Vec<SessionSchedule>> {
            Ok(vec![])
        }

        async fn mark_triggered(
            &self,
            _schedule_id: uuid::Uuid,
            _message_id: MessageId,
        ) -> crate::error::Result<()> {
            Ok(())
        }

        async fn mark_failed(
            &self,
            _schedule_id: uuid::Uuid,
            _error: &str,
        ) -> crate::error::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct MockSessionStore {
        session: Arc<Mutex<Option<crate::session::Session>>>,
    }

    #[async_trait]
    impl crate::traits::SessionStore for MockSessionStore {
        async fn get_session(
            &self,
            _session_id: SessionId,
        ) -> crate::error::Result<Option<crate::session::Session>> {
            Ok(self.session.lock().expect("poisoned").clone())
        }
    }

    fn build_session(session_id: SessionId) -> crate::session::Session {
        crate::session::Session {
            id: session_id,
            organization_id: "org_00000000000000000000000000000001".to_string(),
            harness_id: HarnessId::new(),
            agent_id: None,
            title: None,
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            status: crate::session::SessionStatus::Idle,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
        }
    }

    #[tokio::test]
    async fn schedule_task_creates_schedule() {
        let session_id = SessionId::new();
        let session = build_session(session_id);
        let schedule_store = MockScheduleStore::default();

        let context = ToolContext::new(session_id)
            .with_session_store(Arc::new(MockSessionStore {
                session: Arc::new(Mutex::new(Some(session))),
            }))
            .with_schedule_store(Arc::new(schedule_store.clone()));

        let tool = ScheduleSessionTaskTool;
        let future_time = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let result = tool
            .execute_with_context(
                json!({
                    "description": "Deploy to staging",
                    "scheduled_at": future_time,
                }),
                &context,
            )
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["description"], "Deploy to staging");
                assert_eq!(value["status"], "pending");
                assert!(value["schedule_id"].is_string());
            }
            other => panic!("expected success, got {:?}", other),
        }

        assert_eq!(schedule_store.schedules.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn schedule_task_rejects_past_time() {
        let session_id = SessionId::new();
        let session = build_session(session_id);
        let schedule_store = MockScheduleStore::default();

        let context = ToolContext::new(session_id)
            .with_session_store(Arc::new(MockSessionStore {
                session: Arc::new(Mutex::new(Some(session))),
            }))
            .with_schedule_store(Arc::new(schedule_store));

        let tool = ScheduleSessionTaskTool;
        let past_time = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let result = tool
            .execute_with_context(
                json!({
                    "description": "Too late",
                    "scheduled_at": past_time,
                }),
                &context,
            )
            .await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("future"));
            }
            other => panic!("expected tool error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn list_schedules_returns_session_schedules() {
        let session_id = SessionId::new();
        let session = build_session(session_id);
        let schedule_store = MockScheduleStore::default();

        // Pre-populate a schedule
        let _ = schedule_store
            .create_schedule(CreateSessionSchedule {
                session_id,
                organization_id: "org_00000000000000000000000000000001".to_string(),
                harness_id: HarnessId::new(),
                agent_id: None,
                description: "Test task".to_string(),
                scheduled_at: chrono::Utc::now() + chrono::Duration::hours(2),
            })
            .await;

        let context = ToolContext::new(session_id)
            .with_session_store(Arc::new(MockSessionStore {
                session: Arc::new(Mutex::new(Some(session))),
            }))
            .with_schedule_store(Arc::new(schedule_store));

        let tool = ListSessionSchedulesTool;
        let result = tool.execute_with_context(json!({}), &context).await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["count"], 1);
                assert_eq!(value["schedules"][0]["description"], "Test task");
            }
            other => panic!("expected success, got {:?}", other),
        }
    }
}
