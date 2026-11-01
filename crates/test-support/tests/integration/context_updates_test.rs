//! Exercise collection, ReasonAtom and durable message replay together.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_contracts::runtime::message::CONVERSATION_CONTEXT_UPDATE_METADATA_KEY as KEY;
use everruns_core::capabilities::{Capability, SystemPromptContext};
use everruns_core::events::EventData;
use everruns_core::message::RuntimeMessageRole;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_llmsim::{LlmSimConfig, SimToolCall, SimTurn};
use everruns_test_support::InMemoryAgenticLoop;
use serde_json::{Value, json};

struct MutableInventory(Arc<Mutex<Option<String>>>);

#[async_trait]
impl Capability for MutableInventory {
    fn id(&self) -> &str {
        "mutable_inventory"
    }
    fn name(&self) -> &str {
        "Mutable inventory"
    }
    fn description(&self) -> &str {
        "Fixture for mutable reference data."
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some("Static discovery instructions.")
    }
    async fn conversation_context_update_contribution(
        &self,
        _: &SystemPromptContext,
    ) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn inventory_changes_preserve_the_previous_request_prefix_through_event_replay() {
    let inventory = Arc::new(Mutex::new(None));
    let runner = InMemoryAgenticLoop::builder()
        .with_llm_sim(LlmSimConfig::scripted(
            (0..5).map(|_| SimTurn::Assistant("done".into())).collect(),
        ))
        .capability(MutableInventory(inventory.clone()))
        .max_iterations(5)
        .build()
        .await
        .unwrap();
    for current in [
        None,
        Some("Skill A"),
        Some("Skill B"),
        Some("Skill B"),
        None,
    ] {
        *inventory.lock().unwrap() = current.map(str::to_owned);
        assert!(runner.run_turn("Continue setup").await.unwrap().success);
    }
    let generations = runner.events_by_type("llm.generation").await;
    assert_eq!(generations.len(), 5);
    let mut previous = Vec::new();
    for event in generations {
        let EventData::LlmGeneration(generation) = event.data else {
            panic!("generation event")
        };
        let sent: Vec<_> = generation
            .messages
            .iter()
            .map(|m| (m.role.clone(), serde_json::to_value(&m.content).unwrap()))
            .collect();
        assert!(sent.starts_with(&previous));
        assert_eq!(
            sent.iter()
                .filter(|(role, _)| *role == RuntimeMessageRole::System)
                .count(),
            1
        );
        previous = sent;
    }
    let stored = runner.messages().await.unwrap();
    let snapshots: Vec<_> = stored
        .iter()
        .filter_map(|m| m.metadata.as_ref().and_then(|metadata| metadata.get(KEY)))
        .collect();
    assert_eq!(
        snapshots.len(),
        3,
        "install, edit and removal, without repeat on unchanged inventory"
    );
    assert!(snapshots[2]["content"].is_null());
    assert_eq!(
        previous
            .iter()
            .filter(|(_, content)| content.to_string().contains("<capability-context-update>"))
            .count(),
        3
    );
}

struct PublishInventory(Arc<Mutex<Option<String>>>);

#[async_trait]
impl Tool for PublishInventory {
    fn name(&self) -> &str {
        "publish_inventory"
    }
    fn description(&self) -> &str {
        "Fixture that changes the inventory during a turn."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    async fn execute(&self, _: Value) -> ToolExecutionResult {
        *self.0.lock().unwrap() = Some("Skill A".to_string());
        ToolExecutionResult::success(json!({"updated":true}))
    }
}

#[tokio::test]
async fn a_tool_side_inventory_change_is_visible_on_the_next_reason_step() {
    let inventory = Arc::new(Mutex::new(None));
    let runner = InMemoryAgenticLoop::builder()
        .with_llm_sim(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "publish_inventory".to_string(),
                arguments: json!({}),
                id: Some("call_publish".to_string()),
            }]),
            SimTurn::Assistant("Installed.".to_string()),
            SimTurn::Assistant("Still installed.".to_string()),
        ]))
        .capability(MutableInventory(inventory.clone()))
        .tool(PublishInventory(inventory))
        .max_iterations(5)
        .build()
        .await
        .unwrap();
    let turn = runner.run_turn("Update the catalog").await.unwrap();
    assert!(turn.success);
    assert_eq!(turn.tool_calls_count, 1);
    assert!(runner.run_turn("Continue").await.unwrap().success);

    let generations: Vec<_> = runner
        .events_by_type("llm.generation")
        .await
        .into_iter()
        .map(|event| {
            let EventData::LlmGeneration(data) = event.data else {
                panic!("generation event")
            };
            data
        })
        .collect();
    assert_eq!(generations.len(), 3);
    let middle = &generations[1].messages;
    assert_eq!(
        middle
            .iter()
            .filter(|m| m.role == RuntimeMessageRole::ToolResult)
            .count(),
        1
    );
    assert!(middle.last().unwrap().text().unwrap().contains("Skill A"));
    let content = |messages: &[everruns_core::RuntimeMessage]| -> Vec<_> {
        messages
            .iter()
            .map(|m| (m.role.clone(), serde_json::to_value(&m.content).unwrap()))
            .collect()
    };
    assert!(content(&generations[2].messages).starts_with(&content(middle)));
}
