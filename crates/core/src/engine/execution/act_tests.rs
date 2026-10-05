use super::*;
use crate::engine::test_fixtures::NoopEventEmitter;
use crate::engine::tools::ToolRegistry;
use crate::engine::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use crate::{Capability, DisabledUtilityLlmService, Tool, ToolExecutionResult};
use async_trait::async_trait;
use everruns_contracts::{BuiltinTool, ClientSideTool};
use serde_json::json;

pub(super) struct ArgumentEchoTool;

struct NarratingGrepTool;

pub(super) struct HumanIntentFixtureHook;

impl crate::engine::capabilities::ToolCallHook for HumanIntentFixtureHook {
    fn narration(
        &self,
        _tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        _phase: crate::engine::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: crate::engine::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        crate::engine::tool_types::human_intent(&tool_call.arguments).map(str::to_string)
    }

    fn transform_for_execution(&self, mut tool_call: ToolCall) -> ToolCall {
        tool_call.arguments = tool_call.execution_arguments();
        tool_call
    }
}

#[async_trait]
impl crate::engine::tools::Tool for NarratingGrepTool {
    fn name(&self) -> &str {
        "grep_files"
    }

    fn description(&self) -> &str {
        "Search files"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({}))
    }

    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: crate::engine::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::engine::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::engine::tool_narration::narrate_grep_files(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }
}

struct NarratingCapability;

#[async_trait]
impl Capability for NarratingCapability {
    fn id(&self) -> &str {
        "narrating_test"
    }

    fn name(&self) -> &str {
        "Narrating test"
    }

    fn description(&self) -> &str {
        "Test-only narration capability"
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(NarratingGrepTool)]
    }
}

#[async_trait]
impl crate::engine::tools::Tool for ArgumentEchoTool {
    fn name(&self) -> &str {
        "argument_echo"
    }

    fn description(&self) -> &str {
        "returns the execution arguments"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::success(arguments)
    }
}

#[test]
fn grouped_headline_uses_tool_owned_narration_for_repeated_actions() {
    use crate::engine::capabilities::{Capability, CapabilityNarrationHook};

    let capability: Arc<dyn Capability> = Arc::new(NarratingCapability);
    let tool_definitions = capability
        .tools()
        .into_iter()
        .map(|tool| tool.to_definition())
        .collect::<Vec<_>>();
    let tool_map = tool_definitions
        .iter()
        .map(|tool_def| (tool_def.name(), tool_def))
        .collect::<std::collections::HashMap<_, _>>();
    let atom = ActAtom::new(ToolRegistry::new(), NoopEventEmitter)
        .with_tool_call_hooks(vec![Arc::new(CapabilityNarrationHook(capability))]);
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let tool_calls = vec![
        ToolCall {
            id: "grep-1".to_string(),
            name: "grep_files".to_string(),
            arguments: json!({ "pattern": "full_name" }),
        },
        ToolCall {
            id: "grep-2".to_string(),
            name: "grep_files".to_string(),
            arguments: json!({ "pattern": "login" }),
        },
    ];

    assert_eq!(
        atom.render_group_headline(
            &context,
            &tool_calls,
            &tool_map,
            ToolNarrationPhase::Started,
            None,
        )
        .as_deref(),
        Some("Searching files twice")
    );
    assert_eq!(
        atom.render_group_headline(
            &context,
            &tool_calls,
            &tool_map,
            ToolNarrationPhase::Completed,
            None,
        )
        .as_deref(),
        Some("Searched files twice")
    );
}

/// Tools assembled outside any capability (the unified `spawn_agent`
/// dispatcher, registry augmentations, MCP proxies) have no
/// `CapabilityNarrationHook`, so the act atom must consult the registry
/// before the generic "Running {display_name}" fallback.
#[test]
fn registry_owned_tool_narrates_itself_without_a_capability_hook() {
    let mut registry = ToolRegistry::new();
    registry.register_boxed(Box::new(NarratingGrepTool));
    let atom =
        ActAtom::new(ToolRegistry::new(), NoopEventEmitter).with_tool_registry(Arc::new(registry));
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let tool_call = ToolCall {
        id: "grep-1".to_string(),
        name: "grep_files".to_string(),
        arguments: json!({ "pattern": "full_name" }),
    };

    assert_eq!(
        atom.render_tool_narration(
            &context,
            None,
            &tool_call,
            ToolNarrationPhase::Started,
            None,
        ),
        "Searching files for full_name"
    );
}

struct UtilityLlmContextProbeTool;

