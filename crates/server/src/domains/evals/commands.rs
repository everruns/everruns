use super::queries as q;
use super::types::{
    BulkUpdateEvalRunScoresRequest, CreateEvalCaseRequest, CreateEvalRequest, CreateEvalRunRequest,
    EvalImportPreflight, ImportEvalRunRequest, ListEvalsQuery, UpdateEvalCaseRequest,
    UpdateEvalRequest, UpdateEvalResultScoresRequest,
};
use crate::domains::common::*;
use everruns_core::eval::{Eval, EvalCase, EvalCaseResult, EvalRun};
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
        Err(CommandError::not_found("Evals"))
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

/// Export a completed eval run as a reward-labeled trajectory dataset (NDJSON).
///
/// One record per case that survives the filters: model-view messages (faithful
/// to what the model saw, via the compaction model-view masking) joined with the
/// case reward and efficiency metadata. Gated by `DATASET_EXPORT` and org-scoped
/// through `get_run` (which resolves only runs owned by the caller's org).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportEvalRunDataset {
    pub eval_id: String,
    pub run_id: String,
    pub req: super::dataset::ExportEvalRunDatasetRequest,
}

impl Command for ExportEvalRunDataset {
    type Output = EvalArtifactExport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "export_eval_run_dataset",
            category: "evals",
            description: "Export a completed eval run as a reward-labeled trajectory dataset (NDJSON).",
            method: "POST",
            path: "/v1/evals/{eval_id}/runs/{run_id}/dataset",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::evals::DATASET_EXPORT)
    }

    async fn execute(self, ctx: &Ctx) -> Result<EvalArtifactExport, CommandError> {
        use everruns_core::capabilities::compaction::{
            CompactionConfig, build_model_view_messages,
        };
        use everruns_core::message_retriever::MessageRetriever;

        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let run_id = q::parse_run_id(&self.run_id)?;
        let run = q::service(ctx)
            .get_run(&ctx.caller, &eval_id.to_string(), &run_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("EvalRun"))?;

        if run.status != everruns_core::eval::EvalRunStatus::Completed {
            return Err(CommandError::bad_request(
                "Dataset export requires a completed eval run",
            ));
        }

        // Reconstruct each case's conversation from session events, then apply the
        // compaction model-view masking so the dataset matches what the model saw
        // (not the lossless durable log). Default config is used here; honoring the
        // exact per-run compaction config is a documented follow-up.
        let retriever = crate::storage::message_store::DbMessageRetriever::new(ctx.db.clone());
        let compaction = CompactionConfig::default();

        let mut body = String::new();
        for result in &run.results {
            if !super::dataset::case_passes_filters(result, &self.req.filters) {
                continue;
            }
            // Phase 1 exports eval-run case trajectories, which always have a
            // session; skip any result without one rather than emitting an empty
            // trajectory.
            let Some(session_id) = result.session_id else {
                continue;
            };
            let stored = retriever.load(session_id).await.map_err(|e| {
                CommandError::internal(anyhow::anyhow!("load session messages: {e}"))
            })?;
            let messages = build_model_view_messages(&stored, &compaction, None).messages;
            let record = super::dataset::build_record(
                self.req.format,
                &run,
                result,
                &messages,
                &self.req.redaction,
            );
            let line =
                serde_json::to_string(&record).map_err(|e| CommandError::internal(e.into()))?;
            body.push_str(&line);
            body.push('\n');
        }

        Ok(EvalArtifactExport { body })
    }
}

inventory::submit! { CommandDescriptor::of::<ExportEvalRunDataset>() }

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
