//! Portable delegation responses at the server's authorized command edge.
//!
//! Management commands retain their record-shaped responses. Runtime callers
//! opt into this projection after the same policies authorize each operation.

use crate::domains::agents::record::Agent;
use crate::domains::common::{CommandError, Ctx, dispatch};
use crate::domains::harnesses::record::Harness;
use crate::domains::sessions::record::{Session, SessionParticipant};
use everruns_contracts::typed_id::HarnessId;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashSet;

fn decode<T: DeserializeOwned>(json: &str) -> Result<T, CommandError> {
    serde_json::from_str(json).map_err(|error| anyhow::Error::from(error).into())
}

fn encode<T: Serialize>(value: &T) -> Result<String, CommandError> {
    serde_json::to_string(value).map_err(|error| anyhow::Error::from(error).into())
}

pub(crate) async fn dispatch_runtime_view(
    name: &str,
    params: Value,
    ctx: &Ctx,
) -> Result<String, CommandError> {
    // Reject unsupported projections before dispatch: a malformed internal
    // request must never execute an unrelated mutating command.
    if !matches!(
        name,
        "get_agent"
            | "get_agent_harness"
            | "get_harness"
            | "create_session"
            | "get_session"
            | "add_session_participant"
            | "archive_session"
            | "unarchive_session"
    ) {
        return Err(CommandError::bad_request(
            "Command has no runtime response view",
        ));
    }
    // `get_agent_harness` is a projection of `get_agent`, under its policy.
    let command = if name == "get_agent_harness" {
        "get_agent"
    } else {
        name
    };
    let output = dispatch(command, params, ctx).await?;
    match name {
        "get_agent" => {
            // Spawn/handoff validation historically sees every lifecycle state;
            // the execution-loading seam separately validates before running.
            encode(&decode::<Agent>(&output)?.definition())
        }
        "get_harness" => {
            let leaf: Harness = decode(&output)?;
            let id: HarnessId = leaf.id;
            let mut next = leaf.parent_harness_id;
            let mut chain = vec![leaf];
            let mut seen = HashSet::from([id]);
            while let Some(parent_id) = next {
                if !seen.insert(parent_id) {
                    return Err(CommandError::bad_request(format!(
                        "Harness inheritance cycle detected at {parent_id}"
                    )));
                }
                // THREAT[TM-AGENT-017][TM-TENANT-001]: inherited records are looked up through the
                // same policy and tenant scope, never directly through storage.
                let output =
                    dispatch("get_harness", json!({"id": parent_id.to_string()}), ctx).await?;
                let parent: Harness = decode(&output)?;
                next = parent.parent_harness_id;
                chain.push(parent);
            }
            chain.reverse();
            let definition =
                crate::domains::harnesses::record::resolve_execution_harness(&chain, id)
                    .map_err(|error| CommandError::bad_request(error.to_string()))?;
            encode(&definition)
        }
        "create_session" | "get_session" => {
            encode(&decode::<Session>(&output)?.execution_session())
        }
        "get_agent_harness" => encode(&decode::<Agent>(&output)?.harness_id),
        "add_session_participant" => encode(&decode::<SessionParticipant>(&output)?.id),
        // Coordinators resolve and reopen threads; the result is a bool.
        "archive_session" | "unarchive_session" => Ok(output),
        _ => unreachable!("validated runtime projection"),
    }
}
