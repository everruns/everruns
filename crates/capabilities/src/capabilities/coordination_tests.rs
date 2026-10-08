use super::*;
use crate::capabilities::session_tasks::tests::InMemorySessionTaskRegistry;
use crate::platform_store::tests::MockPlatformStore;
use crate::{PlatformStore, PlatformStoreSubagentDelegate};

/// Session store view over the mock platform store.
struct MockSessionStore(Arc<MockPlatformStore>);

#[async_trait]
impl SessionStore for MockSessionStore {
    async fn get_session(
        &self,
        session_id: SessionId,
    ) -> everruns_contracts::error::Result<Option<everruns_core::session::ExecutionSession>> {
        self.0.get_session_by_id(session_id).await
    }
}

struct Fixture {
    store: Arc<MockPlatformStore>,
    registry: Arc<InMemorySessionTaskRegistry>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            store: Arc::new(MockPlatformStore::new()),
            registry: Arc::new(InMemorySessionTaskRegistry::default()),
        }
    }

    fn coordinator(&self) -> SessionId {
        self.store.session.id
    }

    fn context(&self, session_id: SessionId) -> ToolContext {
        let mut context = ToolContext::new(session_id);
        context.subagent_delegate =
            Some(Arc::new(PlatformStoreSubagentDelegate(self.store.clone())));
        context.session_store = Some(Arc::new(MockSessionStore(self.store.clone())));
        context.session_task_registry = Some(self.registry.clone());
        context
    }

    async fn start(&self, config: Value, arguments: Value) -> ToolExecutionResult {
        let config = Arc::new(CoordinationConfig::parse(&config).unwrap());
        StartThreadTool(config)
            .execute_with_context(arguments, &self.context(self.coordinator()))
            .await
    }

    async fn start_ok(&self, title: &str) -> (SessionId, String) {
        let result = self
            .start(
                Value::Null,
                json!({"title": title, "brief": "Do the thing."}),
            )
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        (
            value["thread_id"].as_str().unwrap().parse().unwrap(),
            value["assignment_id"].as_str().unwrap().to_string(),
        )
    }

    async fn task(&self, task_id: &str) -> SessionTask {
        self.registry
            .get(self.coordinator(), task_id)
            .await
            .unwrap()
            .unwrap()
    }

    async fn assignment(&self, thread: SessionId) -> ThreadAssignment {
        open_assignment_for_thread(
            thread,
            &MockSessionStore(self.store.clone()),
            self.registry.as_ref(),
        )
        .await
        .unwrap()
        .expect("open assignment")
    }

    async fn sent_to(&self, thread: SessionId) -> Vec<String> {
        // Thread messages go out on a spawned task.
        for _ in 0..50 {
            let sent: Vec<String> = self
                .store
                .sent_messages
                .lock()
                .unwrap()
                .iter()
                .filter(|(id, _)| *id == thread)
                .map(|(_, text)| text.clone())
                .collect();
            if !sent.is_empty() {
                return sent;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Vec::new()
    }
}

fn error_text(result: &ToolExecutionResult) -> String {
    format!("{result:?}")
}

async fn run(tool: &dyn Tool, arguments: Value, context: &ToolContext) -> ToolExecutionResult {
    tool.execute_with_context(arguments, context).await
}

#[tokio::test]
async fn start_thread_creates_a_child_session_with_a_tracked_assignment() {
    let fixture = Fixture::new();
    let (thread, task_id) = fixture.start_ok("Weekly report").await;

    let session = fixture
        .store
        .get_session_by_id(thread)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.parent_session_id, Some(fixture.coordinator()));
    assert_eq!(session.title.as_deref(), Some("Weekly report"));

    let task = fixture.task(&task_id).await;
    assert_eq!(task.kind, TASK_KIND_ASSIGNMENT);
    assert_eq!(task.state, SessionTaskState::Running);
    assert_eq!(task.wake_policy, TaskWakePolicy::OnActivity);
    assert_eq!(task.links.child_session_id, Some(thread));
    // No heartbeat: the reaper leaves assignments alone.
    assert!(task.heartbeat_at.is_none());

    let sent = fixture.sent_to(thread).await;
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("Weekly report") && sent[0].contains("Do the thing."));
    assert!(sent[0].contains("complete_assignment"));
}

#[tokio::test]
async fn threads_cannot_start_threads() {
    let fixture = Fixture::new();
    let (thread, _) = fixture.start_ok("Parent work").await;
    let config = Arc::new(CoordinationConfig::default());
    let result = StartThreadTool(config)
        .execute_with_context(
            json!({"title": "Nested", "brief": "x"}),
            &fixture.context(thread),
        )
        .await;
    assert!(error_text(&result).contains("threads do not start threads"));
}

#[tokio::test]
async fn start_thread_enforces_the_active_thread_cap() {
    let fixture = Fixture::new();
    let config = json!({"max_active_threads": 1});
    let first = fixture
        .start(config.clone(), json!({"title": "One", "brief": "x"}))
        .await;
    assert!(matches!(first, ToolExecutionResult::Success(_)));
    let second = fixture
        .start(config, json!({"title": "Two", "brief": "x"}))
        .await;
    assert!(
        error_text(&second).contains("already working"),
        "{second:?}"
    );
}

