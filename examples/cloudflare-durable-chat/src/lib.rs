//! Ordinary Framework Engine placement inside a native Cloudflare Durable Object.
#![cfg(target_arch = "wasm32")]
use everruns::{
    ChatDriver, Engine, Model, Provider,
    hosting::{
        AgentBuild, SessionHosting,
        spi::{
            AssertSend,
            error::{AgentLoopError, Result as HostingResult},
        },
    },
};
use everruns_drivers::openrouter::OpenRouterChatDriver;
use everruns_durable_object::{
    ObjectHosting, SessionApi,
    wasm::{ObjectTransport, invoke},
};
use js_sys::Function;
use serde_json::{Value, json};
use std::sync::Arc;
use wasm_bindgen::prelude::*;

mod agent;
mod fetch;

const BUILD: &str = match option_env!("AGENT_BUILD_ID") {
    Some(id) => id,
    None => "chat-development",
};

#[wasm_bindgen]
pub fn binding() -> String {
    json!({"build_id":BUILD,"framework":1}).to_string()
}

fn engine(
    tenant: &str,
    authority: Function,
    provider: Function,
    tools: Function,
    model: String,
) -> Result<(Engine, ObjectHosting), JsValue> {
    let client = ObjectTransport::client(tenant, authority).map_err(|_| invalid())?;
    let hosting = ObjectHosting::new(client);
    let driver: Arc<dyn ChatDriver> = Arc::from(
        Provider::new(
            "openrouter",
            OpenRouterChatDriver::new()
                .with_transport(Arc::new(fetch::FetchTransport(AssertSend(provider)))),
        )
        .base_url("https://openrouter.ai/api/v1")
        .into_boxed_driver(),
    );
    let callback = Arc::new(AssertSend(tools));
    let handler: agent::Handler = Arc::new(move |context, args| {
        let callback = callback.clone();
        Box::pin(async move {
            let key = context
                .idempotency_key()
                .ok_or("execution identity missing")?;
            invoke(&callback.0, json!({"session":context.session_id(),"key":key,"name":context.tool_name(),"args":args})).await
                .map_err(|_| "room tool unavailable".to_string())
        })
    });
    let model = Model::new(model, Provider::from_driver("openrouter", driver));
    let engine = Engine::builder()
        .hosting(hosting.clone())
        .register(
            AgentBuild::new("durable-chat", BUILD),
            agent::chat(model.clone(), false, handler.clone()).map_err(|_| invalid())?,
        )
        .register(
            AgentBuild::new("history-search", BUILD),
            agent::chat(model, true, handler).map_err(|_| invalid())?,
        )
        .build();
    Ok((engine, hosting))
}

fn invalid() -> JsValue {
    JsValue::from_str("invalid chat hosting configuration")
}

async fn wait_for_admission(
    engine: &Engine,
    session: everruns::SessionId,
    body: &Value,
) -> HostingResult<Value> {
    use everruns_contracts::rt::{Duration, timeout};
    let message = serde_json::from_value(body["message_id"].clone())
        .map_err(|_| AgentLoopError::config("stable admission identity required"))?;
    let handle = engine
        .resume(session)
        .await
        .map_err(|_| AgentLoopError::store("session unavailable"))?;
    let admission = handle
        .admission(message)
        .await
        .map_err(|_| AgentLoopError::store("admission unavailable"))?
        .ok_or_else(|| AgentLoopError::config("admission not found"))?;
    let turn = timeout(Duration::from_secs(90), admission.wait())
        .await
        .map_err(|_| AgentLoopError::store("admission wait timed out"))?
        .map_err(|_| AgentLoopError::store("admission did not complete"))?;
    Ok(json!(turn))
}

// Coalesce read-only queries inside the session DO. A room snapshot stays below
// Cloudflare's outbound subrequest budget even with many conversations.
async fn view(api: &SessionApi, session: everruns::SessionId) -> HostingResult<Value> {
    let status = api.handle(session, "status", json!({})).await?;
    let mut events = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let page = api
            .handle(
                session,
                "events",
                json!({"after":0,"limit":128,"cursor":cursor}),
            )
            .await?;
        events.extend(page["events"].as_array().into_iter().flatten().cloned());
        if events.len() > 4096 {
            return Err(AgentLoopError::store("demo history limit reached"));
        }
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    Ok(json!({"status":status,"events":events}))
}

#[wasm_bindgen]
pub async fn run(
    tenant: String,
    session: String,
    authority: Function,
    provider: Function,
    tools: Function,
    model: String,
) -> Result<(), JsValue> {
    engine(&tenant, authority, provider, tools, model)?
        .0
        .activate(session.parse().map_err(|_| invalid())?)
        .await
        .map_err(|_| JsValue::from_str("chat activation failed"))
}

#[wasm_bindgen]
pub async fn request(
    session: String,
    authority: Function,
    provider: Function,
    tools: Function,
    configuration: String,
    operation: String,
    body: String,
) -> Result<String, JsValue> {
    let configuration: Value = serde_json::from_str(&configuration).map_err(|_| invalid())?;
    let tenant = configuration["tenant"].as_str().ok_or_else(invalid)?;
    let model = configuration["model"]
        .as_str()
        .ok_or_else(invalid)?
        .to_string();
    let kind = configuration["kind"].as_str().ok_or_else(invalid)?;
    let session = session.parse().map_err(|_| invalid())?;
    let body: Value = serde_json::from_str(&body).map_err(|_| invalid())?;
    let (engine, hosting) = engine(tenant, authority, provider, tools, model)?;
    let result = if operation == "seed" {
        let command = serde_json::from_value(body["command_id"].clone()).map_err(|_| invalid())?;
        let messages = serde_json::from_value(body["messages"].clone()).map_err(|_| invalid())?;
        hosting
            .seed_history(session, command, messages)
            .await
            .map(|seeded| json!({"seeded":seeded}))
    } else if operation == "wait" {
        wait_for_admission(&engine, session, &body).await
    } else {
        let api = SessionApi::new(engine, AgentBuild::new(kind, BUILD));
        if operation == "view" {
            view(&api, session).await
        } else {
            api.handle(session, &operation, body).await
        }
    };
    Ok(match result {
        Ok(value) => json!({"status":200,"value":value}),
        Err(error) => json!({"status":SessionApi::error_status(&error),"value":{"error":"session operation failed"}}),
    }.to_string())
}
