//! The platform tools (`platform_discover`, `platform_query`,
//! `platform_execute`: Platform Chat) on the data plane (data plane v2,
//! step 11.6b).
//!
//! The command catalog is the control plane's, so a call is one MCP tool
//! call on its platform commands server (`injected::PLATFORM_COMMANDS`,
//! sent with the create) and runs as the person who sent the turn's input
//! message, as on a v1 worker. The tools, docs and builtin stay native.
//!
//! Decisions:
//! - THREAT[TM-AGENT-017]: the call's session token names the person from
//!   the session's own record (`senders.rs`), whose `act` is the turn's,
//!   and the control plane re-derives the session's `platform` capability
//!   and the person's membership and role before it runs anything.
//! - A call outside a turn (no input message bound) is refused here: there
//!   is no person to act as.
//! - A session without the server (a standalone shard) has no platform
//!   commands: the call is refused.
//! - The shell's `everruns` builtin forwards its line to the `execute`
//!   tool (bashkit's `ForwardingBuiltin`), so it runs here too, bound to
//!   the same turn.

use std::sync::Arc;

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::saved_scripts::SavedScripts;
use everruns_core::tool_context::{ExecutionServices, ExecutionServicesExt, ToolContext};
use serde_json::{Value, json};

/// One session's platform calls, bound to a turn's input message once the
/// turn runs.
#[derive(Clone)]
pub(crate) struct PlatformCalls {
    input: Option<everruns_contracts::typed_id::MessageId>,
    /// The control plane's commands server, when it serves one.
    server: Option<crate::mcp::ControllerServer>,
}

impl PlatformCalls {
    pub(crate) fn new(server: Option<crate::mcp::ControllerServer>) -> Self {
        Self {
            input: None,
            server,
        }
    }

    /// The same calls, for the turn of input message `input`.
    pub(crate) fn bound(&self, input: everruns_contracts::typed_id::MessageId) -> Self {
        Self {
            input: Some(input),
            ..self.clone()
        }
    }

    /// Run `operation` (`discover`, `query`, `execute` or `run`) with
    /// `arguments`; the command's output.
    pub(crate) async fn call(&self, operation: &str, arguments: Value) -> Result<String> {
        if self.input.is_none() {
            return Err(AgentLoopError::tool(
                "Platform commands run only within a turn a person started",
            ));
        }
        let Some(server) = &self.server else {
            return Err(AgentLoopError::tool(
                "Platform commands are not available on this data plane",
            ));
        };
        command(server, operation, arguments).await
    }
}

/// `operation` with `arguments` on the control plane's commands server.
async fn command(
    server: &crate::mcp::ControllerServer,
    operation: &str,
    arguments: Value,
) -> Result<String> {
    server
        .call_tool(
            everruns_contracts::runtime::capabilities::injected::PLATFORM_COMMAND_TOOL,
            json!({ "operation": operation, "arguments": arguments }),
        )
        .await
        .map_err(AgentLoopError::tool)
}

/// Binds the session's platform store, and its saved scripts, to each
/// turn's input message, as the v1 worker's `PlatformExecutionScope` does.
pub(crate) struct BindPlatformStore {
    pub(crate) store: Arc<dyn everruns_capabilities::PlatformStore>,
    pub(crate) scripts: Option<SavedScripts>,
}

impl ExecutionServices for BindPlatformStore {
    fn bind(&self, context: &mut ToolContext, id: uuid::Uuid) {
        if let Some(store) = self.store.for_execution(id) {
            context
                .extensions
                .insert(Arc::new(everruns_capabilities::PlatformStoreExt(store)));
        }
        if let Some(saved) = &self.scripts
            && let Some(store) = saved.store.for_execution(id)
        {
            context.extensions.insert(Arc::new(SavedScripts {
                store,
                can_save: saved.can_save,
            }));
        }
    }
}

/// `ExecutionServicesExt` binding `store` and `scripts`.
pub(crate) fn binding(
    store: Arc<dyn everruns_capabilities::PlatformStore>,
    scripts: Option<SavedScripts>,
) -> ExecutionServicesExt {
    ExecutionServicesExt(Arc::new(BindPlatformStore { store, scripts }))
}

#[cfg(test)]
#[path = "platform_calls_tests.rs"]
mod tests;
