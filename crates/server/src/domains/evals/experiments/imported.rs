// Imported runs as experiment variants (Mira, knowledge/evaluation/external-results-publishing.md).
//
// Decision: an import names the experiment it belongs to, and each eval's
// imported run becomes the variant with the given label. The experiment is
// found by name on that eval and created on first use, so a developer can run
// one variant in Mira and another in Everruns, or both in Mira, and read them
// side by side. Publishing the same label again replaces that variant; the
// earlier run stays, untagged.
//
// Decision: the run summary an import reports carries no per-case trial
// counts, which the comparison and the gate read. They are recounted from the
// stored results when the run joins the experiment; the rest of the reported
// summary stays as the external system sent it.

use super::types::ImportExperimentRef;
use super::{EvalExperimentId, MAX_LABEL_CHARS};
use crate::domains::evals::record::EvalRun;
use crate::domains::evals::summary::build_run_summary;
use crate::errors::BadRequestError;
use crate::storage::{CreateEvalExperimentRow, EvalRunRow, StorageBackend};
use anyhow::Result;
use everruns_core::Caller;
use std::collections::HashMap;
use uuid::Uuid;

const MAX_NAME_CHARS: usize = 200;

/// The system that ran an imported run, such as "mira"; `None` for runs
/// Everruns executed.
pub fn system_of(run: &EvalRunRow) -> Option<String> {
    if run.source != "external" {
        return None;
    }
    run.attribution
        .as_ref()
        .and_then(|a| a.get("system"))
        .and_then(|s| s.as_str())
        .map(str::to_string)
        .or_else(|| Some("external".to_string()))
}

/// Refuse a bad reference before anything is imported.
pub fn validate(reference: &ImportExperimentRef) -> Result<()> {
    let name = reference.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(BadRequestError::new(format!(
            "experiment names are 1 to {MAX_NAME_CHARS} characters"
        ))
        .into());
    }
    let label = reference.variant.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(BadRequestError::new(format!(
            "variant labels are 1 to {MAX_LABEL_CHARS} characters"
        ))
        .into());
    }
    Ok(())
}

