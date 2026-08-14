//! End-to-end integration tests for the ActAtom tool scheduler, driven through
//! the real in-process runtime (`InProcessRuntime`) with a scripted LLM.
//!
//! These complement the unit tests in `everruns_core::atoms::tool_scheduler`
//! and the `ActAtom`-level tests by proving the *full path*: a capability's
//! tools carry `concurrency_class` hints, those hints flow through the reason
//! step into `ReasonResult.tool_definitions`, and the act scheduler then
//! serializes same-class calls while running class-less calls concurrently —
//! all inside a genuine agent turn, not a hand-built `ActInput`.
//!
//! The recording tools observe concurrency directly (peak in-flight counts),
//! so assertions are deterministic and do not depend on wall-clock timing.

use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityStatus};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{AgentDefinition, CapabilityRegistry, ExecutionSession};
use everruns_host::HostComposition;
use everruns_host::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_provider::tool_types::{ToolCall, ToolHints};
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::llmsim_driver::LlmSimConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

const TEST_CAPABILITY_ID: &str = "sched_test";

/// Shared, thread-safe record of how the scheduler ran the tool batch.
#[derive(Default)]
struct SchedLog {
    /// Currently-executing tools across the whole batch.
    global_inflight: usize,
    /// Peak concurrent executions across the whole batch.
    global_max: usize,
    /// Currently-executing tools per concurrency class.
    class_inflight: HashMap<String, usize>,
    /// Peak concurrent executions per concurrency class.
    class_max: HashMap<String, usize>,
}

/// A tool that records entry/exit so a test can observe the scheduler's
/// concurrency. Optionally declares a `concurrency_class`.
struct RecordingTool {
    name: String,
    class: Option<String>,
    log: Arc<Mutex<SchedLog>>,
}

#[async_trait]
impl Tool for RecordingTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "records scheduling concurrency"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn hints(&self) -> ToolHints {
        match &self.class {
            Some(class) => ToolHints::default().with_concurrency_class(class),
            None => ToolHints::default().with_readonly(true),
        }
    }
    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        {
            let mut log = self.log.lock().unwrap();
            log.global_inflight += 1;
            let g = log.global_inflight;
            if g > log.global_max {
                log.global_max = g;
            }
            if let Some(class) = &self.class {
                let n = log.class_inflight.entry(class.clone()).or_default();
                *n += 1;
                let cur = *n;
                let m = log.class_max.entry(class.clone()).or_default();
                if cur > *m {
                    *m = cur;
                }
            }
        }
        // Hold the slot briefly so genuine concurrency is observable. With a
        // correct scheduler this is irrelevant to the assertions (serialization
        // is structural), but it widens the window if a regression makes
        // same-class calls overlap.
        tokio::time::sleep(Duration::from_millis(30)).await;
        {
            let mut log = self.log.lock().unwrap();
            log.global_inflight -= 1;
            if let Some(class) = &self.class
                && let Some(n) = log.class_inflight.get_mut(class)
            {
                *n -= 1;
            }
        }
        ToolExecutionResult::success(json!({ "tool": self.name }))
    }
}

/// Capability that contributes the recording tools. Two share the
/// `workspace` class (must serialize); three are class-less (must parallelize).
struct RecordingCapability {
    log: Arc<Mutex<SchedLog>>,
}

impl Capability for RecordingCapability {
    fn id(&self) -> &str {
        TEST_CAPABILITY_ID
    }
    fn name(&self) -> &str {
        "Scheduler Test"
    }
    fn description(&self) -> &str {
        "Recording tools for scheduler integration tests."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn features(&self) -> Vec<&'static str> {
        vec![TEST_CAPABILITY_ID]
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        let mk = |name: &str, class: Option<&str>| -> Box<dyn Tool> {
            Box::new(RecordingTool {
                name: name.to_string(),
                class: class.map(str::to_string),
                log: self.log.clone(),
            })
        };
        vec![
            mk("ws_write_a", Some("workspace")),
            mk("ws_write_b", Some("workspace")),
            mk("free_read_a", None),
            mk("free_read_b", None),
            mk("free_read_c", None),
        ]
    }
}

fn platform_with_recording(log: Arc<Mutex<SchedLog>>) -> HostComposition {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(RecordingCapability { log });
    HostComposition::new(capabilities, DriverRegistry::new())
}

fn harness(harness_id: everruns_provider::typed_id::HarnessId) -> everruns_host::SeededHarness {
    HarnessBuilder::new("sched", "You are a scheduler test assistant.")
        .id(harness_id)
        .capability(TEST_CAPABILITY_ID)
        .build()
}

