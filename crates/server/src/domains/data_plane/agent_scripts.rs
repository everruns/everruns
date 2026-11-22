//! The saved scripts of a session a data plane runs (parity, saved-script
//! triggers): the `agent_scripts` and `agent_script_save` host calls answer
//! the shell's `tools scripts` on a shard, as the worker commands answer a v1
//! worker's (`grpc_saved_scripts.rs`).
//!
//! Decisions:
//! - Both calls name the session; the agent is the session's own, found
//!   here. The session must be one of the org's sessions on its data plane.
//! - A save runs as the turn's management user (`runtime_invocations`,
//!   recorded from the person the forwarding shard names) under the
//!   commands' own `AGENT_MANAGE` policy, as v1 runs a bound save
//!   (THREAT[TM-TOOL-067]). A turn with no management user (a channel or a
//!   trigger's turn) is refused, and so is an agent whose shell does not
//!   let it save scripts (`manage_scripts`, which a v1 worker reads when it
//!   builds the turn's tools).
//! - A script with the name already taken is updated (description, schema,
//!   body); the name never changes.

use everruns_contracts::typed_id::SessionId;
use everruns_internal_protocol::proto::command_error::Kind;
use serde_json::{Value, json};

use super::ControlPlane;
use crate::domains::common::Ctx;
use everruns_contracts::host_calls::HostCallAnswer;

fn refused(kind: Kind, message: impl Into<String>) -> HostCallAnswer {
    HostCallAnswer::Error {
        kind: kind as i32,
        message: message.into(),
    }
}

/// The session `params` names, when it is one of the org's on its data
/// plane: its record.
async fn session(
    cp: &ControlPlane,
    ctx: &Ctx,
    params: &Value,
) -> anyhow::Result<Result<crate::domains::sessions::record::Session, HostCallAnswer>> {
    let Some(id) = params["session_id"]
        .as_str()
        .and_then(|id| SessionId::parse(id).ok())
    else {
        return Ok(Err(refused(Kind::BadRequest, "Invalid session_id")));
    };
    let org_id = ctx.org_id();
    if !super::sessions::runs_on_data_plane(cp, org_id, id).await? {
        return Ok(Err(refused(Kind::NotFound, "Session not found")));
    }
    let found = crate::domains::sessions::SessionService::new(ctx.db.clone())
        .get(&ctx.caller, id.uuid(), None)
        .await?;
    Ok(found.ok_or_else(|| refused(Kind::NotFound, "Session not found")))
}

/// Answer `agent_scripts`: the session's agent's active scripts.
pub async fn list(cp: &ControlPlane, ctx: &Ctx, params: &Value) -> anyhow::Result<HostCallAnswer> {
    let session = match session(cp, ctx, params).await? {
        Ok(session) => session,
        Err(refusal) => return Ok(refusal),
    };
    let Some(agent) = session.agent_id else {
        return Ok(refused(
            Kind::NotFound,
            "this session has no agent to keep scripts for",
        ));
    };
    answer(
        crate::domains::common::dispatch(
            "list_agent_scripts",
            json!({ "agent_id": agent.to_string() }),
            ctx,
        )
        .await,
    )
}

/// Answer `agent_script_save`: create or update the script as the turn's
/// management user.
pub async fn save(cp: &ControlPlane, ctx: &Ctx, params: &Value) -> anyhow::Result<HostCallAnswer> {
    let session = match session(cp, ctx, params).await? {
        Ok(session) => session,
        Err(refusal) => return Ok(refusal),
    };
    let Some(agent) = session.agent_id else {
        return Ok(refused(
            Kind::NotFound,
            "this session has no agent to keep scripts for",
        ));
    };
    if !may_save(ctx, &session).await? {
        return Ok(refused(
            Kind::Forbidden,
            "this agent's shell does not save scripts; turn on `manage_scripts` in its Shell Code Mode settings",
        ));
    }
    let Some(message) = params["input_message_id"]
        .as_str()
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
    else {
        return Ok(refused(
            Kind::Forbidden,
            "saving a script requires a management-authorized turn; this one has none",
        ));
    };
    let Some(user) = ctx
        .db
        .runtime_invocation_management_user(session.id, message)
        .await?
    else {
        return Ok(refused(
            Kind::Forbidden,
            "Management-authorized invocation required",
        ));
    };
    let Ok(caller) =
        crate::auth::caller_resolution::caller_for_user(&ctx.db, ctx.org_id(), user).await
    else {
        return Ok(refused(Kind::Forbidden, "Management authority revoked"));
    };
    let name = params["name"].as_str().unwrap_or_default().to_owned();
    let intent = crate::domains::change_history::ChangeIntent {
        via_session_id: Some(session.id.uuid()),
        via_agent_id: Some(agent.to_string()),
        reason: Some(format!("Saved script '{name}' from the agent's shell")),
        ..crate::domains::change_history::ChangeIntent::on(
            crate::domains::change_history::ChangeSurface::Worker,
        )
    };
    let as_person = Ctx {
        caller,
        ..ctx.clone()
    }
    .with_change_intent(intent);
    let listed = crate::domains::common::dispatch(
        "list_agent_scripts",
        json!({ "agent_id": agent.to_string() }),
        ctx,
    )
    .await;
    let listed: Value = match listed {
        Ok(json) => serde_json::from_str(&json)?,
        Err(error) => return answer(Err(error)),
    };
    let existing = listed
        .as_array()
        .into_iter()
        .flatten()
        .find(|script| script["name"].as_str() == Some(name.as_str()))
        .and_then(|script| script["id"].as_str().map(str::to_owned));
    let fields = json!({
        "description": params["description"],
        "input_schema": params["input_schema"],
        "body": params["body"],
    });
    let (command, mut body) = match existing {
        Some(script) => (
            "update_agent_script",
            json!({ "agent_id": agent.to_string(), "script_id": script }),
        ),
        None => (
            "create_agent_script",
            json!({ "agent_id": agent.to_string(), "name": name }),
        ),
    };
    for (key, value) in fields.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    answer(crate::domains::common::dispatch(command, body, &as_person).await)
}

/// Whether the session's agent's shell lets it save scripts.
async fn may_save(
    ctx: &Ctx,
    session: &crate::domains::sessions::record::Session,
) -> anyhow::Result<bool> {
    let resolved = crate::worker_link::platform_command_surface::session_capabilities(
        &ctx.db,
        ctx.capability_service.registry(),
        ctx.org_id(),
        session,
    )
    .await?;
    Ok(resolved.iter().any(|capability| {
        everruns_integrations::bashkit::shell_code_mode::is_shell_code_mode_id(
            capability.capability_id(),
        ) && everruns_integrations::bashkit::shell_code_mode::manage_scripts(
            capability.config_value(),
        )
    }))
}

fn answer(
    result: Result<String, crate::domains::common::CommandError>,
) -> anyhow::Result<HostCallAnswer> {
    Ok(match result {
        Ok(json) => HostCallAnswer::Value(serde_json::from_str(&json)?),
        Err(error) => {
            let error = crate::worker_link::grpc_service::command_error_to_proto(error);
            HostCallAnswer::Error {
                kind: error.kind,
                message: error.message,
            }
        }
    })
}
