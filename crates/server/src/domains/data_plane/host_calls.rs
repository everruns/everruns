//! `POST /dp/v1/tenants/{org}/host-calls/{name}`: what a tool on the data
//! plane asks the control plane at call time (data plane v2 parity, Phase
//! A). The calls are declared in `everruns_contracts::host_calls`; this is
//! their one dispatcher.
//!
//! A hosted capability on a cell needs some answers only the control plane
//! has now: a connection token, a GitHub token or a service key for the
//! turn's person, the org's knowledge, a Poppy call, a platform tool's call
//! as a person, a Slack action as the bot. The session's SQL databases come
//! the same way until they move into the runtime, and the org's settings
//! (`org_settings.rs`) until their pushed copy reaches the cell, as does
//! the session owner's own approval settings (`Fate`).
//!
//! Decisions:
//! - Only a declared host call runs, by its one handler: the v1 worker's
//!   internal command, as the org's internal caller with the org's feature
//!   flags, the same caller, checks and failures a v1 worker's
//!   `ExecuteCommand` gets (a session of another org is not found).
//! - A platform tool's call ([`PLATFORM_COMMAND`], Platform Chat) runs on
//!   the command catalog as the person the forwarding cell names
//!   (`acting_user`), never as the internal caller: the session must
//!   resolve `platform` and the person must still be an org member, with
//!   their current role (THREAT[TM-AGENT-017]).
//! - A call that acts as the turn's person (`ActsAs::Person`: a
//!   connection token, a GitHub token, the agent's service key) records
//!   the turn's invocation first, from the person the forwarding cell
//!   names and the session's agent, as v1 records it when the message
//!   comes through its API ([`record_turn`]). A v2 session's messages
//!   reach the cell without the control plane, so without it every
//!   credential check that reads the invocation (which person's
//!   connection, which responder's service identity) would find none and
//!   answer nothing. Only for a session the data plane runs, once per
//!   turn; a person who is no longer a member records nothing, so the
//!   lookup acts as no one and asks again next time (fail closed).
//! - A Slack action ([`SLACK_ACTION`]) runs as the bot of the endpoint the
//!   session came through, resolved within the org; its token stays here.
//! - A session's SQL databases ([`SESSION_SQL_DB`]) stay in the control
//!   plane's store, as for a v1 session, so the console reads the same
//!   ones; the session must be the org's.
//! - Every answer, a refusal included, is sealed with the org's data key
//!   as the call's own secret (`host-call:{call}`, revision 0): the cells
//!   that pass it on hold ciphertext only, and an answer cannot pass for
//!   another call's.
//! - Each call is counted by name and outcome
//!   (`everruns_dp_host_calls_total`).
//! - Nothing else is stored here. The calls are reads, except a Poppy
//!   call, which keeps the session owner's company accounts and
//!   conversations in the control plane, as on v1.

use everruns_contracts::dataplane::SealedSecret;
use everruns_contracts::dataplane_seal as seal;
use everruns_contracts::host_calls::{
    ActsAs, HostCall, HostCallAnswer, MEMORY_FILE, PLATFORM_COMMAND, SESSION_SQL_DB, SLACK_ACTION,
};
use everruns_contracts::typed_id::{MessageId, SessionId};

use super::{ControlPlane, org_public};
use crate::domains::common::Ctx;
use crate::storage::DpOrgRow;

/// Run `call` with `params` as `ctx` (the org's internal caller).
pub async fn answer(
    cp: &ControlPlane,
    ctx: &Ctx,
    call: &HostCall,
    params: serde_json::Value,
) -> anyhow::Result<HostCallAnswer> {
    if call.acts_as == ActsAs::Person
        && call.name != PLATFORM_COMMAND
        && let Err(error) = record_turn(cp, ctx, &params).await
    {
        // The lookup then finds no invocation and answers nothing.
        tracing::warn!(call = call.name, %error, "could not record the turn's invocation");
    }
    let answer = run(ctx, call, params).await;
    let outcome = match &answer {
        Ok(HostCallAnswer::Value(_)) => "ok",
        Ok(HostCallAnswer::Error { .. }) => "refused",
        Err(_) => "failed",
    };
    metrics::counter!(
        "everruns_dp_host_calls_total",
        "call" => call.name,
        "outcome" => outcome,
    )
    .increment(1);
    answer
}

async fn run(
    ctx: &Ctx,
    call: &HostCall,
    params: serde_json::Value,
) -> anyhow::Result<HostCallAnswer> {
    match call.name {
        // A platform tool's call (Platform Chat) is not an internal
        // command: it runs on the command catalog as the person the
        // forwarding cell names.
        PLATFORM_COMMAND => Ok(
            match crate::worker_link::platform_command_surface::data_plane_call(ctx, &params).await
            {
                Ok(output) => HostCallAnswer::Value(serde_json::Value::String(output)),
                Err((kind, message)) => HostCallAnswer::Error {
                    kind: kind as i32,
                    message,
                },
            },
        ),
        SESSION_SQL_DB => Ok(HostCallAnswer::Value(session_sql_db(ctx, &params).await)),
        MEMORY_FILE => Ok(HostCallAnswer::Value(
            super::memory::answer(ctx, &params).await,
        )),
        SLACK_ACTION => Ok(HostCallAnswer::Value(slack_action(ctx, &params).await)),
        _ => Ok(
            match crate::domains::common::dispatch(call.command, params, ctx).await {
                Ok(json) => HostCallAnswer::Value(serde_json::from_str(&json)?),
                Err(error) => {
                    let error = crate::worker_link::grpc_service::command_error_to_proto(error);
                    HostCallAnswer::Error {
                        kind: error.kind,
                        message: error.message,
                    }
                }
            },
        ),
    }
}

