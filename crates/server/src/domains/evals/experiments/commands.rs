// Experiment commands: start, list, get with comparison, and the CI gate.

use super::types::{CreateEvalExperimentRequest, EvalExperiment, ExperimentGateResult};
use super::{GateRefusal, GateRule};
use crate::domains::common::*;
use crate::domains::evals::queries as q;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn require_evals_enabled(ctx: &Ctx) -> Result<(), CommandError> {
    if ctx.feature_flags.evals {
        Ok(())
    } else {
        Err(CommandError::feature_not_enabled("evals"))
    }
}

fn parse_experiment_id(id: &str) -> Result<super::EvalExperimentId, CommandError> {
    id.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid experiment ID: {e}")))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct CreateEvalExperiment {
    /// Eval's prefixed public identifier; its cases are the dataset.
    pub eval_id: String,
    #[serde(flatten)]
    pub req: CreateEvalExperimentRequest,
}

#[command(
    name = "create_eval_experiment",
    category = "evals",
    description = "Start an experiment: run an eval's cases against one to four variants and compare them case by case.",
    method = "POST",
    cli = CliRoute::new(&["evals", "experiments"], "create").with_examples(&[CliExample::new("Compare two models on the same cases, three trials each, within $10", "everruns evals experiments create --eval-id eval_01h9 --variants '[{\"label\":\"luna\",\"model_override\":\"gpt-6-luna\"},{\"label\":\"sol\",\"model_override\":\"gpt-6.1-sol\"}]' --max-cost-usd 10 --reason 'Pick the model for the support agent'")]),
    path = "/v1/evals/{eval_id}/experiments",
    policy = crate::domains::evals::EVAL_RUN,
    http = created,
    request_body(CreateEvalExperimentRequest),
)]
impl Command for CreateEvalExperiment {
    type Output = EvalExperiment;

    async fn execute(self, ctx: &Ctx) -> Result<EvalExperiment, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        super::create(
            &q::service(ctx),
            &ctx.db,
            &ctx.caller,
            &eval_id.to_string(),
            self.req,
        )
        .await
        .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListEvalExperiments {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
}

#[command(
    name = "list_eval_experiments",
    category = "evals",
    description = "List an eval's experiments, newest first.",
    method = "GET",
    cli = CliRoute::new(&["evals", "experiments"], "list").with_examples(&[CliExample::new("Find the last experiment on an eval", "everruns evals experiments list --eval-id eval_01h9")]),
    path = "/v1/evals/{eval_id}/experiments",
    policy = crate::domains::evals::EVAL_VIEW,
    http = list,
)]
impl Command for ListEvalExperiments {
    type Output = Vec<EvalExperiment>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalExperiment>, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        super::list(&ctx.db, &ctx.caller, Some(&eval_id.to_string()))
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, Serialize)]
pub struct ListAllEvalExperiments {}

#[command(
    name = "list_all_eval_experiments",
    category = "evals",
    description = "List the organization's experiments across evals, newest first.",
    method = "GET",
    cli = CliRoute::new(&["evals", "experiments"], "recent").with_examples(&[CliExample::new("See what experiments ran lately", "everruns evals experiments recent")]),
    path = "/v1/evals/experiments",
    policy = crate::domains::evals::EVAL_VIEW,
    http = list,
)]
impl Command for ListAllEvalExperiments {
    type Output = Vec<EvalExperiment>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalExperiment>, CommandError> {
        require_evals_enabled(ctx)?;
        super::list(&ctx.db, &ctx.caller, None)
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct GetEvalExperiment {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Experiment's prefixed public identifier.
    pub experiment_id: String,
}

#[command(
    name = "get_eval_experiment",
    category = "evals",
    description = "Get an experiment with its case-by-case comparison and verdicts.",
    method = "GET",
    cli = CliRoute::new(&["evals", "experiments"], "get").with_examples(&[CliExample::new("See which cases a candidate fixed or broke", "everruns evals experiments get --eval-id eval_01h9 --experiment-id evalexp_01h9")]),
    path = "/v1/evals/{eval_id}/experiments/{experiment_id}",
    policy = crate::domains::evals::EVAL_VIEW,
    http = plain,
    responses((status = 404, description = "Experiment not found")),
)]
impl Command for GetEvalExperiment {
    type Output = EvalExperiment;

    async fn execute(self, ctx: &Ctx) -> Result<EvalExperiment, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let experiment_id = parse_experiment_id(&self.experiment_id)?;
        super::get(
            &ctx.db,
            &ctx.caller,
            &eval_id.to_string(),
            &experiment_id.to_string(),
        )
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("EvalExperiment"))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct GateEvalExperiment {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Experiment's prefixed public identifier.
    pub experiment_id: String,
    /// Variant to judge; the last one when omitted.
    #[serde(default)]
    pub variant: Option<String>,
    /// Minimum pass rate over all trials, 0 to 1.
    #[serde(default)]
    pub min_pass_rate: Option<f64>,
    /// Minimum share of cases passed in every trial, 0 to 1.
    #[serde(default)]
    pub min_passed_every_trial: Option<f64>,
    /// Fail when the variant is a regression against the baseline.
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub no_regression: bool,
}

#[command(
    name = "gate_eval_experiment",
    category = "evals",
    description = "Check an experiment's variant against a bar, for CI. Answers 409 while runs are going and 422 when the variant misses the bar, so the command exits non-zero.",
    method = "GET",
    cli = CliRoute::new(&["evals", "experiments"], "gate").with_examples(&[CliExample::new("Block a merge unless the candidate passes 90% and is no regression", "everruns evals experiments gate --eval-id eval_01h9 --experiment-id evalexp_01h9 --min-pass-rate 0.9 --no-regression true")]),
    path = "/v1/evals/{eval_id}/experiments/{experiment_id}/gate",
    policy = crate::domains::evals::EVAL_VIEW,
    http = plain,
    responses(
        (status = 404, description = "Experiment not found"),
        (status = 409, description = "Some variant's run is still going"),
        (status = 422, description = "The variant missed the bar"),
    ),
)]
impl Command for GateEvalExperiment {
    type Output = ExperimentGateResult;

    async fn execute(self, ctx: &Ctx) -> Result<ExperimentGateResult, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let experiment_id = parse_experiment_id(&self.experiment_id)?;
        let experiment = super::get(
            &ctx.db,
            &ctx.caller,
            &eval_id.to_string(),
            &experiment_id.to_string(),
        )
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("EvalExperiment"))?;
        let rule = GateRule {
            variant: self.variant,
            min_pass_rate: self.min_pass_rate,
            min_passed_every_trial: self.min_passed_every_trial,
            no_regression: self.no_regression,
        };
        super::gate(&experiment, &rule).map_err(|refusal| match refusal {
            GateRefusal::Running => {
                CommandError::conflict("The experiment is still running. Check again later.")
            }
            GateRefusal::Invalid(why) => CommandError::bad_request(why),
            GateRefusal::Failed(reasons) => {
                CommandError::unprocessable(format!("Gate failed: {}.", reasons.join("; ")))
            }
        })
    }
}
