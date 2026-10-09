// Rows the evals repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Eval row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct EvalRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Input for creating an eval
#[derive(Debug, Clone)]
pub struct CreateEvalRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub tags: Vec<String>,
}

/// Input for updating an eval
#[derive(Debug, Clone, Default)]
pub struct UpdateEvalRow {
    pub name: Option<String>,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
}

/// Eval case row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct EvalCaseRow {
    pub id: Uuid,
    pub eval_id: Uuid,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub conversation: serde_json::Value,
    pub post: Option<serde_json::Value>,
    pub artifacts: Option<serde_json::Value>,
    pub scorers: serde_json::Value,
    pub max_turns: Option<i32>,
    pub timeout_seconds: Option<i32>,
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an eval case
#[derive(Debug, Clone)]
pub struct CreateEvalCaseRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub conversation: serde_json::Value,
    pub post: Option<serde_json::Value>,
    pub artifacts: Option<serde_json::Value>,
    pub scorers: serde_json::Value,
    pub max_turns: Option<i32>,
    pub timeout_seconds: Option<i32>,
    pub position: i32,
}

/// Input for updating an eval case
#[derive(Debug, Clone, Default)]
pub struct UpdateEvalCaseRow {
    pub name: Option<String>,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
    pub conversation: Option<serde_json::Value>,
    pub post: Option<serde_json::Value>,
    pub artifacts: Option<serde_json::Value>,
    pub scorers: Option<serde_json::Value>,
    pub max_turns: Option<i32>,
    pub timeout_seconds: Option<i32>,
    pub position: Option<i32>,
}

/// Eval run row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct EvalRunRow {
    pub id: Uuid,
    pub eval_id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub filter_tags: Option<Vec<String>>,
    pub status: String,
    pub triggered_by: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub summary: Option<serde_json::Value>,
    /// 'internal' (everruns executed) or 'external' (imported). See migration 086.
    pub source: String,
    /// External system's run id: cross-eval group key and idempotency key.
    pub source_run_id: Option<String>,
    /// Open-vocab provenance for external runs (system, version, url, labels).
    pub attribution: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A read-only share token for an eval run (migration 091). The raw token is
/// never stored — only its hash. `revoked_at`/`expires_at` disable a link.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct EvalRunShareTokenRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub eval_run_id: Uuid,
    pub token_hash: String,
    pub token_prefix: String,
    pub created_by: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for minting an eval-run share token.
#[derive(Debug, Clone)]
pub struct CreateEvalRunShareTokenRow {
    pub public_id: String,
    pub org_id: i64,
    pub eval_run_id: Uuid,
    pub token_hash: String,
    pub token_prefix: String,
    pub created_by: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Input for creating an eval run
#[derive(Debug, Clone)]
pub struct CreateEvalRunRow {
    pub public_id: String,
    pub eval_id: Uuid,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub filter_tags: Option<Vec<String>>,
    pub triggered_by: String,
}

/// Typed failures from `create_eval_run_with_case_results`. Returned (boxed in
/// `anyhow`) so the service layer can `downcast_ref` to map them to HTTP 400
/// instead of relying on brittle error-message substring matching.
#[derive(Debug, thiserror::Error)]
pub enum CreateEvalRunError {
    #[error("Too many concurrent eval runs: org has {active} active runs (limit {limit})")]
    TooManyConcurrentRuns { active: i64, limit: usize },
    #[error("Eval run too large: {cases} cases exceeds the per-run limit of {limit}")]
    TooManyCases { cases: usize, limit: usize },
    #[error("no eval target configured — set a target on the eval, case, or run")]
    NoTarget,
}

/// Eval case result row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct EvalCaseResultRow {
    pub id: Uuid,
    pub eval_run_id: Uuid,
    pub eval_case_id: Uuid,
    pub public_id: String,
    pub session_id: Option<Uuid>,
    pub target: Option<serde_json::Value>,
    pub target_snapshot: Option<serde_json::Value>,
    pub status: String,
    pub scores: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub turns: Option<i32>,
    pub latency_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub error_message: Option<String>,
    pub artifacts: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an eval case result
#[derive(Debug, Clone)]
pub struct CreateEvalCaseResultRow {
    pub public_id: String,
    pub eval_run_id: Uuid,
    pub eval_case_id: Uuid,
    pub target: Option<serde_json::Value>,
    pub target_snapshot: Option<serde_json::Value>,
    pub artifacts: Option<serde_json::Value>,
}

/// Input for importing one externally-executed eval run (a single eval's worth
/// of a Mira-style run group). The storage layer upserts the eval and its cases
/// by name, replaces any prior run sharing `source_run_id`, then writes a
/// completed external run with fully-populated results. See `import_eval_run`.
#[derive(Debug, Clone)]
pub struct ImportEvalRunInput {
    pub eval_name: String,
    pub eval_description: Option<String>,
    pub eval_tags: Vec<String>,
    pub run_public_id: String,
    pub source: String,
    pub source_run_id: String,
    pub attribution: Option<serde_json::Value>,
    pub triggered_by: String,
    pub summary: Option<serde_json::Value>,
    pub cases: Vec<ImportEvalCaseInput>,
}

/// One case-result within an imported external run. The case is identity-only
/// (name + optional display conversation); everruns never re-executes it, so
/// scorers are left empty. Transcript and open-vocab metrics ride inside
/// `metadata` rather than dedicated columns.
#[derive(Debug, Clone)]
pub struct ImportEvalCaseInput {
    pub case_name: String,
    pub case_description: Option<String>,
    pub conversation: serde_json::Value,
    pub target_snapshot: Option<serde_json::Value>,
    pub status: String,
    pub scores: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub turns: Option<i32>,
    pub latency_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub error_message: Option<String>,
    pub artifacts: Option<serde_json::Value>,
}

/// Input for updating an eval case result
#[derive(Debug, Clone, Default)]
pub struct UpdateEvalCaseResultRow {
    pub session_id: Option<Uuid>,
    pub target: Option<serde_json::Value>,
    pub target_snapshot: Option<serde_json::Value>,
    pub status: Option<String>,
    pub scores: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub turns: Option<i32>,
    pub latency_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub error_message: Option<String>,
    pub artifacts: Option<serde_json::Value>,
}

/// Async dataset-export handle row from database.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct EvalRunDatasetRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub eval_run_id: Option<Uuid>,
    pub request: serde_json::Value,
    pub status: String,
    pub body: Option<String>,
    pub record_count: Option<i64>,
    pub error_message: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an eval run dataset export handle.
#[derive(Debug, Clone)]
pub struct CreateEvalRunDatasetRow {
    pub public_id: String,
    pub eval_run_id: Uuid,
    pub request: serde_json::Value,
}

/// Input for updating an eval run dataset export handle. `None` fields are left
/// unchanged; `started_at`/`completed_at` are set from `status` transitions.
#[derive(Debug, Clone, Default)]
pub struct UpdateEvalRunDatasetRow {
    pub status: Option<String>,
    pub body: Option<String>,
    pub record_count: Option<i64>,
    pub error_message: Option<String>,
}
