//! Session Schedule Capability
//!
//! Provides tools for scheduling future work within a session:
//! - `create_schedule`: Schedule a task (one-shot or recurring cron)
//! - `cancel_schedule`: Cancel/disable a schedule
//! - `list_schedules`: List all schedules for the current session

use super::{Capability, CapabilityStatus};
use crate::session_schedule::MAX_ACTIVE_SCHEDULES_PER_SESSION;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

pub const SESSION_SCHEDULE_CAPABILITY_ID: &str = "session_schedule";

/// Session schedule capability — lets the agent schedule future work.
pub struct SessionScheduleCapability;

impl Capability for SessionScheduleCapability {
    fn id(&self) -> &str {
        SESSION_SCHEDULE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Schedules"
    }

    fn description(&self) -> &str {
        "Schedule future tasks within the current session. Supports one-shot and recurring (cron) schedules."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Session")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "When a schedule fires, you will receive a message with the task description and should execute it. Maximum 5 active schedules per session.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(CreateScheduleTool),
            Box::new(CancelScheduleTool),
            Box::new(ListSchedulesTool),
        ]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["schedules"]
    }
}

// ============================================================================
// create_schedule tool
// ============================================================================

pub struct CreateScheduleTool;

#[async_trait]
impl Tool for CreateScheduleTool {
    fn name(&self) -> &str {
        "create_schedule"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Create Schedule")
    }

