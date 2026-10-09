// Observers domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::records::observer::{ObserverMatch, ObserverScorerConfig, ObserverStatus};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

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
