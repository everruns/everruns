//! The data plane's side of `/dp/v1` config (cp-dp-protocol.md), served by
//! every gateway, and the directory cell's reads behind it.
//!
//! Decisions:
//! - A config update is one command in the directory cell's Raft; the
//!   gateway only forwards it and returns the cell's `ConfigAck`.
//! - A session created by agent reference (`org` + `agent_ref`) copies the
//!   agent's current revision when it is created and is billed to the org,
//!   so later revisions and removals never change it.
//! - The directory reads behind these are answered by its leader only (a
//!   follower answers 503 and the gateway tries the next member), so a
//!   create right after a config update sees that update.
//! - Org admin commands are, like config, commands in the directory cell's
//!   Raft, idempotent by `command_id`. They are served by nodes (not
//!   standalone gateways) because an export writes to the object store.
//! - `ConfigDiff` takes what the restored control plane knows in the
//!   request body (`POST .../config/unknown`): the data plane can't tell
//!   on its own which of its revisions a restored database lost.
//! - An export is one tar (zstd) per org at
//!   `orgs/{org}/exports/{export_id}.tar.zst`: the org's config without its
//!   data key, then every session's full log. The node that accepted the
//!   command writes it and reports progress through the directory cell.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};

use crate::control::AdminCommand;
use crate::dp::{
    AgentDefinition, CommandAck, CommandStatus, ConfigAck, ConfigDiff, ConfigKnown,
    ConfigPosition, ConfigUpdate, OrgDelete, OrgEnable, OrgExport, OrgStateCommand,
    SealedSecret,
};
use crate::gateway::Gateway;
use crate::node::{Node, now_ms};
use crate::state::{Command, Response};

type ApiError = (StatusCode, String);

/// `/dp/v1` on a gateway.
pub fn router(gateway: Arc<Gateway>) -> Router {
    Router::new()
        .route("/dp/v1/orgs/{org}/config", get(position))
        .route("/dp/v1/orgs/{org}/config/{epoch}/{version}", put(config))
        .with_state(gateway)
}

/// The directory cell's reads, and `/dp/v1` org admin, on a node.
pub fn cell_router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/cell/orgs/{org}/config", get(cell_position))
        .route("/cell/orgs/{org}/agents/{agent}", get(cell_agent))
        .route("/cell/orgs/{org}/full", get(cell_full))
        .route("/cell/orgs/{org}/commands/{command}", get(cell_command))
        .route("/cell/orgs/{org}/diff", post(cell_diff))
        .route("/cell/orgs/{org}/secrets/{secret}", get(cell_secret))
        .route("/cell/orgs/{org}/sessions", get(cell_org_sessions))
        .route("/cell/tenants", get(cell_tenants).post(cell_tenants))
        .route(
            "/dp/v1/orgs/{org}",
            put(enable).delete(delete),
        )
        .route("/dp/v1/orgs/{org}/state", post(set_state))
        .route("/dp/v1/orgs/{org}/exports", post(export))
        .route("/dp/v1/orgs/{org}/commands/{command}", get(command_status))
        .route("/dp/v1/orgs/{org}/config/unknown", post(unknown))
        .with_state(node)
}

async fn config(
    State(gateway): State<Arc<Gateway>>,
    Path((org, epoch, version)): Path<(String, u64, u64)>,
    Json(update): Json<ConfigUpdate>,
) -> Result<Json<ConfigAck>, ApiError> {
    if update.epoch != epoch || update.version != version {
        return Err((
            StatusCode::BAD_REQUEST,
            "the path's epoch and version must match the body's".into(),
        ));
    }
    let directory = gateway.directory_cell()?;
    let command = Command::Config {
        org,
        update,
        now_ms: now_ms(),
    };
    match gateway.propose(directory, &command).await? {
        Response::Value { value } => serde_json::from_value(value)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        other => Err((StatusCode::CONFLICT, format!("{other:?}"))),
    }
}

async fn position(
    State(gateway): State<Arc<Gateway>>,
    Path(org): Path<String>,
) -> Result<Json<ConfigPosition>, ApiError> {
    let directory = gateway.directory_cell()?;
    Ok(Json(
        gateway
            .call(directory, &format!("/cell/orgs/{org}/config"), None)
            .await?,
    ))
}

