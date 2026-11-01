// Experiment API types.

use crate::domains::evals::record::{EvalRunStatus, EvalTarget, RunSummary};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One configuration an experiment runs the dataset against.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ExperimentVariantSpec {
    /// Short name shown in comparisons, unique within the experiment.
    #[schema(example = "candidate")]
    pub label: String,
    /// Session setup for this variant. Omitted: the eval's own target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    /// Model for this variant, beating the target's model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-6.1-sol")]
    pub model_override: Option<String>,
}

/// Request to start an experiment.
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct CreateEvalExperimentRequest {
    /// Display name. Defaults to the variant labels joined with "vs".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "Luna vs Sol")]
    pub name: Option<String>,
    /// One to four variants. The first is the baseline the others are
    /// compared against.
    pub variants: Vec<ExperimentVariantSpec>,
    /// Trials per case and variant, 1 to 20 (default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 3, minimum = 1, maximum = 20)]
    pub trials: Option<u32>,
    /// Spending cap in USD for the whole experiment, split evenly between
    /// the variants' runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 10.0)]
    pub max_cost_usd: Option<f64>,
}

/// Where an experiment is: running until every variant's run finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExperimentStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// One variant of an experiment and how its run went.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExperimentVariant {
    #[schema(example = "candidate")]
    pub label: String,
    /// The variant's run.
    #[schema(example = "evalrun_01933b5a000070008000000000000001")]
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-6.1-sol")]
    pub model_override: Option<String>,
    pub status: EvalRunStatus,
    /// Set once the run finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
    /// What one case cost on average, all its trials together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.031)]
    pub cost_per_case_usd: Option<f64>,
}

/// An experiment: one dataset version against several variants.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EvalExperiment {
    #[schema(example = "evalexp_01933b5a000070008000000000000001")]
    pub id: String,
    /// The eval whose cases are the dataset.
    #[schema(example = "eval_01933b5a000070008000000000000001")]
    pub eval_id: String,
    #[schema(example = "Luna vs Sol")]
    pub name: String,
    /// The eval's `version` the experiment ran.
    #[schema(example = 7)]
    pub dataset_version: u32,
    #[schema(example = 3)]
    pub trials: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 10.0)]
    pub max_cost_usd: Option<f64>,
    /// Label of the variant the others are compared against.
    #[schema(example = "baseline")]
    pub baseline: String,
    pub status: ExperimentStatus,
    pub variants: Vec<ExperimentVariant>,
    /// Paired comparison by case (detail view).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<ExperimentComparison>,
    pub created_at: DateTime<Utc>,
}

/// Variants compared case by case against the baseline.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExperimentComparison {
    #[schema(example = "baseline")]
    pub baseline: String,
    /// One verdict per variant other than the baseline.
    pub verdicts: Vec<VariantVerdict>,
    /// Cases whose result changed come first.
    pub cases: Vec<CaseComparison>,
}

/// How a variant compares with the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The pass rate gain is outside run-to-run noise.
    Improvement,
    /// The pass rate loss is outside run-to-run noise.
    Regression,
    /// The difference could be noise.
    WithinNoise,
    /// Too few cases ran on both sides to tell.
    NotEnoughData,
}

/// A variant against the baseline.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VariantVerdict {
    #[schema(example = "candidate")]
    pub label: String,
    /// Cases both ran.
    #[schema(example = 42)]
    pub paired_cases: u32,
    /// Mean per-case pass rate difference, in percentage points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 5.0)]
    pub pass_rate_delta_points: Option<f64>,
    /// 95% interval of that difference (paired bootstrap), in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 1.0)]
    pub interval_low_points: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 9.0)]
    pub interval_high_points: Option<f64>,
    /// Cases this variant passes every trial of and the baseline did not.
    #[schema(example = 4)]
    pub fixed: u32,
    /// Cases the baseline passes every trial of and this variant does not.
    #[schema(example = 1)]
    pub broke: u32,
    pub verdict: Verdict,
}

/// One case across the variants.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CaseComparison {
    #[schema(example = "evalcase_01933b5a000070008000000000000001")]
    pub case_id: String,
    #[schema(example = "Refund request, angry customer")]
    pub case_name: String,
    /// Some variant's pass rate on this case differs from the baseline's.
    pub changed: bool,
    /// One cell per variant, in variant order.
    pub cells: Vec<CaseCell>,
}

/// One case under one variant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CaseCell {
    #[schema(example = "candidate")]
    pub label: String,
    /// Trials that passed, of `trials` that finished.
    #[schema(example = 3)]
    pub passed: u32,
    #[schema(example = 3)]
    pub trials: u32,
    /// What the case cost, all its trials together.
    #[schema(example = 0.05)]
    pub cost_usd: f64,
}

/// Result of an experiment gate that passed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExperimentGateResult {
    pub passed: bool,
    /// The variant the gate judged.
    #[schema(example = "candidate")]
    pub variant: String,
    #[schema(example = 0.93)]
    pub pass_rate: f64,
    /// Share of cases passed in every trial (pass^k).
    #[schema(example = 0.9)]
    pub passed_every_trial: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
}
