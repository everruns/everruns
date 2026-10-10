use super::queries as q;
use super::types::{
    BulkUpdateEvalRunScoresRequest, CreateEvalCaseRequest, CreateEvalRequest, CreateEvalRunRequest,
    EvalImportPreflight, EvalRunShareLink, EvalRunShareStatus, ImportEvalRunRequest,
    ListEvalsQuery, UpdateEvalCaseRequest, UpdateEvalRequest, UpdateEvalResultScoresRequest,
};
use crate::domains::common::*;
use crate::domains::evals::record::{Eval, EvalCase, EvalCaseResult, EvalRun};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize)]
pub struct EvalArtifactExport {
    pub body: String,
}

fn require_evals_enabled(ctx: &Ctx) -> Result<(), CommandError> {
    if ctx.feature_flags.evals {
        Ok(())
    } else {
        Err(CommandError::feature_not_enabled("evals"))
    }
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateEval(pub CreateEvalRequest);

impl CommandSchema for CreateEval {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateEvalRequest>()
    }
}

#[command(
    name = "create_eval",
    category = "evals",
    description = "Create a new eval.",
    method = "POST",
    cli = CliRoute::new(&["evals"], "create").with_examples(&[CliExample::new("Start a regression suite aimed at one agent", "everruns evals create --name support-regression --target '{\"type\":\"session\",\"agent_id\":\"agent_01h9\"}' --reason 'Track support agent quality'")]),
    path = "/v1/evals",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = created,
    request_body(CreateEvalRequest),
)]
impl Command for CreateEval {
    type Output = Eval;

    async fn execute(self, ctx: &Ctx) -> Result<Eval, CommandError> {
        require_evals_enabled(ctx)?;
        q::service(ctx)
            .create(&ctx.caller, self.0)
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Default, Deserialize, serde::Serialize)]
pub struct ListEvals {
    /// Case-insensitive name filter.
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    /// Include archived evals.
    pub include_archived: bool,
}

impl CommandSchema for ListEvals {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<ListEvalsQuery>()
    }
}

#[command(
    name = "list_evals",
    category = "evals",
    description = "List evals.",
    method = "GET",
    cli = CliRoute::new(&["evals"], "list").with_examples(&[CliExample::new("Find an eval by name when you do not know the id", "everruns evals list --search support")]),
    path = "/v1/evals",
    policy = crate::domains::evals::EVAL_VIEW,
    http = list,
    params(ListEvalsQuery),
)]
impl Command for ListEvals {
    type Output = Vec<Eval>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Eval>, CommandError> {
        require_evals_enabled(ctx)?;
        q::service(ctx)
            .list(&ctx.caller, self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)
    }
}

/// Import a full external run group (everruns as host/viewer for external eval
/// systems).
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ImportEvalRun {
    #[serde(flatten)]
    pub req: ImportEvalRunRequest,
}

#[command(
    name = "import_eval_run",
    category = "evals",
    description = "Import externally-executed eval results.",
    method = "POST",
    cli = CliRoute::new(&["evals"], "import").with_examples(&[CliExample::new("Record results from an eval harness that ran outside Everruns", "everruns evals import --source '{\"system\":\"mira\",\"run_id\":\"run-42\"}' --evals '[{\"name\":\"support\",\"cases\":[{\"name\":\"refund\",\"target\":{\"provider\":\"openai\",\"model\":\"gpt-5.1\"},\"status\":\"passed\"}]}]' --reason 'Publish the nightly Mira run'")]),
    path = "/v1/evals/import",
    policy = crate::domains::evals::EVAL_IMPORT,
    http = list,
    request_body(ImportEvalRunRequest),
)]
impl Command for ImportEvalRun {
    type Output = Vec<EvalRun>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalRun>, CommandError> {
        require_evals_enabled(ctx)?;
        q::service(ctx)
            .import_run(&ctx.caller, self.req)
            .await
            .map_err(classify_anyhow)
    }
}

/// Import ATIF trajectories as eval cases.
///
/// `body` is the raw import payload: NDJSON (one trajectory per line), a JSON
/// array of trajectories, a single trajectory object, or `{ "trajectories":
/// [...] }`. Cases are upserted by name, so re-import is idempotent.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ImportAtifTrajectories {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Raw ATIF payload (NDJSON or JSON).
    pub body: String,
}