#[async_trait]
impl crate::engine::tools::Tool for UtilityLlmContextProbeTool {
    fn name(&self) -> &str {
        "utility_llm_context_probe"
    }

    fn description(&self) -> &str {
        "checks whether the utility LLM service is present in tool context"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("context required")
    }

    async fn execute_with_context(
        &self,
        _arguments: serde_json::Value,
        context: &crate::engine::tool_context::ToolContext,
    ) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({
            "utility_llm_service": context.utility_llm_service.is_some(),
            "configured": context
                .utility_llm_service
                .as_ref()
                .is_some_and(|service| service.is_configured()),
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Shared scheduling observations recorded by `RecordingTool`.
#[derive(Default)]
struct SchedObservations {
    /// Currently-executing count per concurrency class.
    class_inflight: std::collections::HashMap<String, usize>,
    /// Peak concurrent executions observed per class.
    class_max: std::collections::HashMap<String, usize>,
    /// Currently-executing count across all tools.
    global_inflight: usize,
    /// Peak concurrent executions across all tools.
    global_max: usize,
}

/// Tool that records start/end so a test can observe how the act scheduler
/// ran a batch (intra-class serialization, cross-class parallelism).
struct RecordingTool {
    name: String,
    class: Option<String>,
    obs: Arc<std::sync::Mutex<SchedObservations>>,
}

#[async_trait]
impl crate::engine::tools::Tool for RecordingTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "records scheduling order"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        // Enter: bump counters in a short critical section (no await held).
        {
            let mut obs = self.obs.lock().unwrap();
            obs.global_inflight += 1;
            let g = obs.global_inflight;
            if g > obs.global_max {
                obs.global_max = g;
            }
            if let Some(class) = &self.class {
                let n = obs.class_inflight.entry(class.clone()).or_default();
                *n += 1;
                let cur = *n;
                let m = obs.class_max.entry(class.clone()).or_default();
                if cur > *m {
                    *m = cur;
                }
            }
        }
        // Hold the slot long enough that any concurrency is observable.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        // Exit.
        {
            let mut obs = self.obs.lock().unwrap();
            obs.global_inflight -= 1;
            if let Some(class) = &self.class
                && let Some(n) = obs.class_inflight.get_mut(class)
            {
                *n -= 1;
            }
        }
        ToolExecutionResult::success(json!({ "tool": self.name }))
    }
}

struct CancellationProbeTool {
    started: Arc<tokio::sync::Notify>,
    dropped_tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl CancellationProbeTool {
    fn new(
        started: Arc<tokio::sync::Notify>,
        dropped_tx: tokio::sync::oneshot::Sender<()>,
    ) -> Self {
        Self {
            started,
            dropped_tx: Arc::new(std::sync::Mutex::new(Some(dropped_tx))),
        }
    }
}

#[async_trait]
impl crate::engine::tools::Tool for CancellationProbeTool {
    fn name(&self) -> &str {
        "cancellation_probe"
    }

    fn description(&self) -> &str {
        "waits until cancelled"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        struct DropSignal {
            tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        }

        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Ok(mut guard) = self.tx.lock()
                    && let Some(tx) = guard.take()
                {
                    let _ = tx.send(());
                }
            }
        }

        let _drop_signal = DropSignal {
            tx: self.dropped_tx.clone(),
        };
        self.started.notify_one();
        std::future::pending::<()>().await;
        unreachable!("pending cancellation probe should only finish by cancellation")
    }
}

/// Build a server-side tool definition carrying scheduling hints.
fn recording_tool_def(name: &str, class: Option<&str>, cpu_bound: bool) -> ToolDefinition {
    let mut hints = crate::engine::tool_types::ToolHints::default();
    if let Some(class) = class {
        hints = hints.with_concurrency_class(class);
    }
    if cpu_bound {
        hints = hints.with_cpu_bound(true);
    }
    ToolDefinition::Builtin(BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: "records scheduling order".to_string(),
        parameters: json!({ "type": "object", "properties": {} }),
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints,
        full_parameters: None,
    })
}

#[tokio::test]
async fn test_act_atom_empty_tool_calls() {
    let executor = ToolRegistry::with_defaults();
    let event_emitter = NoopEventEmitter;
    let atom = ActAtom::new(executor, event_emitter);

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![],
        tool_definitions: vec![],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    assert!(result.completed);
    assert!(result.results.is_empty());
    assert_eq!(result.success_count, 0);
    assert_eq!(result.error_count, 0);
}

