// Eval domain types (EVE-878).
//
// Decision: Evals are product management/reporting aggregates — persisted
// definitions, runs, results, and dataset exports owned by the hosted control
// plane. They configure and observe sessions but never participate directly
// in a turn, so they moved here from `everruns-core`. The eval runner lives in
// `crates/server/src/domains/evals/`; there is no focused eval crate in the
// workspace, so the platform crate owns the records.
//
// Design Decision: Evals are user-facing behavioral tests for agents.
// Each eval case creates a real session — same behavior as production, debuggable.
// Scorers return 0.0–1.0 (not binary) to support nuanced grading.
//
// Design Decision: EvalTarget is the session setup contract.
// Resolution order: EvalRun.target → EvalCase.target → Eval.target → org default harness.
// EvalTarget::Session mirrors CreateSessionRequest params; EvalTarget::App references a deployed app.
// EvalCaseResult stores both a live reference and a frozen snapshot for reproducibility.
//
// See knowledge/evaluation/evals.md for full specification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use everruns_provider::typed_id::{
    AgentId, AppId, EvalCaseId, EvalDatasetId, EvalId, EvalResultId, EvalRunId, HarnessId, ModelId,
    SessionId,
};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Named session file to collect after an eval case completes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ArtifactSpec {
    /// Export key for this artifact (for example `patch` or `log`).
    pub name: String,
    /// Absolute path in the session filesystem.
    pub path: String,
}

// ============================================
// Eval Target
// ============================================

/// Defines how to instantiate a session for an eval case.
///
/// Two modes:
/// - `Session`: mirrors `CreateSessionRequest` — full control over session creation parameters.
/// - `App`: references a deployed app by ID.
///
/// Resolution order: EvalRun.target → EvalCase.target → Eval.target → org default harness.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvalTarget {
    /// Session creation parameters (mirrors CreateSessionRequest).
    Session {
        /// Harness for the session. If omitted, org default harness is used.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
        harness_id: Option<HarnessId>,
        /// Addressable harness name (alternative to harness_id).
        #[serde(skip_serializing_if = "Option::is_none")]
        harness_name: Option<String>,
        /// Agent to work in this session.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
        agent_id: Option<AgentId>,
        /// LLM model override.
        #[serde(skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        /// System prompt override (prepended to agent prompt).
        #[serde(skip_serializing_if = "Option::is_none")]
        system_prompt: Option<String>,
        /// Max LLM iterations per turn.
        #[serde(skip_serializing_if = "Option::is_none")]
        max_iterations: Option<usize>,
    },
    /// Reference to a deployed app.
    App {
        #[cfg_attr(feature = "openapi", schema(value_type = String))]
        app_id: AppId,
    },
    /// Label-only target for externally-executed runs (e.g. imported from Mira).
    ///
    /// Carries provider/model labels and opaque params instead of session setup:
    /// external runs are ingested already-complete, so everruns never builds a
    /// session from this. Mirrors a provider-agnostic `(provider, model)` pair.
    External {
        provider: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<serde_json::Value>,
    },
}

// ============================================
// Eval Status
// ============================================

/// Eval lifecycle status (standard building-block lifecycle).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum EvalStatus {
    Active,
    Archived,
    Deleted,
}

impl std::fmt::Display for EvalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalStatus::Active => write!(f, "active"),
            EvalStatus::Archived => write!(f, "archived"),
            EvalStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for EvalStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => EvalStatus::Archived,
            "deleted" => EvalStatus::Deleted,
            _ => EvalStatus::Active,
        }
    }
}

// ============================================
// Eval Run Status
// ============================================

/// Status of an eval run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum EvalRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for EvalRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalRunStatus::Pending => write!(f, "pending"),
            EvalRunStatus::Running => write!(f, "running"),
            EvalRunStatus::Completed => write!(f, "completed"),
            EvalRunStatus::Failed => write!(f, "failed"),
            EvalRunStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl From<&str> for EvalRunStatus {
    fn from(s: &str) -> Self {
        match s {
            "running" => EvalRunStatus::Running,
            "completed" => EvalRunStatus::Completed,
            "failed" => EvalRunStatus::Failed,
            "cancelled" => EvalRunStatus::Cancelled,
            _ => EvalRunStatus::Pending,
        }
    }
}