#[command(
    name = "import_atif_trajectories",
    category = "evals",
    description = "Import ATIF trajectories as eval cases (upserted by name).",
    method = "POST",
    cli = CliRoute::new(&["evals", "atif-import"], "import").with_examples(&[CliExample::new("Turn recorded agent trajectories into eval cases", "everruns evals atif-import import --eval-id eval_01h9 --body \"$(cat trajectories.ndjson)\" --reason 'Seed cases from production sessions'")]),
    path = "/v1/evals/{eval_id}/atif_import",
    policy = crate::domains::evals::EVAL_MANAGE,
)]
impl Command for ImportAtifTrajectories {
    type Output = crate::domains::evals::types::AtifImportReport;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let trajectories =
            crate::atif::parse_import_body(&self.body).map_err(CommandError::bad_request)?;
        let drafts = trajectories
            .iter()
            .enumerate()
            .map(|(i, t)| crate::atif::trajectory_to_case_draft(t, i))
            .collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::bad_request)?;
        q::service(ctx)
            .import_atif_cases(&ctx.caller, &eval_id.to_string(), drafts)
            .await
            .map_err(classify_anyhow)
    }
}

/// Preflight: report whether the caller can import (feature enabled + has the
/// eval-management permission) without failing. No policy gate so any org
/// member can probe; the report just returns `false`.
#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct EvalImportPreflightCmd {}

#[command(
    name = "eval_import_preflight",
    category = "evals",
    description = "Report whether the caller can import eval results.",
    method = "GET",
    path = "/v1/evals/import/preflight",
    cli = CliRoute::new(&["evals"], "import-preflight").with_examples(&[CliExample::new("Check whether this caller can import eval results", "everruns evals import-preflight",)]),
    http = plain,
)]
impl Command for EvalImportPreflightCmd {
    type Output = EvalImportPreflight;

    async fn execute(self, ctx: &Ctx) -> Result<EvalImportPreflight, CommandError> {
        let evals_enabled = ctx.feature_flags.evals;
        let can_import = evals_enabled
            && ctx
                .permission_resolver
                .has_permission(&ctx.caller, &everruns_core::Permission::OrgAgentsManage);
        Ok(EvalImportPreflight {
            evals_enabled,
            can_import,
        })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetEval {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
}

#[command(
    name = "get_eval",
    category = "evals",
    description = "Get a single eval.",
    method = "GET",
    cli = CliRoute::new(&["evals"], "get").with_args(&[CliArg::new("eval_id").at(1)]).with_examples(&[CliExample::new("Show an eval's target and settings", "everruns evals get eval_01h9")]),
    path = "/v1/evals/{eval_id}",
    policy = crate::domains::evals::EVAL_VIEW,
    positional = "eval_id",
    http = plain,
    responses((status = 404, description = "Eval not found")),
)]
impl Command for GetEval {
    type Output = Eval;

    async fn execute(self, ctx: &Ctx) -> Result<Eval, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .get_by_public_id(&ctx.caller, &eval_id.to_string())
            .await?
            .ok_or_else(|| CommandError::not_found("Eval"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateEval {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    #[serde(flatten)]
    pub req: UpdateEvalRequest,
}

#[command(
    name = "update_eval",
    category = "evals",
    description = "Update an eval.",
    method = "PATCH",
    cli = CliRoute::new(&["evals"], "update").with_examples(&[CliExample::new("Point an eval at a different model by default", "everruns evals update --eval-id eval_01h9 --model-override gpt-5.1 --reason 'Move the suite to the new model'")]),
    path = "/v1/evals/{eval_id}",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = plain,
    request_body(UpdateEvalRequest),
    responses((status = 404, description = "Eval not found")),
)]
impl Command for UpdateEval {
    type Output = Eval;

