//! Mid-turn task wake delivery over the in-process runtime (EVE-681, part A).
//!
//! A parent turn runs several reason iterations while a background child task
//! completes at iteration k < N. The child's terminal wake must be injected
//! into the parent's iteration k+1 context — delivered mid-turn, without the
//! turn idling — and exactly once. The LLM is simulated, so the scenario is
//! deterministic and runs without credentials.

use everruns_host::HostComposition;
use everruns_llmsim::LlmSimRuntimeExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use everruns_builtins::InfinityContextCapability;
use everruns_core::capabilities::{Capability, CapabilityStatus};
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TaskMessage, TaskWakePolicy, apply_task_update,
    generate_task_message_id, new_session_task,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{CapabilityRegistry, MessageRole};
use everruns_host::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
use everruns_llmsim::{LlmSimConfig, SimToolCall, SimTurn};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::error::Result;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};

const CHILD_TASK_ID: &str = "task_wakedemo_child";

// ---------------------------------------------------------------------------
// Minimal in-memory task registry (test double for the durable registry).
// Routes updates through the canonical `apply_task_update` so transition
// detection in `ObservingTaskRegistry` behaves exactly as in production.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestTaskRegistry {
    tasks: Mutex<HashMap<String, SessionTask>>,
    messages: Mutex<Vec<TaskMessage>>,
}

#[async_trait]
impl SessionTaskRegistry for TestTaskRegistry {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        let task = new_session_task(input, chrono::Utc::now());
        self.tasks
            .lock()
            .unwrap()
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(task) = tasks.get_mut(task_id) else {
            return Ok(None);
        };
        if task.session_id != session_id {
            return Ok(None);
        }
        apply_task_update(task, update, chrono::Utc::now());
        Ok(Some(task.clone()))
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        Ok(self
            .tasks
            .lock()
            .unwrap()
            .get(task_id)
            .filter(|t| t.session_id == session_id)
            .cloned())
    }

    async fn list(
        &self,
        session_id: SessionId,
        _filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        Ok(self
            .tasks
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.session_id == session_id)
            .cloned()
            .collect())
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(task) = tasks.get_mut(task_id) else {
            return Ok(None);
        };
        if task.session_id != session_id {
            return Ok(None);
        }
        task.cancel_requested_at
            .get_or_insert_with(chrono::Utc::now);
        Ok(Some(task.clone()))
    }

    async fn record_message(
        &self,
        _session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        let record = TaskMessage {
            id: generate_task_message_id(),
            task_id: task_id.to_string(),
            direction: message.direction,
            content: message.content,
            in_reply_to: message.in_reply_to,
            created_at: chrono::Utc::now(),
        };
        self.messages.lock().unwrap().push(record.clone());
        Ok(record)
    }

    async fn list_messages(
        &self,
        _session_id: SessionId,
        task_id: &str,
        _limit: Option<u32>,
        _after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        Ok(self
            .messages
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.task_id == task_id)
            .cloned()
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Wake-demo capability: a tool that spawns a child task and one that completes
// it, both through the session's task registry (so completing it fires the
// transition observer that feeds the mid-turn wake queue).
// ---------------------------------------------------------------------------

struct WakeDemoCapability {
    /// Wake policy the spawned child is created with (drives the OnActivity /
    /// Silent regression variants).
    policy: TaskWakePolicy,
}

impl Capability for WakeDemoCapability {
    fn id(&self) -> &str {
        "wake_demo"
    }
    fn name(&self) -> &str {
        "Wake Demo"
    }
    fn description(&self) -> &str {
        "Testing capability: spawn and complete a child task through the registry."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(SpawnChildTool {
                policy: self.policy,
            }),
            Box::new(CompleteChildTool),
        ]
    }
}

struct SpawnChildTool {
    policy: TaskWakePolicy,
}

#[async_trait]
impl Tool for SpawnChildTool {
    fn name(&self) -> &str {
        "spawn_child"
    }
    fn description(&self) -> &str {
        "Spawn a background child task."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn requires_context(&self) -> bool {
        true
    }
    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("spawn_child requires context")
    }
    async fn execute_with_context(
        &self,
        _arguments: serde_json::Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(registry) = &context.session_task_registry else {
            return ToolExecutionResult::tool_error("no task registry");
        };
        let created = registry
            .create(CreateSessionTask {
                id: Some(CHILD_TASK_ID.to_string()),
                session_id: context.session_id,
                kind: "background_tool".to_string(),
                display_name: "Doc Reviewer".to_string(),
                spec: serde_json::Value::Null,
                state: SessionTaskState::Running,
                links: Default::default(),
                wake_policy: self.policy,
            })
            .await;
        match created {
            Ok(task) => ToolExecutionResult::success(serde_json::json!({ "task_id": task.id })),
            Err(e) => ToolExecutionResult::tool_error(format!("create failed: {e}")),
        }
    }
}

