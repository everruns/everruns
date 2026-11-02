use super::*;
use crate::PlatformStoreSubagentDelegate;
use crate::capabilities::session_tasks::tests::InMemorySessionTaskRegistry;
use crate::platform_store::tests::MockPlatformStore;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::session_task::{
    CreateSessionTask, SessionTaskRegistry, TaskLinks, TaskMessagePart,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

fn delegate(
    store: Arc<MockPlatformStore>,
) -> Arc<dyn everruns_core::subagent_delegation::SubagentSessionDelegate> {
    Arc::new(PlatformStoreSubagentDelegate(store))
}

fn target_config(
    agent_id: AgentId,
    harness_id: HarnessId,
    required_connections: Vec<&str>,
) -> Value {
    json!({
        "targets": [
            {
                "id": "aws_operator",
                "name": "AWS Operator",
                "description": "Manage fake AWS infrastructure",
                "agent_id": agent_id,
                "harness_id": harness_id,
                "required_connections": required_connections,
                "required_scopes": ["fake_aws:rds:create"]
            }
        ]
    })
}

fn spawn_agent_tool(config: &Value) -> Box<dyn Tool> {
    Box::new(SpawnAgentHandoffTool::new(config))
}

struct TestConnectionResolver {
    providers: HashSet<String>,
}

#[async_trait]
impl UserConnectionResolver for TestConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .providers
            .contains(provider)
            .then(|| "server-side-secret-token".to_string()))
    }

    async fn get_connection_user(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<Uuid>> {
        Ok(None)
    }

    async fn get_connection_token_for_user(
        &self,
        _user_id: Uuid,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

fn context(
    store: Arc<MockPlatformStore>,
    resolver: Option<Arc<dyn UserConnectionResolver>>,
) -> ToolContext {
    let mut context = ToolContext::new(store.session.id);
    context.subagent_delegate = Some(delegate(store));
    context.connection_resolver = resolver;
    context
}

// Metadata constants covered by builtin_capabilities_satisfy_registry_invariants.

#[test]
fn terminal_handoff_status_maps_only_terminal_wait_states() {
    assert_eq!(
        terminal_handoff_status("idle"),
        Some(SubagentStatus::Completed)
    );
    assert_eq!(
        terminal_handoff_status("error"),
        Some(SubagentStatus::Failed)
    );
    assert_eq!(
        terminal_handoff_status("max_iterations_reached"),
        Some(SubagentStatus::MaxIterationsReached)
    );
    assert_eq!(terminal_handoff_status("waiting_for_tool_results"), None);
    assert_eq!(terminal_handoff_status("paused"), None);
}

#[test]
fn validate_config_rejects_duplicate_targets() {
    let agent_id = AgentId::new();
    let harness_id = HarnessId::new();
    let config = json!({
        "targets": [
            { "id": "dup", "name": "One", "agent_id": agent_id, "harness_id": harness_id },
            { "id": "dup", "name": "Two", "agent_id": AgentId::new(), "harness_id": HarnessId::new() }
        ]
    });

    let error = AgentHandoffCapability
        .validate_config(&config)
        .expect_err("duplicate targets should fail");
    assert!(error.contains("Duplicate handoff target id"));
}

#[tokio::test]
async fn spawn_agent_handoff_requires_configured_connection() {
    let store = Arc::new(MockPlatformStore::new());
    let config = target_config(store.agent.id, store.session.harness_id, vec!["fake_aws"]);
    let tool = spawn_agent_tool(&config);
    let resolver = Arc::new(TestConnectionResolver {
        providers: HashSet::new(),
    });
    let context = context(store, Some(resolver));

    let result = tool
        .execute_with_context(
            json!({
                "name": "AWS Operator",
                "instructions": "Create an RDS database named app-db",
                "target": { "type": "agent", "id": "aws_operator" },
                "mode": "foreground"
            }),
            &context,
        )
        .await;

    assert!(matches!(
        result,
        ToolExecutionResult::ConnectionRequired { provider, .. } if provider == "fake_aws"
    ));
}

#[tokio::test]
async fn spawn_agent_handoff_rejects_other_target_types() {
    let store = Arc::new(MockPlatformStore::new());
    let config = target_config(store.agent.id, store.session.harness_id, vec![]);
    let tool = spawn_agent_tool(&config);
    let context = context(store, None);

    let result = tool
        .execute_with_context(
            json!({
                "name": "Wrong Target",
                "instructions": "Do work",
                "target": { "type": "subagent" }
            }),
            &context,
        )
        .await;

    assert!(
        matches!(result, ToolExecutionResult::ToolError(message) if message.contains("target.type must be \"agent\""))
    );
}

#[tokio::test]
async fn spawn_agent_handoff_creates_agent_handoff_task() {
    let store = Arc::new(MockPlatformStore::new());
    let resolver = Arc::new(TestConnectionResolver {
        providers: HashSet::from(["fake_aws".to_string()]),
    });
    let config = target_config(store.agent.id, store.session.harness_id, vec!["fake_aws"]);
    let tool = spawn_agent_tool(&config);
    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let mut context = context(store.clone(), Some(resolver));
    context.session_task_registry = Some(registry.clone());

    let result = tool
        .execute_with_context(
            json!({
                "name": "AWS Operator Run",
                "instructions": "Create an RDS database named app-db",
                "target": { "type": "agent", "id": "aws_operator" },
                "mode": "foreground",
                "public_context": { "region": "us-east-1" }
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };
    let task_id = value["task_id"].as_str().expect("task_id");
    let task = registry
        .get(store.session.id, task_id)
        .await
        .expect("task lookup")
        .expect("task");
    assert_eq!(value["mode"], "foreground");
    assert_eq!(value["target"], "aws_operator");
    assert_eq!(task.kind, TASK_KIND_AGENT_HANDOFF);
    assert_eq!(task.display_name, "AWS Operator Run");
    assert_eq!(task.state, SessionTaskState::Succeeded);
    assert_eq!(task.spec["target_id"], "aws_operator");
    assert_eq!(task.spec["mode"], "foreground");
    assert!(task.links.child_session_id.is_some());
}

#[tokio::test]
async fn schema_bound_handoff_requires_report_result() {
    let store = Arc::new(MockPlatformStore::new());
    *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
    let config = target_config(store.agent.id, store.session.harness_id, vec![]);
    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let mut context = context(store.clone(), None);
    context.session_task_registry = Some(registry.clone());

    let result = spawn_agent_tool(&config)
        .execute_with_context(
            json!({
                "name": "Structured handoff",
                "instructions": "Return structured data",
                "target": {"type": "agent", "id": "aws_operator"},
                "mode": "foreground",
                "result_schema": {
                    "type": "object",
                    "properties": {"answer": {"type": "string"}},
                    "required": ["answer"]
                }
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected terminal handoff result, got {result:?}");
    };
    let task_id = value["task_id"].as_str().expect("task_id");
    let task = registry
        .get(store.session.id, task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task.spec[RESULT_SCHEMA_SPEC_KEY]["required"],
        json!(["answer"])
    );
    assert_eq!(task.state, SessionTaskState::Failed);
    assert_eq!(
        task.error.as_ref().map(|error| error.kind.as_str()),
        Some("no_result")
    );
}

#[tokio::test]
async fn handoff_message_schema_is_task_backed_and_wakes_on_activity() {
    let store = Arc::new(MockPlatformStore::new());
    let config = target_config(store.agent.id, store.session.harness_id, vec![]);
    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let mut context = context(store.clone(), None);
    context.session_task_registry = Some(registry.clone());

    let result = spawn_agent_tool(&config)
        .execute_with_context(
            json!({
                "name": "Progress handoff",
                "instructions": "Report progress",
                "target": {"type": "agent", "id": "aws_operator"},
                "mode": "background",
                "message_schema": {
                    "type": "object",
                    "properties": {"step": {"type": "string"}},
                    "required": ["step"]
                }
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected background handoff, got {result:?}");
    };
    let task_id = value["task_id"].as_str().expect("task_id");
    let task = registry
        .get(store.session.id, task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task.spec[MESSAGE_SCHEMA_SPEC_KEY]["required"],
        json!(["step"])
    );
    assert_eq!(task.wake_policy, TaskWakePolicy::OnActivity);
}

#[tokio::test]
async fn spawn_agent_handoff_background_returns_task_handle() {
    let store = Arc::new(MockPlatformStore::new());
    let config = target_config(store.agent.id, store.session.harness_id, vec![]);
    let tool = spawn_agent_tool(&config);
    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let mut context = context(store.clone(), None);
    context.session_task_registry = Some(registry.clone());

    let result = tool
        .execute_with_context(
            json!({
                "name": "AWS Operator Background",
                "instructions": "List RDS databases",
                "target": { "type": "agent", "id": "aws_operator" },
                "mode": "background"
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };
    let task_id = value["task_id"].as_str().expect("task_id");
    assert_eq!(value["status"], "running");
    assert_eq!(value["mode"], "background");

    let mut task = registry
        .get(store.session.id, task_id)
        .await
        .expect("task lookup")
        .expect("task");
    assert_eq!(task.kind, TASK_KIND_AGENT_HANDOFF);
    assert_eq!(task.wake_policy, TaskWakePolicy::OnTerminal);
    assert_eq!(task.spec["mode"], "background");

    for _ in 0..20 {
        if task.state.is_terminal() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        task = registry
            .get(store.session.id, task_id)
            .await
            .expect("task lookup")
            .expect("task");
    }

    assert_eq!(task.state, SessionTaskState::Succeeded);
    assert_eq!(task.summary.as_deref(), Some("Hi!"));
}

#[tokio::test]
async fn spawn_agent_handoff_invite_adds_member_participant() {
    let store = Arc::new(MockPlatformStore::new());
    let config = target_config(store.agent.id, store.session.harness_id, vec![]);
    let tool = spawn_agent_tool(&config);
    let context = context(store.clone(), None);

    let result = tool
        .execute_with_context(
            json!({
                "name": "AWS Operator Invite",
                "instructions": "Join this incident session",
                "target": { "type": "agent", "id": "aws_operator" },
                "mode": "invite"
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };
    assert_eq!(value["mode"], "invite");
    assert_eq!(value["status"], "joined");
    assert_eq!(value["target"], "aws_operator");
    assert!(value["participant_id"].as_str().is_some());

    assert!(
        store
            .created_session_harness_ids
            .lock()
            .expect("recorder lock")
            .is_empty(),
        "invite mode must not create a child session"
    );
    let participants = store
        .joined_participants
        .lock()
        .expect("participants lock")
        .clone();
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0], (store.session.id, store.agent.id));
}

#[tokio::test]
async fn spawn_agent_handoff_invite_rejects_inherited_harness_capability_conflict() {
    let mut store_value = MockPlatformStore::new();
    let child_harness_id = HarnessId::new();
    // EVE-881: the delegate returns the effective (inheritance-resolved)
    // definition, so a capability inherited from a parent harness arrives
    // already folded into the child's configuration.
    let mut child_harness = store_value.harness.clone();
    child_harness.capabilities = vec![everruns_contracts::CapabilityRef::with_config(
        "web_fetch",
        json!({"max_bytes": 1024}),
    )];
    store_value.session.harness_id = child_harness_id;
    store_value.agent.capabilities = vec![everruns_contracts::CapabilityRef::with_config(
        "web_fetch",
        json!({"max_bytes": 2048}),
    )];
    {
        let mut harnesses = store_value.extra_harnesses.lock().unwrap();
        harnesses.insert(child_harness_id, child_harness);
    }
    let store = Arc::new(store_value);
    let config = target_config(store.agent.id, child_harness_id, vec![]);
    let tool = spawn_agent_tool(&config);
    let context = context(store.clone(), None);

    let result = tool
        .execute_with_context(
            json!({
                "name": "AWS Operator Invite",
                "instructions": "Join this incident session",
                "target": { "type": "agent", "id": "aws_operator" },
                "mode": "invite"
            }),
            &context,
        )
        .await;

    assert!(matches!(result, ToolExecutionResult::ToolError(message)
            if message.contains("Invite-mode handoff cannot join target")
                && message.contains("capability `web_fetch`")
                && message.contains("Use background or foreground mode")));
    assert!(
        store
            .joined_participants
            .lock()
            .expect("participants lock")
            .is_empty(),
        "conflicting inherited harness invite must not join the participant"
    );
}

#[tokio::test]
async fn spawn_agent_handoff_invite_rejects_capability_conflict() {
    let mut store_value = MockPlatformStore::new();
    store_value.session.capabilities = vec![everruns_contracts::CapabilityRef::with_config(
        "web_fetch",
        json!({"max_bytes": 1024}),
    )];
    store_value.agent.capabilities = vec![everruns_contracts::CapabilityRef::with_config(
        "web_fetch",
        json!({"max_bytes": 2048}),
    )];
    let store = Arc::new(store_value);
    let config = target_config(store.agent.id, store.session.harness_id, vec![]);
    let tool = spawn_agent_tool(&config);
    let context = context(store.clone(), None);

    let result = tool
        .execute_with_context(
            json!({
                "name": "AWS Operator Invite",
                "instructions": "Join this incident session",
                "target": { "type": "agent", "id": "aws_operator" },
                "mode": "invite"
            }),
            &context,
        )
        .await;

    assert!(matches!(result, ToolExecutionResult::ToolError(message)
            if message.contains("Invite-mode handoff cannot join target")
                && message.contains("capability `web_fetch`")
                && message.contains("Use background or foreground mode")));
    assert!(
        store
            .joined_participants
            .lock()
            .expect("participants lock")
            .is_empty(),
        "conflicting invite must not join the participant"
    );
}

/// Regression for the confused-deputy issue this PR fixes: the child
/// session must be created with the *target's* harness, not the
/// parent session's harness. If a future refactor reintroduces
/// `parent_session.harness_id` here, the child would inherit the
/// parent's mounts/capabilities while gaining the target's tools.
#[tokio::test]
async fn spawn_agent_handoff_uses_target_harness_not_parent() {
    let store = Arc::new(MockPlatformStore::new());
    let resolver = Arc::new(TestConnectionResolver {
        providers: HashSet::from(["fake_aws".to_string()]),
    });
    let target_harness_id = HarnessId::new();
    // Sanity: the target harness must differ from the parent's,
    // otherwise the assertion below cannot distinguish them.
    assert_ne!(store.session.harness_id, target_harness_id);

    let config = target_config(store.agent.id, target_harness_id, vec!["fake_aws"]);
    let tool = spawn_agent_tool(&config);
    let context = context(store.clone(), Some(resolver));

    let result = tool
        .execute_with_context(
            json!({
                "name": "AWS Operator Run",
                "instructions": "Create an RDS database named app-db",
                "target": { "type": "agent", "id": "aws_operator" },
                "mode": "foreground"
            }),
            &context,
        )
        .await;
    assert!(result.is_success(), "expected success, got {result:?}");

    let recorded = store
        .created_session_harness_ids
        .lock()
        .expect("recorder lock")
        .clone();
    assert_eq!(
        recorded.len(),
        1,
        "expected exactly one child create_session call, got {recorded:?}"
    );
    assert_eq!(
        recorded[0], target_harness_id,
        "child session must inherit the target harness, not the parent's",
    );
    assert_ne!(
        recorded[0], store.session.harness_id,
        "child session must NOT inherit the parent harness (confused-deputy regression)",
    );
}

/// A member listed by agent alone runs on that agent's current harness
/// and is addressed by the name the platform filled in on save.
#[tokio::test]
async fn member_without_harness_runs_on_its_current_harness() {
    let store = Arc::new(MockPlatformStore::new());
    let harness_id = HarnessId::new();
    store
        .agent_harness_ids
        .lock()
        .unwrap()
        .insert(store.agent.id, harness_id);
    let config = json!({
        "members": [{ "id": "everruns-review", "agent_id": store.agent.id }]
    });
    AgentHandoffCapability
        .validate_config(&config)
        .expect("agent_id alone is a valid member");
    let tool = spawn_agent_tool(&config);
    let result = tool
        .execute_with_context(
            json!({
                "name": "Review",
                "instructions": "Review the change",
                "target": { "type": "agent", "id": "everruns-review" },
                "mode": "foreground"
            }),
            &context(store.clone(), None),
        )
        .await;
    assert!(result.is_success(), "expected success, got {result:?}");
    assert_eq!(
        *store.created_session_harness_ids.lock().unwrap(),
        vec![harness_id]
    );

    // An agent that no longer exists is reported, not started.
    let missing = json!({ "members": [{ "id": "gone", "agent_id": AgentId::new() }] });
    let result = spawn_agent_tool(&missing)
        .execute_with_context(
            json!({
                "name": "Gone",
                "instructions": "Anything",
                "target": { "type": "agent", "id": "gone" },
                "mode": "foreground"
            }),
            &context(store.clone(), None),
        )
        .await;
    assert!(
        matches!(&result, ToolExecutionResult::ToolError(m) if m.contains("no longer available")),
        "{result:?}"
    );
}

#[tokio::test]
async fn agent_handoff_task_executor_delivers_followup() {
    let parent_id = SessionId::new();
    let child_id = SessionId::new();
    let mut store_value = MockPlatformStore::new();
    store_value.session.id = child_id;
    store_value.session.parent_session_id = Some(parent_id);
    let store = Arc::new(store_value);

    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let task = registry
        .create(CreateSessionTask {
            session_id: parent_id,
            id: None,
            kind: TASK_KIND_AGENT_HANDOFF.to_string(),
            display_name: "AWS Operator".to_string(),
            // Handoff tasks use the dedicated `agent_handoff` kind and carry
            // target_id/external_agent_id in spec.
            spec: json!({ "target_id": "aws", "external_agent_id": "agent_aws" }),
            state: SessionTaskState::Running,
            links: TaskLinks {
                child_session_id: Some(child_id),
                ..Default::default()
            },
            wake_policy: TaskWakePolicy::Silent,
        })
        .await
        .expect("create task");
    let message = TaskMessage {
        id: "msg_1".to_string(),
        task_id: task.id.clone(),
        direction: everruns_core::session_task::TaskMessageDirection::Inbound,
        content: vec![TaskMessagePart::text("List RDS databases")],
        in_reply_to: None,
        created_at: chrono::Utc::now(),
    };

    let mut ctx = ToolContext::new(parent_id);
    ctx.subagent_delegate = Some(delegate(store.clone()));
    ctx.session_task_registry = Some(registry);

    AgentHandoffTaskExecutor
        .deliver(&task, &message, &ctx)
        .await
        .expect("follow-up delivered");
    assert_eq!(
        *store.sent_messages.lock().unwrap(),
        vec![(child_id, "List RDS databases".to_string())]
    );
}

#[tokio::test]
async fn agent_handoff_task_executor_reconciles_terminal_child() {
    let parent_id = SessionId::new();
    let child_id = SessionId::new();
    let store = Arc::new(MockPlatformStore::new());
    *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();

    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let task = registry
        .create(CreateSessionTask {
            session_id: parent_id,
            id: None,
            kind: TASK_KIND_AGENT_HANDOFF.to_string(),
            display_name: "AWS Operator".to_string(),
            spec: json!({ "target_id": "aws", "external_agent_id": "agent_aws" }),
            state: SessionTaskState::Running,
            links: TaskLinks {
                child_session_id: Some(child_id),
                ..Default::default()
            },
            wake_policy: TaskWakePolicy::Silent,
        })
        .await
        .expect("create task");

    let mut ctx = ToolContext::new(parent_id);
    ctx.subagent_delegate = Some(delegate(store));
    ctx.session_task_registry = Some(registry.clone());

    AgentHandoffTaskExecutor
        .reconcile(&task, &ctx)
        .await
        .expect("reconcile succeeds");
    let task = registry
        .get(parent_id, &task.id)
        .await
        .expect("task lookup")
        .expect("task");
    assert_eq!(task.state, SessionTaskState::Succeeded);
    assert_eq!(task.summary.as_deref(), Some("Hi!"));
}
