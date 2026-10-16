//! Real-model agent inside an isolate, using the ordinary core host and shared actor driver.
#![cfg(target_arch = "wasm32")]
use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::rt::AssertSend;
use everruns_contracts::runtime_provider::Provider;
use everruns_contracts::{ChatDriver, OpenAIProtocolChatDriver};
use everruns_core::host::InProcessRuntime;
use everruns_core::{Capability, Tool, ToolContext, ToolExecutionResult};
use everruns_durable_object::{SessionBinding, activate, wasm::ObjectTransport};
use js_sys::Function;
use serde_json::{Value, json};
use std::sync::Arc;
use wasm_bindgen::prelude::*;

mod fetch;

const BUILD: &str = match option_env!("AGENT_BUILD_ID") {
    Some(id) => id,
    None => "counter-wasm-v1",
};
#[wasm_bindgen]
pub fn binding() -> String {
    json!({"agent":"counter","build_id":BUILD}).to_string()
}

#[wasm_bindgen]
pub async fn run(
    tenant: String,
    session: String,
    authority: Function,
    provider: Function,
    model: String,
) -> std::result::Result<(), JsValue> {
    let client = ObjectTransport::client(&tenant, authority)
        .map_err(|_| JsValue::from_str("invalid object bridge"))?;
    let session = session
        .parse()
        .map_err(|_| JsValue::from_str("invalid session"))?;
    let driver: Arc<dyn ChatDriver> = Arc::from(
        Provider::new(
            "fetch",
            OpenAIProtocolChatDriver::new()
                .with_transport(Arc::new(fetch::FetchTransport(AssertSend(provider)))),
        )
        .base_url("https://isolate.invalid/v1")
        .into_boxed_driver(),
    );
    let factory = move |id, backends| {
        let driver = driver.clone();
        let model = model.clone();
        async move {
            InProcessRuntime::builder()
                .single_session(|s| {
                    s.session_id(id)
                        .agent(
                            "counter",
                            "Use bump_counter when asked to increment. Report its result briefly.",
                        )
                        .with_capability("counter")
                })
                .capability(Counter)
                .provider_with_default_model(Provider::from_driver("fetch", driver), model)
                .backends(backends)
                .build()
                .await
        }
    };
    activate(
        client,
        &SessionBinding {
            agent: "counter".into(),
            build_id: BUILD.into(),
        },
        session,
        Arc::new(factory),
    )
    .await
    .map_err(|_| JsValue::from_str("agent activation failed"))
}

struct Counter;
#[async_trait]
impl Capability for Counter {
    fn id(&self) -> &str {
        "counter"
    }
    fn name(&self) -> &str {
        "Counter"
    }
    fn description(&self) -> &str {
        "A session-owned durable counter."
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(Bump)]
    }
}
struct Bump;
#[async_trait]
impl Tool for Bump {
    fn name(&self) -> &str {
        "bump_counter"
    }
    fn description(&self) -> &str {
        "Increment the session counter once."
    }
    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        // Serialize this read-modify-write with other session KV mutations.
        everruns_contracts::tool_types::ToolHints::default()
            .with_readonly(false)
            .with_idempotent(false)
            .with_concurrency_class("session_storage")
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    async fn execute(&self, _: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("session context is required")
    }
    async fn execute_with_context(&self, _: Value, context: &ToolContext) -> ToolExecutionResult {
        let Some(store) = &context.storage_store else {
            return ToolExecutionResult::tool_error("durable session store is required");
        };
        let result: Result<u64> = async {
            let current = store
                .get_value(context.session_id, "counter")
                .await?
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let next = current
                .checked_add(1)
                .ok_or_else(|| AgentLoopError::tool("counter overflow"))?;
            store
                .set_value(context.session_id, "counter", &next.to_string())
                .await?;
            Ok(next)
        }
        .await;
        match result {
            Ok(value) => ToolExecutionResult::success(json!({"counter":value})),
            Err(_) => ToolExecutionResult::tool_error("counter store unavailable"),
        }
    }
}
