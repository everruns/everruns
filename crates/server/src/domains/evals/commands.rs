use super::queries as q;
use super::types::{
    BulkUpdateEvalRunScoresRequest, CreateEvalCaseRequest, CreateEvalRequest, CreateEvalRunRequest,
    EvalImportPreflight, EvalRunShareLink, EvalRunShareStatus, ImportEvalRunRequest,
    ListEvalsQuery, UpdateEvalCaseRequest, UpdateEvalRequest, UpdateEvalResultScoresRequest,
};
use crate::domains::common::*;
use everruns_platform::eval::{Eval, EvalCase, EvalCaseResult, EvalRun};
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

#[derive(Debug, Deserialize)]
pub struct CreateEval(pub CreateEvalRequest);

impl CommandSchema for CreateEval {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateEvalRequest>()
    }
}

impl Command for CreateEval {
    type Output = Eval;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_eval",
            category: "evals",
            description: "Create a new eval.",
            method: "POST",
            path: "/v1/evals",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Eval, CommandError> {
        require_evals_enabled(ctx)?;
        q::service(ctx)
            .create(&ctx.caller, self.0)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateEval>() }

#[derive(Debug, Default, Deserialize)]
pub struct ListEvals {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl CommandSchema for ListEvals {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<ListEvalsQuery>()
    }
}

impl Command for ListEvals {
    type Output = Vec<Eval>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_evals",
            category: "evals",
            description: "List evals.",
            method: "GET",
            path: "/v1/evals",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Eval>, CommandError> {
        require_evals_enabled(ctx)?;
        q::service(ctx)
            .list(&ctx.caller, self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListEvals>() }

/// Import a full external run group (everruns as host/viewer for external eval
/// systems). See proposals/mira-results-publishing.md.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportEvalRun {
    #[serde(flatten)]
    pub req: ImportEvalRunRequest,
}

impl Command for ImportEvalRun {
    type Output = Vec<EvalRun>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "import_eval_run",
            category: "evals",
            description: "Import externally-executed eval results.",
            method: "POST",
            path: "/v1/evals/import",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_IMPORT)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalRun>, CommandError> {
        require_evals_enabled(ctx)?;
        q::service(ctx)
            .import_run(&ctx.caller, self.req)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ImportEvalRun>() }

/// Import ATIF trajectories as eval cases (knowledge/evaluation/atif-adoption.md).
///
/// `body` is the raw import payload: NDJSON (one trajectory per line), a JSON
/// array of trajectories, a single trajectory object, or `{ "trajectories":
/// [...] }`. Cases are upserted by name, so re-import is idempotent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportAtifTrajectories {
    pub eval_id: String,
    /// Raw ATIF payload (NDJSON or JSON).
    pub body: String,
}

impl Command for ImportAtifTrajectories {
    type Output = crate::api::evals::AtifImportReport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "import_atif_trajectories",
            category: "evals",
            description: "Import ATIF trajectories as eval cases (upserted by name).",
            method: "POST",
            path: "/v1/evals/{eval_id}/atif_import",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

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

inventory::submit! { CommandDescriptor::of::<ImportAtifTrajectories>() }

/// Preflight: report whether the caller can import (feature enabled + has the
/// eval-management permission) without failing. No policy gate so any org
/// member can probe; the report just returns `false`.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct EvalImportPreflightCmd {}

impl Command for EvalImportPreflightCmd {
    type Output = EvalImportPreflight;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "eval_import_preflight",
            category: "evals",
            description: "Report whether the caller can import eval results.",
            method: "GET",
            path: "/v1/evals/import/preflight",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<EvalImportPreflightCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetEval {
    pub eval_id: String,
}

impl Command for GetEval {
    type Output = Eval;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_eval",
            category: "evals",
            description: "Get a single eval.",
            method: "GET",
            path: "/v1/evals/{eval_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("eval_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Eval, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .get_by_public_id(&ctx.caller, &eval_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Eval"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetEval>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateEval {
    pub eval_id: String,
    #[serde(flatten)]
    pub req: UpdateEvalRequest,
}

impl Command for UpdateEval {
    type Output = Eval;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_eval",
            category: "evals",
            description: "Update an eval.",
            method: "PATCH",
            path: "/v1/evals/{eval_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Eval, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .update(&ctx.caller, &eval_id.to_string(), self.req)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Eval"))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateEval>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteEval {
    pub eval_id: String,
}

impl Command for DeleteEval {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_eval",
            category: "evals",
            description: "Delete an eval.",
            method: "DELETE",
            path: "/v1/evals/{eval_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let deleted = q::service(ctx)
            .delete(&ctx.caller, &eval_id.to_string())
            .await
            .map_err(classify_anyhow)?;
        if deleted {
            Ok(true)
        } else {
            Err(CommandError::not_found("Eval"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteEval>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateEvalCase {
    pub eval_id: String,
    #[serde(flatten)]
    pub req: CreateEvalCaseRequest,
}

impl Command for CreateEvalCase {
    type Output = EvalCase;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_eval_case",
            category: "evals",
            description: "Create an eval case.",
            method: "POST",
            path: "/v1/evals/{eval_id}/cases",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<EvalCase, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .create_case(&ctx.caller, &eval_id.to_string(), self.req)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateEvalCase>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListEvalCases {
    pub eval_id: String,
}

impl Command for ListEvalCases {
    type Output = Vec<EvalCase>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_eval_cases",
            category: "evals",
            description: "List eval cases.",
            method: "GET",
            path: "/v1/evals/{eval_id}/cases",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalCase>, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .list_cases(&ctx.caller, &eval_id.to_string())
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListEvalCases>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetEvalCase {
    pub eval_id: String,
    pub case_id: String,
}

impl Command for GetEvalCase {
    type Output = EvalCase;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_eval_case",
            category: "evals",
            description: "Get an eval case.",
            method: "GET",
            path: "/v1/evals/{eval_id}/cases/{case_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<EvalCase, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let case_id = q::parse_case_id(&self.case_id)?;
        q::service(ctx)
            .get_case(&ctx.caller, &eval_id.to_string(), &case_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("EvalCase"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetEvalCase>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateEvalCase {
    pub eval_id: String,
    pub case_id: String,
    #[serde(flatten)]
    pub req: UpdateEvalCaseRequest,
}

impl Command for UpdateEvalCase {
    type Output = EvalCase;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_eval_case",
            category: "evals",
            description: "Update an eval case.",
            method: "PATCH",
            path: "/v1/evals/{eval_id}/cases/{case_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

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
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("EvalCase"))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateEvalCase>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteEvalCase {
    pub eval_id: String,
    pub case_id: String,
}

impl Command for DeleteEvalCase {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_eval_case",
            category: "evals",
            description: "Delete an eval case.",
            method: "DELETE",
            path: "/v1/evals/{eval_id}/cases/{case_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let case_id = q::parse_case_id(&self.case_id)?;
        let deleted = q::service(ctx)
            .delete_case(&ctx.caller, &eval_id.to_string(), &case_id.to_string())
            .await
            .map_err(classify_anyhow)?;
        if deleted {
            Ok(true)
        } else {
            Err(CommandError::not_found("EvalCase"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteEvalCase>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateEvalRun {
    pub eval_id: String,
    #[serde(flatten)]
    pub req: CreateEvalRunRequest,
}

impl Command for CreateEvalRun {
    type Output = EvalRun;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_eval_run",
            category: "evals",
            description: "Create an eval run.",
            method: "POST",
            path: "/v1/evals/{eval_id}/runs",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_RUN)
    }

    async fn execute(self, ctx: &Ctx) -> Result<EvalRun, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .create_run(&ctx.caller, &eval_id.to_string(), self.req)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateEvalRun>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListEvalRuns {
    pub eval_id: String,
}

impl Command for ListEvalRuns {
    type Output = Vec<EvalRun>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_eval_runs",
            category: "evals",
            description: "List eval runs.",
            method: "GET",
            path: "/v1/evals/{eval_id}/runs",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EvalRun>, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        q::service(ctx)
            .list_runs(&ctx.caller, &eval_id.to_string())
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListEvalRuns>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetEvalRun {
    pub eval_id: String,
    pub run_id: String,
}

impl Command for GetEvalRun {
    type Output = EvalRun;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_eval_run",
            category: "evals",
            description: "Get an eval run.",
            method: "GET",
            path: "/v1/evals/{eval_id}/runs/{run_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<EvalRun, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .get_run(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("EvalRun"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetEvalRun>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportEvalRunArtifacts {
    pub eval_id: String,
    pub run_id: String,
}

impl Command for ExportEvalRunArtifacts {
    type Output = EvalArtifactExport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "export_eval_run_artifacts",
            category: "evals",
            description: "Export eval run artifacts as NDJSON.",
            method: "GET",
            path: "/v1/evals/{eval_id}/runs/{run_id}/artifacts",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<EvalArtifactExport, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        let run = q::service(ctx)
            .get_run(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await
            .map_err(classify_anyhow)?
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

inventory::submit! { CommandDescriptor::of::<ExportEvalRunArtifacts>() }

/// Enqueue an async dataset export for a completed eval run and return a handle.
///
/// The export runs as a fire-and-forget background job (mirroring the eval
/// runner): it reconstructs each surviving case's model-view messages (faithful
/// to what the model saw, via the compaction model-view masking), joins reward +
/// efficiency metadata, and stores the produced NDJSON on the handle. Fetch the
/// NDJSON via `GET .../dataset/{dataset_id}` once `status` is `completed`. Gated
/// by `DATASET_EXPORT` and org-scoped through `get_run` (which resolves only
/// runs owned by the caller's org).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportEvalRunDataset {
    pub eval_id: String,
    pub run_id: String,
    pub req: super::dataset::ExportEvalRunDatasetRequest,
}

impl Command for ExportEvalRunDataset {
    type Output = everruns_platform::eval::EvalRunDataset;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "export_eval_run_dataset",
            category: "evals",
            description: "Enqueue an async reward-labeled trajectory dataset export from a completed eval run.",
            method: "POST",
            path: "/v1/evals/{eval_id}/runs/{run_id}/dataset",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::DATASET_EXPORT)
    }

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
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("EvalRun"))
    }
}

inventory::submit! { CommandDescriptor::of::<ExportEvalRunDataset>() }

/// Fetch an async dataset-export handle: status plus the produced NDJSON `body`
/// once the export is `completed`. Org-scoped through `get_dataset`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetEvalRunDataset {
    pub eval_id: String,
    pub run_id: String,
    pub dataset_id: String,
}

impl Command for GetEvalRunDataset {
    type Output = everruns_platform::eval::EvalRunDataset;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_eval_run_dataset",
            category: "evals",
            description: "Fetch an eval-run dataset export handle (status + NDJSON body).",
            method: "GET",
            path: "/v1/evals/{eval_id}/runs/{run_id}/dataset/{dataset_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::DATASET_EXPORT)
    }

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
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("EvalRunDataset"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetEvalRunDataset>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelEvalRun {
    pub eval_id: String,
    pub run_id: String,
}

impl Command for CancelEvalRun {
    type Output = EvalRun;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "cancel_eval_run",
            category: "evals",
            description: "Cancel an eval run.",
            method: "POST",
            path: "/v1/evals/{eval_id}/runs/{run_id}/cancel",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<EvalRun, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        q::service(ctx)
            .cancel_run(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("EvalRun"))
    }
}

inventory::submit! { CommandDescriptor::of::<CancelEvalRun>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateEvalRunShare {
    pub eval_id: String,
    pub run_id: String,
}

impl Command for CreateEvalRunShare {
    type Output = EvalRunShareLink;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_eval_run_share",
            category: "evals",
            description: "Mint a read-only share link for an eval run.",
            method: "POST",
            path: "/v1/evals/{eval_id}/runs/{run_id}/share",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

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

inventory::submit! { CommandDescriptor::of::<CreateEvalRunShare>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetEvalRunShare {
    pub eval_id: String,
    pub run_id: String,
}

impl Command for GetEvalRunShare {
    type Output = EvalRunShareStatus;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_eval_run_share",
            category: "evals",
            description: "Whether an eval run has an active share link.",
            method: "GET",
            path: "/v1/evals/{eval_id}/runs/{run_id}/share",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<EvalRunShareStatus, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        let active = q::service(ctx)
            .run_has_active_share(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await
            .map_err(classify_anyhow)?;
        Ok(EvalRunShareStatus { active })
    }
}

inventory::submit! { CommandDescriptor::of::<GetEvalRunShare>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct RevokeEvalRunShare {
    pub eval_id: String,
    pub run_id: String,
}

impl Command for RevokeEvalRunShare {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "revoke_eval_run_share",
            category: "evals",
            description: "Revoke all share links for an eval run.",
            method: "DELETE",
            path: "/v1/evals/{eval_id}/runs/{run_id}/share",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

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

inventory::submit! { CommandDescriptor::of::<RevokeEvalRunShare>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateEvalResultScores {
    pub eval_id: String,
    pub run_id: String,
    pub result_id: String,
    #[serde(flatten)]
    pub req: UpdateEvalResultScoresRequest,
}

impl Command for UpdateEvalResultScores {
    type Output = EvalCaseResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_eval_result_scores",
            category: "evals",
            description: "Update scores for one eval result.",
            method: "PATCH",
            path: "/v1/evals/{eval_id}/runs/{run_id}/results/{result_id}/scores",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

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
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("EvalCaseResult"))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateEvalResultScores>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkUpdateEvalRunScores {
    pub eval_id: String,
    pub run_id: String,
    #[serde(flatten)]
    pub req: BulkUpdateEvalRunScoresRequest,
}

impl Command for BulkUpdateEvalRunScores {
    type Output = Vec<EvalCaseResult>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "bulk_update_eval_run_scores",
            category: "evals",
            description: "Bulk update scores for all results in an eval run.",
            method: "PATCH",
            path: "/v1/evals/{eval_id}/runs/{run_id}/scores",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::EVAL_MANAGE)
    }

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

inventory::submit! { CommandDescriptor::of::<BulkUpdateEvalRunScores>() }
