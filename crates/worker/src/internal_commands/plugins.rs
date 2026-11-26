//! `PluginsInvoker` over the `worker_plugins_call` command.

use super::{InternalCommandTransport, is_bad_request};
use async_trait::async_trait;
use everruns_capabilities::capabilities::{PluginsCall, PluginsInvoker};
use everruns_contracts::typed_id::SessionId;
use serde_json::{Value, json};

/// The Plugins client of one session, on either transport. The org's plugin
/// list and the session's records stay in the control plane; only the call
/// and its answer cross.
pub struct CommandPluginsInvoker<T> {
    transport: T,
    session_id: SessionId,
}

impl<T: InternalCommandTransport> CommandPluginsInvoker<T> {
    pub fn new(transport: T, session_id: SessionId) -> Self {
        Self {
            transport,
            session_id,
        }
    }
}

#[async_trait]
impl<T: InternalCommandTransport> PluginsInvoker for CommandPluginsInvoker<T> {
    async fn invoke(&self, call: PluginsCall) -> Result<Value, String> {
        let params = json!({ "session_id": self.session_id.to_string(), "call": call });
        match self
            .transport
            .execute_internal_command("worker_plugins_call", params)
            .await
        {
            Ok(Ok(value)) => Ok(value),
            // Not approved, does not fit, not ready: for the model.
            Ok(Err(error)) if is_bad_request(&error) => Err(error.message),
            Ok(Err(error)) => {
                tracing::warn!(message = %error.message, "plugins call failed");
                Err("Plugins are unavailable right now.".to_string())
            }
            Err(error) => {
                tracing::warn!(%error, "plugins call did not reach the control plane");
                Err("Plugins are unavailable right now.".to_string())
            }
        }
    }
}
