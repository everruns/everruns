//! Real-model agent inside an isolate, using the ordinary core host and shared actor driver.
#![cfg(target_arch = "wasm32")]
use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream, LlmStreamEvent, Message,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::rt::AssertSend;
use everruns_contracts::runtime_provider::{Provider, ProviderEndpoint};
use everruns_core::host::InProcessRuntime;
use everruns_core::{Capability, Tool, ToolContext, ToolExecutionResult};
use everruns_durable_object::{
    SessionBinding, activate,
    wasm::{ObjectTransport, invoke},
};
use js_sys::Function;
use serde_json::{Value, json};
use std::sync::Arc;
use wasm_bindgen::prelude::*;

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
    let driver = Arc::new(FetchDriver(AssertSend(provider)));
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

// A fetch transport implements the normal provider SPI. Wire conversion is
// supplied by the shared contracts; agent reasoning and tools run in core.
struct FetchDriver(AssertSend<Function>);
#[async_trait]
impl ChatDriver for FetchDriver {
    async fn chat_completion_stream(
        &self,
        _: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let mut body = json!({"model":config.model,"messages":messages.iter().map(everruns_contracts::openai_wire::message_to_openai).collect::<Vec<_>>(),"stream":false});
        if !config.tools.is_empty() {
            body["tools"] = json!(
                config
                    .tools
                    .iter()
                    .map(everruns_contracts::openai_wire::tool_to_openai)
                    .collect::<Vec<_>>()
            );
        }
        if let Some(value) = config.temperature {
            body["temperature"] = json!(value);
        }
        if let Some(value) = config.max_tokens {
            body["max_completion_tokens"] = json!(value);
        }
        let reply = invoke(&self.0.0, body).await?;
        let status = reply["status"].as_u64().unwrap_or(503) as u16;
        if !(200..300).contains(&status) {
            return Err(AgentLoopError::llm_http(
                status,
                "",
                "isolate provider rejected request",
            ));
        }
        let reply = &reply["body"];
        let message =
            everruns_contracts::openai_wire::message_from_openai(&reply["choices"][0]["message"])
                .map_err(|_| AgentLoopError::llm("invalid provider response"))?;
        let mut events = Vec::new();
        let text = message.content.to_text();
        if !text.is_empty() {
            events.push(LlmStreamEvent::TextDelta(text));
        }
        if let Some(calls) = message.tool_calls.filter(|v| !v.is_empty()) {
            events.push(LlmStreamEvent::ToolCalls(calls));
        }
        let usage = &reply["usage"];
        let mut metadata = LlmCompletionMetadata::default();
        metadata.prompt_tokens = usage["prompt_tokens"]
            .as_u64()
            .and_then(|v| v.try_into().ok());
        metadata.completion_tokens = usage["completion_tokens"]
            .as_u64()
            .and_then(|v| v.try_into().ok());
        metadata.total_tokens = usage["total_tokens"]
            .as_u64()
            .and_then(|v| v.try_into().ok());
        metadata.response_id = reply["id"].as_str().map(str::to_owned);
        metadata.model = Some(config.model.clone());
        metadata.response_model = reply["model"].as_str().map(str::to_owned);
        events.push(LlmStreamEvent::Done(Box::new(metadata)));
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }
}
