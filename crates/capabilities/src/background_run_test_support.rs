//! Shared fixtures for the `background_run` tests: in-memory session
//! services and the simplest background tool. Split out of `background_run.rs`
//! to keep it under the source size ratchet.

use super::*;
use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
use everruns_contracts::typed_id::HarnessId;
use everruns_core::background::BackgroundExecutableTool;
use everruns_core::session_file::{FileInfo, FileStat, SessionFile};
use everruns_core::subagent_delegation::SubagentSessionDelegate;
use everruns_core::{session_services::KeyInfo, session_services::SecretInfo};
use std::sync::Mutex;

/// The act phase's chains with no hooks registered: every call passes.
pub(super) struct AllowAllPolicy;

#[async_trait]
impl NestedToolPolicy for AllowAllPolicy {
    async fn authorize(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> std::result::Result<ToolCall, ToolResult> {
        Ok(tool_call)
    }

    async fn after_exec(
        &self,
        _tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        _result: &mut ToolResult,
        _context: &ToolContext,
    ) {
    }
}

pub(super) fn allow_all_policy() -> Arc<dyn NestedToolPolicy> {
    Arc::new(AllowAllPolicy)
}

#[derive(Default)]
pub(super) struct TestBackgroundTool;

#[async_trait]
impl BackgroundExecutableTool for TestBackgroundTool {
    async fn execute_background(
        &self,
        arguments: Value,
        _context: ToolContext,
        sink: Arc<dyn BackgroundEventSink>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
        sink.status("Waiting for test result")
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        sink.output("stdout", "hello from background")
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        sink.progress(BackgroundProgress {
            current: Some(1),
            total: Some(1),
            unit: Some("step".to_string()),
            label: Some("done".to_string()),
        })
        .await
        .map_err(ToolExecutionResult::internal_error)?;

        Ok(BackgroundOutcome {
            summary: arguments["summary"].as_str().unwrap_or("done").to_string(),
            result: json!({"ok": true}),
            raw_output: None,
        })
    }
}

#[async_trait]
impl Tool for TestBackgroundTool {
    fn name(&self) -> &str {
        "test_background"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Test Background")
    }

    fn description(&self) -> &str {
        "test tool"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string" }
            }
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("foreground unsupported")
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_supports_background(true)
    }

    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        Some(self)
    }
}

#[derive(Default)]
pub(super) struct NoopStorageStore;

#[async_trait]
impl everruns_core::session_services::SessionStorageStore for NoopStorageStore {
    async fn set_value(
        &self,
        _session_id: SessionId,
        _key: &str,
        _value: &str,
    ) -> everruns_contracts::error::Result<()> {
        Ok(())
    }
    async fn get_value(
        &self,
        _session_id: SessionId,
        _key: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(None)
    }
    async fn delete_value(
        &self,
        _session_id: SessionId,
        _key: &str,
    ) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }
    async fn list_keys(
        &self,
        _session_id: SessionId,
    ) -> everruns_contracts::error::Result<Vec<KeyInfo>> {
        Ok(Vec::new())
    }
    async fn set_secret(
        &self,
        _session_id: SessionId,
        _name: &str,
        _value: &str,
    ) -> everruns_contracts::error::Result<()> {
        Ok(())
    }
    async fn get_secret(
        &self,
        _session_id: SessionId,
        _name: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(None)
    }
    async fn delete_secret(
        &self,
        _session_id: SessionId,
        _name: &str,
    ) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }
    async fn list_secrets(
        &self,
        _session_id: SessionId,
    ) -> everruns_contracts::error::Result<Vec<SecretInfo>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub(super) struct TestFileStore {
    files: Mutex<HashMap<String, SessionFile>>,
}