#[tokio::test]
async fn start_thread_only_uses_allowed_workers() {
    let fixture = Fixture::new();
    let agent = AgentId::new();
    let refused = fixture
        .start(
            Value::Null,
            json!({"title": "T", "brief": "x", "worker": agent.to_string()}),
        )
        .await;
    assert!(error_text(&refused).contains("not an allowed worker"));

    let allowed = fixture
        .start(
            json!({"workers": [{"id": "any"}]}),
            json!({"title": "T", "brief": "x", "worker": agent.to_string()}),
        )
        .await;
    let ToolExecutionResult::Success(value) = allowed else {
        panic!("expected success, got {allowed:?}");
    };
    let thread: SessionId = value["thread_id"].as_str().unwrap().parse().unwrap();
    let session = fixture
        .store
        .get_session_by_id(thread)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.agent_id, Some(agent));

    let self_refused = fixture
        .start(
            json!({"workers": [{"id": "any"}]}),
            json!({"title": "T", "brief": "x"}),
        )
        .await;
    assert!(error_text(&self_refused).contains("not configured to work threads itself"));
}

#[tokio::test]
async fn worker_tools_update_the_checklist_and_complete_the_assignment() {
    let fixture = Fixture::new();
    let (thread, task_id) = fixture.start_ok("Fix the bug").await;
    let assignment = fixture.assignment(thread).await;
    let tools = worker_tools(&assignment);
    let tool = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name() == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    let context = fixture.context(thread);

    let result = run(
        tool("update_checklist").as_ref(),
        json!({"steps": [
            {"title": "Reproduce", "status": "done"},
            {"title": "Fix", "status": "in_progress"},
            {"title": "Test", "status": "pending"}
        ]}),
        &context,
    )
    .await;
    assert!(
        matches!(result, ToolExecutionResult::Success(_)),
        "{result:?}"
    );
    let task = fixture.task(&task_id).await;
    let progress = task.progress.expect("progress");
    assert_eq!(progress.steps.len(), 3);
    assert_eq!((progress.current, progress.total), (Some(1), Some(3)));
    assert_eq!(task.state_detail.as_deref(), Some("Fix"));

    // Only the thread itself may report on its assignment.
    let foreign = run(
        tool("complete_assignment").as_ref(),
        json!({"summary": "done"}),
        &fixture.context(fixture.coordinator()),
    )
    .await;
    assert!(error_text(&foreign).contains("only works inside the thread"));

    let result = run(
        tool("complete_assignment").as_ref(),
        json!({
            "summary": "Fixed the off-by-one.",
            "validation": "cargo test passes",
            "artifacts": [{"name": "PR", "type": "pr", "url": "https://example.com/pr/1"}]
        }),
        &context,
    )
    .await;
    assert!(
        matches!(result, ToolExecutionResult::Success(_)),
        "{result:?}"
    );
    let task = fixture.task(&task_id).await;
    assert_eq!(task.state, SessionTaskState::Succeeded);
    assert!(
        task.summary
            .unwrap()
            .contains("Validation: cargo test passes")
    );
    assert_eq!(task.artifacts.len(), 1);

    // A finished thread has no open assignment, so no worker tools.
    assert!(
        open_assignment_for_thread(
            thread,
            &MockSessionStore(fixture.store.clone()),
            fixture.registry.as_ref()
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn coordinator_answers_a_question_through_message_thread() {
    let fixture = Fixture::new();
    let (thread, task_id) = fixture.start_ok("Pick a name").await;
    let assignment = fixture.assignment(thread).await;
    let ask = AskDecisionTool(assignment);
    let result = run(
        &ask,
        json!({"question": "Which name?", "options": ["A", "B"], "recommended": 0}),
        &fixture.context(thread),
    )
    .await;
    assert!(
        matches!(result, ToolExecutionResult::Success(_)),
        "{result:?}"
    );
    assert_eq!(
        fixture.task(&task_id).await.state,
        SessionTaskState::AwaitingInput
    );

    let result = run(
        &MessageThreadTool,
        json!({"thread_id": thread.to_string(), "message": "Use A."}),
        &fixture.context(fixture.coordinator()),
    )
    .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };
    assert_eq!(value["delivered"], "current_assignment");
    let task = fixture.task(&task_id).await;
    assert_eq!(task.state, SessionTaskState::Running);
    assert!(task.input_request.is_none());
    for _ in 0..50 {
        if fixture.sent_to(thread).await.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let sent = fixture.sent_to(thread).await;
    assert!(
        sent.iter()
            .any(|text| text.contains("Message from the coordinator") && text.contains("Use A."))
    );
}

#[tokio::test]
async fn message_to_an_idle_thread_starts_a_follow_up_assignment() {
    let fixture = Fixture::new();
    let (thread, first) = fixture.start_ok("Report").await;
    fixture
        .registry
        .update(
            fixture.coordinator(),
            &first,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let result = run(
        &MessageThreadTool,
        json!({"thread_id": thread.to_string(), "message": "Add a chart."}),
        &fixture.context(fixture.coordinator()),
    )
    .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };
    assert_eq!(value["delivered"], "new_assignment");
    assert_eq!(value["title"], "Follow-up: Report");
    let second = fixture.task(value["assignment_id"].as_str().unwrap()).await;
    assert_eq!(second.links.child_session_id, Some(thread));
    assert_eq!(second.state, SessionTaskState::Running);
}

#[tokio::test]
async fn message_thread_refuses_sessions_that_are_not_its_threads() {
    let fixture = Fixture::new();
    let result = run(
        &MessageThreadTool,
        json!({"thread_id": SessionId::new().to_string(), "message": "hi"}),
        &fixture.context(fixture.coordinator()),
    )
    .await;
    assert!(error_text(&result).contains("is not a thread of this session"));
}

#[tokio::test]
async fn resolve_refuses_working_threads_and_hides_resolved_ones() {
    let fixture = Fixture::new();
    let (thread, task_id) = fixture.start_ok("Cleanup").await;
    let context = fixture.context(fixture.coordinator());

    let refused = run(
        &ResolveThreadTool,
        json!({"thread_id": thread.to_string()}),
        &context,
    )
    .await;
    assert!(error_text(&refused).contains("still working"));

    let resolved = run(
        &ResolveThreadTool,
        json!({"thread_id": thread.to_string(), "cancel": true}),
        &context,
    )
    .await;
    assert!(
        matches!(resolved, ToolExecutionResult::Success(_)),
        "{resolved:?}"
    );
    let task = fixture.task(&task_id).await;
    assert_eq!(task.state, SessionTaskState::Canceled);
    assert_eq!(task.state_detail.as_deref(), Some(THREAD_RESOLVED_DETAIL));

    let ToolExecutionResult::Success(listed) = run(&ListThreadsTool, json!({}), &context).await
    else {
        panic!("list failed");
    };
    assert_eq!(listed["threads"].as_array().unwrap().len(), 0);
    let ToolExecutionResult::Success(listed) = run(
        &ListThreadsTool,
        json!({"include_resolved": true}),
        &context,
    )
    .await
    else {
        panic!("list failed");
    };
    assert_eq!(listed["threads"][0]["status"], "resolved");

    let reopened = run(
        &ResolveThreadTool,
        json!({"thread_id": thread.to_string(), "reopen": true}),
        &context,
    )
    .await;
    assert!(matches!(reopened, ToolExecutionResult::Success(_)));
    assert_eq!(
        fixture.task(&task_id).await.state_detail.as_deref(),
        Some(THREAD_REOPENED_DETAIL)
    );
}

#[tokio::test]
async fn get_thread_returns_assignments_and_recent_messages() {
    let fixture = Fixture::new();
    let (thread, _) = fixture.start_ok("Research").await;
    let ToolExecutionResult::Success(value) = run(
        &GetThreadTool,
        json!({"thread_id": thread.to_string(), "messages": 2}),
        &fixture.context(fixture.coordinator()),
    )
    .await
    else {
        panic!("get_thread failed");
    };
    assert_eq!(value["status"], "working");
    assert_eq!(value["assignments"][0]["title"], "Research");
    assert_eq!(value["recent_messages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn redirect_posts_a_structured_message_to_the_coordinator() {
    let fixture = Fixture::new();
    let (thread, task_id) = fixture.start_ok("Docs").await;
    let tool = RedirectToCoordinatorTool(fixture.assignment(thread).await);
    let result = run(
        &tool,
        json!({"request": "Also fix billing", "reason": "billing is another topic"}),
        &fixture.context(thread),
    )
    .await;
    assert!(
        matches!(result, ToolExecutionResult::Success(_)),
        "{result:?}"
    );
    let messages = fixture
        .registry
        .list_messages(fixture.coordinator(), &task_id, None, None)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].direction, TaskMessageDirection::Outbound);
    assert!(messages[0].content.iter().any(|part| matches!(
        part,
        TaskMessagePart::Data { data } if data["type"] == "redirect"
    )));
}

#[test]
fn config_validation_and_roles() {
    let capability = CoordinationCapability;
    assert!(capability.validate_config(&Value::Null).is_ok());
    assert!(
        capability
            .validate_config(&json!({"workers": [{"id": "self"}, {"id": "any"}]}))
            .is_ok()
    );
    assert!(capability.validate_config(&json!({"workers": []})).is_err());
    assert!(
        capability
            .validate_config(&json!({"workers": [{"id": "someone"}]}))
            .is_err()
    );
    assert!(
        capability
            .validate_config(&json!({"max_active_threads": 0}))
            .is_err()
    );
    assert!(capability.validate_config(&json!({"unknown": 1})).is_err());

    let names = |config: Value| {
        capability
            .tools_with_config(&config)
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(Value::Null),
        COORDINATOR_TOOL_NAMES.map(str::to_string)
    );
    assert!(names(json!({"role": "worker"})).is_empty());
}
