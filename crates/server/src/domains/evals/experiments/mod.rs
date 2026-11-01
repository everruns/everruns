// Experiments: one dataset version (an eval's cases at its `version`) against
// one to four variants, with paired comparison and a gate for CI
// (knowledge/evaluation/experiments.md).
//
// Decision: a variant is an ordinary eval run tagged with the experiment and
// its label. Runs already have durable per-case tasks, trials, budgets and the
// starter's authority; an experiment adds the grouping, the comparison and
// the gate, not a second runner.
//
// Decision: the experiment's spending cap is split evenly between its
// variants' runs, so one expensive variant cannot starve the others.
//
// Decision: the gate answers with an error status when it does not pass (409
// while runs are still going, 422 below the bar), so `everruns evals
// experiments gate` exits non-zero and a CI step fails without parsing JSON.

pub mod commands;
pub mod compare;
pub mod types;

use crate::domains::evals::EvalService;
use crate::domains::evals::record::{EvalRunStatus, EvalTarget, RunSummary, validate_run_bounds};
use crate::domains::evals::types::CreateEvalRunRequest;
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::storage::{CreateEvalExperimentRow, EvalExperimentRow, EvalRunRow, StorageBackend};
use anyhow::Result;
use compare::{VariantResults, compare};
use everruns_contracts::typed_id::{IdMarker, TypedId};
use everruns_core::Caller;
use types::*;
use uuid::Uuid;

/// Marker for experiment ids (`evalexp_...`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EvalExperimentIdMarker;
impl IdMarker for EvalExperimentIdMarker {
    const PREFIX: &'static str = "evalexp";
}
pub type EvalExperimentId = TypedId<EvalExperimentIdMarker>;

pub const MAX_VARIANTS: usize = 4;
pub const DEFAULT_TRIALS: u32 = 3;
const MAX_LABEL_CHARS: usize = 40;
const LIST_LIMIT: i64 = 100;

fn validate(req: &CreateEvalExperimentRequest) -> Result<u32> {
    if req.variants.is_empty() || req.variants.len() > MAX_VARIANTS {
        return Err(BadRequestError::new(format!(
            "an experiment has 1 to {MAX_VARIANTS} variants"
        ))
        .into());
    }
    let mut seen = std::collections::HashSet::new();
    for variant in &req.variants {
        let label = variant.label.trim();
        if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
            return Err(BadRequestError::new(format!(
                "variant labels are 1 to {MAX_LABEL_CHARS} characters"
            ))
            .into());
        }
        if !seen.insert(label.to_string()) {
            return Err(
                BadRequestError::new(format!("variant label '{label}' is used twice")).into(),
            );
        }
    }
    validate_run_bounds(Some(req.trials.unwrap_or(DEFAULT_TRIALS)), req.max_cost_usd)
}

/// Start an experiment: one run per variant.
pub async fn create(
    service: &EvalService,
    db: &StorageBackend,
    caller: &Caller,
    eval_public_id: &str,
    req: CreateEvalExperimentRequest,
) -> Result<EvalExperiment> {
    let trials = validate(&req)?;
    let eval = db
        .get_eval_by_public_id(caller.org_id, eval_public_id)
        .await?
        .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;
    // All variants start together; refuse up front rather than leave half an
    // experiment behind when the org's concurrent run limit is hit.
    let running = db.count_running_eval_runs_for_org(caller.org_id).await? as usize;
    let limit = service.limits().max_concurrent_runs_per_org;
    if running + req.variants.len() > limit {
        return Err(BadRequestError::new(format!(
            "this experiment needs {} runs and the organization has {} of {limit} running",
            req.variants.len(),
            running
        ))
        .into());
    }

    let id = Uuid::now_v7();
    let labels: Vec<String> = req
        .variants
        .iter()
        .map(|v| v.label.trim().to_string())
        .collect();
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| labels.join(" vs "));
    let row = db
        .create_eval_experiment(
            caller.org_id,
            CreateEvalExperimentRow {
                id,
                public_id: EvalExperimentId::from_uuid(id).to_string(),
                eval_id: eval.id,
                name,
                dataset_version: eval.version,
                trials: trials as i32,
                max_cost_usd: req.max_cost_usd,
                baseline_label: labels[0].clone(),
                created_by_user_id: caller.user_id,
            },
        )
        .await?;

    let per_variant_cap = req.max_cost_usd.map(|cap| cap / req.variants.len() as f64);
    for (variant, label) in req.variants.into_iter().zip(&labels) {
        let run = service
            .create_run(
                caller,
                eval_public_id,
                CreateEvalRunRequest {
                    target: variant.target,
                    model_override: variant.model_override,
                    trials: Some(trials),
                    max_cost_usd: per_variant_cap,
                    case_ids: None,
                },
            )
            .await?;
        db.tag_eval_run_variant(run.internal_id, id, label).await?;
    }
    let runs = db.list_eval_experiment_runs(id).await?;
    Ok(to_experiment(&row, &eval.public_id, &runs))
}

