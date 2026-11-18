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
    AgentDefinition, ClearLeftovers, CommandAck, CommandStatus, ConfigAck, ConfigDiff, ConfigKnown,
    ConfigLeftovers, ConfigPosition, ConfigUpdate, OrgDelete, OrgEnable, OrgExport,
    OrgStateCommand, SealedSecret,
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
        .route("/dp/v1/orgs/{org}/sessions", post(org_session))
        .route(
            "/dp/v1/orgs/{org}/sessions/{session}/sse",
            get(org_session_sse),
        )
        .route(
            "/dp/v1/orgs/{org}/sessions/{session}/{command}",
            post(org_session_command).get(org_session_read),
        )
        .layer(axum::middleware::from_fn_with_state(
            gateway.link_token(),
            crate::node_control::require_link_token,
        ))
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
        .route("/cell/orgs/{org}/leftovers", get(cell_leftovers))
        .route(
            "/cell/orgs/{org}/secrets/{secret}",
            get(cell_secret).post(cell_secret),
        )
        .route(
            "/cell/orgs/{org}/data-key",
            get(cell_data_key).post(cell_data_key),
        )
        .route(
            "/cell/orgs/{org}/connections/{connection}",
            get(cell_connection).post(cell_connection),
        )
        .route("/cell/orgs/{org}/lookups", post(cell_lookup))
        .route("/cell/orgs/{org}/sessions", get(cell_org_sessions))
        .route("/cell/tenants", get(cell_tenants).post(cell_tenants))
        .with_state(node.clone())
        .merge(admin_router(node))
}

/// `/dp/v1` org admin and feed replay on a node, behind the link's token.
fn admin_router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/dp/v1/orgs/{org}", put(enable).delete(delete))
        .route("/dp/v1/orgs/{org}/state", post(set_state))
        .route("/dp/v1/orgs/{org}/exports", post(export))
        .route("/dp/v1/orgs/{org}/commands/{command}", get(command_status))
        .route("/dp/v1/orgs/{org}/config/unknown", post(unknown))
        .route(
            "/dp/v1/orgs/{org}/config/leftovers",
            get(leftovers).post(clear_leftovers),
        )
        .route("/dp/v1/feed/replay", post(replay))
        .layer(axum::middleware::from_fn_with_state(
            node.cfg
                .control_plane
                .as_ref()
                .map_or_else(crate::node_control::link_token, |cp| cp.token.clone())
                .map(Arc::from),
            crate::node_control::require_link_token,
        ))
        .with_state(node)
}

/// `POST /dp/v1/feed/replay`: resend archived feed records.
async fn replay(
    State(node): State<Arc<Node>>,
    Json(replay): Json<crate::dp::FeedReplay>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resent = node
        .replay_feed(&replay.source, replay.incarnation, replay.from_index)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "command_id": replay.command_id,
        "resent": resent,
    })))
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

/// A session the control plane creates for an org on the data plane.
#[derive(serde::Deserialize)]
struct OrgSession {
    /// The control plane's id for it, so its record and the data plane's
    /// session are one session; creating again is a no-op.
    id: everruns_contracts::typed_id::SessionId,
    /// The org's agent, at its current revision.
    agent_ref: String,
    /// The session's own starter files, written where absent.
    #[serde(default)]
    initial_files: Vec<everruns_core::session_file::InitialFile>,
    /// The managed sandbox the control plane pinned for the session, as a
    /// `session_sandbox` capability entry (`sandbox.rs`). Its provider key
    /// is a sealed org secret, so the entry names no credential value.
    #[serde(default)]
    sandbox: Option<serde_json::Value>,
}

/// `POST /dp/v1/orgs/{org}/sessions`: the control plane creating an org's
/// session (its `POST /v1/sessions` for an org on a data plane).
async fn org_session(
    State(gateway): State<Arc<Gateway>>,
    Path(org): Path<String>,
    Json(body): Json<OrgSession>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let revision = gateway.agent_revision(&org, &body.agent_ref).await?;
    let mut package = revision.package;
    if let Some(sandbox) = &body.sandbox {
        let entry = crate::sandbox::capability_entry(sandbox).ok_or((
            StatusCode::BAD_REQUEST,
            "sandbox: not a session_sandbox capability entry".to_string(),
        ))?;
        with_capability(&mut package, entry);
    }
    let created = gateway
        .create(package, None, Some(body.id), Some(org))
        .await?;
    if !body.initial_files.is_empty() {
        let seed = crate::api_files::FsOp::Seed {
            files: body.initial_files,
        };
        let seed = serde_json::to_value(seed)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let reply = gateway.session_command(body.id, "fs", &seed).await?;
        if let Ok(crate::api_files::FsResult::Failed { status, message }) =
            serde_json::from_value(reply)
        {
            return Err((
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
                message,
            ));
        }
    }
    Ok((StatusCode::CREATED, Json(created)))
}

