// Observer API routes, served by the generic `#[command(http = ..)]` handler — online scoring of production sessions.
// See knowledge/evaluation/online-evals.md. Gated behind the `observers` feature flag.

use crate::api::state::ApiState;
use axum::Router;
use serde::Deserialize;

use crate::records::observer::{ObserverMatch, ObserverScorerConfig, ObserverStatus};

use crate::api::command_http::CommandRouterExt;
use crate::domains::observers::{
    CreateObserver, DeleteObserver, GetObserver, ListObserverScores, ListObservers, UpdateObserver,
};

use utoipa::{IntoParams, ToSchema};

// ============================================
// Request/Response types
// ============================================

/// Request to create a new observer.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateObserverRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "Support quality")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Score replies from the support agent")]
    pub description: Option<String>,
    /// Which production sessions to score. Empty matches all org traffic.
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_config: Option<ObserverMatch>,
    /// Fraction of matching turns to score (0.0–1.0). Defaults to 0.1.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.25)]
    pub sampling_rate: Option<f64>,
    /// Scoring rules. Must contain at least one.
    pub scorers: Vec<ObserverScorerConfig>,
}

/// Request to update an observer. Omitted fields are unchanged.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateObserverRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "Support quality")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Score replies from the support agent")]
    pub description: Option<String>,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_config: Option<ObserverMatch>,
    /// Fraction of matching turns to score (0.0–1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.5)]
    pub sampling_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorers: Option<Vec<ObserverScorerConfig>>,
    /// New lifecycle status (`active`, `paused`, or `archived`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "paused")]
    pub status: Option<ObserverStatus>,
}

/// Query parameters for listing observers.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListObserversQuery {
    /// Include archived observers.
    pub include_archived: Option<bool>,
}

/// Query parameters for listing trace scores.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListTraceScoresQuery {
    /// Filter to one session.
    pub session_id: Option<String>,
    /// Maximum number of scores returned (default 100).
    pub limit: Option<i64>,
    /// Zero-based offset into the result set.
    pub offset: Option<i64>,
}

// ============================================
// Routes
// ============================================

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .command::<CreateObserver>()
        .command::<ListObservers>()
        .command::<GetObserver>()
        .command::<UpdateObserver>()
        .command::<DeleteObserver>()
        .command::<ListObserverScores>()
        .with_state(state)
}