impl Gateway {
    /// `org`'s current revision of `agent`.
    pub async fn agent_revision(
        &self,
        org: &str,
        agent: &str,
    ) -> Result<AgentDefinition, ApiError> {
        let directory = self.directory_cell()?;
        let found: Option<AgentDefinition> = self
            .call(directory, &format!("/cell/orgs/{org}/agents/{agent}"), None)
            .await?;
        found.ok_or((
            StatusCode::NOT_FOUND,
            format!("org {org} has no agent {agent}"),
        ))
    }
}

fn leader_only(node: &Node) -> Result<(), ApiError> {
    if node.is_leader() {
        Ok(())
    } else {
        Err((StatusCode::SERVICE_UNAVAILABLE, "not the leader".into()))
    }
}

async fn cell_position(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
) -> Result<Json<ConfigPosition>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(
        state
            .orgs
            .orgs
            .get(&org)
            .map(|c| ConfigPosition {
                epoch: c.epoch,
                applied_version: c.applied_version,
            })
            .unwrap_or_default(),
    ))
}

async fn cell_agent(
    State(node): State<Arc<Node>>,
    Path((org, agent)): Path<(String, String)>,
) -> Result<Json<Option<AgentDefinition>>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(
        state
            .orgs
            .orgs
            .get(&org)
            .and_then(|c| c.agents.get(&agent))
            .cloned(),
    ))
}

// ------------------------------------------------------------ org admin

fn gateway_of(node: &Node) -> Result<Arc<Gateway>, ApiError> {
    node.gateway
        .get()
        .cloned()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "starting".into()))
}

/// Propose `command` for `org` in the directory cell.
async fn admin(node: &Node, org: String, command: AdminCommand) -> Result<CommandAck, ApiError> {
    let gateway = gateway_of(node)?;
    let directory = gateway.directory_cell()?;
    let command = Command::Admin {
        org,
        command,
        now_ms: now_ms(),
    };
    match gateway.propose(directory, &command).await? {
        Response::Value { value } => serde_json::from_value(value)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        other => Err((StatusCode::CONFLICT, format!("{other:?}"))),
    }
}

async fn enable(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
    Json(body): Json<OrgEnable>,
) -> Result<Json<CommandAck>, ApiError> {
    admin(&node, org, AdminCommand::Enable(body)).await.map(Json)
}

async fn set_state(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
    Json(body): Json<OrgStateCommand>,
) -> Result<Json<CommandAck>, ApiError> {
    admin(&node, org, AdminCommand::State(body)).await.map(Json)
}

async fn delete(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
    Json(body): Json<OrgDelete>,
) -> Result<Json<CommandAck>, ApiError> {
    admin(&node, org, AdminCommand::Delete(body)).await.map(Json)
}

async fn export(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
    Json(body): Json<OrgExport>,
) -> Result<Json<CommandAck>, ApiError> {
    let gateway = gateway_of(&node)?;
    let directory = gateway.directory_cell()?;
    let before: Option<CommandStatus> = gateway
        .call(
            directory,
            &format!("/cell/orgs/{org}/commands/{}", body.command_id),
            None,
        )
        .await?;
    let ack = admin(&node, org.clone(), AdminCommand::Export(body.clone())).await?;
    // Only the call that started the command runs it; a retried one
    // (already recorded) only gets the acknowledgement again.
    if ack.accepted && before.is_none() {
        let store = node.cfg.store.clone();
        tokio::spawn(async move {
            let status = match write_export(&gateway, &store, &org, &body.export_id).await {
                Ok(()) => CommandStatus {
                    command_id: body.command_id.clone(),
                    state: "done".into(),
                    progress: Some(1.0),
                    error: None,
                },
                Err(error) => CommandStatus {
                    command_id: body.command_id.clone(),
                    state: "failed".into(),
                    progress: None,
                    error: Some(format!("{error:?}")),
                },
            };
            let command = Command::Admin {
                org,
                command: AdminCommand::Progress(status),
                now_ms: now_ms(),
            };
            if let Err(error) = gateway.propose(directory, &command).await {
                tracing::warn!(?error, "export status not recorded");
            }
        });
    }
    Ok(Json(ack))
}

/// One session in an export.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExportedSession {
    pub session: everruns_contracts::typed_id::SessionId,
    pub events: Vec<everruns_core::events::Event>,
}