    async fn execute(self, ctx: &Ctx) -> Result<Eval, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .update(&ctx.caller, &eval_id.to_string(), self.req)
            .await?
            .ok_or_else(|| CommandError::not_found("Eval"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteEval {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
}

#[command(
    name = "delete_eval",
    category = "evals",
    description = "Delete an eval.",
    method = "DELETE",
    cli = CliRoute::new(&["evals"], "delete").with_examples(&[CliExample::new("Remove an eval that is no longer tracked", "everruns evals delete --eval-id eval_01h9 --reason 'Suite retired'")]),
    path = "/v1/evals/{eval_id}",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = no_content,
    responses((status = 404, description = "Eval not found")),
)]
impl Command for DeleteEval {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let deleted = q::service(ctx)
            .delete(&ctx.caller, &eval_id.to_string())
            .await?;
        if deleted {
            Ok(true)
        } else {
            Err(CommandError::not_found("Eval"))
        }
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateEvalCase {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    #[serde(flatten)]
    pub req: CreateEvalCaseRequest,
}

#[command(
    name = "create_eval_case",
    category = "evals",
    description = "Create an eval case.",
    method = "POST",
    cli = CliRoute::new(&["evals", "cases"], "create").with_examples(&[CliExample::new("Add a scripted conversation with a pass condition to an eval", "everruns evals cases create --eval-id eval_01h9 --name fix-failing-test --conversation '[{\"content\":\"Fix the failing test in src/lib.rs\"}]' --scorers '[{\"type\":\"contains\",\"text\":\"tests pass\"}]' --reason 'Cover the test-fix flow'")]),
    path = "/v1/evals/{eval_id}/cases",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = created,
    request_body(CreateEvalCaseRequest),
)]
impl Command for CreateEvalCase {
    type Output = EvalCase;

    async fn execute(self, ctx: &Ctx) -> Result<EvalCase, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .create_case(&ctx.caller, &eval_id.to_string(), self.req)
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListEvalCases {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
}

#[command(
    name = "list_eval_cases",
    category = "evals",
    description = "List eval cases.",
    method = "GET",
    cli = CliRoute::new(&["evals", "cases"], "list").with_examples(&[CliExample::new("See which cases an eval contains", "everruns evals cases list --eval-id eval_01h9")]),
    path = "/v1/evals/{eval_id}/cases",
    policy = crate::domains::evals::EVAL_VIEW,
    http = list,
)]
impl Command for ListEvalCases {
    type Output = Vec<EvalCase>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalCase>, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .list_cases(&ctx.caller, &eval_id.to_string())
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetEvalCase {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval case's prefixed public identifier.
    pub case_id: String,
}

#[command(
    name = "get_eval_case",
    category = "evals",
    description = "Get an eval case.",
    method = "GET",
    cli = CliRoute::new(&["evals", "cases"], "get").with_examples(&[CliExample::new("Read one case's conversation and scorers", "everruns evals cases get --eval-id eval_01h9 --case-id evalcase_01h9")]),
    path = "/v1/evals/{eval_id}/cases/{case_id}",
    policy = crate::domains::evals::EVAL_VIEW,
    http = plain,
    responses((status = 404, description = "Eval case not found")),
)]
impl Command for GetEvalCase {
    type Output = EvalCase;

    async fn execute(self, ctx: &Ctx) -> Result<EvalCase, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let case_id = q::parse_case_id(&self.case_id)?;
        q::service(ctx)
            .get_case(&ctx.caller, &eval_id.to_string(), &case_id.to_string())
            .await?
            .ok_or_else(|| CommandError::not_found("EvalCase"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateEvalCase {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval case's prefixed public identifier.
    pub case_id: String,
    #[serde(flatten)]
    pub req: UpdateEvalCaseRequest,
}

#[command(
    name = "update_eval_case",
    category = "evals",
    description = "Update an eval case.",
    method = "PATCH",
    cli = CliRoute::new(&["evals", "cases"], "update").with_examples(&[CliExample::new("Give a slow case more turns", "everruns evals cases update --eval-id eval_01h9 --case-id evalcase_01h9 --max-turns 20 --reason 'Case timed out at the old limit'")]),
    path = "/v1/evals/{eval_id}/cases/{case_id}",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = plain,
    request_body(UpdateEvalCaseRequest),
    responses((status = 404, description = "Eval case not found")),
)]
impl Command for UpdateEvalCase {
    type Output = EvalCase;