#[tokio::test]
async fn test_act_atom_threads_utility_llm_service_to_tool_context() {
    let mut executor = ToolRegistry::with_defaults();
    executor.register(UtilityLlmContextProbeTool);
    let event_emitter = NoopEventEmitter;
    let atom = ActAtom::new(executor, event_emitter)
        .with_utility_llm_service(Arc::new(DisabledUtilityLlmService));

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "utility_llm_context_probe".to_string(),
            arguments: json!({}),
        }],
        tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
            name: "utility_llm_context_probe".to_string(),
            display_name: None,
            description: "checks context".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: crate::engine::tool_types::ToolHints::default(),
            full_parameters: None,
        })],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    assert_eq!(result.success_count, 1);
    let payload = result.results[0].result.result.as_ref().unwrap();
    assert_eq!(payload["utility_llm_service"], true);
    assert_eq!(payload["configured"], false);
}

/// End-to-end ActAtom scheduling: a single batch with two same-class tools
/// (one of them `cpu_bound`, exercising the spawn path) plus an independent
/// tool. Asserts the scheduler serializes within the class, parallelizes
/// across classes, runs every tool, and preserves call order in results.
#[tokio::test]
async fn test_act_atom_schedules_batch_by_concurrency_class() {
    let obs = Arc::new(std::sync::Mutex::new(SchedObservations::default()));

    let mut executor = ToolRegistry::new();
    executor.register(RecordingTool {
        name: "writer_a".to_string(),
        class: Some("ws".to_string()),
        obs: obs.clone(),
    });
    executor.register(RecordingTool {
        name: "writer_b".to_string(),
        class: Some("ws".to_string()),
        obs: obs.clone(),
    });
    executor.register(RecordingTool {
        name: "reader".to_string(),
        class: None,
        obs: obs.clone(),
    });

    let atom = ActAtom::new(executor, NoopEventEmitter);
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());

    // Call order: writer_a, reader, writer_b. writer_a and writer_b share
    // class "ws" (writer_b is cpu_bound → executed on its own task).
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![
            ToolCall {
                id: "call_a".to_string(),
                name: "writer_a".to_string(),
                arguments: json!({}),
            },
            ToolCall {
                id: "call_r".to_string(),
                name: "reader".to_string(),
                arguments: json!({}),
            },
            ToolCall {
                id: "call_b".to_string(),
                name: "writer_b".to_string(),
                arguments: json!({}),
            },
        ],
        tool_definitions: vec![
            recording_tool_def("writer_a", Some("ws"), false),
            recording_tool_def("reader", None, false),
            recording_tool_def("writer_b", Some("ws"), true),
        ],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    // Every tool ran and succeeded.
    assert_eq!(result.success_count, 3, "all three tools should succeed");
    // Results are returned in the model's original call order.
    let names: Vec<&str> = result
        .results
        .iter()
        .map(|r| r.tool_call.name.as_str())
        .collect();
    assert_eq!(names, vec!["writer_a", "reader", "writer_b"]);

    let obs = obs.lock().unwrap();
    // Same-class tools never overlapped (serialized) — even though one is
    // cpu_bound and runs on its own task.
    assert_eq!(
        obs.class_max.get("ws").copied(),
        Some(1),
        "same-class tools must serialize"
    );
    // The independent tool overlapped with the class group: peak global
    // concurrency exceeded 1, proving cross-class parallelism.
    assert!(
        obs.global_max >= 2,
        "independent tool should run concurrently with the class group (global_max={})",
        obs.global_max
    );
}

/// A tool that leaves work running past its own future: it hands the
/// call's cancellation token to a detached task and returns immediately.
/// That task is the thing a dropped future cannot reach.
struct DetachedWorkTool {
    cancelled_tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl DetachedWorkTool {
    fn new(cancelled_tx: tokio::sync::oneshot::Sender<()>) -> Self {
        Self {
            cancelled_tx: Arc::new(std::sync::Mutex::new(Some(cancelled_tx))),
        }
    }
}

#[async_trait]
impl crate::engine::tools::Tool for DetachedWorkTool {
    fn name(&self) -> &str {
        "detached_work"
    }

    fn description(&self) -> &str {
        "spawns work that outlives the call unless cancelled"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("requires context")
    }

