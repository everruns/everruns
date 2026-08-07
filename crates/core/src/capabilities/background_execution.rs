//! BackgroundExecution Capability — exposes `spawn_background` to the model
//! whenever the active tool set contains at least one built-in tool that opts
//! into detached execution via `ToolHints::supports_background`.
//!
//! This capability is auto-activated by
//! [`super::collect_capabilities_with_configs`] after the explicit-capability
//! collection loop. It can also be selected explicitly by id
//! (`"background_execution"`) — the auto-activator skips it in that case.
//!
//! See `knowledge/execution/background-execution.md` for the cross-cutting / meta-tool
//! capability contract this implements.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::tools::{SpawnBackgroundTool, Tool};

/// Capability id used by the auto-activator in
/// `collect_capabilities_with_configs`.
pub const BACKGROUND_EXECUTION_CAPABILITY_ID: &str = "background_execution";

/// Cross-cutting capability that contributes `spawn_background`.
///
/// Auto-activation rule: any collected tool with
/// `hints.supports_background == Some(true)` causes this capability to be
/// added to the agent's tool set, exposing `spawn_background` to the model
/// and registering it in the worker tool registry.
pub struct BackgroundExecutionCapability;

impl Capability for BackgroundExecutionCapability {
    fn id(&self) -> &str {
        BACKGROUND_EXECUTION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Background Execution"
    }

    fn description(&self) -> &str {
        "Run any background-capable built-in tool asynchronously via \
         `spawn_background`. Auto-activated whenever the agent has a tool \
         that declares `supports_background=true`."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Фонове виконання",
            "Запускайте будь-який вбудований інструмент із підтримкою фонового режиму асинхронно через `spawn_background`. Активується автоматично, щойно агент має інструмент, який оголошує `supports_background=true`.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("zap")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(SpawnBackgroundTool)]
    }
}

/// Task executor for `background_tool` tasks.
pub struct BackgroundToolTaskExecutor;

#[async_trait::async_trait]
impl crate::session_task::TaskExecutor for BackgroundToolTaskExecutor {
    fn kind(&self) -> &str {
        crate::session_task::TASK_KIND_BACKGROUND_TOOL
    }

    /// Re-attach when `spec["reattachable"]` is true (set at spawn time when
    /// the tool declared `idempotent` or `readonly` hints). Tools without this
    /// flag are still failed as orphaned so side-effecting runs are not doubled.
    fn can_reattach_task(&self, task: &crate::session_task::SessionTask) -> bool {
        task.spec
            .get("reattachable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Re-execute the tool from spec after worker loss.
    async fn start(
        &self,
        task: &crate::session_task::SessionTask,
        context: &crate::traits::ToolContext,
    ) -> crate::error::Result<()> {
        crate::tools::reattach_background_run(task, context).await
    }

    /// Cooperative cancellation: `cancel_task` records `cancel_requested_at`
    /// in the registry; the background run's heartbeat loop polls it every ~2 s
    /// and winds down when set.  No in-process token is needed — the record-
    /// polling design works even when this call executes on a different worker.
    async fn cancel(
        &self,
        _task: &crate::session_task::SessionTask,
        _context: &crate::traits::ToolContext,
    ) -> crate::error::Result<()> {
        Ok(())
    }
}

inventory::submit! {
    crate::session_task::TaskExecutorPlugin {
        executor: || std::sync::Arc::new(BackgroundToolTaskExecutor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Metadata/tool-list/registration constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn can_reattach_task_true_when_spec_reattachable_true() {
        use crate::session_task::{SessionTask, SessionTaskState, TaskExecutor, TaskWakePolicy};
        use chrono::Utc;

        let exec = BackgroundToolTaskExecutor;
        let task = SessionTask {
            id: "t1".to_string(),
            session_id: crate::SessionId::new(),
            root_session_id: None,
            kind: crate::session_task::TASK_KIND_BACKGROUND_TOOL.to_string(),
            display_name: "test".to_string(),
            spec: serde_json::json!({ "tool": "get_current_time", "reattachable": true }),
            state: SessionTaskState::Running,
            state_detail: None,
            progress: None,
            input_request: None,
            cancel_requested_at: None,
            summary: None,
            result_path: None,
            artifacts: vec![],
            error: None,
            attempt: 1,
            worker_id: None,
            heartbeat_at: None,
            links: crate::session_task::TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now(),
        };
        assert!(exec.can_reattach_task(&task));
    }

    #[test]
    fn can_reattach_task_false_when_spec_reattachable_false() {
        use crate::session_task::{SessionTask, SessionTaskState, TaskExecutor, TaskWakePolicy};
        use chrono::Utc;

        let exec = BackgroundToolTaskExecutor;
        let task = SessionTask {
            id: "t2".to_string(),
            session_id: crate::SessionId::new(),
            root_session_id: None,
            kind: crate::session_task::TASK_KIND_BACKGROUND_TOOL.to_string(),
            display_name: "test".to_string(),
            spec: serde_json::json!({ "tool": "some_side_effecting_tool", "reattachable": false }),
            state: SessionTaskState::Running,
            state_detail: None,
            progress: None,
            input_request: None,
            cancel_requested_at: None,
            summary: None,
            result_path: None,
            artifacts: vec![],
            error: None,
            attempt: 1,
            worker_id: None,
            heartbeat_at: None,
            links: crate::session_task::TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now(),
        };
        assert!(!exec.can_reattach_task(&task));
    }

    #[test]
    fn can_reattach_task_false_when_spec_has_no_reattachable_field() {
        use crate::session_task::{SessionTask, SessionTaskState, TaskExecutor, TaskWakePolicy};
        use chrono::Utc;

        let exec = BackgroundToolTaskExecutor;
        let task = SessionTask {
            id: "t3".to_string(),
            session_id: crate::SessionId::new(),
            root_session_id: None,
            kind: crate::session_task::TASK_KIND_BACKGROUND_TOOL.to_string(),
            display_name: "test".to_string(),
            spec: serde_json::json!({ "tool": "old_task_without_reattachable_flag" }),
            state: SessionTaskState::Running,
            state_detail: None,
            progress: None,
            input_request: None,
            cancel_requested_at: None,
            summary: None,
            result_path: None,
            artifacts: vec![],
            error: None,
            attempt: 1,
            worker_id: None,
            heartbeat_at: None,
            links: crate::session_task::TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now(),
        };
        // Old tasks without the flag are conservatively treated as non-reattachable.
        assert!(!exec.can_reattach_task(&task));
    }
}