    async fn execute(self, ctx: &Ctx) -> Result<EvalCase, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let case_id = q::parse_case_id(&self.case_id)?;
        q::service(ctx)
            .update_case(
                &ctx.caller,
                &eval_id.to_string(),
                &case_id.to_string(),
                self.req,
            )
            .await?
            .ok_or_else(|| CommandError::not_found("EvalCase"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteEvalCase {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval case's prefixed public identifier.
    pub case_id: String,
}

#[command(
    name = "delete_eval_case",
    category = "evals",
    description = "Delete an eval case.",
    method = "DELETE",
    cli = CliRoute::new(&["evals", "cases"], "delete").with_examples(&[CliExample::new("Drop a case that no longer reflects expected behavior", "everruns evals cases delete --eval-id eval_01h9 --case-id evalcase_01h9 --reason 'Duplicate of fix-failing-test'")]),
    path = "/v1/evals/{eval_id}/cases/{case_id}",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = no_content,
    responses((status = 404, description = "Eval case not found")),
)]
impl Command for DeleteEvalCase {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let case_id = q::parse_case_id(&self.case_id)?;
        let deleted = q::service(ctx)
            .delete_case(&ctx.caller, &eval_id.to_string(), &case_id.to_string())
            .await?;
        if deleted {
            Ok(true)
        } else {
            Err(CommandError::not_found("EvalCase"))
        }
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateEvalRun {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    #[serde(flatten)]
    pub req: CreateEvalRunRequest,
}

#[command(
    name = "create_eval_run",
    category = "evals",
    description = "Create an eval run.",
    method = "POST",
    cli = CliRoute::new(&["evals", "runs"], "create").with_examples(&[CliExample::new("Run every case in an eval, optionally on a different model", "everruns evals runs create --eval-id eval_01h9 --model-override gpt-5.1 --reason 'Check the suite on the new model'")]),
    path = "/v1/evals/{eval_id}/runs",
    policy = crate::domains::evals::EVAL_RUN,
    http = created,
    request_body(CreateEvalRunRequest),
)]
impl Command for CreateEvalRun {
    type Output = EvalRun;

    async fn execute(self, ctx: &Ctx) -> Result<EvalRun, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .create_run(&ctx.caller, &eval_id.to_string(), self.req)
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListEvalRuns {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
}

#[command(
    name = "list_eval_runs",
    category = "evals",
    description = "List eval runs.",
    method = "GET",
    cli = CliRoute::new(&["evals", "runs"], "list").with_examples(&[CliExample::new("Find a past run to compare against", "everruns evals runs list --eval-id eval_01h9")]),
    path = "/v1/evals/{eval_id}/runs",
    policy = crate::domains::evals::EVAL_VIEW,
    http = list,
)]
impl Command for ListEvalRuns {
    type Output = Vec<EvalRun>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalRun>, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .list_runs(&ctx.caller, &eval_id.to_string())
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetEvalRun {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
}

#[command(
    name = "get_eval_run",
    category = "evals",
    description = "Get an eval run.",
    method = "GET",
    cli = CliRoute::new(&["evals", "runs"], "get").with_examples(&[CliExample::new("Check a run's status and per-case results", "everruns evals runs get --eval-id eval_01h9 --run-id evalrun_01h9")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}",
    policy = crate::domains::evals::EVAL_VIEW,
    http = plain,
    responses((status = 404, description = "Eval run not found")),
)]
impl Command for GetEvalRun {
    type Output = EvalRun;