/// One experiment with its paired comparison.
pub async fn get(
    db: &StorageBackend,
    caller: &Caller,
    eval_public_id: &str,
    experiment_public_id: &str,
) -> Result<Option<EvalExperiment>> {
    let Some(row) = db
        .get_eval_experiment_by_public_id(caller.org_id, experiment_public_id)
        .await?
    else {
        return Ok(None);
    };
    let eval = db
        .get_eval_by_public_id(caller.org_id, eval_public_id)
        .await?
        .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;
    if eval.id != row.eval_id {
        return Ok(None);
    }
    let runs = db.list_eval_experiment_runs(row.id).await?;
    let mut experiment = to_experiment(&row, &eval.public_id, &runs);

    let cases = db.list_eval_cases(eval.id).await?;
    let mut results = Vec::with_capacity(runs.len());
    for run in &runs {
        results.push(db.list_eval_case_results(run.id).await?);
    }
    let variants: Vec<VariantResults<'_>> = runs
        .iter()
        .zip(&results)
        .map(|(run, results)| VariantResults {
            label: run.variant_label.as_deref().unwrap_or_default(),
            results,
        })
        .collect();
    experiment.comparison = Some(compare(row.id, &cases, &variants));
    Ok(Some(experiment))
}

/// The org's experiments, newest first; one eval's when `eval_public_id` is
/// given.
pub async fn list(
    db: &StorageBackend,
    caller: &Caller,
    eval_public_id: Option<&str>,
) -> Result<Vec<EvalExperiment>> {
    let eval_id = match eval_public_id {
        Some(public_id) => Some(
            db.get_eval_by_public_id(caller.org_id, public_id)
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Eval"))?
                .id,
        ),
        None => None,
    };
    let rows = db
        .list_eval_experiments(caller.org_id, eval_id, LIST_LIMIT)
        .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let runs = db.list_runs_of_eval_experiments(&ids).await?;
    let mut eval_ids: Vec<Uuid> = rows.iter().map(|r| r.eval_id).collect();
    eval_ids.sort_unstable();
    eval_ids.dedup();
    let eval_public_ids = db.eval_public_ids(caller.org_id, &eval_ids).await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let mine: Vec<EvalRunRow> = runs
                .iter()
                .filter(|run| run.experiment_id == Some(row.id))
                .cloned()
                .collect();
            // An eval deleted since keeps no route to its experiments.
            let eval_public_id = eval_public_ids.get(&row.eval_id)?;
            Some(to_experiment(row, eval_public_id, &mine))
        })
        .collect())
}

/// What the gate asks of a variant.
#[derive(Debug, Clone, Default)]
pub struct GateRule {
    /// Variant to judge; the last one when omitted.
    pub variant: Option<String>,
    /// Minimum pass rate over all trials, 0 to 1.
    pub min_pass_rate: Option<f64>,
    /// Minimum share of cases passed in every trial, 0 to 1.
    pub min_passed_every_trial: Option<f64>,
    /// Fail when the variant is a regression against the baseline.
    pub no_regression: bool,
}

/// Why a gate did not pass.
#[derive(Debug)]
pub enum GateRefusal {
    /// Some variant's run has not finished.
    Running,
    /// No variant with that label, or a bar outside 0 to 1.
    Invalid(String),
    /// The variant missed the bar; one reason per missed rule.
    Failed(Vec<String>),
}

