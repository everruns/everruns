//! Late usage of OpenAI Agents API generations (EVE-1145).
//!
//! The provider fills a turn's usage after `turn.completed`. A generation
//! billed before its usage arrived is recorded with zero tokens and
//! `usage_pending`, and the reconciler
//! ([`crate::services::agents_api_usage`]) applies the usage once the
//! provider reports it. See `knowledge/execution/openai-agents-api-runtime.md`.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// A generation billed while its token usage was still unknown.
#[derive(Clone, Debug)]
pub struct CreatePendingUsageGeneration {
    pub org_id: i64,
    pub session_id: Uuid,
    pub turn_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub model: String,
    pub provider: Option<String>,
    /// The components that were priced without the tokens (per-call tools).
    pub estimated_cost_usd: Option<f64>,
    pub duration_ms: Option<i32>,
    pub finish_reason: Option<String>,
    /// The provider turn id.
    pub provider_response_id: String,
    pub provider_session_id: String,
    /// Public id of the Everruns provider whose credentials ran the turn.
    pub provider_config_id: String,
    pub created_at: DateTime<Utc>,
}

/// A generation whose usage is still pending and ready for another read.
#[derive(Clone, Debug, PartialEq, FromRow)]
pub struct PendingUsageGeneration {
    pub id: Uuid,
    pub org_id: i64,
    pub session_id: Option<Uuid>,
    pub turn_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub provider_response_id: String,
    pub provider_session_id: String,
    pub provider_config_id: String,
}

/// Usage the provider reported late, in Everruns' disjoint token buckets.
#[derive(Clone, Debug, PartialEq)]
pub struct LateGenerationUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    /// Price-table cost of the tokens alone; `None` when the model is
    /// unpriced (budgets then fall back to their token meter).
    pub estimated_cost_usd: Option<f64>,
}

/// What a generation record holds for its usage.
#[derive(Clone, Debug, PartialEq, FromRow)]
pub struct GenerationUsageSnapshot {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub estimated_cost_usd: Option<f64>,
    pub usage_pending: bool,
    pub reconciliation_attempts: i32,
}