    fn description(&self) -> &str {
        "Schedule a future task in this session. Provide description and either scheduled_at (one-shot) or cron_expression (recurring)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "What the agent should do when the schedule fires"
                },
                "cron_expression": {
                    "type": "string",
                    "description": "Standard 5-field cron expression for recurring schedules (e.g., '0 3 * * *' for daily at 3am)"
                },
                "scheduled_at": {
                    "type": "string",
                    "description": "ISO 8601 datetime for one-shot schedule (e.g., '2026-02-19T03:00:00Z')"
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone (e.g., 'America/New_York'). Default: UTC"
                }
            },
            "required": ["description"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "create_schedule requires context. This tool must be executed with session context.",
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

        let cron_expression = arguments
            .get("cron_expression")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let scheduled_at = arguments
            .get("scheduled_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        if cron_expression.is_none() && scheduled_at.is_none() {
            return ToolExecutionResult::tool_error(
                "Must provide either cron_expression (recurring) or scheduled_at (one-shot)",
            );
        }

        let timezone = arguments
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("UTC")
            .to_string();

        let Some(store) = &context.schedule_store else {
            return ToolExecutionResult::tool_error("Schedule store not available in this context");
        };

        // Check max active limit
        match store.count_active_schedules(context.session_id).await {
            Ok(count) if count >= MAX_ACTIVE_SCHEDULES_PER_SESSION => {
                return ToolExecutionResult::tool_error(format!(
                    "Maximum {MAX_ACTIVE_SCHEDULES_PER_SESSION} active schedules per session. Cancel an existing schedule first."
                ));
            }
            Err(e) => return ToolExecutionResult::internal_error(e),
            _ => {}
        }

        match store
            .create_schedule(
                context.session_id,
                description,
                cron_expression,
                scheduled_at,
                timezone,
            )
            .await
        {
            Ok(schedule) => ToolExecutionResult::success(json!({
                "schedule_id": schedule.id.to_string(),
                "description": schedule.description,
                "schedule_type": schedule.schedule_type,
                "cron_expression": schedule.cron_expression,
                "scheduled_at": schedule.scheduled_at,
                "timezone": schedule.timezone,
                "next_trigger_at": schedule.next_trigger_at,
                "enabled": schedule.enabled,
                "created": true,
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

// ============================================================================
// cancel_schedule tool
// ============================================================================

pub struct CancelScheduleTool;

#[async_trait]
impl Tool for CancelScheduleTool {
    fn name(&self) -> &str {
        "cancel_schedule"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Cancel Schedule")
    }

    fn description(&self) -> &str {
        "Cancel (disable) an active schedule by its ID."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "schedule_id": {
                    "type": "string",
                    "description": "The schedule ID to cancel (e.g., 'sched_...')"
                }
            },
            "required": ["schedule_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "cancel_schedule requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let schedule_id_str = match arguments.get("schedule_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: schedule_id"),
        };

        let schedule_id = match schedule_id_str.parse::<crate::typed_id::ScheduleId>() {
            Ok(id) => id,
            Err(_) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid schedule_id format: {schedule_id_str}"
                ));
            }
        };

        let Some(store) = &context.schedule_store else {
            return ToolExecutionResult::tool_error("Schedule store not available in this context");
        };

        match store.cancel_schedule(context.session_id, schedule_id).await {
            Ok(schedule) => ToolExecutionResult::success(json!({
                "schedule_id": schedule.id.to_string(),
                "description": schedule.description,
                "enabled": schedule.enabled,
                "cancelled": true,
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

// ============================================================================
// list_schedules tool
// ============================================================================

pub struct ListSchedulesTool;

#[async_trait]
impl Tool for ListSchedulesTool {
    fn name(&self) -> &str {
        "list_schedules"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Schedules")
    }

    fn description(&self) -> &str {
        "List all schedules for the current session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "list_schedules requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(store) = &context.schedule_store else {
            return ToolExecutionResult::tool_error("Schedule store not available in this context");
        };

        match store.list_schedules(context.session_id).await {
            Ok(schedules) => {
                let items: Vec<Value> = schedules
                    .iter()
                    .map(|s| {
                        json!({
                            "schedule_id": s.id.to_string(),
                            "description": s.description,
                            "schedule_type": s.schedule_type,
                            "cron_expression": s.cron_expression,
                            "scheduled_at": s.scheduled_at,
                            "timezone": s.timezone,
                            "enabled": s.enabled,
                            "next_trigger_at": s.next_trigger_at,
                            "last_triggered_at": s.last_triggered_at,
                            "trigger_count": s.trigger_count,
                        })
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "schedules": items,
                    "total": schedules.len(),
                }))
            }
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_schedule::SessionSchedule;
    use crate::traits::SessionScheduleStore;
    use crate::typed_id::{ScheduleId, SessionId};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockScheduleStore {
        schedules: Arc<Mutex<Vec<SessionSchedule>>>,
    }

    impl MockScheduleStore {
        fn new() -> Self {
            Self {
                schedules: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl SessionScheduleStore for MockScheduleStore {
        async fn create_schedule(
            &self,
            session_id: SessionId,
            description: String,
            cron_expression: Option<String>,
            scheduled_at: Option<chrono::DateTime<Utc>>,
            timezone: String,
        ) -> crate::error::Result<SessionSchedule> {
            let schedule = SessionSchedule {
                id: ScheduleId::new(),
                session_id,
                description,
                schedule_type: SessionSchedule::derive_type(&cron_expression),
                cron_expression,
                scheduled_at,
                timezone,
                enabled: true,
                next_trigger_at: Some(Utc::now() + chrono::Duration::hours(1)),
                last_triggered_at: None,
                trigger_count: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            self.schedules.lock().unwrap().push(schedule.clone());
            Ok(schedule)
        }

        async fn cancel_schedule(
            &self,
            _session_id: SessionId,
            schedule_id: ScheduleId,
        ) -> crate::error::Result<SessionSchedule> {
            let mut schedules = self.schedules.lock().unwrap();
            let schedule = schedules
                .iter_mut()
                .find(|s| s.id == schedule_id)
                .ok_or_else(|| crate::error::AgentLoopError::tool("Schedule not found"))?;
            schedule.enabled = false;
            Ok(schedule.clone())
        }

        async fn list_schedules(
            &self,
            session_id: SessionId,
        ) -> crate::error::Result<Vec<SessionSchedule>> {
            let schedules = self.schedules.lock().unwrap();
            Ok(schedules
                .iter()
                .filter(|s| s.session_id == session_id)
                .cloned()
                .collect())
        }

        async fn count_active_schedules(&self, session_id: SessionId) -> crate::error::Result<u32> {
            let schedules = self.schedules.lock().unwrap();
            Ok(schedules
                .iter()
                .filter(|s| s.session_id == session_id && s.enabled)
                .count() as u32)
        }
    }

    #[tokio::test]
    async fn create_schedule_one_shot() {
        let store = MockScheduleStore::new();
        let session_id = SessionId::new();
        let mut context = ToolContext::new(session_id);
        context.schedule_store = Some(Arc::new(store));

        let tool = CreateScheduleTool;
        let result = tool
            .execute_with_context(
                json!({
                    "description": "Run backup",
                    "scheduled_at": "2026-02-19T03:00:00Z"
                }),
                &context,
            )
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["created"], true);
                assert_eq!(value["description"], "Run backup");
                assert_eq!(value["schedule_type"], "oneshot");
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_schedule_recurring() {
        let store = MockScheduleStore::new();
        let session_id = SessionId::new();
        let mut context = ToolContext::new(session_id);
        context.schedule_store = Some(Arc::new(store));

        let tool = CreateScheduleTool;
        let result = tool
            .execute_with_context(
                json!({
                    "description": "Check logs",
                    "cron_expression": "0 3 * * *"
                }),
                &context,
            )
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["created"], true);
                assert_eq!(value["schedule_type"], "recurring");
                assert_eq!(value["cron_expression"], "0 3 * * *");
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_schedule_rejects_missing_time() {
        let store = MockScheduleStore::new();
        let session_id = SessionId::new();
        let mut context = ToolContext::new(session_id);
        context.schedule_store = Some(Arc::new(store));

        let tool = CreateScheduleTool;
        let result = tool
            .execute_with_context(json!({"description": "No time"}), &context)
            .await;

        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    #[tokio::test]
    async fn create_schedule_enforces_max_limit() {
        let store = MockScheduleStore::new();
        let session_id = SessionId::new();
        let mut context = ToolContext::new(session_id);
        context.schedule_store = Some(Arc::new(store.clone()));

        let tool = CreateScheduleTool;

        // Create 5 schedules
        for i in 0..5 {
            let result = tool
                .execute_with_context(
                    json!({
                        "description": format!("Task {i}"),
                        "scheduled_at": "2026-12-01T00:00:00Z"
                    }),
                    &context,
                )
                .await;
            assert!(matches!(result, ToolExecutionResult::Success(_)));
        }

        // 6th should fail
        let result = tool
            .execute_with_context(
                json!({
                    "description": "Task 6",
                    "scheduled_at": "2026-12-01T00:00:00Z"
                }),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    #[tokio::test]
    async fn cancel_schedule_works() {
        let store = MockScheduleStore::new();
        let session_id = SessionId::new();
        let mut context = ToolContext::new(session_id);
        context.schedule_store = Some(Arc::new(store.clone()));

        // Create a schedule first
        let schedule = store
            .create_schedule(
                session_id,
                "test".to_string(),
                None,
                Some(Utc::now() + chrono::Duration::hours(1)),
                "UTC".to_string(),
            )
            .await
            .unwrap();

        let tool = CancelScheduleTool;
        let result = tool
            .execute_with_context(json!({"schedule_id": schedule.id.to_string()}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["cancelled"], true);
                assert_eq!(value["enabled"], false);
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_schedules_works() {
        let store = MockScheduleStore::new();
        let session_id = SessionId::new();
        let mut context = ToolContext::new(session_id);
        context.schedule_store = Some(Arc::new(store.clone()));

        // Create 2 schedules
        store
            .create_schedule(
                session_id,
                "first".to_string(),
                None,
                Some(Utc::now() + chrono::Duration::hours(1)),
                "UTC".to_string(),
            )
            .await
            .unwrap();
        store
            .create_schedule(
                session_id,
                "second".to_string(),
                Some("0 * * * *".to_string()),
                None,
                "UTC".to_string(),
            )
            .await
            .unwrap();

        let tool = ListSchedulesTool;
        let result = tool.execute_with_context(json!({}), &context).await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["total"], 2);
                assert_eq!(value["schedules"].as_array().unwrap().len(), 2);
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[test]
    fn capability_metadata() {
        let cap = SessionScheduleCapability;
        assert_eq!(cap.id(), "session_schedule");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert!(cap.system_prompt_addition().is_some());
        assert_eq!(cap.tools().len(), 3);
    }
}