fn agent(agent_id: everruns_provider::typed_id::AgentId) -> AgentDefinition {
    AgentBuilder::new("sched-agent", "Use the tools provided.")
        .id(agent_id)
        .display_name("Scheduler Agent")
        .max_iterations(8)
        .build()
}

fn session(
    session_id: everruns_provider::typed_id::SessionId,
    harness_id: everruns_provider::typed_id::HarnessId,
    agent_id: everruns_provider::typed_id::AgentId,
) -> ExecutionSession {
    SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .title("Scheduler ExecutionSession")
        .build()
}

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: json!({}),
    }
}

/// Build a runtime whose first LLM turn emits `batch` as a single tool-call
/// batch, then a final empty turn. Returns the runtime and the shared log.
async fn runtime_emitting_batch(
    seed: u128,
    batch: Vec<ToolCall>,
) -> (everruns_host::InProcessRuntime, Arc<Mutex<SchedLog>>) {
    let log = Arc::new(Mutex::new(SchedLog::default()));
    let harness_id = everruns_provider::typed_id::HarnessId::from_seed(seed);
    let agent_id = everruns_provider::typed_id::AgentId::from_seed(seed);
    let session_id = everruns_provider::typed_id::SessionId::from_seed(seed);

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform_with_recording(log.clone()))
        .llm_sim(
            LlmSimConfig::fixed("Running the requested tools.")
                .with_tool_call_sequence(vec![batch, vec![]]),
        )
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .harness(harness(harness_id))
        .agent(agent(agent_id))
        .session(session(session_id, harness_id, agent_id))
        .build()
        .await
        .expect("runtime builds");

    (runtime, log)
}

#[tokio::test]
async fn same_class_tool_calls_serialize_through_runtime() {
    // Two tools sharing concurrency_class "workspace", emitted in one batch.
    let batch = vec![call("c1", "ws_write_a"), call("c2", "ws_write_b")];
    let (runtime, log) = runtime_emitting_batch(61, batch).await;
    let session_id = everruns_provider::typed_id::SessionId::from_seed(61);

    let result = runtime
        .run_text_turn(session_id, "Write to the workspace twice.")
        .await
        .expect("turn runs");
    assert!(result.success, "turn should succeed");

    let log = log.lock().unwrap();
    assert_eq!(
        log.class_max.get("workspace").copied(),
        Some(1),
        "same-class tools must never run concurrently (peak in-flight should be 1)"
    );
    // Both tools actually ran.
    assert_eq!(
        log.global_max, 1,
        "the whole batch was a single class → serial"
    );
}

#[tokio::test]
async fn classless_tool_calls_run_in_parallel_through_runtime() {
    // Three class-less (read-only) tools, emitted in one batch.
    let batch = vec![
        call("c1", "free_read_a"),
        call("c2", "free_read_b"),
        call("c3", "free_read_c"),
    ];
    let (runtime, log) = runtime_emitting_batch(62, batch).await;
    let session_id = everruns_provider::typed_id::SessionId::from_seed(62);

    let result = runtime
        .run_text_turn(session_id, "Read three things at once.")
        .await
        .expect("turn runs");
    assert!(result.success, "turn should succeed");

    let log = log.lock().unwrap();
    assert_eq!(
        log.global_max, 3,
        "class-less tools should run concurrently (peak in-flight should be 3)"
    );
}

#[tokio::test]
async fn mixed_batch_serializes_class_and_parallelizes_rest_through_runtime() {
    // One batch mixing both: 2 workspace-class + 3 class-less.
    let batch = vec![
        call("c1", "ws_write_a"),
        call("c2", "free_read_a"),
        call("c3", "ws_write_b"),
        call("c4", "free_read_b"),
        call("c5", "free_read_c"),
    ];
    let (runtime, log) = runtime_emitting_batch(63, batch).await;
    let session_id = everruns_provider::typed_id::SessionId::from_seed(63);

    let result = runtime
        .run_text_turn(session_id, "Do a mix of writes and reads.")
        .await
        .expect("turn runs");
    assert!(result.success, "turn should succeed");

    let log = log.lock().unwrap();
    // workspace writes never overlap each other...
    assert_eq!(
        log.class_max.get("workspace").copied(),
        Some(1),
        "workspace-class writes must serialize"
    );
    // ...but the batch as a whole still runs concurrently (1 workspace + 3 reads).
    assert!(
        log.global_max >= 2,
        "class-less tools should run concurrently with the serialized class (global_max={})",
        log.global_max
    );
}