// ============================================
// Eval Run Source
// ============================================

/// Where an eval run came from.
///
/// `Internal` runs are executed by everruns (sessions spawned per case).
/// `External` runs are ingested already-complete from an external eval system
/// (e.g. Mira) via the import API; everruns hosts and visualizes them but never
/// executes them. See knowledge/evaluation/evals.md.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum EvalRunSource {
    #[default]
    Internal,
    External,
}

impl std::fmt::Display for EvalRunSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalRunSource::Internal => write!(f, "internal"),
            EvalRunSource::External => write!(f, "external"),
        }
    }
}

impl From<&str> for EvalRunSource {
    fn from(s: &str) -> Self {
        match s {
            "external" => EvalRunSource::External,
            _ => EvalRunSource::Internal,
        }
    }
}

// ============================================
// Case Result Status
// ============================================

/// Status of an individual eval case result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum CaseResultStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Errored,
    Timeout,
    /// Case was not executed (e.g. an external system skipped it: model
    /// unavailable, filtered out). Excluded from pass/fail tallies.
    Skipped,
}

impl std::fmt::Display for CaseResultStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaseResultStatus::Pending => write!(f, "pending"),
            CaseResultStatus::Running => write!(f, "running"),
            CaseResultStatus::Passed => write!(f, "passed"),
            CaseResultStatus::Failed => write!(f, "failed"),
            CaseResultStatus::Errored => write!(f, "errored"),
            CaseResultStatus::Timeout => write!(f, "timeout"),
            CaseResultStatus::Skipped => write!(f, "skipped"),
        }
    }
}

impl From<&str> for CaseResultStatus {
    fn from(s: &str) -> Self {
        match s {
            "running" => CaseResultStatus::Running,
            "passed" => CaseResultStatus::Passed,
            "failed" => CaseResultStatus::Failed,
            "errored" => CaseResultStatus::Errored,
            "timeout" => CaseResultStatus::Timeout,
            "skipped" => CaseResultStatus::Skipped,
            _ => CaseResultStatus::Pending,
        }
    }
}

// ============================================
// Scorer types
// ============================================

