// Evals domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::domains::evals::record::*;
use everruns_contracts::typed_id::EvalResultId;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Request to create a new eval
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateEvalRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "Support agent regression")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Regression suite for the support agent")]
    pub description: Option<String>,
    /// Session setup target (harness+agent, app, or full session params).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    /// Default model override applied to runs of this eval.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-5.1")]
    pub model_override: Option<String>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
    #[schema(example = json!(["regression", "nightly"]))]
    pub tags: Option<Vec<String>>,
}

/// Request to update an eval
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateEvalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "Support agent regression")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Regression suite for the support agent")]
    pub description: Option<String>,
    /// Session setup target (harness+agent, app, or full session params).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    /// Default model override applied to runs of this eval.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-5.1")]
    pub model_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Free-form tags attached to this resource.
    #[schema(example = json!(["regression", "nightly"]))]
    pub tags: Option<Vec<String>>,
}

/// Request to create an eval case
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateEvalCaseRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "fix-failing-test")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Agent fixes a failing unit test")]
    pub description: Option<String>,
    /// Optional per-case target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
    #[schema(example = json!(["regression", "nightly"]))]
    pub tags: Option<Vec<String>>,
    /// Input messages sent to the agent sequentially.
    #[schema(example = json!([{"content": "Fix the failing test in src/lib.rs"}]))]
    pub conversation: Vec<EvalInputMessage>,
    /// Verification messages sent after conversation completes and session idles.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"content": "Run the tests again and report the result"}]))]
    pub post: Option<Vec<EvalInputMessage>>,
    /// Session files to capture after scoring completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"name": "patch", "path": "/workspace/fix.patch"}]))]
    pub artifacts: Option<Vec<ArtifactSpec>>,
    /// Scoring rules applied to the case output.
    #[schema(example = json!([{"type": "contains", "text": "tests pass"}]))]
    pub scorers: Vec<Scorer>,
    /// Maximum agent turns before the case stops.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 10)]
    pub max_turns: Option<u32>,
    /// Per-case timeout in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 120)]
    pub timeout_seconds: Option<u32>,
    /// Display order within the eval.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0)]
    pub position: Option<i32>,
}

/// Request to update an eval case
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateEvalCaseRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "fix-failing-test")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Agent fixes a failing unit test")]
    pub description: Option<String>,
    /// Optional per-case target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Free-form tags attached to this resource.
    #[schema(example = json!(["regression", "nightly"]))]
    pub tags: Option<Vec<String>>,
    /// Input messages sent to the agent sequentially.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"content": "Fix the failing test in src/lib.rs"}]))]
    pub conversation: Option<Vec<EvalInputMessage>>,
    /// Verification messages sent after conversation completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"content": "Run the tests again and report the result"}]))]
    pub post: Option<Vec<EvalInputMessage>>,
    /// Session files to capture after scoring completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"name": "patch", "path": "/workspace/fix.patch"}]))]
    pub artifacts: Option<Vec<ArtifactSpec>>,
    /// Scoring rules applied to the case output.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"type": "contains", "text": "tests pass"}]))]
    pub scorers: Option<Vec<Scorer>>,
    /// Maximum agent turns before the case stops.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 10)]
    pub max_turns: Option<u32>,
    /// Per-case timeout in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 120)]
    pub timeout_seconds: Option<u32>,
    /// Display order within the eval.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0)]
    pub position: Option<i32>,
}

/// Request to create an eval run
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateEvalRunRequest {
    /// Optional per-run target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    /// Model override for this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-5.1")]
    pub model_override: Option<String>,
}

/// Result status reported alongside externally computed scores.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalScoreStatus {
    Passed,
    Failed,
    Errored,
}

impl std::fmt::Display for ExternalScoreStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExternalScoreStatus::Passed => write!(f, "passed"),
            ExternalScoreStatus::Failed => write!(f, "failed"),
            ExternalScoreStatus::Errored => write!(f, "errored"),
        }
    }
}

/// Request to write external scores to one eval case result.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateEvalResultScoresRequest {
    /// Externally computed scores to store on the result.
    pub scores: Vec<Score>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current lifecycle status.
    pub status: Option<ExternalScoreStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Free-form metadata attached to this resource.
    #[schema(example = json!({"grader": "external-judge"}))]
    pub metadata: Option<serde_json::Value>,
}