/// Make each imported run the variant `reference.variant` of the experiment
/// `reference.name` on its eval.
pub async fn attach(
    db: &StorageBackend,
    caller: &Caller,
    runs: &[EvalRun],
    reference: &ImportExperimentRef,
) -> Result<()> {
    let name = reference.name.trim();
    let label = reference.variant.trim();
    for run in runs {
        let Some(row) = db.get_eval_run_by_id(run.internal_id).await? else {
            continue;
        };
        let cases = db.list_eval_cases(row.eval_id).await?;
        let results = db.list_eval_case_results(row.id).await?;
        let mut per_case: HashMap<Uuid, u32> = HashMap::new();
        for result in &results {
            *per_case.entry(result.eval_case_id).or_default() += 1;
        }
        let trials = per_case.values().copied().max().unwrap_or(1).max(1);

        let experiment = match db
            .find_eval_experiment_by_name(caller.org_id, row.eval_id, name)
            .await?
        {
            Some(experiment) => experiment,
            None => {
                let version = db.eval_dataset_version(row.eval_id).await?.unwrap_or(1);
                let id = Uuid::now_v7();
                db.create_eval_experiment(
                    caller.org_id,
                    CreateEvalExperimentRow {
                        id,
                        public_id: EvalExperimentId::from_uuid(id).to_string(),
                        eval_id: row.eval_id,
                        name: name.to_string(),
                        dataset_version: version,
                        trials: trials as i32,
                        max_cost_usd: None,
                        baseline_label: label.to_string(),
                        created_by_user_id: caller.user_id,
                    },
                )
                .await?
            }
        };

        db.release_eval_experiment_label(experiment.id, label, row.id)
            .await?;
        db.tag_eval_run_variant(row.id, experiment.id, label)
            .await?;

        let counted = build_run_summary(&cases, &results, trials);
        let mut summary = row
            .summary
            .clone()
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::to_value(&counted).unwrap_or_default());
        if let Some(fields) = summary.as_object_mut() {
            fields.insert("trials".into(), counted.trials.into());
            fields.insert("cases".into(), counted.cases.into());
            fields.insert(
                "cases_passed_every_trial".into(),
                counted.cases_passed_every_trial.into(),
            );
            fields.insert(
                "cases_passed_any_trial".into(),
                counted.cases_passed_any_trial.into(),
            );
        }
        db.set_eval_run_summary(row.id, summary).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::domains::common::{Command, Ctx};
    use crate::domains::evals::{ExportEvalSamples, ImportEvalRun, ImportEvalSamples};
    use crate::records::feature_flags::FeatureFlags;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID};
    use serde_json::json;
    use std::sync::Arc;

    fn ctx(db: &Arc<StorageBackend>) -> Ctx {
        let flags = FeatureFlags {
            evals: true,
            ..Default::default()
        };
        Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None)
            .with_feature_flags(flags)
    }

    /// A Mira import of three cases, two trials each, as one variant.
    fn import(run_id: &str, variant: &str, passing: &[&str]) -> ImportEvalRun {
        let cases: Vec<_> = ["refund", "greeting", "handoff"]
            .iter()
            .flat_map(|name| {
                let status = if passing.contains(name) {
                    "passed"
                } else {
                    "failed"
                };
                [0, 1].map(|_| {
                    json!({"name": name, "input": [format!("{name}?")],
                           "target": {"provider": "openai", "model": variant},
                           "status": status})
                })
            })
            .collect();
        serde_json::from_value(json!({
            "source": {"system": "mira", "run_id": run_id},
            "evals": [{"name": "support", "cases": cases}],
            "experiment": {"name": "Luna vs Sol", "variant": variant},
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn published_runs_become_variants_and_a_republish_replaces_one() {
        let db = Arc::new(StorageBackend::test_database());
        let ctx = ctx(&db);
        let caller = Caller::internal(DEFAULT_ORG_ID);

        import("r1", "luna", &["greeting"]).run(&ctx).await.unwrap();
        import("r2", "sol", &["greeting", "refund"])
            .run(&ctx)
            .await
            .unwrap();

        let listed = super::super::list(&db, &caller, None).await.unwrap();
        assert_eq!(listed.len(), 1);
        let experiment = &listed[0];
        assert_eq!(experiment.baseline, "luna");
        let labels: Vec<_> = experiment
            .variants
            .iter()
            .map(|v| v.label.as_str())
            .collect();
        assert_eq!(labels, ["luna", "sol"]);
        let sol = &experiment.variants[1];
        assert_eq!(sol.run_elsewhere.as_deref(), Some("mira"));
        let summary = sol.summary.as_ref().unwrap();
        assert_eq!((summary.cases, summary.trials), (3, 2));
        assert_eq!(summary.cases_passed_every_trial, 2);

        // The baseline is published again under a new run: it replaces the
        // old variant and still leads the comparison.
        import("r3", "luna", &["greeting", "refund", "handoff"])
            .run(&ctx)
            .await
            .unwrap();
        let detail = super::super::get(&db, &caller, &experiment.eval_id, &experiment.id)
            .await
            .unwrap()
            .unwrap();
        let labels: Vec<_> = detail.variants.iter().map(|v| v.label.as_str()).collect();
        assert_eq!(labels, ["luna", "sol"]);
        assert_eq!(
            detail.variants[0]
                .summary
                .as_ref()
                .unwrap()
                .cases_passed_every_trial,
            3
        );
        let comparison = detail.comparison.unwrap();
        assert_eq!(comparison.baseline, "luna");
        assert_eq!(comparison.cases.len(), 3);
    }

    #[tokio::test]
    async fn a_bad_experiment_reference_imports_nothing() {
        let db = Arc::new(StorageBackend::test_database());
        let mut cmd = import("r1", "luna", &[]);
        cmd.req.experiment.as_mut().unwrap().variant = "x".repeat(41);
        assert!(cmd.run(&ctx(&db)).await.is_err());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        assert!(
            super::super::list(&db, &caller, None)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_dataset_round_trips_through_mira_samples() {
        let db = Arc::new(StorageBackend::test_database());
        let ctx = ctx(&db);
        let runs = import("r1", "luna", &[]).run(&ctx).await.unwrap();
        let eval_id = {
            let row = db
                .get_eval_run_by_id(runs[0].internal_id)
                .await
                .unwrap()
                .unwrap();
            db.eval_public_ids(DEFAULT_ORG_ID, &[row.eval_id])
                .await
                .unwrap()
                .into_values()
                .next()
                .unwrap()
        };

        let samples: Vec<crate::domains::evals::samples::EvalSample> =
            serde_json::from_value(json!([
                {"id": "refund", "input": ["Refund $450"], "expected": "approval",
                 "attachments": [{"type": "image", "uri": "https://x/receipt.png"}]},
                {"id": "new-case", "input": ["Hi", "And again"], "tags": ["smoke"]}
            ]))
            .unwrap();
        let report = ImportEvalSamples {
            eval_id: eval_id.clone(),
            samples,
        }
        .run(&ctx)
        .await
        .unwrap();
        assert_eq!((report.created, report.updated), (1, 1));
        assert_eq!(report.left_behind, vec!["refund: attachments"]);

        let exported = ExportEvalSamples {
            eval_id: eval_id.clone(),
        }
        .run(&ctx)
        .await
        .unwrap();
        let refund = exported.samples.iter().find(|s| s.id == "refund").unwrap();
        assert_eq!(refund.input, vec!["Refund $450"]);
        assert_eq!(
            refund.metadata["everruns"]["scorers"][0],
            json!({"type": "contains", "text": "approval", "weight": 1.0})
        );

        // Importing the export changes nothing but the version.
        let again = ImportEvalSamples {
            eval_id,
            samples: exported.samples.clone(),
        }
        .run(&ctx)
        .await
        .unwrap();
        assert_eq!((again.created, again.updated), (0, 4));
    }
}