/// A scoring rule applied to eval case output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Scorer {
    /// Final assistant message contains substring.
    Contains {
        text: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Final assistant message does NOT contain substring.
    NotContains {
        text: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Final assistant message matches regex pattern.
    Regex {
        pattern: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Agent called named tool at least `min` times.
    ToolCalled {
        tool: String,
        #[serde(default = "default_min_one")]
        min: u32,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Agent did NOT call named tool.
    ToolNotCalled {
        tool: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Total tool calls within range.
    ToolCallCount {
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<u32>,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Completed within N turns.
    TurnsWithin {
        max: u32,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Session filesystem file contains substring.
    FileContains {
        path: String,
        text: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Final assistant message parses as JSON matching schema.
    JsonSchema {
        schema: serde_json::Value,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Citation faithfulness: the answer's citations (see `knowledge/runtime-resources/citations.md`)
    /// must cover the claim and be verified as supported. Scored from the
    /// `TextAnnotation`s on the final message — pair with the
    /// `citation_verification` capability so verdicts are present.
    CitationFaithful {
        /// Minimum number of citations the answer must carry.
        #[serde(default)]
        #[cfg_attr(feature = "openapi", schema(example = 1))]
        min_citations: u32,
        /// Minimum fraction of citations verified `entailed` to pass.
        #[serde(default = "default_pass_threshold")]
        #[cfg_attr(feature = "openapi", schema(example = 0.8))]
        pass_threshold: f64,
        /// Relative weight of this scorer in the case's weighted average.
        #[serde(default = "default_weight")]
        #[cfg_attr(feature = "openapi", schema(example = 1.0))]
        weight: f64,
    },
    /// Citation faithfulness judged by an LLM: each cited claim/source pair is
    /// graded by a model, so the eval works even without the
    /// `citation_verification` capability. See `knowledge/runtime-resources/citations.md`.
    CitationJudged {
        /// Rubric override; a citation-faithfulness rubric is used when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            feature = "openapi",
            schema(example = "Score the fraction of cited claims supported by their source.")
        )]
        rubric: Option<String>,
        /// Judge model; the org's default is used when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<ModelId>,
        /// Minimum judged score `[0,1]` to pass.
        #[serde(default = "default_pass_threshold")]
        #[cfg_attr(feature = "openapi", schema(example = 0.8))]
        pass_threshold: f64,
        /// Relative weight of this scorer in the case's weighted average.
        #[serde(default = "default_weight")]
        #[cfg_attr(feature = "openapi", schema(example = 1.0))]
        weight: f64,
    },
}

fn default_pass_threshold() -> f64 {
    0.8
}

impl Scorer {
    /// Stable kind tag for this scorer, matching the serde `type` discriminant.
    ///
    /// Scores are persisted as an ordered `Vec<Score>` that carries no scorer
    /// identity, so consumers that need a name (e.g. dataset export) join this
    /// positionally against the case's `scorers`. Keep this exhaustive so adding
    /// a variant forces a decision here.
    pub fn kind(&self) -> &'static str {
        match self {
            Scorer::Contains { .. } => "contains",
            Scorer::NotContains { .. } => "not_contains",
            Scorer::Regex { .. } => "regex",
            Scorer::ToolCalled { .. } => "tool_called",
            Scorer::ToolNotCalled { .. } => "tool_not_called",
            Scorer::ToolCallCount { .. } => "tool_call_count",
            Scorer::TurnsWithin { .. } => "turns_within",
            Scorer::FileContains { .. } => "file_contains",
            Scorer::JsonSchema { .. } => "json_schema",
            Scorer::CitationFaithful { .. } => "citation_faithful",
            Scorer::CitationJudged { .. } => "citation_judged",
        }
    }
}

fn default_weight() -> f64 {
    1.0
}

fn default_min_one() -> u32 {
    1
}

// ============================================
// Score result
// ============================================

/// Result from a single scorer evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Score {
    /// Whether this scorer passed.
    pub pass: bool,
    /// Score value 0.0–1.0.
    pub value: f64,
    /// Human-readable explanation.
    pub reason: String,
}

// ============================================
// Run summary
// ============================================

/// Aggregate metrics for a completed eval run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RunSummary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub errored: u32,
    pub pass_rate: f64,
    pub avg_score: f64,
    pub avg_turns: f64,
    pub avg_latency_ms: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

// ============================================
// Input message for eval cases
// ============================================

/// A message to send to the agent during an eval case.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EvalInputMessage {
    /// The text content to send.
    pub content: String,
}

// ============================================
// Main entity structs
// ============================================

/// An eval: a named collection of test cases for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Eval {
    /// External identifier (eval_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "eval_01933b5a000070008000000000000001"))]
    pub public_id: EvalId,
    /// Internal UUID primary key. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Organization ID. Internal only.
    #[serde(skip, default)]
    pub org_id: i64,
    /// Display name.
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Session setup target. Defines how to create sessions for eval cases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    /// Optional default model override for runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Organization tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Lifecycle status.
    pub status: EvalStatus,
    /// Number of cases.
    #[serde(default)]
    pub case_count: i64,
    /// Last run summary (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<EvalRunSummaryView>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Compact run summary for listing evals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EvalRunSummaryView {
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub public_id: EvalRunId,
    pub status: EvalRunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
    pub created_at: DateTime<Utc>,
}

/// A single test case within an eval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EvalCase {
    /// External identifier (evalcase_<32-hex>).
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "evalcase_01933b5a000070008000000000000001"))]
    pub public_id: EvalCaseId,
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional per-case target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Input messages sent sequentially.
    pub conversation: Vec<EvalInputMessage>,
    /// Verification messages sent after conversation completes and session idles.
    /// Scorers run after post messages complete (not after conversation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post: Option<Vec<EvalInputMessage>>,
    /// Session files to collect after scoring completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<ArtifactSpec>>,
    /// Scoring rules.
    pub scorers: Vec<Scorer>,
    /// Max agent turns (default: 10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// Per-case timeout in seconds (default: 120).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// Display order.
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An eval run: one execution of all/some cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EvalRun {
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "evalrun_01933b5a000070008000000000000001"))]
    pub public_id: EvalRunId,
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    #[serde(skip, default)]
    pub org_id: i64,
    /// Optional per-run target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    /// Model override for this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Only run cases matching these tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_tags: Option<Vec<String>>,
    pub status: EvalRunStatus,
    /// Whether everruns executed this run (`internal`) or it was imported from
    /// an external eval system (`external`).
    #[serde(default)]
    pub source: EvalRunSource,
    /// Provenance for external runs: which system produced them, version, link
    /// back, and any environment labels. `None` for internal runs. Open-vocab
    /// JSON so new attribution fields need no schema change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<serde_json::Value>,
    /// What triggered this run.
    pub triggered_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Aggregate metrics (set on completion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
    /// Case results (populated on detail view).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<EvalCaseResult>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// Eval Run Dataset (async dataset export — knowledge/evaluation/dataset-export.md)
// ============================================

/// Status of an async dataset export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum EvalDatasetStatus {
    /// Enqueued, export not started yet.
    Pending,
    /// Export in progress.
    Running,
    /// Export finished; NDJSON `body` is available on the detail view.
    Completed,
    /// Export failed; see `error_message`.
    Failed,
}