/// Record the invocation of the turn a call names (`input_message_id`) on
/// a session the data plane runs, as v1 does when the message comes
/// through its API: the person the forwarding cell names (`acting_user`,
/// `null` for a channel message or a trigger) with their default runtime
/// identity, and the session's agent as the responder. The checks are the
/// v1 ones (`record_runtime_invocation_with`): an active member, an active
/// agent of the org. A call outside a turn, a v1 session's, or one already
/// recorded is left alone.
async fn record_turn(
    cp: &ControlPlane,
    ctx: &Ctx,
    params: &serde_json::Value,
) -> anyhow::Result<()> {
    let Some(message) = params["input_message_id"].as_str().and_then(input_message) else {
        return Ok(());
    };
    let Some(session) = params["session_id"]
        .as_str()
        .and_then(|id| SessionId::parse(id).ok())
    else {
        return Ok(());
    };
    let org_id = ctx.org_id();
    if !super::sessions::runs_on_data_plane(cp, org_id, session).await?
        || ctx
            .db
            .runtime_invocation_exists(session, message.uuid())
            .await?
    {
        return Ok(());
    }
    // Another org's session: the call itself refuses it.
    let Some(row) = ctx.db.get_session(org_id, session).await? else {
        return Ok(());
    };
    let person = match params["acting_user"].as_str() {
        None => None,
        Some(user) => {
            let user = uuid::Uuid::parse_str(user)?;
            Some((user, ctx.db.default_virtual_user(org_id, user).await?))
        }
    };
    ctx.db
        .record_runtime_invocation_with(
            org_id,
            session,
            message.uuid(),
            person.as_ref().map(|(_, subject)| subject.id),
            person.as_ref().map(|(user, _)| *user),
            row.agent_id.map(|agent| agent.uuid()),
            crate::storage::runtime_identity::InvocationRows {
                default_subject: person.as_ref().map(|(user, subject)| (*user, subject)),
                responder: None,
            },
        )
        .await
}

/// An input message id as a call names it: prefixed or a bare UUID.
fn input_message(id: &str) -> Option<MessageId> {
    MessageId::parse(id)
        .ok()
        .or_else(|| uuid::Uuid::parse_str(id).ok().map(MessageId::from_uuid))
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

/// One operation on a session's SQL databases (`SESSION_SQL_DB`), in the
/// control plane's store as for a v1 worker. The store is keyed by session
/// alone, so the session must be the org's first.
async fn session_sql_db(ctx: &Ctx, params: &serde_json::Value) -> serde_json::Value {
    use everruns_contracts::session_sqldb::SessionSqlDbError as Error;
    let result: Result<serde_json::Value, Error> = async {
        let internal = |error: &dyn std::fmt::Display| Error::Internal(error.to_string());
        let session = params["session_id"]
            .as_str()
            .and_then(|id| everruns_contracts::typed_id::SessionId::parse(id).ok())
            .ok_or_else(|| Error::Internal("invalid session_id".into()))?;
        ctx.db
            .get_session(ctx.org_id(), session)
            .await
            .map_err(|error| internal(&error))?
            .ok_or_else(|| Error::Internal("session not found".into()))?;
        let store = ctx
            .sqldb_store
            .as_ref()
            .ok_or_else(|| Error::Internal("session SQL databases are not configured".into()))?;
        let name = params["name"].as_str().unwrap_or_default();
        let sql = params["sql"].as_str().unwrap_or_default();
        let value = match params["op"].as_str() {
            Some("create") => serde_json::to_value(store.create_database(session, name).await?),
            Some("list") => serde_json::to_value(store.list_databases(session).await?),
            Some("get") => serde_json::to_value(store.get_database(session, name).await?),
            Some("delete") => serde_json::to_value(store.delete_database(session, name).await?),
            Some("execute") => serde_json::to_value(store.sql_execute(session, name, sql).await?),
            Some("query") => serde_json::to_value(store.sql_query(session, name, sql).await?),
            Some("schema") => serde_json::to_value(
                store
                    .sql_schema(session, name, params["table"].as_str())
                    .await?,
            ),
            _ => return Err(Error::Internal("unknown SQL database operation".into())),
        };
        value.map_err(|error| internal(&error))
    }
    .await;
    match result {
        Ok(value) => serde_json::json!({ "ok": value }),
        Err(error) => serde_json::json!({ "error": error }),
    }
}

/// `answer` sealed for `org` as the secret `secret_id`
/// (`host_calls::host_call_secret`, or a legacy lookup's).
pub fn seal_answer(
    cp: &ControlPlane,
    org: &DpOrgRow,
    secret_id: &str,
    answer: &HostCallAnswer,
) -> anyhow::Result<SealedSecret> {
    let key = cp.data_key(org)?;
    Ok(seal::seal(
        &key,
        &org.data_key_id,
        &org_public(org.org_id),
        secret_id,
        0,
        &serde_json::to_vec(answer)?,
        seal::random_nonce(),
    )?)
}
