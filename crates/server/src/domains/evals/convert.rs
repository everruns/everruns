// Row-to-record conversions shared by the eval service and experiments.

use crate::domains::evals::record::*;
use everruns_contracts::typed_id::{EvalCaseId, EvalRunId};

pub(crate) fn case_row_to_case(row: crate::storage::EvalCaseRow) -> EvalCase {
    let target: Option<EvalTarget> = row
        .target
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    EvalCase {
        public_id: row
            .public_id
            .parse()
            .unwrap_or_else(|_| EvalCaseId::from_uuid(row.id)),
        internal_id: row.id,
        name: row.name,
        description: row.description,
        target,
        tags: row.tags,
        conversation: serde_json::from_value(row.conversation).unwrap_or_default(),
        post: row.post.and_then(|v| serde_json::from_value(v).ok()),
        artifacts: row.artifacts.and_then(|v| serde_json::from_value(v).ok()),
        scorers: serde_json::from_value(row.scorers).unwrap_or_default(),
        max_turns: row.max_turns.map(|v| v as u32),
        timeout_seconds: row.timeout_seconds.map(|v| v as u32),
        position: row.position,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub(crate) fn run_row_to_run(
    row: crate::storage::EvalRunRow,
    results: Vec<EvalCaseResult>,
) -> EvalRun {
    let target: Option<EvalTarget> = row
        .target
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    EvalRun {
        public_id: row
            .public_id
            .parse()
            .unwrap_or_else(|_| EvalRunId::from_uuid(row.id)),
        internal_id: row.id,
        org_id: row.org_id,
        target,
        model_override: row.model_override,
        filter_tags: row.filter_tags,
        status: EvalRunStatus::from(row.status.as_str()),
        source: EvalRunSource::from(row.source.as_str()),
        attribution: row.attribution,
        triggered_by: row.triggered_by,
        trials: row.trials.max(1) as u32,
        max_cost_usd: row.max_cost_usd,
        dataset_version: row.dataset_version.map(|v| v.max(1) as u32),
        experiment_id: row
            .experiment_id
            .map(|id| super::experiments::EvalExperimentId::from_uuid(id).to_string()),
        variant_label: row.variant_label,
        started_at: row.started_at,
        completed_at: row.completed_at,
        summary: row.summary.and_then(|s| serde_json::from_value(s).ok()),
        results,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