struct CompleteChildTool;

#[async_trait]
impl Tool for CompleteChildTool {
    fn name(&self) -> &str {
        "complete_child"
    }
    fn description(&self) -> &str {
        "Mark the background child task complete."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn requires_context(&self) -> bool {
        true
    }
    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("complete_child requires context")
    }
    async fn execute_with_context(
        &self,
        _arguments: serde_json::Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(registry) = &context.session_task_registry else {
            return ToolExecutionResult::tool_error("no task registry");
        };
        let updated = registry
            .update(
                context.session_id,
                CHILD_TASK_ID,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    summary: Some("reviewed 3 documents; all clear".to_string()),
                    ..Default::default()
                },
            )
            .await;
        match updated {
            Ok(_) => ToolExecutionResult::success(serde_json::json!({ "completed": true })),
            Err(e) => ToolExecutionResult::tool_error(format!("update failed: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn platform(policy: TaskWakePolicy) -> HostComposition {
    let mut caps = CapabilityRegistry::new();
    caps.register(WakeDemoCapability { policy });
    caps.register(InfinityContextCapability);
    let mut drivers = DriverRegistry::new();
    everruns_llmsim::register_driver(&mut drivers);
    HostComposition::new(caps, drivers)
}

async fn build_runtime(
    policy: TaskWakePolicy,
    script: Vec<SimTurn>,
    registry: Arc<TestTaskRegistry>,
) -> (everruns_host::InProcessRuntime, SessionId) {
    let harness_id = HarnessId::new();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();

    let harness = HarnessBuilder::new("wake", "Coordinate background work.")
        .id(harness_id)
        .capability("wake_demo")
        .capability("infinity_context")
        .build();
    let agent = AgentBuilder::new("agent", "Delegate, then react to results.")
        .id(agent_id)
        .max_iterations(8)
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .build();

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform(policy))
        .llm_sim_as_default(LlmSimConfig::scripted(script))
        .default_model(ModelSpec::on(
            (DriverId::LlmSim).as_str(),
            "llmsim-model".to_string(),
        ))
        .harness(harness)
        .agent(agent)
        .session(session)
        .with_session_task_registry(registry)
        .build()
        .await
        .expect("build runtime");

    (runtime, session_id)
}

fn tool_call(name: &str) -> SimTurn {
    SimTurn::ToolCalls(vec![SimToolCall {
        name: name.to_string(),
        arguments: serde_json::json!({}),
        id: None,
    }])
}

fn wake_message_count(messages: &[everruns_core::Message], needle: &str) -> usize {
    messages
        .iter()
        .filter(|m| {
            m.content
                .iter()
                .any(|p| p.as_text().is_some_and(|t| t.contains(needle)))
        })
        .count()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Child completes at iteration 2; the terminal wake is injected into the
/// parent's iteration-3 context, mid-turn, exactly once, and the turn never
/// idled in between.
#[tokio::test]
async fn terminal_wake_injected_mid_turn_exactly_once() {
    let registry = Arc::new(TestTaskRegistry::default());
    let script = vec![
        tool_call("spawn_child"),    // iter 1: spawn background child (running)
        tool_call("complete_child"), // iter 2: child completes -> wake enqueued
        SimTurn::Assistant("Acknowledged the completed review.".to_string()), // iter 3
    ];
    let (runtime, session_id) =
        build_runtime(TaskWakePolicy::OnTerminal, script, registry.clone()).await;

    let turn = runtime
        .run_text_turn(
            session_id,
            "Kick off a doc review and wrap up when it lands.",
        )
        .await
        .expect("run turn");

    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    // input reason(1)->act, reason(2)->act, reason(3, sees wake)->complete.
    assert!(
        turn.iterations >= 3,
        "turn must run past the completion iteration, not idle: {turn:?}"
    );

    let messages = runtime.messages(session_id).await.expect("messages");
    let wakes = wake_message_count(&messages, "finished: succeeded");
    assert_eq!(
        wakes, 1,
        "terminal wake must be injected exactly once (mid-turn XOR next-turn): {messages:#?}"
    );

    // The wake carried the child's summary so the agent can react without a
    // follow-up read.
    assert_eq!(
        wake_message_count(&messages, "reviewed 3 documents"),
        1,
        "wake payload should include the task summary"
    );
}

#[tokio::test]
async fn query_history_reads_automatic_background_wake_message() {
    let registry = Arc::new(TestTaskRegistry::default());
    let script = vec![
        tool_call("spawn_child"),
        tool_call("complete_child"),
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "query_history".to_string(),
            arguments: serde_json::json!({"query": "reviewed 3 documents"}),
            id: Some("call_wake_history".to_string()),
        }]),
        SimTurn::Assistant("Recovered the wake from history.".to_string()),
    ];
    let (runtime, session_id) =
        build_runtime(TaskWakePolicy::OnTerminal, script, registry.clone()).await;

