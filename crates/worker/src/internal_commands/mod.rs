//! Runtime stores served by the control plane's internal domain commands.
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
use everruns_internal_protocol::proto;
use serde_json::Value;

mod session_schedules;

pub use session_schedules::CommandSessionScheduleStore;

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

fn is_bad_request(error: &proto::CommandError) -> bool {
    proto::command_error::Kind::try_from(error.kind) == Ok(proto::command_error::Kind::BadRequest)
}

fn failure(operation: &str, error: proto::CommandError) -> AgentLoopError {
    AgentLoopError::store(format!("{operation}: {}", error.message))
}

fn decode<T: serde::de::DeserializeOwned>(operation: &str, value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| {
        AgentLoopError::store(format!("{operation} returned unexpected shape: {error}"))
    })
}