    async fn execute_with_context(
        &self,
        _arguments: serde_json::Value,
        context: &crate::engine::tool_context::ToolContext,
    ) -> ToolExecutionResult {
        let token = context
            .cancellation
            .clone()
            .expect("act must supply a cancellation token");
        assert!(!token.is_cancelled(), "token is live during the call");
        let tx = self.cancelled_tx.clone();
        tokio::spawn(async move {
            token.cancelled().await;
            if let Ok(mut guard) = tx.lock()
                && let Some(tx) = guard.take()
            {
                let _ = tx.send(());
            }
        });
        ToolExecutionResult::success(json!({ "spawned": true }))
    }
}

/// Work a tool leaves running must learn that its call ended. Dropping the
/// act future cannot tell it — a dropped future is never polled again — so
/// the token on `ToolContext` is the only signal that reaches it.
#[tokio::test]
async fn test_act_atom_cancels_detached_tool_work_when_the_call_ends() {
    let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel();

    let mut executor = ToolRegistry::new();
    executor.register(DetachedWorkTool::new(cancelled_tx));

    let atom = ActAtom::new(executor, NoopEventEmitter);
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "detached_work".to_string(),
            arguments: json!({}),
        }],
        tool_definitions: vec![recording_tool_def("detached_work", None, false)],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    atom.execute(input).await.expect("act should succeed");

    tokio::time::timeout(std::time::Duration::from_secs(1), cancelled_rx)
        .await
        .expect("detached work should be cancelled once the call ends")
        .expect("cancellation signal should be sent");
}

#[tokio::test]
async fn test_act_atom_cancels_detached_tool_work_when_the_turn_is_cancelled() {
    let started = Arc::new(tokio::sync::Notify::new());
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();

    let mut executor = ToolRegistry::new();
    executor.register(CancellationProbeTool::new(started.clone(), dropped_tx));

    let atom = ActAtom::new(executor, NoopEventEmitter);
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "cancellation_probe".to_string(),
            arguments: json!({}),
        }],
        tool_definitions: vec![recording_tool_def("cancellation_probe", None, true)],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let act_task = tokio::spawn(async move { atom.execute(input).await });
    started.notified().await;
    act_task.abort();
    assert!(act_task.await.unwrap_err().is_cancelled());

    // The existing abort path still holds: the tool future itself is dropped.
    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("tool future should be dropped when the turn is cancelled")
        .expect("drop signal should be sent");
}

#[tokio::test]
async fn test_act_atom_aborts_cpu_bound_tool_task_on_cancellation() {
    let started = Arc::new(tokio::sync::Notify::new());
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();

    let mut executor = ToolRegistry::new();
    executor.register(CancellationProbeTool::new(started.clone(), dropped_tx));

    let atom = ActAtom::new(executor, NoopEventEmitter);
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "cancellation_probe".to_string(),
            arguments: json!({}),
        }],
        tool_definitions: vec![recording_tool_def("cancellation_probe", None, true)],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let act_task = tokio::spawn(async move { atom.execute(input).await });
    started.notified().await;
    act_task.abort();
    assert!(act_task.await.unwrap_err().is_cancelled());

    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("cpu-bound tool task should be aborted when ActAtom is cancelled")
        .expect("drop signal should be sent by cancelled tool future");
}

/// With `parallel_tool_calls = Some(false)`, the whole batch runs strictly
/// sequentially regardless of class — peak concurrency must be 1.
#[tokio::test]
async fn test_act_atom_parallel_tool_calls_false_serializes_everything() {
    let obs = Arc::new(std::sync::Mutex::new(SchedObservations::default()));
    let mut executor = ToolRegistry::new();
    for name in ["t0", "t1", "t2"] {
        executor.register(RecordingTool {
            name: name.to_string(),
            class: None,
            obs: obs.clone(),
        });
    }
    let atom = ActAtom::new(executor, NoopEventEmitter);
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![
            ToolCall {
                id: "c0".to_string(),
                name: "t0".to_string(),
                arguments: json!({}),
            },
            ToolCall {
                id: "c1".to_string(),
                name: "t1".to_string(),
                arguments: json!({}),
            },
            ToolCall {
                id: "c2".to_string(),
                name: "t2".to_string(),
                arguments: json!({}),
            },
        ],
        tool_definitions: vec![
            recording_tool_def("t0", None, false),
            recording_tool_def("t1", None, false),
            recording_tool_def("t2", None, false),
        ],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: Some(false),
    };

    let result = atom.execute(input).await.unwrap();
    assert_eq!(result.success_count, 3);
    assert_eq!(
        obs.lock().unwrap().global_max,
        1,
        "parallel_tool_calls=false must serialize the whole batch"
    );
}

