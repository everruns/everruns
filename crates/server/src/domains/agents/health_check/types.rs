// Agent health check domain types (specs/agent-checks.md, tier-3).
//
// These types are both the API surface and the JSONB shape persisted on the
// `agent_health_check_runs` row (`summary` and `results` columns).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::storage::models::AgentHealthCheckRunRow;

/// A generated behavioral smoke-test case.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthCheckCase {
    /// Short name describing what the case probes.
    pub name: String,
    /// The user message sent to the agent.
    pub user_message: String,
    /// Natural-language description of what a good response looks like; used
    /// by the LLM judge to score the transcript.
    pub rubric: String,
}

/// Outcome of a single case after the agent ran and was scored.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthCheckCaseResult {
    pub name: String,
    pub user_message: String,
    pub rubric: String,
    /// Public ID of the real session created for this case (browsable in UI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// True only when both the deterministic checks and the LLM judge pass.
    pub passed: bool,
    /// LLM judge score, 0.0–1.0.
    pub score: f64,
    /// LLM judge explanation.
    pub judge_reason: String,
    /// Deterministic-check explanation (completion, non-empty, turn bound).
    pub deterministic_reason: String,
    pub turns: u32,
    pub latency_ms: u64,
    /// Set when the case errored or timed out instead of completing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregate metrics across all cases in a run.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthCheckSummary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub errored: u32,
    pub pass_rate: f64,
    pub avg_score: f64,
    pub avg_turns: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl HealthCheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// API view of a health check run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HealthCheckRun {
    /// Public ID (`healthcheck_…`).
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub config_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub status: HealthCheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<HealthCheckSummary>,
    /// Per-case results (present once the run has produced any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<HealthCheckCaseResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<AgentHealthCheckRunRow> for HealthCheckRun {
    fn from(row: AgentHealthCheckRunRow) -> Self {
        use everruns_core::typed_id::AgentId;
        Self {
            id: row.public_id,
            agent_id: row.agent_id.map(|id| AgentId::from_uuid(id).to_string()),
            config_hash: row.config_hash,
            model_id: row.model_id,
            status: HealthCheckStatus::parse(&row.status),
            summary: row.summary.and_then(|v| serde_json::from_value(v).ok()),
            results: row.results.and_then(|v| serde_json::from_value(v).ok()),
            error_message: row.error_message,
            created_at: row.created_at,
            completed_at: row.completed_at,
        }
    }
}
