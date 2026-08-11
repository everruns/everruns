// Observer API routes — online scoring of production sessions.
// See knowledge/evaluation/online-evals.md. Gated behind the `observers` feature flag.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;

use everruns_core::Caller;
use everruns_platform::observer::{
    Observer, ObserverMatch, ObserverScorerConfig, ObserverStatus, TraceScore,
};

use crate::api::common::{ApiResult, ErrorResponse, ListResponse};
use crate::api::dispatch::{Dispatchable, impl_dispatchable};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::Ctx;
use crate::domains::observers::{
    CreateObserver, DeleteObserver, GetObserver, ListObserverScores, ListObservers, UpdateObserver,
};
use crate::storage::StorageBackend;

use utoipa::{IntoParams, ToSchema};

// ============================================
// State
// ============================================

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

crate::api::common::impl_auth_state!(AppState);
impl_dispatchable!(AppState);

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
    }
}

// ============================================
// Request/Response types
// ============================================

/// Request to create a new observer.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateObserverRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    /// Which production sessions to score. Empty matches all org traffic.
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_config: Option<ObserverMatch>,
    /// Fraction of matching turns to score (0.0–1.0). Defaults to 0.1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling_rate: Option<f64>,
    /// Scoring rules. Must contain at least one.
    pub scorers: Vec<ObserverScorerConfig>,
}

/// Request to update an observer. Omitted fields are unchanged.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateObserverRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_config: Option<ObserverMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorers: Option<Vec<ObserverScorerConfig>>,
    /// New lifecycle status (`active`, `paused`, or `archived`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ObserverStatus>,
}

/// Query parameters for listing observers.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListObserversQuery {
    pub include_archived: Option<bool>,
}

/// Query parameters for listing trace scores.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListTraceScoresQuery {
    /// Filter to one session.
    pub session_id: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ============================================
// Routes
// ============================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/observers", post(create).get(list))
        .route(
            "/v1/observers/{observer_id}",
            get(get_observer).patch(update).delete(delete),
        )
        .route("/v1/observers/{observer_id}/scores", get(list_scores))
        .with_state(state)
}

// ============================================
// Handlers
// ============================================

async fn create(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateObserverRequest>,
) -> Result<(StatusCode, Json<Observer>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created(CreateObserver(req))
        .await
}

async fn list(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListObserversQuery>,
) -> ApiResult<ListResponse<Observer>> {
    state
        .dispatcher(&org)
        .run_list(ListObservers {
            include_archived: query.include_archived.unwrap_or(false),
        })
        .await
}

async fn get_observer(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(observer_id): Path<String>,
) -> ApiResult<Observer> {
    state
        .dispatcher(&org)
        .run(GetObserver { observer_id })
        .await
}

async fn update(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(observer_id): Path<String>,
    Json(req): Json<UpdateObserverRequest>,
) -> ApiResult<Observer> {
    state
        .dispatcher(&org)
        .run(UpdateObserver { observer_id, req })
        .await
}

async fn delete(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(observer_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(DeleteObserver { observer_id })
        .await
}

async fn list_scores(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(observer_id): Path<String>,
    Query(query): Query<ListTraceScoresQuery>,
) -> ApiResult<ListResponse<TraceScore>> {
    state
        .dispatcher(&org)
        .run_list(ListObserverScores {
            observer_id,
            session_id: query.session_id,
            limit: query.limit.unwrap_or(100),
            offset: query.offset.unwrap_or(0),
        })
        .await
}