/// Score update for one eval case result.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct BulkUpdateEvalResultScoresItem {
    /// Eval case result to update.
    #[schema(example = "evalresult_01933b5a000070008000000000000001", value_type = String)]
    pub result_id: EvalResultId,
    /// Externally computed scores to store on the result.
    pub scores: Vec<Score>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current lifecycle status.
    pub status: Option<ExternalScoreStatus>,
}

/// Request to write external scores to several results of an eval run.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct BulkUpdateEvalRunScoresRequest {
    /// Per-result score updates applied together.
    pub results: Vec<BulkUpdateEvalResultScoresItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Free-form metadata attached to this resource.
    #[schema(example = json!({"grader": "external-judge"}))]
    pub metadata: Option<serde_json::Value>,
}

// Import (external eval results): everruns as host/viewer.
// See knowledge/evaluation/evals.md.

/// A whole external run group: one external run, one entry per eval. Maps to
/// one everruns EvalRun per eval, all sharing `source.run_id`.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct ImportEvalRunRequest {
    /// External system that produced the run.
    pub source: ImportEvalSource,
    /// Evals and their case results in this run.
    pub evals: Vec<ImportEvalGroup>,
}

/// Attribution for the external system that produced the run.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct ImportEvalSource {
    /// External system name, e.g. "mira".
    #[schema(example = "mira")]
    pub system: String,
    /// Version of the external system.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "0.4.0")]
    pub version: Option<String>,
    /// Link back to the run in the external system.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://ci.example.com/runs/42")]
    pub url: Option<String>,
    /// Stable external run id: cross-eval group key + idempotency key.
    #[schema(example = "run-2026-01-15-001")]
    pub run_id: String,
    /// Optional environment/labels (git commit, host, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"git_commit": "a1b2c3d"}))]
    pub metadata: Option<serde_json::Value>,
}

/// One eval's worth of results within the run. The eval is upserted by `name`.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct ImportEvalGroup {
    /// Eval name; the eval is upserted by it.
    #[schema(example = "Support agent regression")]
    pub name: String,
    /// Optional eval description.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Regression suite for the support agent")]
    pub description: Option<String>,
    /// Free-form tags for the eval.
    #[serde(default)]
    #[schema(example = json!(["regression", "nightly"]))]
    pub tags: Vec<String>,
    /// Case results for this eval.
    pub cases: Vec<ImportEvalCaseEntry>,
}

/// One case result. The case is upserted by `name` (identity-only: everruns
/// never re-executes it).
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct ImportEvalCaseEntry {
    /// Case name; the case is upserted by it.
    #[schema(example = "fix-failing-test")]
    pub name: String,
    /// Optional case description.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Agent fixes a failing unit test")]
    pub description: Option<String>,
    /// Display-only input turns shown in the UI.
    #[serde(default)]
    #[schema(example = json!(["Fix the failing test in src/lib.rs"]))]
    pub input: Vec<String>,
    /// Provider/model labels this result was produced against.
    pub target: ImportEvalTarget,
    /// Verdict for the case, trusted as reported.
    pub status: ImportCaseStatus,
    /// Named, attributed scores. Stored opaque; everruns does not re-grade.
    #[serde(default)]
    pub scores: Vec<ImportScore>,
    /// Normalized transcript (messages, tool calls, events, parts, files).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"messages": []}))]
    pub transcript: Option<serde_json::Value>,
    /// Open-vocab metrics bag (cost_usd, cache/reasoning tokens, ttft, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"cost_usd": 0.02}))]
    pub metrics: Option<serde_json::Value>,
    /// Number of agent turns taken.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 3)]
    pub turns: Option<u32>,
    /// Execution time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 8450)]
    pub latency_ms: Option<u64>,
    /// Input tokens used.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 1200)]
    pub input_tokens: Option<u64>,
    /// Output tokens used.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 850)]
    pub output_tokens: Option<u64>,
    /// Error detail when the case errored.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Session timed out")]
    pub error_message: Option<String>,
}