#[tokio::test]
async fn test_act_atom_tool_not_found() {
    let executor = ToolRegistry::with_defaults();
    let event_emitter = NoopEventEmitter;
    let atom = ActAtom::new(executor, event_emitter);

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "nonexistent_tool".to_string(),
            arguments: json!({}),
        }],
        tool_definitions: vec![],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    assert!(result.completed);
    assert_eq!(result.results.len(), 1);
    assert!(!result.results[0].success);
    assert_eq!(result.results[0].status, "error");
    assert!(
        result.results[0]
            .result
            .error
            .as_ref()
            .unwrap()
            .contains("not found")
    );
}

#[tokio::test]
async fn test_act_atom_uses_tool_call_hooks_for_execution_arguments() {
    let mut executor = ToolRegistry::new();
    executor.register(ArgumentEchoTool);
    let tool_def = executor.get("argument_echo").unwrap().to_definition();
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let atom = ActAtom::new(executor, emitter.clone())
        .with_tool_call_hooks(vec![std::sync::Arc::new(HumanIntentFixtureHook)]);

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "argument_echo".to_string(),
            arguments: json!({
                "value": "visible",
                "human_intent": "Echoing test arguments"
            }),
        }],
        tool_definitions: vec![tool_def],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    assert!(result.results[0].success);
    assert_eq!(
        result.results[0].result.result,
        Some(json!({ "value": "visible" }))
    );

    let events = emitter.events().await;
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec![
            "act.started",
            "tool.started",
            "tool.completed",
            "act.completed",
        ],
        "all hosts must observe the engine-owned phase order",
    );
    let act_started = events
        .iter()
        .find(|event| event.event_type == "act.started")
        .expect("act.started event");
    let crate::engine::events::EventData::ActStarted(data) = &act_started.data else {
        panic!("expected act.started data");
    };
    assert_eq!(data.headline.as_deref(), Some("Echoing test arguments"));
    assert_eq!(
        data.tool_calls[0].narration.as_deref(),
        Some("Echoing test arguments")
    );

    let tool_started = events
        .iter()
        .find(|event| event.event_type == "tool.started")
        .expect("tool.started event");
    let crate::engine::events::EventData::ToolStarted(data) = &tool_started.data else {
        panic!("expected tool.started data");
    };
    let started_fingerprint = data
        .tool_call_fingerprint
        .as_ref()
        .expect("tool.started call fingerprint");
    assert_eq!(data.narration.as_deref(), Some("Echoing test arguments"));

    let tool_completed = events
        .iter()
        .find(|event| event.event_type == "tool.completed")
        .expect("tool.completed event");
    let crate::engine::events::EventData::ToolCompleted(data) = &tool_completed.data else {
        panic!("expected tool.completed data");
    };
    assert_eq!(
        data.tool_call_fingerprint.as_ref(),
        Some(started_fingerprint)
    );
    assert!(data.tool_result_fingerprint.is_some());
    assert_eq!(data.narration.as_deref(), Some("Echoing test arguments"));
}

#[tokio::test]
async fn test_act_atom_strips_human_intent_from_client_tool_calls() {
    let executor = ToolRegistry::new();
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let atom = ActAtom::new(executor, emitter)
        .with_tool_call_hooks(vec![std::sync::Arc::new(HumanIntentFixtureHook)]);

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_client".to_string(),
            name: "browser_click".to_string(),
            arguments: json!({
                "selector": "#btn",
                "human_intent": "Clicking approve"
            }),
        }],
        tool_definitions: vec![ToolDefinition::ClientSide(ClientSideTool::new(
            "browser_click",
            "Click button",
            json!({
                "type": "object",
                "properties": {
                    "selector": {"type": "string"}
                },
                "required": ["selector"]
            }),
        ))],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    assert_eq!(result.client_tool_calls.len(), 1);
    assert_eq!(
        result.client_tool_calls[0].arguments,
        json!({ "selector": "#btn" })
    );
}

#[test]
fn test_act_result_connection_required_serialization() {
    let result = ActResult {
        results: vec![ToolCallResult {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "daytona_create_sandbox".to_string(),
                arguments: json!({}),
            },
            result: ToolResult {
                tool_call_id: "call_1".to_string(),
                result: Some(json!({"connection_required": "daytona"})),
                images: None,
                error: None,
                connection_required: Some(ConnectionRequired::provider_only("daytona")),
                raw_output: None,
            },
            success: false,
            status: "success".to_string(),
            connection_required: Some(ConnectionRequired::provider_only("daytona")),
            determinism_fatal: None,
        }],
        completed: true,
        success_count: 0,
        error_count: 0,
        waiting_for_tool_results: true,
        waiting_for_url_elicitation: false,
        blocked: false,
        client_tool_calls: vec![],
        client_tool_definitions: vec![],
    };
    let json_str = serde_json::to_string(&result).unwrap();
    let parsed: ActResult = serde_json::from_str(&json_str).unwrap();

    assert!(parsed.waiting_for_tool_results);
    assert_eq!(
        parsed.results[0].connection_required,
        result.results[0].connection_required
    );
}