    runtime
        .run_text_turn(session_id, "Kick off a doc review and verify its wake.")
        .await
        .expect("run turn");

    let history_result = runtime
        .messages(session_id)
        .await
        .unwrap()
        .into_iter()
        .find(|message| {
            message.role == MessageRole::ToolResult
                && message.tool_call_id() == Some("call_wake_history")
        })
        .expect("query_history result");
    assert!(
        history_result
            .content_to_llm_string()
            .contains("reviewed 3 documents"),
        "background wake history must reach query_history",
    );
}

/// Exactly-once across the mid-turn/next-turn boundary: once a wake is drained
/// mid-turn it is claimed and must not be redelivered on a later turn.
#[tokio::test]
async fn wake_is_not_redelivered_on_the_next_turn() {
    let registry = Arc::new(TestTaskRegistry::default());
    let script = vec![
        // Turn 1: spawn, complete (wake enqueued), then react (drains it).
        tool_call("spawn_child"),
        tool_call("complete_child"),
        SimTurn::Assistant("Acknowledged.".to_string()),
        // Turn 2: nothing new — the earlier wake must not reappear.
        SimTurn::Assistant("Nothing else to do.".to_string()),
    ];
    let (runtime, session_id) =
        build_runtime(TaskWakePolicy::OnTerminal, script, registry.clone()).await;

    runtime
        .run_text_turn(session_id, "Kick off a review.")
        .await
        .expect("turn 1");
    let after_turn_1 = wake_message_count(
        &runtime.messages(session_id).await.unwrap(),
        "finished: succeeded",
    );
    assert_eq!(after_turn_1, 1, "wake delivered once in turn 1");

    runtime
        .run_text_turn(session_id, "Anything else?")
        .await
        .expect("turn 2");
    let after_turn_2 = wake_message_count(
        &runtime.messages(session_id).await.unwrap(),
        "finished: succeeded",
    );
    assert_eq!(
        after_turn_2, 1,
        "claimed wake must not be redelivered on the next turn (exactly-once)"
    );
}

/// Silent tasks never wake, even on terminal completion (regression).
#[tokio::test]
async fn silent_task_never_wakes_mid_turn() {
    let registry = Arc::new(TestTaskRegistry::default());
    let script = vec![
        tool_call("spawn_child"),
        tool_call("complete_child"),
        SimTurn::Assistant("Done.".to_string()),
    ];
    let (runtime, session_id) =
        build_runtime(TaskWakePolicy::Silent, script, registry.clone()).await;

    let turn = runtime
        .run_text_turn(session_id, "Run a silent background job.")
        .await
        .expect("run turn");
    assert!(turn.success);

    let messages = runtime.messages(session_id).await.expect("messages");
    assert_eq!(
        wake_message_count(&messages, "finished"),
        0,
        "Silent policy must never inject a wake: {messages:#?}"
    );
}

/// OnActivity delivers a mid-turn wake on the child's terminal transition too
/// (superset of OnTerminal).
#[tokio::test]
async fn on_activity_delivers_terminal_wake_mid_turn() {
    let registry = Arc::new(TestTaskRegistry::default());
    let script = vec![
        tool_call("spawn_child"),
        tool_call("complete_child"),
        SimTurn::Assistant("Reacting to the update.".to_string()),
    ];
    let (runtime, session_id) =
        build_runtime(TaskWakePolicy::OnActivity, script, registry.clone()).await;

    let turn = runtime
        .run_text_turn(session_id, "Monitor and react.")
        .await
        .expect("run turn");
    assert!(turn.success);

    let messages = runtime.messages(session_id).await.expect("messages");
    assert_eq!(
        wake_message_count(&messages, "finished: succeeded"),
        1,
        "OnActivity should deliver the terminal wake mid-turn exactly once"
    );
}