impl std::fmt::Display for EvalDatasetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalDatasetStatus::Pending => write!(f, "pending"),
            EvalDatasetStatus::Running => write!(f, "running"),
            EvalDatasetStatus::Completed => write!(f, "completed"),
            EvalDatasetStatus::Failed => write!(f, "failed"),
        }
    }
}

impl From<&str> for EvalDatasetStatus {
    fn from(s: &str) -> Self {
        match s {
            "running" => EvalDatasetStatus::Running,
            "completed" => EvalDatasetStatus::Completed,
            "failed" => EvalDatasetStatus::Failed,
            _ => EvalDatasetStatus::Pending,
        }
    }
}

/// An async dataset-export handle: the durable result of enqueuing a dataset
/// export from a completed eval run. The `body` (NDJSON) is only populated on
/// the `GET .../dataset/{dataset_id}` detail view once `status` is `completed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EvalRunDataset {
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "evaldataset_01933b5a000070008000000000000001"))]
    pub public_id: EvalDatasetId,
    /// The eval run this dataset was exported from.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub eval_run_id: EvalRunId,
    pub status: EvalDatasetStatus,
    /// Number of NDJSON records (surviving cases). Set on completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_count: Option<u64>,
    /// Failure detail when `status` is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// The produced NDJSON. Only present on the detail view once completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Result of a single case within a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EvalCaseResult {
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "evalresult_01933b5a000070008000000000000001"))]
    pub public_id: EvalResultId,
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// The case this result is for.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub eval_case_id: EvalCaseId,
    /// Case name (denormalized for display).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_name: Option<String>,
    /// Session created for this case (browsable in UI).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub session_id: Option<SessionId>,
    /// Resolved target used for this result (live reference).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    /// Frozen snapshot of the resolved target at execution time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_snapshot: Option<EvalTarget>,
    pub status: CaseResultStatus,
    /// Per-scorer results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scores: Option<serde_json::Value>,
    /// External scorer metadata captured during deferred write-back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Turn count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    /// Execution time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Token usage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Error message if errored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Collected session file contents keyed by artifact name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<BTreeMap<String, String>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eval_status_display() {
        assert_eq!(EvalStatus::Active.to_string(), "active");
        assert_eq!(EvalStatus::Archived.to_string(), "archived");
        assert_eq!(EvalStatus::Deleted.to_string(), "deleted");
    }

    #[test]
    fn test_eval_status_from_str() {
        assert_eq!(EvalStatus::from("active"), EvalStatus::Active);
        assert_eq!(EvalStatus::from("archived"), EvalStatus::Archived);
        assert_eq!(EvalStatus::from("deleted"), EvalStatus::Deleted);
        assert_eq!(EvalStatus::from("unknown"), EvalStatus::Active);
    }

    #[test]
    fn test_eval_status_serde_roundtrip() {
        let json = serde_json::to_string(&EvalStatus::Archived).unwrap();
        assert_eq!(json, r#""archived""#);
        let parsed: EvalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, EvalStatus::Archived);
    }

    #[test]
    fn test_eval_run_status_display() {
        assert_eq!(EvalRunStatus::Pending.to_string(), "pending");
        assert_eq!(EvalRunStatus::Running.to_string(), "running");
        assert_eq!(EvalRunStatus::Completed.to_string(), "completed");
        assert_eq!(EvalRunStatus::Failed.to_string(), "failed");
        assert_eq!(EvalRunStatus::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn test_eval_run_status_from_str() {
        assert_eq!(EvalRunStatus::from("pending"), EvalRunStatus::Pending);
        assert_eq!(EvalRunStatus::from("running"), EvalRunStatus::Running);
        assert_eq!(EvalRunStatus::from("completed"), EvalRunStatus::Completed);
        assert_eq!(EvalRunStatus::from("failed"), EvalRunStatus::Failed);
        assert_eq!(EvalRunStatus::from("cancelled"), EvalRunStatus::Cancelled);
        assert_eq!(EvalRunStatus::from("unknown"), EvalRunStatus::Pending);
    }

    #[test]
    fn test_case_result_status_display() {
        assert_eq!(CaseResultStatus::Pending.to_string(), "pending");
        assert_eq!(CaseResultStatus::Passed.to_string(), "passed");
        assert_eq!(CaseResultStatus::Failed.to_string(), "failed");
        assert_eq!(CaseResultStatus::Errored.to_string(), "errored");
        assert_eq!(CaseResultStatus::Timeout.to_string(), "timeout");
    }

    #[test]
    fn test_case_result_status_from_str() {
        assert_eq!(CaseResultStatus::from("passed"), CaseResultStatus::Passed);
        assert_eq!(CaseResultStatus::from("failed"), CaseResultStatus::Failed);
        assert_eq!(CaseResultStatus::from("errored"), CaseResultStatus::Errored);
        assert_eq!(CaseResultStatus::from("timeout"), CaseResultStatus::Timeout);
        assert_eq!(CaseResultStatus::from("unknown"), CaseResultStatus::Pending);
    }

    #[test]
    fn test_scorer_serde_roundtrip() {
        let scorer = Scorer::Contains {
            text: "hello".to_string(),
            weight: 1.0,
        };
        let json = serde_json::to_value(&scorer).unwrap();
        assert_eq!(json["type"], "contains");
        assert_eq!(json["text"], "hello");
        assert_eq!(json["weight"], 1.0);

        let parsed: Scorer = serde_json::from_value(json).unwrap();
        match parsed {
            Scorer::Contains { text, weight } => {
                assert_eq!(text, "hello");
                assert_eq!(weight, 1.0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_scorer_tool_called_defaults() {
        let json = r#"{"type": "tool_called", "tool": "read_file"}"#;
        let scorer: Scorer = serde_json::from_str(json).unwrap();
        match scorer {
            Scorer::ToolCalled { tool, min, weight } => {
                assert_eq!(tool, "read_file");
                assert_eq!(min, 1);
                assert_eq!(weight, 1.0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_score_serde() {
        let score = Score {
            pass: true,
            value: 0.85,
            reason: "Output contains expected text".to_string(),
        };
        let json = serde_json::to_value(&score).unwrap();
        assert_eq!(json["pass"], true);
        assert_eq!(json["value"], 0.85);
    }

    #[test]
    fn test_run_summary_serde() {
        let summary = RunSummary {
            total: 10,
            passed: 8,
            failed: 1,
            errored: 1,
            pass_rate: 0.8,
            avg_score: 0.85,
            avg_turns: 3.5,
            avg_latency_ms: 2500,
            total_input_tokens: 50000,
            total_output_tokens: 10000,
        };
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["total"], 10);
        assert_eq!(json["pass_rate"], 0.8);
    }

    #[test]
    fn test_eval_input_message_serde() {
        let msg = EvalInputMessage {
            content: "What is 2+2?".to_string(),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["content"], "What is 2+2?");
    }

    #[test]
    fn test_eval_target_session_serde_roundtrip() {
        let target = EvalTarget::Session {
            harness_id: Some(HarnessId::from_uuid(Uuid::nil())),
            harness_name: None,
            agent_id: Some(AgentId::from_uuid(Uuid::nil())),
            model_id: Some("gpt-4".to_string()),
            system_prompt: None,
            max_iterations: None,
        };
        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json["type"], "session");
        assert!(json.get("harness_id").is_some());
        assert!(json.get("model_id").is_some());
        assert!(json.get("system_prompt").is_none()); // skip_serializing_if
        assert!(json.get("harness_name").is_none());
        assert!(json.get("max_iterations").is_none());

        let parsed: EvalTarget = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn test_eval_target_session_minimal() {
        // Session with just harness_name, no other params
        let target = EvalTarget::Session {
            harness_id: None,
            harness_name: Some("generic".to_string()),
            agent_id: None,
            model_id: None,
            system_prompt: None,
            max_iterations: None,
        };
        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json["type"], "session");
        assert_eq!(json["harness_name"], "generic");
        assert!(json.get("harness_id").is_none());

        let parsed: EvalTarget = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn test_eval_target_app_variant() {
        let target = EvalTarget::App {
            app_id: AppId::from_uuid(Uuid::nil()),
        };
        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json["type"], "app");
        assert!(json.get("app_id").is_some());

        let parsed: EvalTarget = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn test_eval_serde_skips_internal_fields() {
        let eval = Eval {
            public_id: EvalId::from_uuid(Uuid::nil()),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: "test".into(),
            description: None,
            target: Some(EvalTarget::Session {
                harness_id: Some(HarnessId::from_uuid(Uuid::nil())),
                harness_name: None,
                agent_id: Some(AgentId::from_uuid(Uuid::nil())),
                model_id: None,
                system_prompt: None,
                max_iterations: None,
            }),
            model_override: None,
            tags: vec![],
            status: EvalStatus::Active,
            case_count: 0,
            last_run: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        };
        let json = serde_json::to_value(&eval).unwrap();
        assert!(json.get("id").is_some());
        assert!(json.get("internal_id").is_none());
        assert!(json.get("org_id").is_none());
        assert!(json.get("target").is_some());
        assert!(json.get("description").is_none());
        assert!(json.get("model_override").is_none());
    }

    #[test]
    fn test_eval_case_artifacts_serde_roundtrip() {
        let json = serde_json::json!({
            "id": "evalcase_01933b5a000070008000000000000001",
            "name": "case",
            "conversation": [{"content": "hello"}],
            "artifacts": [{"name": "patch", "path": "/workspace/fix.patch"}],
            "scorers": [{"type": "contains", "text": "done", "weight": 1.0}],
            "tags": [],
            "position": 0,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        });

        let case: EvalCase = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            case.artifacts,
            Some(vec![ArtifactSpec {
                name: "patch".to_string(),
                path: "/workspace/fix.patch".to_string(),
            }])
        );
        assert_eq!(serde_json::to_value(case).unwrap(), json);
    }
}