#[test]
fn test_act_result_backward_compat_deserialization() {
    // Old JSON without new fields still deserializes
    let json_str = r#"{
        "results": [],
        "completed": true,
        "success_count": 0,
        "error_count": 0
    }"#;
    let parsed: ActResult = serde_json::from_str(json_str).unwrap();

    assert!(!parsed.waiting_for_tool_results);
    assert!(parsed.client_tool_calls.is_empty());
}

/// Verify that a denying `OutboundToolRateLimiter` short-circuits tool execution
/// and returns a rate-limit error result rather than calling the actual tool.
#[tokio::test]
async fn test_outbound_tool_rate_limiter_blocks_execution() {
    use crate::engine::typed_id::OrgId;

    struct DenyAll;
    #[async_trait]
    impl crate::engine::tool_execution::OutboundToolRateLimiter for DenyAll {
        async fn check_org(&self, _org_id: &OrgId) -> bool {
            false
        }
    }

    let mut executor = ToolRegistry::with_defaults();
    executor.register(ArgumentEchoTool);
    let atom = ActAtom::new(executor, NoopEventEmitter)
        .with_org_id(OrgId::from_seed(1))
        .with_outbound_tool_rate_limiter(Arc::new(DenyAll));

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "argument_echo".to_string(),
            arguments: json!({"value": "should_not_reach"}),
        }],
        tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
            name: "argument_echo".to_string(),
            display_name: None,
            description: "echo".to_string(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: crate::engine::tool_types::ToolHints::default(),
            full_parameters: None,
        })],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    assert_eq!(result.success_count, 0);
    assert_eq!(result.error_count, 1);
    let tool_result = &result.results[0];
    assert!(!tool_result.success);
    assert_eq!(tool_result.status, "error");
    assert!(
        tool_result
            .result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("rate limit exceeded")
    );
    assert!(tool_result.result.result.is_none());
}

/// Verify that an allowing `OutboundToolRateLimiter` does not block execution.
#[tokio::test]
async fn test_outbound_tool_rate_limiter_allows_execution() {
    use crate::engine::typed_id::OrgId;

    struct AllowAll;
    #[async_trait]
    impl crate::engine::tool_execution::OutboundToolRateLimiter for AllowAll {
        async fn check_org(&self, _org_id: &OrgId) -> bool {
            true
        }
    }

    let mut executor = ToolRegistry::with_defaults();
    executor.register(ArgumentEchoTool);
    let atom = ActAtom::new(executor, NoopEventEmitter)
        .with_org_id(OrgId::from_seed(1))
        .with_outbound_tool_rate_limiter(Arc::new(AllowAll));

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "argument_echo".to_string(),
            arguments: json!({"value": "hello"}),
        }],
        tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
            name: "argument_echo".to_string(),
            display_name: None,
            description: "echo".to_string(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: crate::engine::tool_types::ToolHints::default(),
            full_parameters: None,
        })],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);
}

// -----------------------------------------------------------------------
// DurableToolResultStore idempotency tests (EVE-530)
// -----------------------------------------------------------------------

use crate::engine::tool_types::{SideEffectClass, ToolHints};
use crate::engine::{durability::DurableToolResultStore, durability::ToolCallClaimResult};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct InMemoryDurableStore {
    rows: Mutex<HashMap<(String, String), StoreRow>>,
}

#[derive(Clone)]
struct StoreRow {
    status: String,
    result_json: serde_json::Value,
    args_fingerprint: String,
    #[allow(dead_code)]
    claim_token: Uuid,
}

