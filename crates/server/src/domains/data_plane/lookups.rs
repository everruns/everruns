//! `POST /dp/v1/tenants/{org}/lookups`: what a tool on the data plane asks
//! the control plane at call time (data plane v2, step 11.6b).
//!
//! A hosted capability on a cell needs the same org answers it gets on a v1
//! worker: a session's connection token, a GitHub token for one use, the
//! agent's service API key, the org's web search settings and account keys,
//! the tool approval policy, the org's knowledge, a Poppy call, the org's
//! egress grants. The v1 worker asks through internal commands; an executor
//! asks the same commands over the link, and its cell forwards the request
//! here.
//!
//! Decisions:
//! - Only [`LOOKUP_COMMANDS`] run here, as the org's internal caller with
//!   the org's feature flags: the same caller, checks and failures a v1
//!   worker's `ExecuteCommand` gets (a session of another org is not
//!   found).
//! - Every answer, a refusal included, is sealed with the org's data key
//!   as the request's own secret (`lookup:{request}`, revision 0): the
//!   cells that pass it on hold ciphertext only, and an answer cannot pass
//!   for another request's.
//! - Nothing is stored here. The commands are reads, except a Poppy call,
//!   which keeps the session owner's company accounts and conversations in
//!   the control plane, as on v1.

use everruns_contracts::dataplane::SealedSecret;
use everruns_contracts::dataplane::{LOOKUP_COMMANDS, LookupAnswer, LookupRequest, lookup_secret};
use everruns_contracts::dataplane_seal as seal;

use super::{ControlPlane, org_public};
use crate::domains::common::Ctx;
use crate::storage::DpOrgRow;

/// Whether `command` is one a data plane may ask.
pub fn allowed(command: &str) -> bool {
    LOOKUP_COMMANDS.contains(&command)
}

/// Run `request` as `ctx` (the org's internal caller) and seal its answer
/// for `org`.
pub async fn answer(
    cp: &ControlPlane,
    org: &DpOrgRow,
    ctx: &Ctx,
    request: LookupRequest,
) -> anyhow::Result<SealedSecret> {
    let answer = match crate::domains::common::dispatch(&request.command, request.params, ctx).await
    {
        Ok(json) => LookupAnswer::Value(serde_json::from_str(&json)?),
        Err(error) => {
            let error = crate::worker_link::grpc_service::command_error_to_proto(error);
            LookupAnswer::Error {
                kind: error.kind,
                message: error.message,
            }
        }
    };
    seal_answer(cp, org, &request.request, &answer)
}

/// `answer` sealed for `org` as `request`'s secret.
pub fn seal_answer(
    cp: &ControlPlane,
    org: &DpOrgRow,
    request: &str,
    answer: &LookupAnswer,
) -> anyhow::Result<SealedSecret> {
    let key = cp.data_key(org)?;
    Ok(seal::seal(
        &key,
        &org.data_key_id,
        &org_public(org.org_id),
        &lookup_secret(request),
        0,
        &serde_json::to_vec(answer)?,
        seal::random_nonce(),
    )?)
}
