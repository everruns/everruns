// Agent-trigger HTTP routes (EVE-757).
//
// Sub-resource of an agent: `/v1/agents/{agent_id}/triggers`. Policy enforcement
// happens inside the domain commands (AGENT_VIEW / AGENT_MANAGE). Creating and
// firing triggers needs the durable workflow store plus the session/message
// services, so this router carries its own state rather than reusing the
// agents router (whose state lacks those).

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::agent_triggers::types::{
    AgentTriggerRun, CreateAgentTriggerRequest, UpdateAgentTriggerRequest,
};
use crate::domains::agent_triggers::{
    CreateAgentTrigger, DeleteAgentTrigger, GetAgentTrigger, ListAgentTriggerRuns,
    ListAgentTriggers, TriggerAgentTriggerNow, TriggerAgentTriggerOutput, UpdateAgentTriggerCmd,
};
use crate::domains::common::Command;
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::services::CapabilityService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{AgentTrigger, Caller};
use everruns_durable::WorkflowEventStore;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::IntoParams;

use super::common::{ApiResult, ErrorResponse, impl_auth_state};

/// App state for agent-trigger routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    pub session_service: Option<Arc<SessionService>>,
    pub message_service: Option<Arc<MessageService>>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
        session_service: Arc<SessionService>,
        message_service: Arc<MessageService>,
    ) -> Self {
        Self {
            db,
            workflow_store,
            capability_service,
            auth,
            session_service: Some(session_service),
            message_service: Some(message_service),
        }
    }

    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        let mut ctx = crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
        .with_workflow_store(self.workflow_store.clone());
        if let Some(session_service) = &self.session_service {
            ctx = ctx.with_session_service(session_service.clone());
        }
        if let Some(message_service) = &self.message_service {
            ctx = ctx.with_message_service(message_service.clone());
        }
        ctx
    }
}

impl_auth_state!(AppState);

/// Create agent-trigger routes.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/triggers",
            get(list_agent_triggers).post(create_agent_trigger),
        )
        .route(
            "/v1/agents/{agent_id}/triggers/{trigger_id}",
            get(get_agent_trigger)
                .patch(update_agent_trigger)
                .put(update_agent_trigger)
                .delete(delete_agent_trigger),
        )
        .route(
            "/v1/agents/{agent_id}/triggers/{trigger_id}/trigger",
            post(trigger_agent_trigger),
        )
        .route(
            "/v1/agents/{agent_id}/triggers/{trigger_id}/runs",
            get(list_agent_trigger_runs),
        )
        .with_state(state)
}

/// GET /v1/agents/{agent_id}/triggers
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListAgentTriggersQuery {
    /// Include archived triggers (default false).
    #[serde(default)]
    pub include_archived: bool,
}

#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/triggers",
    description = "List an agent's triggers. Set include_archived=true to also return archived triggers.",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ListAgentTriggersQuery
    ),
    responses(
        (status = 200, description = "Agent triggers", body = Vec<AgentTrigger>),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agent-triggers"
)]
pub async fn list_agent_triggers(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(query): Query<ListAgentTriggersQuery>,
) -> ApiResult<Vec<AgentTrigger>> {
    let triggers = ListAgentTriggers {
        agent_id,
        include_archived: query.include_archived,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(triggers))
}

/// POST /v1/agents/{agent_id}/triggers
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/triggers",
    params(("agent_id" = String, Path, description = "Agent ID (prefixed)")),
    request_body = CreateAgentTriggerRequest,
    responses(
        (status = 201, description = "Trigger created", body = AgentTrigger),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agent-triggers"
)]
pub async fn create_agent_trigger(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateAgentTriggerRequest>,
) -> Result<(StatusCode, Json<AgentTrigger>), (StatusCode, Json<ErrorResponse>)> {
    let trigger = CreateAgentTrigger { agent_id, req }
        .run(&state.ctx(&org))
        .await?;
    Ok((StatusCode::CREATED, Json(trigger)))
}

/// GET /v1/agents/{agent_id}/triggers/{trigger_id}
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/triggers/{trigger_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ("trigger_id" = String, Path, description = "Trigger ID (prefixed)")
    ),
    responses(
        (status = 200, description = "Trigger", body = AgentTrigger),
        (status = 404, description = "Trigger not found", body = ErrorResponse)
    ),
    tag = "agent-triggers"
)]
pub async fn get_agent_trigger(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, trigger_id)): Path<(String, String)>,
) -> ApiResult<AgentTrigger> {
    let trigger = GetAgentTrigger {
        agent_id,
        trigger_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(trigger))
}

#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/triggers/{trigger_id}/runs",
    description = "List the ten most recent durable execution outcomes for an agent trigger.",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ("trigger_id" = String, Path, description = "Trigger ID (prefixed)")
    ),
    responses(
        (status = 200, description = "Recent trigger outcomes", body = Vec<AgentTriggerRun>),
        (status = 404, description = "Trigger not found", body = ErrorResponse)
    ),
    tag = "agent-triggers"
)]
pub async fn list_agent_trigger_runs(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, trigger_id)): Path<(String, String)>,
) -> ApiResult<Vec<AgentTriggerRun>> {
    let runs = ListAgentTriggerRuns {
        agent_id,
        trigger_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(runs))
}

/// PATCH|PUT /v1/agents/{agent_id}/triggers/{trigger_id}
#[utoipa::path(
    patch,
    path = "/v1/agents/{agent_id}/triggers/{trigger_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ("trigger_id" = String, Path, description = "Trigger ID (prefixed)")
    ),
    request_body = UpdateAgentTriggerRequest,
    responses(
        (status = 200, description = "Trigger updated", body = AgentTrigger),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Trigger not found", body = ErrorResponse)
    ),
    tag = "agent-triggers"
)]
pub async fn update_agent_trigger(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, trigger_id)): Path<(String, String)>,
    Json(req): Json<UpdateAgentTriggerRequest>,
) -> ApiResult<AgentTrigger> {
    let trigger = UpdateAgentTriggerCmd {
        agent_id,
        trigger_id,
        req,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(trigger))
}

/// DELETE /v1/agents/{agent_id}/triggers/{trigger_id}
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/triggers/{trigger_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ("trigger_id" = String, Path, description = "Trigger ID (prefixed)")
    ),
    responses(
        (status = 204, description = "Trigger archived"),
        (status = 404, description = "Trigger not found", body = ErrorResponse)
    ),
    tag = "agent-triggers"
)]
pub async fn delete_agent_trigger(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, trigger_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteAgentTrigger {
        agent_id,
        trigger_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/agents/{agent_id}/triggers/{trigger_id}/trigger
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/triggers/{trigger_id}/trigger",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ("trigger_id" = String, Path, description = "Trigger ID (prefixed)")
    ),
    responses(
        (status = 200, description = "Trigger fired", body = TriggerAgentTriggerOutput),
        (status = 404, description = "Trigger not found", body = ErrorResponse)
    ),
    tag = "agent-triggers"
)]
pub async fn trigger_agent_trigger(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, trigger_id)): Path<(String, String)>,
) -> ApiResult<TriggerAgentTriggerOutput> {
    let output = TriggerAgentTriggerNow {
        agent_id,
        trigger_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(output))
}