#[async_trait]
impl DurableToolResultStore for InMemoryDurableStore {
    async fn try_claim_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        _tool_name: &str,
        args_fingerprint: &str,
    ) -> crate::engine::error::Result<ToolCallClaimResult> {
        let key = (turn_id.to_string(), tool_call_id.to_string());
        let mut rows = self.rows.lock().unwrap();
        if let Some(row) = rows.get(&key) {
            match row.status.as_str() {
                "settled" => {
                    if row.args_fingerprint != args_fingerprint {
                        return Ok(ToolCallClaimResult::DeterminismViolation {
                            stored_fingerprint: row.args_fingerprint.clone(),
                            current_fingerprint: args_fingerprint.to_string(),
                        });
                    }
                    return Ok(ToolCallClaimResult::AlreadySettled {
                        result_json: row.result_json.clone(),
                        args_fingerprint: row.args_fingerprint.clone(),
                    });
                }
                _ => {
                    return Ok(ToolCallClaimResult::AlreadyRunning {
                        args_fingerprint: row.args_fingerprint.clone(),
                    });
                }
            }
        }
        let token = Uuid::new_v4();
        rows.insert(
            key,
            StoreRow {
                status: "running".to_string(),
                result_json: serde_json::Value::Null,
                args_fingerprint: args_fingerprint.to_string(),
                claim_token: token,
            },
        );
        Ok(ToolCallClaimResult::Claimed { claim_token: token })
    }

    async fn settle_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        result_json: serde_json::Value,
        status: &str,
        _claim_token: Uuid,
    ) -> crate::engine::error::Result<bool> {
        let key = (turn_id.to_string(), tool_call_id.to_string());
        let mut rows = self.rows.lock().unwrap();
        if let Some(row) = rows.get_mut(&key) {
            row.status = status.to_string();
            row.result_json = result_json;
            return Ok(true);
        }
        Ok(false)
    }

    async fn get_tool_call_status(
        &self,
        turn_id: &str,
        tool_call_id: &str,
    ) -> crate::engine::error::Result<Option<crate::engine::durability::DurableToolCallStatus>>
    {
        let key = (turn_id.to_string(), tool_call_id.to_string());
        let rows = self.rows.lock().unwrap();
        Ok(rows.get(&key).map(|row| match row.status.as_str() {
            "settled" => crate::engine::durability::DurableToolCallStatus::Settled {
                result_json: row.result_json.clone(),
            },
            "interrupted" => crate::engine::durability::DurableToolCallStatus::Interrupted {
                result_json: Some(row.result_json.clone()),
            },
            _ => crate::engine::durability::DurableToolCallStatus::Running,
        }))
    }
}

fn make_act_input_with_store(
    tool_call: ToolCall,
    tool_defs: Vec<ToolDefinition>,
    context: ExecutionContext,
) -> ActInput {
    ActInput {
        org_id: None,
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![tool_call],
        tool_definitions: tool_defs,
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    }
}

fn arg_echo_tool_def(side_effect: SideEffectClass) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: "argument_echo".to_string(),
        display_name: None,
        description: "echo".to_string(),
        parameters: json!({"type": "object"}),
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints: ToolHints::default().with_side_effect_class(side_effect),
        full_parameters: None,
    })
}

/// First execution succeeds normally and the result is settled in the store.
#[tokio::test]
async fn test_idempotency_first_execution_claims_and_settles() {
    let store = Arc::new(InMemoryDurableStore::default());
    let mut executor = ToolRegistry::with_defaults();
    executor.register(ArgumentEchoTool);
    let atom =
        ActAtom::new(executor, NoopEventEmitter).with_durable_tool_result_store(store.clone());

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let tc = ToolCall {
        id: "c1".to_string(),
        name: "argument_echo".to_string(),
        arguments: json!({"value": "hello"}),
    };
    let input = make_act_input_with_store(
        tc,
        vec![arg_echo_tool_def(SideEffectClass::AtMostOnce)],
        context,
    );

    let result = atom.execute(input).await.unwrap();
    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);

    // Row should be settled now.
    let rows = store.rows.lock().unwrap();
    let row = rows.values().next().unwrap();
    assert_eq!(row.status, "settled");
}

