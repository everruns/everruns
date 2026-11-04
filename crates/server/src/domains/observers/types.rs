// Observers domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::domains::observers::record::{
    ObserverAlert, ObserverMatch, ObserverScorerConfig, ObserverStatus,
};
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
    /// Pass-rate alert: opens a health issue and notifies the agent's owners
    /// when the observer's pass rate falls under a threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<ObserverAlert>,
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
    /// Only completed scores nobody has reviewed yet, failures first, then a
    /// random sample of the rest (the review queue).
    pub review_queue: Option<bool>,
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
    /// Pass-rate alert. `null` removes it.
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<ObserverAlert>)]
    pub alert: Option<Option<ObserverAlert>>,
}

/// Present-but-null is `Some(None)`, absent is `None`.
fn double_option<'de, T: serde::Deserialize<'de>, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(de).map(Some)
}

/// A person's verdict on one score.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct ReviewScoreRequest {
    /// True when the grader got it right.
    pub agrees: bool,
    /// Optional note on why (at most 2000 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Score past production traffic with an observer.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct BackfillObserverRequest {
    /// How far back to read, in hours (1–720). Defaults to 24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 24)]
    pub since_hours: Option<u32>,
    /// Most past events to read, newest first (1–5000). Defaults to 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 500)]
    pub limit: Option<u32>,
    /// Apply the observer's sampling rate. Defaults to true; false scores
    /// every matching slice read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<bool>,
}

/// Query parameters for the live-quality summary.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct QualitySummaryQuery {
    /// Window in days (1–90). Defaults to 7.
    pub days: Option<u32>,
    /// Only scores of this agent.
    pub agent_id: Option<String>,
    /// Only scores of this observer.
    pub observer_id: Option<String>,
}
