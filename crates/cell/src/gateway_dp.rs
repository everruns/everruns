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

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};

use crate::dp::{AgentDefinition, ConfigAck, ConfigPosition, ConfigUpdate};
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

/// The directory cell's reads, on a node.
pub fn cell_router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/cell/orgs/{org}/config", get(cell_position))
        .route("/cell/orgs/{org}/agents/{agent}", get(cell_agent))
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