/// Second execution replays the stored result without re-running the tool.
#[tokio::test]
async fn test_idempotency_replay_already_settled() {
    use crate::engine::tool_fingerprint::tool_call_fingerprint;

    let store = Arc::new(InMemoryDurableStore::default());
    let tc = ToolCall {
        id: "c1".to_string(),
        name: "argument_echo".to_string(),
        arguments: json!({"value": "hello"}),
    };
    let fp = tool_call_fingerprint(&tc);

    // Pre-populate as settled.
    {
        let stored_result = serde_json::to_value(ToolResult {
            tool_call_id: "c1".to_string(),
            result: Some(json!({"value": "hello"})),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        })
        .unwrap();
        store.rows.lock().unwrap().insert(
            (
                "turn_00000000000000000000000000000000".to_string(),
                "c1".to_string(),
            ),
            StoreRow {
                status: "settled".to_string(),
                result_json: stored_result,
                args_fingerprint: fp,
                claim_token: Uuid::new_v4(),
            },
        );
    }

    let mut executor = ToolRegistry::with_defaults();
    executor.register(ArgumentEchoTool);
    let atom =
        ActAtom::new(executor, NoopEventEmitter).with_durable_tool_result_store(store.clone());

    let context = ExecutionContext::new(
        SessionId::new(),
        TurnId::from_uuid(Uuid::nil()),
        MessageId::new(),
    );
    let input = make_act_input_with_store(
        tc,
        vec![arg_echo_tool_def(SideEffectClass::AtMostOnce)],
        context,
    );

    let result = atom.execute(input).await.unwrap();
    assert_eq!(result.success_count, 1, "replay should count as success");
    assert_eq!(result.error_count, 0);
}

/// AtMostOnce tool with a stale running claim returns an interrupted error.
#[tokio::test]
async fn test_idempotency_at_most_once_stale_running_returns_interrupted() {
    use crate::engine::tool_fingerprint::tool_call_fingerprint;

    let store = Arc::new(InMemoryDurableStore::default());
    let tc = ToolCall {
        id: "c1".to_string(),
        name: "argument_echo".to_string(),
        arguments: json!({"value": "x"}),
    };
    let fp = tool_call_fingerprint(&tc);

    // Pre-populate as running (stale from dead worker).
    store.rows.lock().unwrap().insert(
        (
            "turn_00000000000000000000000000000000".to_string(),
            "c1".to_string(),
        ),
        StoreRow {
            status: "running".to_string(),
            result_json: serde_json::Value::Null,
            args_fingerprint: fp,
            claim_token: Uuid::new_v4(),
        },
    );

    let mut executor = ToolRegistry::with_defaults();
    executor.register(ArgumentEchoTool);
    let atom =
        ActAtom::new(executor, NoopEventEmitter).with_durable_tool_result_store(store.clone());

    let context = ExecutionContext::new(
        SessionId::new(),
        TurnId::from_uuid(Uuid::nil()),
        MessageId::new(),
    );
    let input = make_act_input_with_store(
        tc,
        vec![arg_echo_tool_def(SideEffectClass::AtMostOnce)],
        context,
    );

    let result = atom.execute(input).await.unwrap();
    assert_eq!(
        result.error_count, 1,
        "AtMostOnce stale running should error"
    );
    let err = result.results[0].result.error.as_deref().unwrap_or("");
    assert!(
        err.contains("interrupted"),
        "error should mention interrupted: {err}"
    );

    // Row should be settled as interrupted.
    let rows = store.rows.lock().unwrap();
    let row = rows.values().next().unwrap();
    assert_eq!(row.status, "interrupted");
}

/// Pure/Idempotent tool with a stale running claim proceeds to execution normally.
#[tokio::test]
async fn test_idempotency_idempotent_tool_stale_running_reexecutes() {
    use crate::engine::tool_fingerprint::tool_call_fingerprint;

    let store = Arc::new(InMemoryDurableStore::default());
    let tc = ToolCall {
        id: "c1".to_string(),
        name: "argument_echo".to_string(),
        arguments: json!({"value": "x"}),
    };
    let fp = tool_call_fingerprint(&tc);

    store.rows.lock().unwrap().insert(
        (
            "turn_00000000000000000000000000000000".to_string(),
            "c1".to_string(),
        ),
        StoreRow {
            status: "running".to_string(),
            result_json: serde_json::Value::Null,
            args_fingerprint: fp,
            claim_token: Uuid::new_v4(),
        },
    );

    let mut executor = ToolRegistry::with_defaults();
    executor.register(ArgumentEchoTool);
    let atom =
        ActAtom::new(executor, NoopEventEmitter).with_durable_tool_result_store(store.clone());

    let context = ExecutionContext::new(
        SessionId::new(),
        TurnId::from_uuid(Uuid::nil()),
        MessageId::new(),
    );
    let input = make_act_input_with_store(
        tc,
        vec![arg_echo_tool_def(SideEffectClass::Idempotent)],
        context,
    );

    let result = atom.execute(input).await.unwrap();
    assert_eq!(
        result.success_count, 1,
        "Idempotent should re-execute successfully"
    );
    assert_eq!(result.error_count, 0);
}

#[path = "act_payment_tests.rs"]
mod payment_tests;
