//! Runtime stores served by the control plane's internal domain commands.
//!
//! Shared by the v1 worker (`everruns_worker::internal_commands`) and the
//! data plane's executors (`everruns-cell`, step 11.6b), which reach the same
//! commands over the data plane link.
//!
//! Decision: a worker operation that is neither hot nor streaming is a domain
//! command on the server (`INTERNAL_COMMAND_PATH_PREFIX` keeps it off public
//! surfaces), and the runtime-facing store for it is written once, here, over
//! [`InternalCommandTransport`]. A gRPC worker carries the call over
//! `ExecuteCommand`; the server's in-process worker dispatches it directly.
//! Neither side keeps per-operation code: no bespoke RPC, no handler, no
//! direct adapter. Hot and streaming operations (event emission, turn context,
//! durable task claims and heartbeats) keep their dedicated RPCs.
//!
//! Failures cross as `proto::CommandError` on both transports, so a store maps
//! one vocabulary whichever way it was reached.

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use serde_json::Value;

use crate::proto;

mod connections;
pub mod knowledge;
mod org_egress_allowlist;

pub use connections::CommandConnectionResolver;
pub use org_egress_allowlist::CommandOrgEgressAllowlist;

/// Runs one internal domain command as the organization's internal caller.
///
/// The outer `Result` is the transport failing; the inner one is the command's
/// own answer.
#[async_trait]
pub trait InternalCommandTransport: Send + Sync {
    async fn execute_internal_command(
        &self,
        name: &str,
        params: Value,
    ) -> Result<std::result::Result<Value, proto::CommandError>>;
}

pub fn is_bad_request(error: &proto::CommandError) -> bool {
    proto::command_error::Kind::try_from(error.kind) == Ok(proto::command_error::Kind::BadRequest)
}

pub fn failure(operation: &str, error: proto::CommandError) -> AgentLoopError {
    AgentLoopError::store(format!("{operation}: {}", error.message))
}

pub fn decode<T: serde::de::DeserializeOwned>(operation: &str, value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| {
        AgentLoopError::store(format!("{operation} returned unexpected shape: {error}"))
    })
}

/// [`call`] for commands whose answer carries a credential or secret value.
///
/// A shape mismatch reports only that the shape was wrong: serde's message
/// quotes the offending value, which here would be the secret itself.
pub async fn call_secret<R: serde::de::DeserializeOwned>(
    transport: &(impl InternalCommandTransport + ?Sized),
    operation: &str,
    name: &str,
    params: Value,
) -> Result<R> {
    match transport.execute_internal_command(name, params).await? {
        Ok(value) => serde_json::from_value(value)
            .map_err(|_| AgentLoopError::store(format!("{operation} returned unexpected shape"))),
        Err(error) => Err(failure(operation, error)),
    }
}

/// Run `name` and decode its answer, naming `operation` in any failure.
pub async fn call<R: serde::de::DeserializeOwned>(
    transport: &(impl InternalCommandTransport + ?Sized),
    operation: &str,
    name: &str,
    params: Value,
) -> Result<R> {
    match transport.execute_internal_command(name, params).await? {
        Ok(value) => decode(operation, value),
        Err(error) => Err(failure(operation, error)),
    }
}
