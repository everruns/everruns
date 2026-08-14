// EVE-595: mid-turn reasoning-effort changes within a single run_turn.
//
// Verifies that when a tool mutates the shared reasoning-effort handle during a
// turn, the NEXT LLM step in the SAME turn observes the new effort, while the
// first step used the original (message-derived) effort.
//
// Run with: cargo test -p everruns-core --test mid_turn_reasoning_effort_test

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use everruns_core::message::{ContentPart, MessageRole};
use everruns_core::message::{Controls, ReasoningConfig};
use everruns_core::message_retriever::InputMessage;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{tool_context::ReasoningEffortHandle, tool_context::ToolContext};
use everruns_test_support::InMemoryAgenticLoop;
use everruns_test_support::llmsim_driver::{LlmSimConfig, SimToolCall, SimTurn};

/// A tool that, when invoked, sets the shared reasoning-effort handle to a new
/// value. This is the "seam" exercised by the test: a tool changing effort
/// mid-turn.
struct SetEffortTool {
    new_effort: String,
}

#[async_trait]
impl Tool for SetEffortTool {
    fn name(&self) -> &str {
        "set_effort"
    }

    fn description(&self) -> &str {
        "Change the reasoning effort used by subsequent LLM steps in this turn."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        // Should never be called: this tool requires context.
        ToolExecutionResult::tool_error("set_effort requires context")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        match &context.reasoning_effort_handle {
            Some(handle) => {
                handle.set(Some(self.new_effort.clone()));
                ToolExecutionResult::success(json!({ "effort": self.new_effort }))
            }
            None => ToolExecutionResult::tool_error("no reasoning effort handle in context"),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn mid_turn_effort_change_is_observed_by_next_llm_step() {
    // Shared sinks/handles wired through the loop.
    let effort_capture: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let handle = ReasoningEffortHandle::new();

    // Scripted LLM: step 1 calls `set_effort` (mid-turn change), step 2 finishes
    // with plain assistant text. This guarantees exactly two LLM steps in one turn.
    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "set_effort".to_string(),
            arguments: json!({}),
            id: Some("call_set_effort".to_string()),
        }]),
        SimTurn::Assistant("All done.".to_string()),
    ])
    .with_effort_capture(effort_capture.clone());

    let runner = InMemoryAgenticLoop::builder()
        .with_llm_sim(sim)
        .tool(SetEffortTool {
            new_effort: "high".to_string(),
        })
        .reasoning_effort_handle(handle.clone())
        .max_iterations(5)
        .build()
        .await
        .expect("build in-memory loop");

    // The initial user message requests effort "low" via controls. The tool will
    // bump it to "high" between step 1 and step 2.
    let input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text("Please proceed.")],
        controls: Some(Controls {
            reasoning: Some(ReasoningConfig {
                effort: Some("low".to_string()),
            }),
            ..Default::default()
        }),
        metadata: None,
        tags: vec![],
    };

    let result = runner.run_turn(input).await.expect("run_turn");
    assert!(result.success, "turn should succeed: {result:?}");
    // Two reason iterations: tool-call step + final step.
    assert_eq!(result.iterations, 2, "expected two LLM steps in one turn");

    // The tool must have run and mutated the live handle.
    assert_eq!(
        handle.get().as_deref(),
        Some("high"),
        "tool should have set the live handle to high"
    );

    let efforts = effort_capture.lock().unwrap().clone();
    assert_eq!(
        efforts.len(),
        2,
        "expected exactly two LLM step requests, got: {efforts:?}"
    );

    // Step 1 used the original message-derived effort.
    assert_eq!(
        efforts[0].as_deref(),
        Some("low"),
        "first LLM step should use the original (message) effort"
    );
    // Step 2 used the NEW effort set mid-turn by the tool — the core assertion.
    assert_eq!(
        efforts[1].as_deref(),
        Some("high"),
        "second LLM step should observe the mid-turn effort change"
    );

    // Sanity: the live handle reflects the override.
    assert_eq!(handle.get().as_deref(), Some("high"));
}

