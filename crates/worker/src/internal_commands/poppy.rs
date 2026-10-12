//! `PoppyInvoker` over the `worker_poppy_call` command.

use super::{InternalCommandTransport, is_bad_request};
use async_trait::async_trait;
use everruns_capabilities::capabilities::{PoppyCall, PoppyInvoker};
use everruns_contracts::typed_id::SessionId;
use serde_json::{Value, json};

/// The Poppy client of one session, on either transport. The keys and
/// Session Tokens stay in the control plane; only the call and its answer
/// cross.
pub struct CommandPoppyInvoker<T> {
    transport: T,
    session_id: SessionId,
}

impl<T: InternalCommandTransport> CommandPoppyInvoker<T> {
    pub fn new(transport: T, session_id: SessionId) -> Self {
        Self {
            transport,
            session_id,
        }
    }
}

#[async_trait]
impl<T: InternalCommandTransport> PoppyInvoker for CommandPoppyInvoker<T> {
    async fn invoke(&self, call: PoppyCall) -> Result<Value, String> {
        let params = json!({ "session_id": self.session_id.to_string(), "call": call });
        match self
            .transport
            .execute_internal_command("worker_poppy_call", params)
            .await
        {
            Ok(Ok(value)) => Ok(value),
            // A company's refusal or an unreachable company: for the model.
            Ok(Err(error)) if is_bad_request(&error) => Err(error.message),
            Ok(Err(error)) => {
                tracing::warn!(message = %error.message, "Poppy call failed");
                Err("Poppy is unavailable right now.".to_string())
            }
            Err(error) => {
                tracing::warn!(%error, "Poppy call did not reach the control plane");
                Err("Poppy is unavailable right now.".to_string())
            }
        }
    }
}