/// Provider/model labels for an externally-executed result.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct ImportEvalTarget {
    /// Provider name.
    #[schema(example = "openai")]
    pub provider: String,
    /// Model name.
    #[schema(example = "gpt-5.1")]
    pub model: String,
    /// Opaque provider parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"temperature": 0.2}))]
    pub params: Option<serde_json::Value>,
}

/// Verdict for an imported case (trusted as-is; not recomputed).
#[derive(Debug, Clone, Copy, Deserialize, ToSchema, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportCaseStatus {
    Passed,
    Failed,
    Errored,
    Timeout,
    Skipped,
}

impl ImportCaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImportCaseStatus::Passed => "passed",
            ImportCaseStatus::Failed => "failed",
            ImportCaseStatus::Errored => "errored",
            ImportCaseStatus::Timeout => "timeout",
            ImportCaseStatus::Skipped => "skipped",
        }
    }
}

/// A single named score from an external scorer.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ImportScore {
    /// Scorer name.
    #[schema(example = "contains")]
    pub scorer: String,
    /// Score value from 0.0 to 1.0.
    #[schema(example = 1.0)]
    pub value: f64,
    /// Whether the scorer passed.
    #[schema(example = true)]
    pub pass: bool,
    /// Human-readable explanation of the score.
    #[serde(default)]
    #[schema(example = "Output contains expected text")]
    pub reason: String,
    /// Scorer was not applicable (excluded from aggregate).
    #[serde(default)]
    #[schema(example = false)]
    pub na: bool,
}

/// Result of an ATIF trajectory import: eval cases
/// created/updated from imported trajectories, upserted by case name.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AtifImportReport {
    /// Number of eval cases created.
    #[schema(example = 3)]
    pub created: u64,
    /// Number of existing eval cases updated (matched by name).
    #[schema(example = 1)]
    pub updated: u64,
    /// Public ids of the affected cases, in import order.
    #[schema(example = json!(["evalcase_01933b5a000070008000000000000001"]))]
    pub case_ids: Vec<String>,
}

/// Preflight capability report so optional-feature clients (e.g. Mira) can
/// check before publishing instead of failing mid-import.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvalImportPreflight {
    /// Whether the `evals` feature is enabled for this org.
    #[schema(example = true)]
    pub evals_enabled: bool,
    /// Whether the caller may import (holds eval-management permission).
    #[schema(example = true)]
    pub can_import: bool,
}

/// Query parameters for listing evals
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListEvalsQuery {
    /// Case-insensitive name filter.
    pub search: Option<String>,
    /// Include archived evals.
    pub include_archived: Option<bool>,
}

// Share links (read-only public views).
// See knowledge/evaluation/evals.md, knowledge/execution/public-endpoints.md.

/// A freshly minted share link. The raw `token` is returned once and never
/// stored; build the public URL `/shared/eval-runs/<token>` from it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvalRunShareLink {
    /// Raw share token, returned once and never stored.
    #[schema(
        example = "evr_share_3f9a1c7e5b2d4086a1f3c9e7b5d2408613579bdf02468ace13579bdf02468ace"
    )]
    pub token: String,
    /// Short, non-secret prefix identifying the token.
    #[schema(example = "evr_share_3f9a1c7e...")]
    pub token_prefix: String,
    /// When the share link was created.
    #[schema(example = "2026-01-15T10:30:00Z")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Whether a run currently has an active share link.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvalRunShareStatus {
    /// Whether an active share link exists for the run.
    #[schema(example = true)]
    pub active: bool,
}

/// Attribution shown on a public share (external runs). Display fields only.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PublicAttribution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Sanitized, anonymous view of one eval run, returned by the public share
/// endpoint. Omits org/internal ids, session ids, internal targets, and
/// attribution env labels — only the shared content remains.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PublicEvalRun {
    pub id: String,
    pub status: EvalRunStatus,
    pub source: EvalRunSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<PublicAttribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub results: Vec<PublicEvalCaseResult>,
}

/// One case result in a public run view.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PublicEvalCaseResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_name: Option<String>,
    /// Only label-only (external) targets are exposed; internal targets are dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    pub status: CaseResultStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scores: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}
