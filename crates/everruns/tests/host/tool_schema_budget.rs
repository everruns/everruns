//! Exercise the complete embedded turn path with a recording provider boundary.
use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, LlmCallConfig, LlmResponseStream, LlmStreamEvent, Message,
};
use everruns_contracts::{error::Result, message::MessageRole, tool_types::ToolDefinition};
use everruns_core::host::{AgentBuilder, HarnessBuilder, HostComposition, SessionBuilder};
use everruns_core::{CapabilityRegistry, builtins::CompactionCapability};
use std::sync::{Arc, Mutex};

struct BudgetDriver(Arc<Mutex<Vec<Vec<Message>>>>);
#[async_trait]
impl ChatDriver for BudgetDriver {
    fn effective_context_window(&self, _: &str) -> Option<usize> {
        Some(4_000)
    }
    async fn chat_completion_stream(
        &self,
        _: &everruns_contracts::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        assert_eq!(config.tools.len(), 1);
        self.0.lock().unwrap().push(messages);
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(LlmStreamEvent::TextDelta("done".into())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

#[tokio::test]
async fn embedded_turn_reserves_tools_before_sending_history_to_provider() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(CompactionCapability);
    let harness_id = everruns_contracts::typed_id::HarnessId::from_seed(3481);
    let agent_id = everruns_contracts::typed_id::AgentId::from_seed(3481);
    let session_id = everruns_contracts::typed_id::SessionId::from_seed(3481);
    let runtime = everruns::batteries::runtime_builder()
        .host_composition(
            HostComposition::builder()
                .capability_registry(capabilities)
                .build(),
        )
        .provider_with_default_model(
            everruns_contracts::runtime_provider::Provider::new(
                "budget",
                BudgetDriver(requests.clone()),
            ),
            "budget-model",
        )
        .harness(
            HarnessBuilder::new("budget", "Follow the user's instructions.")
                .id(harness_id)
                .build(),
        )
        .agent(
            AgentBuilder::new("budget", "Answer concisely.")
                .id(agent_id)
                .tool(ToolDefinition::function(
                    "lookup",
                    "d".repeat(8_000),
                    serde_json::json!({"type":"object"}),
                ))
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .capability("compaction")
                .build(),
        )
        .build()
        .await
        .expect("runtime builds");
    for turn in 0..10 {
        runtime
            .run_text_turn(session_id, format!("turn {turn}: {}", "x".repeat(800)))
            .await
            .expect("turn succeeds");
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 10);
    let last = requests.last().unwrap();
    let users: Vec<_> = last
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .collect();
    println!(
        "provider request retained {} of 10 user turns with an 8,000-byte tool description",
        users.len()
    );
    assert!(
        users.len() < 10,
        "message-only budgeting would keep all ten turns"
    );
    assert!(
        users
            .iter()
            .any(|m| m.content_as_text().starts_with("turn 9:"))
    );
    assert!(last.iter().any(|m| m.role == MessageRole::System));
}
