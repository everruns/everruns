//! Framework Engine placement inside a Cloudflare or celld isolate.
#![cfg(target_arch = "wasm32")]
use everruns_contracts::rt::AssertSend;
use everruns_contracts::runtime_provider::Provider;
use everruns_contracts::{ChatDriver, OpenAIProtocolChatDriver};
use everruns_durable_object::wasm::ObjectTransport;
use js_sys::Function;
use serde_json::json;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

#[path = "../../../crates/durable-object/examples/shared/agent.rs"]
mod agent;
mod fetch;

const BUILD: &str = match option_env!("AGENT_BUILD_ID") {
    Some(id) => id,
    None => "counter-wasm-v1",
};
#[wasm_bindgen]
pub fn binding() -> String {
    json!({"agent":"counter","build_id":BUILD,"framework":1}).to_string()
}

fn engine(
    tenant: &str,
    authority: Function,
    provider: Function,
    model: String,
) -> Result<everruns::Engine, JsValue> {
    let client = ObjectTransport::client(tenant, authority)
        .map_err(|_| JsValue::from_str("invalid object bridge"))?;
    let driver: Arc<dyn ChatDriver> = Arc::from(
        Provider::new(
            "fetch",
            OpenAIProtocolChatDriver::new()
                .with_transport(Arc::new(fetch::FetchTransport(AssertSend(provider)))),
        )
        .base_url("https://isolate.invalid/v1")
        .into_boxed_driver(),
    );
    let agent = agent::counter(everruns::Model::new(
        model,
        Provider::from_driver("fetch", driver),
    ))
    .map_err(|_| JsValue::from_str("invalid agent definition"))?;
    Ok(everruns::Engine::builder()
        .hosting(everruns_durable_object::ObjectHosting::new(client))
        .register(everruns::hosting::AgentBuild::new("counter", BUILD), agent)
        .build())
}

#[wasm_bindgen]
pub async fn run(
    tenant: String,
    session: String,
    authority: Function,
    provider: Function,
    model: String,
) -> Result<(), JsValue> {
    let session = session
        .parse()
        .map_err(|_| JsValue::from_str("invalid session"))?;
    engine(&tenant, authority, provider, model)?
        .activate(session)
        .await
        .map_err(|_| JsValue::from_str("agent activation failed"))
}

#[wasm_bindgen]
pub async fn request(
    tenant: String,
    session: String,
    authority: Function,
    provider: Function,
    model: String,
    operation: String,
    body: String,
) -> Result<String, JsValue> {
    let session = session
        .parse()
        .map_err(|_| JsValue::from_str("invalid session"))?;
    let body =
        serde_json::from_str(&body).map_err(|_| JsValue::from_str("invalid session command"))?;
    let api = everruns_durable_object::SessionApi::new(
        engine(&tenant, authority, provider, model)?,
        everruns::hosting::AgentBuild::new("counter", BUILD),
    );
    Ok(match api.handle(session,&operation,body).await {
        Ok(value) => json!({"status":200,"value":value}),
        Err(error) => json!({"status":everruns_durable_object::SessionApi::error_status(&error),"value":{"error":"session operation failed"}}),
    }.to_string())
}
