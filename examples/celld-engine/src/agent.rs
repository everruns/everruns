//! The demo agent: a harness, one capability and its one tool.

use std::time::Duration;

use async_trait::async_trait;
use everruns_capability::CapabilityRef;
use everruns_core::capabilities::Capability;
use everruns_core::tools::{Tool, ToolExecutionResult, ToolRegistry};
use everruns_core::{CapabilityRegistry, HarnessDefinition};
use serde_json::{Value, json};

pub const CAPABILITY_ID: &str = "celld_notes";

pub fn harness() -> HarnessDefinition {
    let mut harness = HarnessDefinition::new(
        "celld-cell",
        "You answer questions about celld. Call look_up before answering.",
    );
    harness.capabilities = vec![CapabilityRef::new(CAPABILITY_ID)];
    harness
}

pub fn capabilities(delay: Duration) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register(Notes { delay });
    registry
}

pub fn tools(delay: Duration) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(LookUp { delay });
    registry
}

struct Notes {
    delay: Duration,
}

#[async_trait]
impl Capability for Notes {
    fn id(&self) -> &str {
        CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "celld notes"
    }

    fn description(&self) -> &str {
        "Looks up short notes about celld."
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(LookUp { delay: self.delay })]
    }
}

/// A slow, deterministic tool: the delay leaves room to lose the node while
/// the act step runs.
struct LookUp {
    delay: Duration,
}

#[async_trait]
impl Tool for LookUp {
    fn name(&self) -> &str {
        "look_up"
    }

    fn description(&self) -> &str {
        "Look up a note about a celld topic."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "topic": { "type": "string" } },
            "required": ["topic"],
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        everruns_provider::rt::sleep(self.delay).await;
        let topic = arguments["topic"].as_str().unwrap_or("celld");
        ToolExecutionResult::success(json!({
            "topic": topic,
            "note": "celld runs Durable Objects on your own machines.",
        }))
    }
}
