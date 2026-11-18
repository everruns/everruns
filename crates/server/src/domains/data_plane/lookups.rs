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
//! - A platform tool's call ([`PLATFORM_COMMAND_SURFACE`], Platform Chat)
//!   runs on the command catalog as the person the forwarding cell names
//!   (`acting_user`), never as the internal caller: the session must
//!   resolve `platform` and the person must still be an org member, with
//!   their current role (THREAT[TM-AGENT-017]).
//! - A Slack action ([`SLACK_ACTION`]) runs as the bot of the endpoint the
//!   session came through, resolved within the org; its token stays here.
//! - Every answer, a refusal included, is sealed with the org's data key
//!   as the request's own secret (`lookup:{request}`, revision 0): the
//!   cells that pass it on hold ciphertext only, and an answer cannot pass
//!   for another request's.
//! - Nothing is stored here. The commands are reads, except a Poppy call,
//!   which keeps the session owner's company accounts and conversations in
//!   the control plane, as on v1.

use everruns_contracts::dataplane::SealedSecret;
use everruns_contracts::dataplane::{
    LOOKUP_COMMANDS, LookupAnswer, LookupRequest, PLATFORM_COMMAND_SURFACE, SLACK_ACTION,
    lookup_secret,
};
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
    // A platform tool's call (Platform Chat) is not an internal command: it
    // runs on the command catalog as the person the forwarding cell names.
    if request.command == PLATFORM_COMMAND_SURFACE {
        let answer = match crate::worker_link::platform_command_surface::data_plane_call(
            ctx,
            &request.params,
        )
        .await
        {
            Ok(output) => LookupAnswer::Value(serde_json::Value::String(output)),
            Err((kind, message)) => LookupAnswer::Error {
                kind: kind as i32,
                message,
            },
        };
        return seal_answer(cp, org, &request.request, &answer);
    }
    if request.command == SLACK_ACTION {
        let answer = LookupAnswer::Value(slack_action(ctx, &request.params).await);
        return seal_answer(cp, org, &request.request, &answer);
    }
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

/// A Slack action for one session of the org (`SLACK_ACTION`), as the bot
/// of the Slack endpoint the session came through, as a v1 worker's
/// `InvokeSlackAction`: the session is read within the org, so another
/// org's session has no endpoint, and the bot token stays here.
async fn slack_action(ctx: &Ctx, params: &serde_json::Value) -> serde_json::Value {
    use everruns_contracts::slack_action::{SlackAction, SlackActionError};
    let session = params["session_id"]
        .as_str()
        .and_then(|id| everruns_contracts::typed_id::SessionId::parse(id).ok());
    let action = serde_json::from_value::<SlackAction>(params["action"].clone());
    let outcome = match (session, action) {
        (Some(session), Ok(action)) => {
            crate::channels::slack::actions::in_process_invoker(
                &ctx.db,
                ctx.encryption.as_ref(),
                ctx.org_id(),
                session,
            )
            .invoke(action)
            .await
        }
        (None, _) => Err(SlackActionError::InvalidArgument(
            "invalid session_id".into(),
        )),
        (_, Err(error)) => Err(SlackActionError::InvalidArgument(format!(
            "invalid Slack action: {error}"
        ))),
    };
    match outcome {
        Ok(outcome) => serde_json::json!({ "outcome": outcome }),
        Err(error) => serde_json::json!({ "error": error }),
    }
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
