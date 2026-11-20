//! The platform tools (`platform_discover`, `platform_query`,
//! `platform_execute`: Platform Chat) on the data plane (data plane v2,
//! step 11.6b).
//!
//! The command catalog is the control plane's, so a call goes there over
//! the lookups channel (`lookups.rs`) and runs as the person who sent the
//! turn's input message, as on a v1 worker.
//!
//! Decisions:
//! - THREAT[TM-AGENT-017]: a call names only the session and the turn's
//!   input message ([`PLATFORM_COMMAND_SURFACE`]'s params). The cell that
//!   forwards it adds the person from the session's own record
//!   (`senders.rs`, `gateway_dp.rs`), replacing anything the call carried,
//!   and the control plane re-derives the session's `platform` capability
//!   and the person's membership and role before it runs anything.
//! - A call outside a turn (no input message bound) is refused here: there
//!   is no person to act as.
//! - The shell's `everruns` builtin forwards its line to the `execute`
//!   tool (bashkit's `ForwardingBuiltin`), so it runs here too, bound to
//!   the same turn.

use std::sync::Arc;

/// The v1 command the `platform_command` host call answers.
const PLATFORM_COMMAND_SURFACE: &str = "platform_command_surface";
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::tool_context::{ExecutionServices, ExecutionServicesExt, ToolContext};
use everruns_internal_protocol::commands::InternalCommandTransport;
use serde_json::{Value, json};

use crate::lookups::OrgLookups;

/// One session's platform calls, bound to a turn's input message once the
/// turn runs.
#[derive(Clone)]
pub(crate) struct PlatformCalls {
    lookups: OrgLookups,
    session: SessionId,
    input: Option<MessageId>,
}

impl PlatformCalls {
    pub(crate) fn new(lookups: OrgLookups, session: SessionId) -> Self {
        Self {
            lookups,
            session,
            input: None,
        }
    }

    /// The same calls, for the turn of input message `input`.
    pub(crate) fn bound(&self, input: MessageId) -> Self {
        Self {
            input: Some(input),
            ..self.clone()
        }
    }

    /// Run `operation` (`discover`, `query`, `execute` or `run`) with
    /// `arguments`; the command's output.
    pub(crate) async fn call(&self, operation: &str, arguments: Value) -> Result<String> {
        let Some(input) = self.input else {
            return Err(AgentLoopError::tool(
                "Platform commands run only within a turn a person started",
            ));
        };
        let params = json!({
            "session_id": self.session.to_string(),
            "input_message_id": input.to_string(),
            "operation": operation,
            "arguments": arguments,
        });
        match self
            .lookups
            .execute_internal_command(PLATFORM_COMMAND_SURFACE, params)
            .await?
        {
            Ok(Value::String(output)) => Ok(output),
            Ok(other) => Ok(other.to_string()),
            Err(error) => Err(AgentLoopError::tool(error.message)),
        }
    }
}

impl PlatformCalls {
    /// Ask the control plane for a record for a new child of this session
    /// (the `subagent_record` host call, `subagents.rs`): the child's id and
    /// its Memory mounts. Needs no turn: the child acts as no one.
    pub(crate) async fn record_subagent(&self, params: Value) -> Result<(SessionId, Vec<String>)> {
        let answer = self
            .lookups
            .execute_internal_command(everruns_contracts::host_calls::SUBAGENT_RECORD, params)
            .await?
            .map_err(|error| AgentLoopError::store(error.message))?;
        let id = answer["session_id"]
            .as_str()
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| AgentLoopError::store("the control plane named no child session"))?;
        let memory = serde_json::from_value(answer["memory"].clone()).unwrap_or_default();
        Ok((id, memory))
    }
}

/// Binds the session's platform store to each turn's input message, as the
/// v1 worker's `PlatformExecutionScope` does.
pub(crate) struct BindPlatformStore(pub(crate) Arc<dyn everruns_capabilities::PlatformStore>);

impl ExecutionServices for BindPlatformStore {
    fn bind(&self, context: &mut ToolContext, id: uuid::Uuid) {
        if let Some(store) = self.0.for_execution(id) {
            context
                .extensions
                .insert(Arc::new(everruns_capabilities::PlatformStoreExt(store)));
        }
    }
}

/// `ExecutionServicesExt` binding `store`.
pub(crate) fn binding(
    store: Arc<dyn everruns_capabilities::PlatformStore>,
) -> ExecutionServicesExt {
    ExecutionServicesExt(Arc::new(BindPlatformStore(store)))
}
