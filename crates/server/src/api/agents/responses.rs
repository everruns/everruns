use crate::api::common::{AllowedAction, ApiResultExt, ErrorResponse, ResourceUrlable};
use crate::domains::agent_channels::record::{ChannelStatus, ChannelType};
use crate::domains::agents::record::Agent;
use crate::storage::StorageBackend;
use axum::{Json, http::StatusCode};
use everruns_contracts::typed_id::{AgentId, HarnessId};
use futures::future::try_join_all;
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHarnessSource {
    Explicit,
    OrganizationDefault,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHarnessStatus {
    Active,
    Archived,
    Deleted,
    Unresolved,
}

/// Harness that a newly created session for this agent will resolve to.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentHarnessSummary {
    #[schema(value_type = Option<String>)]
    pub id: Option<HarnessId>,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub source: AgentHarnessSource,
    pub status: AgentHarnessStatus,
}

/// Non-secret metadata for an agent's inbound channels. Schedules are triggers.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentChannelSummary {
    pub id: String,
    pub channel_type: ChannelType,
    pub enabled: bool,
    pub status: ChannelStatus,
}

/// Agent list/detail payload with relationship counts and resolved harness metadata.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentWithCounts {
    pub session_count: u64,
    pub app_count: u64,
    pub effective_harness: AgentHarnessSummary,
    pub channels: Vec<AgentChannelSummary>,
    #[serde(flatten)]
    pub inner: Agent,
}

impl ResourceUrlable for AgentWithCounts {
    fn api_path() -> &'static str {
        Agent::api_path()
    }

    fn ui_path() -> &'static str {
        Agent::ui_path()
    }

    fn resource_id(&self) -> String {
        self.inner.resource_id()
    }

    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        self.inner.allowed_actions(api_base)
    }
}

fn harness_source(value: &str) -> AgentHarnessSource {
    if value == "organization_default" {
        AgentHarnessSource::OrganizationDefault
    } else {
        AgentHarnessSource::Explicit
    }
}

fn harness_status(value: &str) -> AgentHarnessStatus {
    match value {
        "active" => AgentHarnessStatus::Active,
        "archived" => AgentHarnessStatus::Archived,
        "deleted" => AgentHarnessStatus::Deleted,
        _ => AgentHarnessStatus::Unresolved,
    }
}

async fn resolve_agent_harnesses(
    db: &StorageBackend,
    org_id: i64,
    agents: &[Agent],
    fallback_harness_name: Option<&str>,
) -> Result<HashMap<uuid::Uuid, AgentHarnessSummary>, (StatusCode, Json<ErrorResponse>)> {
    let agent_ids: Vec<AgentId> = agents
        .iter()
        .map(|agent| AgentId::from_uuid(agent.internal_id))
        .collect();
    let rows = db
        .get_agents_by_ids(org_id, &agent_ids)
        .await
        .log_internal_error_json("load agent harness bindings")?;

    let inherited_harness_id = if rows
        .iter()
        .any(|row| row.harness_source == "organization_default")
    {
        crate::domains::sessions::queries::resolve_session_harness_id(
            db,
            org_id,
            None,
            None,
            fallback_harness_name,
        )
        .await
        .ok()
    } else {
        None
    };

    let effective_ids: Vec<HarnessId> = rows
        .iter()
        .filter_map(|row| {
            if row.harness_source == "organization_default" {
                inherited_harness_id
            } else {
                Some(row.harness_id)
            }
        })
        .collect();
    let harnesses = if effective_ids.is_empty() {
        Vec::new()
    } else {
        db.get_harness_ancestry_by_ids(org_id, &effective_ids)
            .await
            .log_internal_error_json("load effective agent harnesses")?
    };
    let harnesses_by_id: HashMap<HarnessId, _> = harnesses
        .into_iter()
        .map(|harness| (harness.id, harness))
        .collect();

    Ok(rows
        .into_iter()
        .map(|row| {
            let source = harness_source(&row.harness_source);
            let effective_id = match source {
                AgentHarnessSource::Explicit => Some(row.harness_id),
                AgentHarnessSource::OrganizationDefault => inherited_harness_id,
            };
            let summary = effective_id
                .and_then(|id| harnesses_by_id.get(&id).map(|harness| (id, harness)))
                .map(|(id, harness)| AgentHarnessSummary {
                    id: Some(id),
                    name: Some(harness.name.clone()),
                    display_name: harness.display_name.clone(),
                    source,
                    status: harness_status(&harness.status),
                })
                .unwrap_or(AgentHarnessSummary {
                    id: effective_id,
                    name: None,
                    display_name: None,
                    source,
                    status: AgentHarnessStatus::Unresolved,
                });
            (row.id.uuid(), summary)
        })
        .collect())
}

async fn add_agent_counts(
    db: &StorageBackend,
    org_id: i64,
    agent: Agent,
    effective_harness: AgentHarnessSummary,
    channels: Vec<AgentChannelSummary>,
) -> Result<AgentWithCounts, (StatusCode, Json<ErrorResponse>)> {
    let agent_id = AgentId::from_uuid(agent.internal_id);
    let session_count = async {
        db.count_sessions_for_agent(org_id, agent_id)
            .await
            .log_internal_error_json("count agent sessions")
    };
    let app_count = async {
        db.count_apps_for_agent(org_id, agent_id)
            .await
            .log_internal_error_json("count agent apps")
    };
    let (session_count, app_count) = tokio::try_join!(session_count, app_count)?;

    Ok(AgentWithCounts {
        session_count,
        app_count,
        effective_harness,
        channels,
        inner: agent,
    })
}

pub(super) async fn add_agents_counts(
    db: &StorageBackend,
    org_id: i64,
    agents: Vec<Agent>,
    fallback_harness_name: Option<&str>,
) -> Result<Vec<AgentWithCounts>, (StatusCode, Json<ErrorResponse>)> {
    let mut harnesses = resolve_agent_harnesses(db, org_id, &agents, fallback_harness_name).await?;
    let mut channels: HashMap<_, Vec<AgentChannelSummary>> = HashMap::new();
    if !agents.is_empty() {
        let ids: Vec<_> = agents.iter().map(|agent| agent.internal_id).collect();
        for row in db
            .list_agent_channel_summaries(org_id, &ids)
            .await
            .log_internal_error_json("list agent channels")?
        {
            let channel_type = ChannelType::from_str_opt(&row.channel_type)
                .ok_or_else(ErrorResponse::internal_error)?;
            channels
                .entry(row.agent_id)
                .or_default()
                .push(AgentChannelSummary {
                    id: row.public_id,
                    channel_type,
                    enabled: row.enabled,
                    status: ChannelStatus::from(row.status.as_str()),
                });
        }
    }
    try_join_all(agents.into_iter().map(|agent| {
        let effective_harness =
            harnesses
                .remove(&agent.internal_id)
                .unwrap_or(AgentHarnessSummary {
                    id: None,
                    name: None,
                    display_name: None,
                    source: AgentHarnessSource::Explicit,
                    status: AgentHarnessStatus::Unresolved,
                });
        let agent_channels = channels.remove(&agent.internal_id).unwrap_or_default();
        add_agent_counts(db, org_id, agent, effective_harness, agent_channels)
    }))
    .await
}