    async fn execute(self, ctx: &Ctx) -> Result<EvalRun, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .get_run(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await?
            .ok_or_else(|| CommandError::not_found("EvalRun"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ExportEvalRunArtifacts {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
}

#[command(
    name = "export_eval_run_artifacts",
    category = "evals",
    description = "Export eval run artifacts as NDJSON.",
    method = "GET",
    cli = CliRoute::new(&["evals", "runs", "artifacts"], "export").with_examples(&[CliExample::new("Download the files a run captured, one NDJSON line per result", "everruns evals runs artifacts export --eval-id eval_01h9 --run-id evalrun_01h9")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/artifacts",
    policy = crate::domains::evals::EVAL_VIEW,
)]
impl Command for ExportEvalRunArtifacts {
    type Output = EvalArtifactExport;

    async fn execute(self, ctx: &Ctx) -> Result<EvalArtifactExport, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        let run = q::service(ctx)
            .get_run(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await?
            .ok_or_else(|| CommandError::not_found("EvalRun"))?;

        let mut body = String::new();
        for result in run.results {
            let line = serde_json::to_string(&q::run_artifact_export_value(&result))
                .map_err(|e| CommandError::internal(e.into()))?;
            body.push_str(&line);
            body.push('\n');
        }

        Ok(EvalArtifactExport { body })
    }
}

/// Enqueue an async dataset export for a completed eval run and return a handle.
///
/// The export runs as a fire-and-forget background job (mirroring the eval
/// runner): it reconstructs each surviving case's model-view messages (faithful
/// to what the model saw, via the compaction model-view masking), joins reward +
/// efficiency metadata, and stores the produced NDJSON on the handle. Fetch the
/// NDJSON via `GET .../dataset/{dataset_id}` once `status` is `completed`. Gated
/// by `DATASET_EXPORT` and org-scoped through `get_run` (which resolves only
/// runs owned by the caller's org).
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ExportEvalRunDataset {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Completed eval run's prefixed public identifier.
    pub run_id: String,
    #[serde(flatten)]
    pub req: super::dataset::ExportEvalRunDatasetRequest,
}

#[command(
    name = "export_eval_run_dataset",
    category = "evals",
    description = "Enqueue an async reward-labeled trajectory dataset export from a completed eval run.",
    method = "POST",
    cli = CliRoute::new(&["evals", "runs", "dataset"], "export").with_examples(&[CliExample::new("Build a training dataset from a finished run", "everruns evals runs dataset export --eval-id eval_01h9 --run-id evalrun_01h9 --format sft --reason 'Fine-tune on the passing cases'")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/dataset",
    policy = crate::domains::evals::DATASET_EXPORT,
)]
impl Command for ExportEvalRunDataset {
    type Output = crate::domains::evals::record::EvalRunDataset;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .create_dataset_export(
                &ctx.caller,
                &eval_id.to_string(),
                &run_id.to_string(),
                self.req,
            )
            .await?
            .ok_or_else(|| CommandError::not_found("EvalRun"))
    }
}

/// Fetch an async dataset-export handle: status plus the produced NDJSON `body`
/// once the export is `completed`. Org-scoped through `get_dataset`.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetEvalRunDataset {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
    /// Dataset export's prefixed public identifier.
    pub dataset_id: String,
}

#[command(
    name = "get_eval_run_dataset",
    category = "evals",
    description = "Fetch an eval-run dataset export handle (status + NDJSON body).",
    method = "GET",
    cli = CliRoute::new(&["evals", "runs", "dataset"], "get").with_examples(&[CliExample::new("Poll a dataset export until it completes, then read its NDJSON", "everruns evals runs dataset get --eval-id eval_01h9 --run-id evalrun_01h9 --dataset-id evaldataset_01h9")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/dataset/{dataset_id}",
    policy = crate::domains::evals::DATASET_EXPORT,
    http = plain,
    responses((status = 404, description = "Dataset export not found")),
)]
impl Command for GetEvalRunDataset {
    type Output = crate::domains::evals::record::EvalRunDataset;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .get_dataset(
                &ctx.caller,
                &eval_id.to_string(),
                &run_id.to_string(),
                &self.dataset_id,
            )
            .await?
            .ok_or_else(|| CommandError::not_found("EvalRunDataset"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CancelEvalRun {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
}

#[command(
    name = "cancel_eval_run",
    category = "evals",
    description = "Cancel an eval run.",
    method = "POST",
    cli = CliRoute::new(&["evals", "runs"], "cancel").with_examples(&[CliExample::new("Stop a run that was started with the wrong settings", "everruns evals runs cancel --eval-id eval_01h9 --run-id evalrun_01h9 --reason 'Wrong model override'")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/cancel",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = plain,
    responses((status = 404, description = "Eval run not found")),
)]
impl Command for CancelEvalRun {
    type Output = EvalRun;

    async fn execute(self, ctx: &Ctx) -> Result<EvalRun, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .cancel_run(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await?
            .ok_or_else(|| CommandError::not_found("EvalRun"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateEvalRunShare {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
}

#[command(
    name = "create_eval_run_share",
    category = "evals",
    description = "Mint a read-only share link for an eval run.",
    method = "POST",
    cli = CliRoute::new(&["evals", "runs", "share"], "create").with_examples(&[CliExample::new("Mint a read-only link to show a run's results to someone outside the org", "everruns evals runs share create --eval-id eval_01h9 --run-id evalrun_01h9 --reason 'Share results with the vendor'")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/share",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = plain,
)]
impl Command for CreateEvalRunShare {
    type Output = EvalRunShareLink;

    async fn execute(self, ctx: &Ctx) -> Result<EvalRunShareLink, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .create_run_share(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetEvalRunShare {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
}

#[command(
    name = "get_eval_run_share",
    category = "evals",
    description = "Whether an eval run has an active share link.",
    method = "GET",
    cli = CliRoute::new(&["evals", "runs", "share"], "get").with_examples(&[CliExample::new("Check whether a run is currently shared", "everruns evals runs share get --eval-id eval_01h9 --run-id evalrun_01h9")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/share",
    policy = crate::domains::evals::EVAL_VIEW,
    http = plain,
)]
impl Command for GetEvalRunShare {
    type Output = EvalRunShareStatus;

    async fn execute(self, ctx: &Ctx) -> Result<EvalRunShareStatus, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        let active = q::service(ctx)
            .run_has_active_share(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await?;
        Ok(EvalRunShareStatus { active })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct RevokeEvalRunShare {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
}

#[command(
    name = "revoke_eval_run_share",
    category = "evals",
    description = "Revoke all share links for an eval run.",
    method = "DELETE",
    cli = CliRoute::new(&["evals", "runs", "share"], "revoke").with_examples(&[CliExample::new("Turn off every share link for a run", "everruns evals runs share revoke --eval-id eval_01h9 --run-id evalrun_01h9 --reason 'Vendor review finished'")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/share",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = no_content,
)]
impl Command for RevokeEvalRunShare {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .revoke_run_share(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateEvalResultScores {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
    /// Eval case result's prefixed public identifier.
    pub result_id: String,
    #[serde(flatten)]
    pub req: UpdateEvalResultScoresRequest,
}

#[command(
    name = "update_eval_result_scores",
    category = "evals",
    description = "Update scores for one eval result.",
    method = "PATCH",
    cli = CliRoute::new(&["evals", "runs", "results", "scores"], "update").with_examples(&[CliExample::new("Override one result's scores after a manual review", "everruns evals runs results scores update --eval-id eval_01h9 --run-id evalrun_01h9 --result-id evalresult_01h9 --scores '[{\"pass\":false,\"value\":0.0,\"reason\":\"Wrong refund amount\"}]' --status failed --reason 'Manual review'")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/results/{result_id}/scores",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = plain,
    request_body(UpdateEvalResultScoresRequest),
    responses((status = 404, description = "Eval result not found")),
)]
impl Command for UpdateEvalResultScores {
    type Output = EvalCaseResult;

    async fn execute(self, ctx: &Ctx) -> Result<EvalCaseResult, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        let result_id = q::parse_result_id(&self.result_id)?;
        q::service(ctx)
            .update_result_scores(
                &ctx.caller,
                &eval_id.to_string(),
                &run_id.to_string(),
                &result_id.to_string(),
                self.req,
            )
            .await?
            .ok_or_else(|| CommandError::not_found("EvalCaseResult"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct BulkUpdateEvalRunScores {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Eval run's prefixed public identifier.
    pub run_id: String,
    #[serde(flatten)]
    pub req: BulkUpdateEvalRunScoresRequest,
}

#[command(
    name = "bulk_update_eval_run_scores",
    category = "evals",
    description = "Bulk update scores for all results in an eval run.",
    method = "PATCH",
    cli = CliRoute::new(&["evals", "runs", "scores"], "bulk").with_examples(&[CliExample::new("Attach scores from an external grader to every result in a run", "everruns evals runs scores bulk --eval-id eval_01h9 --run-id evalrun_01h9 --results '[{\"result_id\":\"evalresult_01h9\",\"scores\":[{\"pass\":true,\"value\":1.0,\"reason\":\"Matches the reference\"}],\"status\":\"passed\"}]' --reason 'Apply grader output'")]),
    path = "/v1/evals/{eval_id}/runs/{run_id}/scores",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = list,
    request_body(BulkUpdateEvalRunScoresRequest),
)]
impl Command for BulkUpdateEvalRunScores {
    type Output = Vec<EvalCaseResult>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalCaseResult>, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .bulk_update_run_scores(
                &ctx.caller,
                &eval_id.to_string(),
                &run_id.to_string(),
                self.req,
            )
            .await
            .map_err(classify_anyhow)
    }
}
