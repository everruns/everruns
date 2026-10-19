//! Isolate-local authority bridge. Storage and alarms remain owned by the object.
use crate::{AuthorityClient, AuthorityTransport};
use async_trait::async_trait;
use everruns::hosting::spi::actor::actor_lease_lost;
use everruns::hosting::spi::{
    AssertSend,
    error::{AgentLoopError, Result},
    ids::SessionId,
};
use js_sys::{Function, Promise};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

/// Resolve the receipt namespace and normalize IDs with the same typed command.
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn session_command_key(command: &str) -> String {
    use everruns::hosting::spi::session_commands::SessionCommand;
    match serde_json::from_str::<SessionCommand>(command) {
        Ok(command) => json!({"key":command.key(),"command":command}).to_string(),
        Err(_) => json!({"error":"invalid session command","status":400}).to_string(),
    }
}

/// The same Rust reducer runs synchronously inside the object transaction.
/// Only the object constructs the view; callers send semantic commands.
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn session_command(command: &str, view: &str) -> String {
    use everruns::hosting::spi::session_commands::{
        self, CommandError, CommandView, SessionCommand,
    };
    let result = (|| {
        let command: SessionCommand = serde_json::from_str(command)
            .map_err(|_| AgentLoopError::config("invalid session command"))?;
        let view: CommandView = serde_json::from_str(view)
            .map_err(|_| AgentLoopError::config("invalid authority view"))?;
        session_commands::prepare(command, view)
    })();
    match result {
        Ok(plan) => json!({"plan":plan}).to_string(),
        Err(error) => {
            let status = match &error {
                CommandError::Invalid(AgentLoopError::SessionNotFound(_)) => 404,
                CommandError::Conflict(_) => 409,
                CommandError::Capacity(_) => 429,
                CommandError::TooLarge(_) => 413,
                _ => 400,
            };
            json!({"error":error.to_string(),"status":status}).to_string()
        }
    }
}

/// The callback accepts one JSON RPC string and returns a JSON envelope Promise.
/// Only the embedding object holds this function; it is not an agent tool.
pub struct ObjectTransport(AssertSend<Function>);
impl ObjectTransport {
    pub fn client(tenant: &str, callback: Function) -> Result<AuthorityClient> {
        AuthorityClient::with_transport(tenant, Arc::new(Self(AssertSend(callback))))
    }
}

/// Call a trusted isolate port without making JavaScript values cross threads.
pub async fn invoke(callback: &Function, request: Value) -> Result<Value> {
    let encoded = serde_json::to_string(&request)
        .map_err(|_| AgentLoopError::store("invalid isolate request"))?;
    if encoded.len() > 1024 * 1024 {
        return Err(AgentLoopError::store("isolate request exceeds limit"));
    }
    let callback = callback.clone();
    AssertSend(async move {
        let result = callback
            .call1(&JsValue::UNDEFINED, &JsValue::from_str(&encoded))
            .map_err(|_| AgentLoopError::store("isolate port failed"))?;
        let result = JsFuture::from(Promise::resolve(&result))
            .await
            .map_err(|_| AgentLoopError::store("isolate port unavailable"))?;
        let encoded = result
            .as_string()
            .ok_or_else(|| AgentLoopError::store("invalid isolate reply"))?;
        if encoded.len() > 4 * 1024 * 1024 {
            return Err(AgentLoopError::store("isolate reply exceeds limit"));
        }
        serde_json::from_str(&encoded).map_err(|_| AgentLoopError::store("invalid isolate reply"))
    })
    .await
}

#[async_trait]
impl AuthorityTransport for ObjectTransport {
    async fn invoke(
        &self,
        tenant: &str,
        session: SessionId,
        operation: &str,
        body: Value,
    ) -> Result<Value> {
        #[derive(Deserialize)]
        struct Reply {
            status: u16,
            value: Value,
        }
        let reply = invoke(
            &self.0.0,
            json!({"tenant":tenant,"session":session,"operation":operation,"body":body}),
        )
        .await?;
        let reply: Reply = serde_json::from_value(reply)
            .map_err(|_| AgentLoopError::store("invalid authority reply"))?;
        if reply.status == 412 {
            return Err(actor_lease_lost(session, "object rejected stale ownership"));
        }
        if !(200..300).contains(&reply.status) {
            return Err(crate::client::rejected(reply.status));
        }
        Ok(reply.value)
    }
}
