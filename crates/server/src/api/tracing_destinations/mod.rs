//! Customer destination management. Auth and policy changes are explicit, tenant-scoped operations.
use super::{common::ErrorResponse, state::ApiState};
use crate::domains::change_history::{ChangeAction, EntityKind, rest::RestChange};
use crate::domains::common::Command;
use crate::{
    auth::ResolvedOrg, domains::trace_exports::*, storage::trace_exports::TracingPolicyView,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_contracts::typed_id::{AgentId, MessageId, SessionId, TurnId};
use everruns_core::{EgressScope, Event, EventContext};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

type Result<T> = std::result::Result<T, (StatusCode, Json<ErrorResponse>)>;
fn error(status: StatusCode, message: &str) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message).into_response(status)
}
fn internal(_: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Tracing operation failed",
    )
}
fn admin(state: &ApiState, org: &ResolvedOrg, write: bool) -> Result<()> {
    let ctx = state.ctx(org);
    commands::TRACING_MANAGE
        .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
        .map_err(|_| {
            error(
                StatusCode::FORBIDDEN,
                "Organization settings permission is required",
            )
        })?;
    if write && org.read_only {
        return Err(error(
            StatusCode::FORBIDDEN,
            "Read-only access cannot change tracing",
        ));
    }
    Ok(())
}
#[derive(Serialize, ToSchema)]
pub struct DestinationView {
    pub id: Uuid,
    pub config: DestinationConfig,
    pub revision: i64,
    pub last_accepted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub dropped_events: i64,
    pub organization_default: bool,
    pub agent_selections: i64,
}
impl TryFrom<crate::storage::trace_exports::TracingDestinationRow> for DestinationView {
    type Error = serde_json::Error;
    fn try_from(
        row: crate::storage::trace_exports::TracingDestinationRow,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            config: serde_json::from_value(row.config)?,
            revision: row.revision,
            last_accepted_at: row.last_accepted_at,
            last_attempt_at: row.last_attempt_at,
            last_error: row.last_error,
            dropped_events: row.dropped_events,
            organization_default: row.organization_default,
            agent_selections: row.agent_selections,
        })
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveDestinationRequest {
    pub config: DestinationConfig,
    #[schema(write_only)]
    pub secrets: Option<TraceSecrets>,
    pub revision: Option<i64>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveRoutingRequest {
    pub policy: RoutingPolicy,
    pub revision: i64,
}
#[derive(Deserialize, utoipa::IntoParams)]
pub struct DestinationRevision {
    pub revision: i64,
}
#[derive(Serialize, ToSchema)]
pub struct TestTraceResponse {
    pub accepted: bool,
    pub message: String,
}

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route(
            "/v1/tracing/destinations",
            get(list_destinations).post(create_destination),
        )
        .route(
            "/v1/tracing/destinations/{id}",
            axum::routing::put(update_destination).delete(delete_destination),
        )
        .route("/v1/tracing/destinations/{id}/test", post(test_destination))
        .route("/v1/tracing/defaults", get(get_defaults).put(save_defaults))
        .route(
            "/v1/agents/{agent_id}/tracing",
            get(get_agent_routing).put(save_agent_routing),
        )
        .with_state(state)
}
#[utoipa::path(get,path="/v1/tracing/destinations",responses((status=200,body=Vec<DestinationView>)),tag="tracing")]
pub async fn list_destinations(
    org: ResolvedOrg,
    State(state): State<ApiState>,
) -> Result<Json<Vec<DestinationView>>> {
    // Non-secret names and health are available to members; only admins can assign or export data.
    let rows = state
        .db
        .tracing_destinations(org.org_id)
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(DestinationView::try_from)
            .collect::<std::result::Result<_, _>>()
            .map_err(internal)?,
    ))
}
fn validate(body: &SaveDestinationRequest) -> Result<()> {
    body.config
        .validate()
        .map_err(|m| error(StatusCode::BAD_REQUEST, m))?;
    if let Some(s) = &body.secrets {
        s.validate()
            .map_err(|m| error(StatusCode::BAD_REQUEST, m))?;
    }
    Ok(())
}
#[utoipa::path(post,path="/v1/tracing/destinations",request_body=SaveDestinationRequest,responses((status=201,body=DestinationView)),tag="tracing")]
pub async fn create_destination(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Json(body): Json<SaveDestinationRequest>,
) -> Result<(StatusCode, Json<DestinationView>)> {
    admin(&state, &org, true)?;
    validate(&body)?;
    let secrets = body.secrets.unwrap_or(TraceSecrets {
        headers: Default::default(),
    });
    if body.config.provider == TraceProvider::Braintrust
        && !secrets
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && !v.trim().is_empty())
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Braintrust requires an Authorization header",
        ));
    }
    let encryption = state.encryption.as_ref().ok_or_else(|| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Secret encryption is not configured",
        )
    })?;
    let encrypted = encryption
        .encrypt_string(&serde_json::to_string(&secrets).map_err(internal)?)
        .map_err(internal)?;
    let journal = RestChange::begin_with_context(
        state.ctx(&org),
        "create_tracing_destination",
        EntityKind::TracingDestination,
        ChangeAction::Created,
        None,
        &["name"],
    )
    .await
    .map_err(internal)?;
    let id = state
        .db
        .create_tracing_destination(org.org_id, &body.config, encrypted)
        .await
        .map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "Unable to create destination; organization limit is 32",
            )
        })?;
    let row = state
        .db
        .tracing_destination(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Destination not found"))?;
    journal.finish(&id.to_string()).await;
    Ok((
        StatusCode::CREATED,
        Json(DestinationView::try_from(row).map_err(internal)?),
    ))
}
#[utoipa::path(put,path="/v1/tracing/destinations/{id}",params(("id"=Uuid,Path,description="Destination ID")),request_body=SaveDestinationRequest,responses((status=200,body=DestinationView)),tag="tracing")]
pub async fn update_destination(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SaveDestinationRequest>,
) -> Result<Json<DestinationView>> {
    admin(&state, &org, true)?;
    validate(&body)?;
    let current = state
        .db
        .tracing_destination(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Destination not found"))?;
    let previous: DestinationConfig = serde_json::from_value(current.config).map_err(internal)?;
    // Credentials are bound to the receiver: changing it requires new credentials rather than silently forwarding a stored key elsewhere.
    if (previous.endpoint != body.config.endpoint || previous.provider != body.config.provider)
        && body.secrets.is_none()
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Changing the receiver requires new authentication settings",
        ));
    }
    let encrypted = if let Some(secrets) = body.secrets {
        if body.config.provider == TraceProvider::Braintrust
            && !secrets
                .headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && !v.trim().is_empty())
        {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "Braintrust requires an Authorization header",
            ));
        }
        let encryption = state.encryption.as_ref().ok_or_else(|| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Secret encryption is not configured",
            )
        })?;
        Some(
            encryption
                .encrypt_string(&serde_json::to_string(&secrets).map_err(internal)?)
                .map_err(internal)?,
        )
    } else {
        None
    };
    let revision = body
        .revision
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Revision required"))?;
    let journal = RestChange::begin_with_context(
        state.ctx(&org),
        "update_tracing_destination",
        EntityKind::TracingDestination,
        ChangeAction::Updated,
        Some(&id.to_string()),
        &[
            "name",
            "enabled",
            "endpoint",
            "project_id",
            "capture_content",
            "capture_reasoning",
            "authentication",
        ],
    )
    .await
    .map_err(internal)?;
    if !state
        .db
        .update_tracing_destination(org.org_id, id, revision, &body.config, encrypted)
        .await
        .map_err(internal)?
    {
        return Err(error(
            StatusCode::CONFLICT,
            "Destination changed; reload before saving",
        ));
    }
    journal.finish(&id.to_string()).await;
    let _ = health::refresh(&state.db, org.org_id).await;
    let row = state
        .db
        .tracing_destination(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Destination not found"))?;
    Ok(Json(DestinationView::try_from(row).map_err(internal)?))
}
#[utoipa::path(delete,path="/v1/tracing/destinations/{id}",params(DestinationRevision,("id"=Uuid,Path,description="Destination ID")),responses((status=204)),tag="tracing")]
pub async fn delete_destination(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    Query(q): Query<DestinationRevision>,
) -> Result<StatusCode> {
    admin(&state, &org, true)?;
    let journal = RestChange::begin_with_context(
        state.ctx(&org),
        "delete_tracing_destination",
        EntityKind::TracingDestination,
        ChangeAction::Deleted,
        Some(&id.to_string()),
        &[],
    )
    .await
    .map_err(internal)?;
    if !state
        .db
        .delete_tracing_destination(org.org_id, id, q.revision)
        .await
        .map_err(|_| {
            error(
                StatusCode::CONFLICT,
                "Remove destination from routing policies before deleting",
            )
        })?
    {
        return Err(error(
            StatusCode::CONFLICT,
            "Destination changed or was removed",
        ));
    }
    journal.finish(&id.to_string()).await;
    let _ = health::refresh(&state.db, org.org_id).await;
    Ok(StatusCode::NO_CONTENT)
}
async fn checked_agent(state: &ApiState, org: &ResolvedOrg, agent: AgentId) -> Result<()> {
    let row = state
        .db
        .get_agent(org.org_id, agent)
        .await
        .map_err(internal)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Agent not found"))?;
    if row.is_built_in && row.name == crate::platform_chat_agent::NAME {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Personal Chat does not support organization trace exports",
        ));
    }
    Ok(())
}
async fn save_policy(
    state: &ApiState,
    org: &ResolvedOrg,
    agent: Option<Uuid>,
    body: SaveRoutingRequest,
) -> Result<Json<TracingPolicyView>> {
    admin(state, org, true)?;
    body.policy
        .validate()
        .map_err(|m| error(StatusCode::BAD_REQUEST, m))?;
    if agent.is_none() && body.policy.mode != RoutingMode::Replace {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Organization defaults use selected destinations",
        ));
    }
    let id = match agent {
        Some(a) => AgentId::from_uuid(a).to_string(),
        None => org.public_id.clone(),
    };
    commands::UpdateTracingPolicy {
        id,
        mode: body.policy.mode,
        destination_ids: body.policy.destination_ids,
        expected_revision: Some(body.revision),
    }
    .run(&state.ctx(org))
    .await
    .map_err(|e| error(e.status(), &e.to_string()))?;
    Ok(Json(
        state
            .db
            .tracing_policy(org.org_id, agent)
            .await
            .map_err(internal)?,
    ))
}
#[utoipa::path(get,path="/v1/tracing/defaults",responses((status=200,body=TracingPolicyView)),tag="tracing")]
pub async fn get_defaults(
    org: ResolvedOrg,
    State(s): State<ApiState>,
) -> Result<Json<TracingPolicyView>> {
    Ok(Json(
        s.db.tracing_policy(org.org_id, None)
            .await
            .map_err(internal)?,
    ))
}
#[utoipa::path(put,path="/v1/tracing/defaults",request_body=SaveRoutingRequest,responses((status=200,body=TracingPolicyView)),tag="tracing")]
pub async fn save_defaults(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Json(b): Json<SaveRoutingRequest>,
) -> Result<Json<TracingPolicyView>> {
    save_policy(&s, &org, None, b).await
}
#[utoipa::path(get,path="/v1/agents/{agent_id}/tracing",params(("agent_id"=String,Path,description="Agent ID")),responses((status=200,body=TracingPolicyView)),tag="tracing")]
pub async fn get_agent_routing(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(a): Path<AgentId>,
) -> Result<Json<TracingPolicyView>> {
    checked_agent(&s, &org, a).await?;
    Ok(Json(
        s.db.tracing_policy(org.org_id, Some(a.uuid()))
            .await
            .map_err(internal)?,
    ))
}
#[utoipa::path(put,path="/v1/agents/{agent_id}/tracing",params(("agent_id"=String,Path,description="Agent ID")),request_body=SaveRoutingRequest,responses((status=200,body=TracingPolicyView)),tag="tracing")]
pub async fn save_agent_routing(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(a): Path<AgentId>,
    Json(b): Json<SaveRoutingRequest>,
) -> Result<Json<TracingPolicyView>> {
    admin(&s, &org, true)?;
    checked_agent(&s, &org, a).await?;
    save_policy(&s, &org, Some(a.uuid()), b).await
}
#[utoipa::path(post,path="/v1/tracing/destinations/{id}/test",params(("id"=Uuid,Path,description="Destination ID")),responses((status=200,body=TestTraceResponse)),tag="tracing")]
pub async fn test_destination(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TestTraceResponse>> {
    admin(&state, &org, true)?;
    static TEST_EXPORTERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
    let _permit = TEST_EXPORTERS.try_acquire().map_err(|_| {
        error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many test exports; try again shortly",
        )
    })?;

    let row = state
        .db
        .tracing_destination(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Destination not found"))?;
    // Atomic cooldown prevents concurrent test requests from creating unlimited exporters.
    let claimed=sqlx::query("UPDATE tracing_destinations SET last_test_at=NOW() WHERE org_id=$1 AND id=$2 AND (last_test_at IS NULL OR last_test_at<NOW()-INTERVAL '10 seconds')").bind(org.org_id).bind(id).execute(state.db.database().pool()).await.map_err(internal)?.rows_affected();
    if claimed == 0 {
        return Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "Wait 10 seconds before testing again",
        ));
    }
    let encryption = state.encryption.clone().ok_or_else(|| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Secret encryption is not configured",
        )
    })?;
    let egress = state
        .ctx(&org)
        .egress_service
        .unwrap_or_else(|| state.capability_service.egress_service());
    let listener = router::make_listener(
        state.db.clone(),
        encryption,
        egress,
        row,
        EgressScope {
            org_id: Some(org.public_id.parse().map_err(internal)?),
            session_id: None,
        },
    )
    .await
    .map_err(internal)?;
    let tested_at = chrono::Utc::now();
    let turn = TurnId::new();
    let message = MessageId::new();
    let session = SessionId::new();
    let context = EventContext::turn(turn, message);
    listener
        .on_event(&Event::new(
            session,
            context.clone(),
            everruns_core::events::TurnStartedData {
                turn_id: turn,
                input_message_id: message,
                input_content: None,
                agent_id: None,
                agent_name: Some("Everruns connection test".into()),
                agent_description: None,
            },
        ))
        .await;
    listener
        .on_event(&Event::new(
            session,
            context,
            everruns_core::events::TurnCompletedData {
                turn_id: turn,
                iterations: 0,
                ..Default::default()
            },
        ))
        .await;
    let flushed = tokio::time::timeout(std::time::Duration::from_secs(15), listener.flush()).await;
    let updated = state
        .db
        .tracing_destination(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Destination removed"))?;
    let accepted = matches!(flushed, Ok(Ok(())))
        && updated.last_error.is_none()
        && updated.last_accepted_at.is_some_and(|at| at >= tested_at);
    Ok(Json(TestTraceResponse {
        accepted,
        message: if accepted {
            "Synthetic trace accepted by receiver"
        } else {
            "Test export failed; review destination delivery status"
        }
        .into(),
    }))
}

#[cfg(test)]
mod tests;