#[tokio::test]
async fn without_handle_mutation_effort_is_stable_across_steps() {
    // Regression guard: when no tool changes effort, both steps see the same
    // message-derived effort (default behavior unchanged).
    let effort_capture: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));

    // A no-op tool so step 1 can issue a tool call without touching effort.
    struct NoopTool;
    #[async_trait]
    impl Tool for NoopTool {
        fn name(&self) -> &str {
            "noop"
        }
        fn description(&self) -> &str {
            "Does nothing."
        }
        fn parameters_schema(&self) -> Value {
            json!({ "type": "object", "properties": {}, "additionalProperties": false })
        }
        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::success(json!({ "ok": true }))
        }
    }

    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "noop".to_string(),
            arguments: json!({}),
            id: Some("call_noop".to_string()),
        }]),
        SimTurn::Assistant("Done.".to_string()),
    ])
    .with_effort_capture(effort_capture.clone());

    let runner = InMemoryAgenticLoop::builder()
        .with_llm_sim(sim)
        .tool(NoopTool)
        // Provide a handle but never mutate it.
        .reasoning_effort_handle(ReasoningEffortHandle::new())
        .max_iterations(5)
        .build()
        .await
        .expect("build in-memory loop");

    let input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text("Go.")],
        controls: Some(Controls {
            reasoning: Some(ReasoningConfig {
                effort: Some("medium".to_string()),
            }),
            ..Default::default()
        }),
        metadata: None,
        tags: vec![],
    };

    let result = runner.run_turn(input).await.expect("run_turn");
    assert!(result.success);

    let efforts = effort_capture.lock().unwrap().clone();
    assert_eq!(efforts.len(), 2, "expected two LLM steps: {efforts:?}");
    assert_eq!(efforts[0].as_deref(), Some("medium"));
    assert_eq!(
        efforts[1].as_deref(),
        Some("medium"),
        "effort must stay stable when no tool changes it"
    );
}

#[tokio::test]
async fn mid_turn_effort_override_is_cleared_before_next_turn() {
    let effort_capture: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let handle = ReasoningEffortHandle::new();

    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "set_effort".to_string(),
            arguments: json!({}),
            id: Some("call_set_effort".to_string()),
        }]),
        SimTurn::Assistant("First turn done.".to_string()),
        SimTurn::Assistant("Second turn done.".to_string()),
    ])
    .with_effort_capture(effort_capture.clone());

    let runner = InMemoryAgenticLoop::builder()
        .with_llm_sim(sim)
        .tool(SetEffortTool {
            new_effort: "high".to_string(),
        })
        .reasoning_effort_handle(handle.clone())
        .max_iterations(5)
        .build()
        .await
        .expect("build in-memory loop");

    let first_input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text("First turn.")],
        controls: Some(Controls {
            reasoning: Some(ReasoningConfig {
                effort: Some("low".to_string()),
            }),
            ..Default::default()
        }),
        metadata: None,
        tags: vec![],
    };
    let first_result = runner.run_turn(first_input).await.expect("first run_turn");
    assert!(
        first_result.success,
        "first turn should succeed: {first_result:?}"
    );
    assert_eq!(handle.get().as_deref(), Some("high"));

    let second_input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text("Second turn.")],
        controls: Some(Controls {
            reasoning: Some(ReasoningConfig {
                effort: Some("low".to_string()),
            }),
            ..Default::default()
        }),
        metadata: None,
        tags: vec![],
    };
    let second_result = runner
        .run_turn(second_input)
        .await
        .expect("second run_turn");
    assert!(
        second_result.success,
        "second turn should succeed: {second_result:?}"
    );

    let efforts = effort_capture.lock().unwrap().clone();
    assert_eq!(
        efforts
            .iter()
            .map(|effort| effort.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("low"), Some("high"), Some("low")],
        "second turn should use its message-derived effort, not the stale override"
    );
}