async fn write_export(
    gateway: &Gateway,
    store: &crate::store::Store,
    org: &str,
    export_id: &str,
) -> anyhow::Result<()> {
    let topology = gateway.topology().map_err(|(_, e)| anyhow::anyhow!(e))?;
    let directory = topology.directory_cell;
    let config: Option<serde_json::Value> = gateway
        .call(directory, &format!("/cell/orgs/{org}/full"), None)
        .await
        .map_err(|(_, e)| anyhow::anyhow!(e))?;
    let mut tar = tar::Builder::new(Vec::new());
    let mut add = |name: &str, bytes: &[u8]| -> anyhow::Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, name, bytes)?;
        Ok(())
    };
    add("config.json", &serde_json::to_vec(&config)?)?;
    for cell in topology.cells.keys() {
        let sessions: Vec<ExportedSession> = gateway
            .call(*cell, &format!("/cell/orgs/{org}/sessions"), None)
            .await
            .map_err(|(_, e)| anyhow::anyhow!(e))?;
        for session in sessions {
            add(
                &format!("sessions/{}.json", session.session),
                &serde_json::to_vec(&session.events)?,
            )?;
        }
    }
    let tar = tar.into_inner()?;
    let compressed = zstd::encode_all(tar.as_slice(), 3)?;
    store
        .put_object(&format!("orgs/{org}/exports/{export_id}.tar.zst"), compressed)
        .await
}

async fn command_status(
    State(node): State<Arc<Node>>,
    Path((org, command)): Path<(String, String)>,
) -> Result<Json<CommandStatus>, ApiError> {
    let gateway = gateway_of(&node)?;
    let directory = gateway.directory_cell()?;
    let status: Option<CommandStatus> = gateway
        .call(directory, &format!("/cell/orgs/{org}/commands/{command}"), None)
        .await?;
    status
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, format!("no command {command}")))
}

async fn unknown(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
    Json(known): Json<ConfigKnown>,
) -> Result<Json<ConfigDiff>, ApiError> {
    let gateway = gateway_of(&node)?;
    let directory = gateway.directory_cell()?;
    let body = serde_json::to_value(&known)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(
        gateway
            .call(directory, &format!("/cell/orgs/{org}/diff"), Some(&body))
            .await?,
    ))
}

/// `org`'s whole config without its data key (an export).
async fn cell_full(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
) -> Result<Json<Option<crate::control::OrgConfig>>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(state.orgs.orgs.get(&org).cloned().map(|mut c| {
        c.data_key = None;
        c
    })))
}

async fn cell_command(
    State(node): State<Arc<Node>>,
    Path((org, command)): Path<(String, String)>,
) -> Result<Json<Option<CommandStatus>>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(
        state
            .orgs
            .orgs
            .get(&org)
            .and_then(|c| c.command(&command))
            .cloned(),
    ))
}

async fn cell_diff(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
    Json(known): Json<ConfigKnown>,
) -> Result<Json<ConfigDiff>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(
        state
            .orgs
            .orgs
            .get(&org)
            .map(|c| c.diff(&known))
            .unwrap_or_default(),
    ))
}

/// A sealed secret and the org's wrapped data key: what an executor needs
/// to unseal it in memory.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretForExecutor {
    pub secret: SealedSecret,
    pub data_key: crate::control::WrappedKey,
}

async fn cell_secret(
    State(node): State<Arc<Node>>,
    Path((org, secret)): Path<(String, String)>,
) -> Result<Json<Option<SecretForExecutor>>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(state.orgs.orgs.get(&org).and_then(|c| {
        Some(SecretForExecutor {
            secret: c.secrets.get(&secret)?.clone(),
            data_key: c.data_key.clone()?,
        })
    })))
}

/// Every session of `org` in this cell, with its full log (an export).
async fn cell_org_sessions(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
) -> Result<Json<Vec<ExportedSession>>, ApiError> {
    leader_only(&node)?;
    let ids: Vec<everruns_contracts::typed_id::SessionId> = {
        let state = node.shared.state.read();
        state
            .sessions
            .iter()
            .filter(|(_, s)| s.tenant.as_deref() == Some(org.as_str()))
            .map(|(id, _)| *id)
            .collect()
    };
    let mut out = Vec::with_capacity(ids.len());
    for session in ids {
        match node.shared.read_events(session, 0).await {
            Some(Ok(events)) => out.push(ExportedSession { session, events }),
            Some(Err(error)) => {
                return Err((StatusCode::SERVICE_UNAVAILABLE, error.to_string()));
            }
            // Moved away since the listing: its new cell exports it.
            None => {}
        }
    }
    Ok(Json(out))
}

/// The tenants that may not start turns; cell leaders and executors poll
/// it (see `tenants.rs`).
async fn cell_tenants(
    State(node): State<Arc<Node>>,
) -> Result<Json<crate::tenants::Tenants>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(crate::tenants::Tenants {
        stopped: state.orgs.stopped(),
    }))
}