/// The session commands the control plane runs on an org's session
/// itself: admitting a message it produced (an agent trigger's run, step
/// 11.4, or a channel's message, 11.5), cancelling the running turn, a
/// channel's prior conversation, the result of a parked call the control
/// plane resolved (a question set's answers, 11.6), and an event it
/// records (a reaction, 11.6), and a person's change to the session's
/// storage (11.6). The cell checks each body.
const ORG_SESSION_COMMANDS: &[&str] = &[
    "messages",
    "cancel-turn",
    "seed",
    "tool-results",
    "record",
    "storage",
    // A session's files: the control plane copies a message's uploads
    // there before it sends the message (`gateway_input.rs`).
    "fs",
];

/// The org's session `session`, or 404.
async fn owned_session(
    gateway: &Gateway,
    org: &str,
    session: &str,
) -> Result<everruns_contracts::typed_id::SessionId, ApiError> {
    let session = everruns_contracts::typed_id::SessionId::parse(session)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // THREAT[TM-TENANT]: the link authenticates the control plane, not the
    // org; a session of another org reads as missing.
    if gateway.session_tenant(session).await?.as_deref() != Some(org) {
        return Err((StatusCode::NOT_FOUND, "session not found".into()));
    }
    Ok(session)
}

/// `POST /dp/v1/orgs/{org}/sessions/{session}/{command}`: one of
/// [`ORG_SESSION_COMMANDS`] on one of the org's sessions.
async fn org_session_command(
    State(gateway): State<Arc<Gateway>>,
    Path((org, session, command)): Path<(String, String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !ORG_SESSION_COMMANDS.contains(&command.as_str()) {
        return Err((
            StatusCode::NOT_FOUND,
            format!("no session command {command}"),
        ));
    }
    let session = owned_session(&gateway, &org, &session).await?;
    Ok(Json(
        gateway.session_command(session, &command, &body).await?,
    ))
}

/// `GET /dp/v1/orgs/{org}/sessions/{session}/{read}`: one of
/// [`crate::gateway_reads::READS`] on one of the org's sessions (step 11.6).
async fn org_session_read(
    State(gateway): State<Arc<Gateway>>,
    Path((org, session, read)): Path<(String, String, String)>,
    axum::extract::RawQuery(raw): axum::extract::RawQuery,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !crate::gateway_reads::READS.contains(&read.as_str()) {
        return Err((StatusCode::NOT_FOUND, format!("no session read {read}")));
    }
    let session = owned_session(&gateway, &org, &session).await?;
    Ok(Json(
        crate::gateway_reads::read(&gateway, session, &read, raw.as_deref()).await?,
    ))
}

/// `GET /dp/v1/orgs/{org}/sessions/{session}/sse`: the session's live
/// stream for the control plane (a channel delivering a reply, step 11.5),
/// in the shape the console's stream has.
async fn org_session_sse(
    State(gateway): State<Arc<Gateway>>,
    Path((org, session)): Path<(String, String)>,
    axum::extract::RawQuery(raw): axum::extract::RawQuery,
) -> Result<axum::response::Response, ApiError> {
    let id = owned_session(&gateway, &org, &session).await?;
    crate::gateway_sse::stream(&gateway, &id.to_string(), raw.as_deref()).await
}

/// `definition` running the pinned sandbox `entry`: it replaces the
/// agent's other compute (a shell, the workspace's own file tools), as the
/// platform's workers apply a managed sandbox.
fn with_capability(definition: &mut serde_json::Value, entry: serde_json::Value) {
    if !definition["capabilities"].is_array() {
        definition["capabilities"] = serde_json::json!([]);
    }
    if let Some(capabilities) = definition["capabilities"].as_array_mut() {
        capabilities.retain(|c| {
            let id = c["ref"].as_str().or_else(|| c.as_str()).unwrap_or_default();
            !crate::sandbox::REPLACED.contains(&id)
        });
        capabilities.push(entry);
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
    admin(&node, org, AdminCommand::Enable(body))
        .await
        .map(Json)
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
    admin(&node, org, AdminCommand::Delete(body))
        .await
        .map(Json)
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
        .put_object(
            &format!("orgs/{org}/exports/{export_id}.tar.zst"),
            compressed,
        )
        .await
}

async fn command_status(
    State(node): State<Arc<Node>>,
    Path((org, command)): Path<(String, String)>,
) -> Result<Json<CommandStatus>, ApiError> {
    let gateway = gateway_of(&node)?;
    let directory = gateway.directory_cell()?;
    let status: Option<CommandStatus> = gateway
        .call(
            directory,
            &format!("/cell/orgs/{org}/commands/{command}"),
            None,
        )
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
    let body =
        serde_json::to_value(&known).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(
        gateway
            .call(directory, &format!("/cell/orgs/{org}/diff"), Some(&body))
            .await?,
    ))
}

/// `GET /dp/v1/orgs/{org}/config/leftovers`: what a restored control
/// plane's first snapshot lacked (`control::OrgConfig::leftovers`).
async fn leftovers(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
) -> Result<Json<Option<ConfigLeftovers>>, ApiError> {
    let gateway = gateway_of(&node)?;
    let directory = gateway.directory_cell()?;
    Ok(Json(
        gateway
            .call(directory, &format!("/cell/orgs/{org}/leftovers"), None)
            .await?,
    ))
}

/// `POST /dp/v1/orgs/{org}/config/leftovers`: forget them (the control
/// plane either sent them again as config or dropped them).
async fn clear_leftovers(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
    Json(body): Json<ClearLeftovers>,
) -> Result<Json<CommandAck>, ApiError> {
    admin(&node, org, AdminCommand::ClearLeftovers(body))
        .await
        .map(Json)
}

async fn cell_leftovers(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
) -> Result<Json<Option<ConfigLeftovers>>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(
        state.orgs.orgs.get(&org).and_then(|c| c.leftovers.clone()),
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

impl SecretForExecutor {
    /// Open the secret in memory with the data plane's key.
    pub fn open(
        &self,
        org: &str,
        dp_key: &everruns_contracts::dataplane_seal::Key,
    ) -> Result<Vec<u8>, everruns_contracts::dataplane_seal::SealError> {
        use everruns_contracts::dataplane_seal::{SealError, unseal, unwrap_data_key};
        if self.secret.data_key_id != self.data_key.data_key_id {
            return Err(SealError(format!(
                "the secret is sealed under {}, the org's key is {}",
                self.secret.data_key_id, self.data_key.data_key_id
            )));
        }
        let key = unwrap_data_key(
            dp_key,
            org,
            &self.data_key.data_key_id,
            &self.data_key.wrapped,
        )?;
        unseal(&key, org, &self.secret)
    }
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

/// `org`'s data key, wrapped under the data plane's key: what an executor
/// seals and opens the org's session secrets with (`session_secrets.rs`).
async fn cell_data_key(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
) -> Result<Json<Option<crate::control::WrappedKey>>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(
        state.orgs.orgs.get(&org).and_then(|c| c.data_key.clone()),
    ))
}

/// `org`'s connection, for the executor that runs a turn needing it (its
/// token stays sealed: the executor opens it with `secrets`).
async fn cell_connection(
    State(node): State<Arc<Node>>,
    Path((org, connection)): Path<(String, String)>,
) -> Result<Json<Option<everruns_contracts::dataplane::Connection>>, ApiError> {
    leader_only(&node)?;
    let state = node.shared.state.read();
    Ok(Json(
        state
            .orgs
            .orgs
            .get(&org)
            .and_then(|c| c.connections.get(&connection).cloned()),
    ))
}

/// An executor's call-time lookup for `org`, passed on to the control plane
/// over the link (`lookups.rs`). The answer comes back sealed with the
/// org's data key and goes to the executor unopened.
async fn cell_lookup(
    State(node): State<Arc<Node>>,
    Path(org): Path<String>,
    Json(mut request): Json<everruns_contracts::dataplane::LookupRequest>,
) -> Result<Json<everruns_contracts::dataplane::SealedSecret>, ApiError> {
    let cp = node.cfg.control_plane.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "this cell has no control plane".to_owned(),
    ))?;
    if request.command == everruns_contracts::dataplane::PLATFORM_COMMAND_SURFACE {
        act_as_sender(&node, &mut request.params).await?;
    }
    let failed = |error: reqwest::Error| {
        tracing::warn!(%org, %error, "a lookup did not reach the control plane");
        (
            StatusCode::BAD_GATEWAY,
            "the control plane did not answer the lookup".to_owned(),
        )
    };
    let reply = cp
        .post(&node.http, &format!("/dp/v1/tenants/{org}/lookups"))
        .json(&request)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(failed)?;
    if !reply.status().is_success() {
        let status = reply.status();
        tracing::warn!(%org, %status, command = %request.command, "the control plane refused a lookup");
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("the control plane refused the lookup ({status})"),
        ));
    }
    Ok(Json(reply.json().await.map_err(failed)?))
}

/// Set a platform call's `acting_user` to the person who sent its input
/// message, from the session's own record (`senders.rs`), whatever the
/// executor put there (THREAT[TM-AGENT-017]): `null` when no one did, which
/// the control plane refuses.
async fn act_as_sender(node: &Node, params: &mut serde_json::Value) -> Result<(), ApiError> {
    let bad = |what: &str| {
        (
            StatusCode::BAD_REQUEST,
            format!("a platform call needs {what}"),
        )
    };
    let session = params["session_id"]
        .as_str()
        .and_then(|id| everruns_contracts::typed_id::SessionId::parse(id).ok())
        .ok_or_else(|| bad("its session"))?;
    let message = params["input_message_id"]
        .as_str()
        .and_then(|id| everruns_contracts::typed_id::MessageId::parse(id).ok())
        .ok_or_else(|| bad("its input message"))?;
    let gateway = node.gateway.get().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "this cell has no gateway".to_owned(),
    ))?;
    let found = gateway
        .session_command(
            session,
            "sender",
            &serde_json::json!({ "message": message }),
        )
        .await?;
    params["acting_user"] = found["user"].clone();
    Ok(())
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
        taps: state.orgs.policy_taps(),
    }))
}
