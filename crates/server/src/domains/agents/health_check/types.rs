// Agent health check domain types (knowledge/evaluation/agent-checks.md, tier-3).
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
    /// Input tokens for this case: the agent session turns plus the judge call.
    #[serde(default)]
    pub input_tokens: u32,
    /// Output tokens for this case: the agent session turns plus the judge call.
    #[serde(default)]
    pub output_tokens: u32,
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

/// A non-terminal run untouched for longer than this is treated as `failed`
/// when read. This is defense-in-depth for a process that crashed without
/// restarting (the startup reaper covers the restart case). The threshold
/// comfortably exceeds the runner's worst-case wall-clock budget — `updated_at`
/// is stamped when the run transitions to `running`, and a run finishes well
/// inside this window — so an in-flight run is never misreported as failed.
/// See knowledge/evaluation/agent-checks.md (durability) and EVE-586.
const STALE_AFTER_SECS: i64 = 30 * 60;

const STALE_ERROR_MESSAGE: &str =
    "Run stalled without completing (no progress past the staleness window).";

impl From<AgentHealthCheckRunRow> for HealthCheckRun {
    fn from(row: AgentHealthCheckRunRow) -> Self {
        use everruns_provider::typed_id::AgentId;
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

impl HealthCheckRun {
    /// Build the API view, applying the staleness guard relative to `now`.
    ///
    /// Read paths (`get`/`list`) use this so a run whose process crashed
    /// mid-flight surfaces as `failed` instead of a perpetual `running`
    /// spinner, even when the server has not restarted to run the boot reaper.
    pub fn from_row_at(row: AgentHealthCheckRunRow, now: chrono::DateTime<chrono::Utc>) -> Self {
        let last_progress = row.updated_at;
        let mut run = Self::from(row);
        // Derive "non-terminal" from the same parsed status the API presents
        // (rather than the raw DB string) so any unexpected status value is
        // judged for staleness consistently with how it is displayed.
        let non_terminal = matches!(
            run.status,
            HealthCheckStatus::Pending | HealthCheckStatus::Running
        );
        if non_terminal && now - last_progress > chrono::TimeDelta::seconds(STALE_AFTER_SECS) {
            run.status = HealthCheckStatus::Failed;
            run.error_message
                .get_or_insert_with(|| STALE_ERROR_MESSAGE.to_string());
            run.completed_at.get_or_insert(last_progress);
        }
        run
    }
}

/// The most recent health-check run for an agent, paired with whether the
/// agent's current resolved config differs from the config that run was
/// executed against. Returned by the latest-run endpoint so the agent editor
/// can show prior results on mount without triggering a new run, and surface a
/// "config changed since last run" hint. See knowledge/evaluation/agent-checks.md and EVE-588.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LatestHealthCheckRun {
    /// The latest run, or `None` if the agent has never been health-checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<HealthCheckRun>,
    /// True when `run` exists but was executed against a different resolved
    /// config than the agent currently has (UI shows a re-run hint).
    pub config_changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: &str, updated_minutes_ago: i64) -> AgentHealthCheckRunRow {
        let now = chrono::Utc::now();
        AgentHealthCheckRunRow {
            id: uuid::Uuid::now_v7(),
            org_id: 1,
            public_id: "healthcheck_00000000000000000000000000000001".to_string(),
            agent_id: None,
            config_hash: "cfg".to_string(),
            model_id: None,
            status: status.to_string(),
            summary: None,
            results: None,
            error_message: None,
            started_at: None,
            completed_at: None,
            created_at: now - chrono::TimeDelta::minutes(updated_minutes_ago),
            updated_at: now - chrono::TimeDelta::minutes(updated_minutes_ago),
        }
    }

    #[test]
    fn stale_running_run_is_reported_failed() {
        let now = chrono::Utc::now();
        let run = HealthCheckRun::from_row_at(row("running", 45), now);
        assert_eq!(run.status, HealthCheckStatus::Failed);
        assert!(run.error_message.is_some());
        assert!(run.completed_at.is_some());
    }

    #[test]
    fn fresh_running_run_is_left_running() {
        let now = chrono::Utc::now();
        let run = HealthCheckRun::from_row_at(row("running", 2), now);
        assert_eq!(run.status, HealthCheckStatus::Running);
        assert!(run.error_message.is_none());
    }

    #[test]
    fn terminal_run_is_never_remapped_even_when_old() {
        let now = chrono::Utc::now();
        let run = HealthCheckRun::from_row_at(row("completed", 600), now);
        assert_eq!(run.status, HealthCheckStatus::Completed);
    }
}
