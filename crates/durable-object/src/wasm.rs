//! Isolate-local authority bridge. Storage and alarms remain owned by the object.
use crate::{AuthorityClient, AuthorityTransport};
use async_trait::async_trait;
use everruns_contracts::{
    error::{AgentLoopError, Result},
    rt::AssertSend,
    typed_id::SessionId,
};
use everruns_core::host::session_actor::store::actor_lease_lost;
use js_sys::{Function, Promise};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

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