/// Judge `experiment` (with its comparison) by `rule`.
pub fn gate(
    experiment: &EvalExperiment,
    rule: &GateRule,
) -> std::result::Result<ExperimentGateResult, GateRefusal> {
    for bar in [rule.min_pass_rate, rule.min_passed_every_trial]
        .into_iter()
        .flatten()
    {
        if !(0.0..=1.0).contains(&bar) {
            return Err(GateRefusal::Invalid(
                "gate bars are fractions from 0 to 1".into(),
            ));
        }
    }
    if experiment.status == ExperimentStatus::Running {
        return Err(GateRefusal::Running);
    }
    let variant = match &rule.variant {
        Some(label) => experiment.variants.iter().find(|v| &v.label == label),
        None => experiment.variants.last(),
    }
    .ok_or_else(|| GateRefusal::Invalid("no variant with that label".into()))?;
    let summary = variant.summary.clone().unwrap_or_else(empty_summary);
    let passed_every_trial = if summary.cases > 0 {
        summary.cases_passed_every_trial as f64 / summary.cases as f64
    } else {
        0.0
    };
    let verdict = experiment.comparison.as_ref().and_then(|c| {
        c.verdicts
            .iter()
            .find(|v| v.label == variant.label)
            .map(|v| v.verdict)
    });

    let mut failures = Vec::new();
    if variant.status != EvalRunStatus::Completed {
        failures.push(format!("the {} run is {}", variant.label, variant.status));
    }
    if let Some(min) = rule.min_pass_rate
        && summary.pass_rate < min
    {
        failures.push(format!(
            "pass rate {:.0}% is under {:.0}%",
            summary.pass_rate * 100.0,
            min * 100.0
        ));
    }
    if let Some(min) = rule.min_passed_every_trial
        && passed_every_trial < min
    {
        failures.push(format!(
            "{:.0}% of cases passed every trial, under {:.0}%",
            passed_every_trial * 100.0,
            min * 100.0
        ));
    }
    if rule.no_regression && verdict == Some(Verdict::Regression) {
        failures.push(format!(
            "{} is a regression against {}",
            variant.label, experiment.baseline
        ));
    }
    if !failures.is_empty() {
        return Err(GateRefusal::Failed(failures));
    }
    Ok(ExperimentGateResult {
        passed: true,
        variant: variant.label.clone(),
        pass_rate: summary.pass_rate,
        passed_every_trial,
        verdict,
    })
}

fn empty_summary() -> RunSummary {
    serde_json::from_value(serde_json::json!({
        "total": 0, "passed": 0, "failed": 0, "errored": 0, "pass_rate": 0.0,
        "avg_score": 0.0, "avg_turns": 0.0, "avg_latency_ms": 0,
        "total_input_tokens": 0, "total_output_tokens": 0,
    }))
    .expect("an empty summary deserializes")
}

fn to_experiment(
    row: &EvalExperimentRow,
    eval_public_id: &str,
    runs: &[EvalRunRow],
) -> EvalExperiment {
    let variants: Vec<ExperimentVariant> = runs
        .iter()
        .map(|run| {
            let summary: Option<RunSummary> = run
                .summary
                .clone()
                .and_then(|s| serde_json::from_value(s).ok());
            let cost_per_case_usd = summary.as_ref().and_then(|s| {
                s.total_cost_usd
                    .filter(|_| s.cases > 0)
                    .map(|cost| cost / s.cases as f64)
            });
            ExperimentVariant {
                label: run.variant_label.clone().unwrap_or_default(),
                run_id: run.public_id.clone(),
                target: run
                    .target
                    .clone()
                    .and_then(|t| serde_json::from_value::<EvalTarget>(t).ok()),
                model_override: run.model_override.clone(),
                status: EvalRunStatus::from(run.status.as_str()),
                summary,
                cost_per_case_usd,
            }
        })
        .collect();
    EvalExperiment {
        id: row.public_id.clone(),
        eval_id: eval_public_id.to_string(),
        name: row.name.clone(),
        dataset_version: row.dataset_version.max(1) as u32,
        trials: row.trials.max(1) as u32,
        max_cost_usd: row.max_cost_usd,
        baseline: row.baseline_label.clone(),
        status: status_of(&variants),
        variants,
        comparison: None,
        created_at: row.created_at,
    }
}

fn status_of(variants: &[ExperimentVariant]) -> ExperimentStatus {
    let any = |s: EvalRunStatus| variants.iter().any(|v| v.status == s);
    if variants.is_empty() || any(EvalRunStatus::Pending) || any(EvalRunStatus::Running) {
        ExperimentStatus::Running
    } else if any(EvalRunStatus::Failed) {
        ExperimentStatus::Failed
    } else if any(EvalRunStatus::Cancelled) {
        ExperimentStatus::Cancelled
    } else {
        ExperimentStatus::Completed
    }
}

#[cfg(test)]
mod tests;