#[async_trait]
impl everruns_core::session_files::SessionFileSystem for TestFileStore {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(
        &self,
        _session_id: SessionId,
        path: &str,
    ) -> everruns_contracts::error::Result<Option<SessionFile>> {
        Ok(self.files.lock().unwrap().get(path).cloned())
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> everruns_contracts::error::Result<SessionFile> {
        let now = chrono::Utc::now();
        let file = SessionFile {
            id: uuid::Uuid::now_v7(),
            session_id: session_id.uuid(),
            path: path.to_string(),
            name: FileInfo::name_from_path(path),
            content: Some(content.to_string()),
            encoding: encoding.to_string(),
            is_directory: false,
            is_readonly: false,
            size_bytes: content.len() as i64,
            created_at: now,
            updated_at: now,
        };
        self.files
            .lock()
            .unwrap()
            .insert(path.to_string(), file.clone());
        Ok(file)
    }

    async fn delete_file(
        &self,
        _session_id: SessionId,
        _path: &str,
        _recursive: bool,
    ) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn list_directory(
        &self,
        _session_id: SessionId,
        _path: &str,
    ) -> everruns_contracts::error::Result<Vec<FileInfo>> {
        Ok(Vec::new())
    }

    async fn stat_file(
        &self,
        _session_id: SessionId,
        path: &str,
    ) -> everruns_contracts::error::Result<Option<FileStat>> {
        let file = self.files.lock().unwrap().get(path).cloned();
        Ok(file.map(|entry| FileStat {
            path: entry.path,
            name: entry.name,
            is_directory: entry.is_directory,
            is_readonly: entry.is_readonly,
            size_bytes: entry.size_bytes,
            created_at: entry.created_at,
            updated_at: entry.updated_at,
        }))
    }

    async fn grep_files(
        &self,
        _session_id: SessionId,
        _pattern: &str,
        _path_pattern: Option<&str>,
    ) -> everruns_contracts::error::Result<Vec<everruns_core::session_file::GrepMatch>> {
        Ok(Vec::new())
    }

    async fn create_directory(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> everruns_contracts::error::Result<FileInfo> {
        let now = chrono::Utc::now();
        let id = uuid::Uuid::now_v7();
        let dir = SessionFile {
            id,
            session_id: session_id.uuid(),
            path: path.to_string(),
            name: FileInfo::name_from_path(path),
            content: None,
            encoding: "text".to_string(),
            is_directory: true,
            is_readonly: false,
            size_bytes: 0,
            created_at: now,
            updated_at: now,
        };
        self.files.lock().unwrap().insert(path.to_string(), dir);
        Ok(FileInfo {
            id,
            session_id: session_id.uuid(),
            path: path.to_string(),
            name: FileInfo::name_from_path(path),
            is_directory: true,
            is_readonly: false,
            size_bytes: 0,
            created_at: now,
            updated_at: now,
        })
    }
}

#[derive(Default)]
pub(super) struct TestSubagentDelegate {
    pub(super) sent_messages: Mutex<Vec<String>>,
}

#[async_trait]
#[async_trait]
impl SubagentSessionDelegate for TestSubagentDelegate {
    async fn get_agent_by_id(
        &self,
        _id: everruns_contracts::typed_id::AgentId,
    ) -> everruns_contracts::error::Result<Option<everruns_core::AgentDefinition>> {
        Ok(None)
    }
    async fn add_agent_session_participant(
        &self,
        _session_id: SessionId,
        _agent_id: everruns_contracts::typed_id::AgentId,
    ) -> everruns_contracts::error::Result<everruns_contracts::typed_id::SessionParticipantId> {
        Err(AgentLoopError::tool("test store does not add participants"))
    }
    async fn get_harness(
        &self,
        _id: HarnessId,
    ) -> everruns_contracts::error::Result<Option<everruns_core::HarnessDefinition>> {
        Ok(None)
    }
    async fn create_session_with_options(
        &self,
        _request: everruns_core::subagent_delegation::PlatformCreateSessionRequest,
    ) -> everruns_contracts::error::Result<everruns_core::ExecutionSession> {
        Err(AgentLoopError::tool("test store does not create sessions"))
    }
    async fn get_session_by_id(
        &self,
        _id: SessionId,
    ) -> everruns_contracts::error::Result<Option<everruns_core::ExecutionSession>> {
        Ok(None)
    }
    async fn send_message(
        &self,
        _session_id: SessionId,
        content: &str,
    ) -> everruns_contracts::error::Result<()> {
        self.sent_messages.lock().unwrap().push(content.to_string());
        Ok(())
    }
    async fn get_messages(
        &self,
        _session_id: SessionId,
        _limit: Option<usize>,
    ) -> everruns_contracts::error::Result<Vec<everruns_core::subagent_delegation::PlatformMessage>>
    {
        Ok(Vec::new())
    }
    async fn wait_for_idle(
        &self,
        _session_id: SessionId,
        _timeout_secs: Option<u64>,
    ) -> everruns_contracts::error::Result<String> {
        Ok("idle".to_string())
    }
}

#[derive(Default)]
pub(super) struct InMemoryTaskRegistry {
    pub(super) tasks: Mutex<HashMap<String, everruns_core::session_task::SessionTask>>,
}

#[async_trait]
impl everruns_core::session_task::SessionTaskRegistry for InMemoryTaskRegistry {
    async fn create(
        &self,
        input: everruns_core::session_task::CreateSessionTask,
    ) -> everruns_contracts::error::Result<everruns_core::session_task::SessionTask> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(id) = &input.id
            && let Some(existing) = tasks.get(id)
        {
            return Ok(existing.clone());
        }
        let task = everruns_core::session_task::new_session_task(input, chrono::Utc::now());
        tasks.insert(task.id.clone(), task.clone());
        Ok(task)
    }

    async fn update(
        &self,
        _session_id: SessionId,
        task_id: &str,
        update: everruns_core::session_task::SessionTaskUpdate,
    ) -> everruns_contracts::error::Result<Option<everruns_core::session_task::SessionTask>> {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(task) = tasks.get_mut(task_id) else {
            return Ok(None);
        };
        everruns_core::session_task::apply_task_update(task, update, chrono::Utc::now());
        Ok(Some(task.clone()))
    }

    async fn get(
        &self,
        _session_id: SessionId,
        task_id: &str,
    ) -> everruns_contracts::error::Result<Option<everruns_core::session_task::SessionTask>> {
        Ok(self.tasks.lock().unwrap().get(task_id).cloned())
    }

    async fn list(
        &self,
        _session_id: SessionId,
        filter: Option<&everruns_core::session_task::SessionTaskFilter>,
    ) -> everruns_contracts::error::Result<Vec<everruns_core::session_task::SessionTask>> {
        let tasks = self.tasks.lock().unwrap();
        Ok(tasks
            .values()
            .filter(|task| {
                filter.is_none_or(|f| {
                    f.kind.as_deref().is_none_or(|kind| task.kind == kind)
                        && f.state.is_none_or(|state| task.state == state)
                })
            })
            .cloned()
            .collect())
    }

    async fn request_cancel(
        &self,
        _session_id: SessionId,
        task_id: &str,
    ) -> everruns_contracts::error::Result<Option<everruns_core::session_task::SessionTask>> {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(task) = tasks.get_mut(task_id) else {
            return Ok(None);
        };
        task.cancel_requested_at
            .get_or_insert_with(chrono::Utc::now);
        task.updated_at = chrono::Utc::now();
        Ok(Some(task.clone()))
    }

    async fn record_message(
        &self,
        _session_id: SessionId,
        task_id: &str,
        message: everruns_core::session_task::NewTaskMessage,
    ) -> everruns_contracts::error::Result<everruns_core::session_task::TaskMessage> {
        let tasks = self.tasks.lock().unwrap();
        let _task = tasks
            .get(task_id)
            .ok_or_else(|| AgentLoopError::tool(format!("no task {task_id}")))?;
        Ok(everruns_core::session_task::TaskMessage {
            id: everruns_core::session_task::generate_task_message_id(),
            task_id: task_id.to_string(),
            direction: message.direction,
            content: message.content,
            in_reply_to: message.in_reply_to,
            created_at: chrono::Utc::now(),
        })
    }

    async fn list_messages(
        &self,
        _session_id: SessionId,
        _task_id: &str,
        _limit: Option<u32>,
        _after_id: Option<&str>,
    ) -> everruns_contracts::error::Result<Vec<everruns_core::session_task::TaskMessage>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub(super) struct TestScheduleStore {
    pub(super) schedules: Mutex<Vec<everruns_core::session_schedule::SessionSchedule>>,
}

#[async_trait]
impl everruns_core::session_services::SessionScheduleStore for TestScheduleStore {
    async fn create_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: String,
    ) -> everruns_contracts::error::Result<everruns_core::session_schedule::SessionSchedule> {
        let schedule = everruns_core::session_schedule::SessionSchedule {
            id: everruns_contracts::typed_id::ScheduleId::new(),
            session_id,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            description,
            cron_expression: cron_expression.clone(),
            scheduled_at,
            timezone,
            enabled: true,
            schedule_type: everruns_core::session_schedule::SessionSchedule::derive_type(
                &cron_expression,
            ),
            next_trigger_at: Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
            last_triggered_at: None,
            trigger_count: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        self.schedules.lock().unwrap().push(schedule.clone());
        Ok(schedule)
    }

    async fn cancel_schedule(
        &self,
        _session_id: SessionId,
        schedule_id: everruns_contracts::typed_id::ScheduleId,
    ) -> everruns_contracts::error::Result<everruns_core::session_schedule::SessionSchedule> {
        let mut schedules = self.schedules.lock().unwrap();
        let schedule = schedules
            .iter_mut()
            .find(|schedule| schedule.id == schedule_id)
            .ok_or_else(|| AgentLoopError::tool("Schedule not found".to_string()))?;
        schedule.enabled = false;
        Ok(schedule.clone())
    }

    async fn list_schedules(
        &self,
        session_id: SessionId,
    ) -> everruns_contracts::error::Result<Vec<everruns_core::session_schedule::SessionSchedule>>
    {
        Ok(self
            .schedules
            .lock()
            .unwrap()
            .iter()
            .filter(|schedule| schedule.session_id == session_id)
            .cloned()
            .collect())
    }

    async fn count_active_schedules(
        &self,
        session_id: SessionId,
    ) -> everruns_contracts::error::Result<u32> {
        Ok(self
            .schedules
            .lock()
            .unwrap()
            .iter()
            .filter(|schedule| schedule.session_id == session_id && schedule.enabled)
            .count() as u32)
    }

    async fn count_active_org_schedules(&self) -> everruns_contracts::error::Result<u32> {
        // Test store is single-org; count all enabled schedules.
        Ok(self
            .schedules
            .lock()
            .unwrap()
            .iter()
            .filter(|schedule| schedule.enabled)
            .count() as u32)
    }
}

pub(super) fn make_reattach_task(
    spec: serde_json::Value,
) -> everruns_core::session_task::SessionTask {
    use everruns_core::session_task::{SessionTaskState, TaskLinks, TaskWakePolicy};
    everruns_core::session_task::SessionTask {
        id: "t-reattach".to_string(),
        session_id: SessionId::new(),
        root_session_id: None,
        kind: everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL.to_string(),
        display_name: "Reattach test".to_string(),
        spec,
        state: SessionTaskState::Running,
        state_detail: None,
        progress: None,
        input_request: None,
        cancel_requested_at: None,
        summary: None,
        result_path: None,
        artifacts: vec![],
        error: None,
        attempt: 2,
        worker_id: None,
        heartbeat_at: None,
        links: TaskLinks::default(),
        wake_policy: TaskWakePolicy::Silent,
        created_at: chrono::Utc::now(),
        started_at: None,
        finished_at: None,
        updated_at: chrono::Utc::now(),
    }
}
