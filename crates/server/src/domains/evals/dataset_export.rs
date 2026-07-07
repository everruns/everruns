// Async dataset export job. See `specs/dataset-export.md` (Phase 2).
//
// Phase 1 built the reward-labeled NDJSON synchronously inside the export
// command. Phase 2 makes the export a handle: the command persists a dataset
// row and spawns this fire-and-forget background job (mirroring
// `spawn_eval_run`). The job reconstructs each surviving case's model-view
// messages, serializes one NDJSON record per case via the pure `dataset`
// helpers, and stores the produced NDJSON back on the row. If the server
// crashes mid-export the row stays non-terminal and the caller can re-enqueue.

use std::sync::Arc;

use everruns_core::capabilities::compaction::{CompactionConfig, build_model_view_messages};
use everruns_core::eval::EvalRun;
use everruns_core::message_retriever::MessageRetriever;
use uuid::Uuid;

use super::dataset::{self, ExportEvalRunDatasetRequest};
use crate::storage::StorageBackend;
use crate::storage::message_store::DbMessageRetriever;
use crate::storage::models::UpdateEvalRunDatasetRow;

/// Build the reward-labeled NDJSON body for a completed run.
///
/// Returns the NDJSON text and the number of records (surviving cases) written.
/// Pure over the DB read side: it only reads session messages, so it is reused
/// by both the async export job and the integration tests.
pub async fn build_dataset_ndjson(
    db: &Arc<StorageBackend>,
    run: &EvalRun,
    req: &ExportEvalRunDatasetRequest,
) -> anyhow::Result<(String, u64)> {
    // Reconstruct each case's conversation from session events, then apply the
    // compaction model-view masking so the dataset matches what the model saw
    // (not the lossless durable log). The default config is used here; honoring
    // the exact per-run compaction config is a documented follow-up.
    let retriever = DbMessageRetriever::new(db.clone());
    let compaction = CompactionConfig::default();

    let mut body = String::new();
    let mut count: u64 = 0;
    for result in &run.results {
        if !dataset::case_passes_filters(result, &req.filters) {
            continue;
        }
        // Eval-run case trajectories always have a session; skip any result
        // without one rather than emitting an empty trajectory.
        let Some(session_id) = result.session_id else {
            continue;
        };
        let stored = retriever
            .load(session_id)
            .await
            .map_err(|e| anyhow::anyhow!("load session messages: {e}"))?;
        let messages = build_model_view_messages(&stored, &compaction, None).messages;
        let record = dataset::build_record(req.format, run, result, &messages, &req.redaction);
        let line = serde_json::to_string(&record)?;
        body.push_str(&line);
        body.push('\n');
        count += 1;
    }

    Ok((body, count))
}

/// Spawn the async dataset export in the background.
///
/// `dataset_id` is the internal id of the pre-created `pending` dataset row;
/// `run` is the already-org-scoped, `Completed` run to export.
pub fn spawn_dataset_export(
    db: Arc<StorageBackend>,
    dataset_id: Uuid,
    run: EvalRun,
    req: ExportEvalRunDatasetRequest,
) {
    tokio::spawn(async move {
        if let Err(e) = run_dataset_export(&db, dataset_id, &run, &req).await {
            tracing::error!(dataset_id = %dataset_id, error = %e, "Dataset export failed");
            if let Err(update_err) = db
                .update_eval_run_dataset(
                    dataset_id,
                    UpdateEvalRunDatasetRow {
                        status: Some("failed".to_string()),
                        error_message: Some(e.to_string()),
                        ..Default::default()
                    },
                )
                .await
            {
                tracing::error!(
                    dataset_id = %dataset_id, error = %update_err,
                    "Failed to mark dataset export as failed"
                );
            }
        }
    });
}

async fn run_dataset_export(
    db: &Arc<StorageBackend>,
    dataset_id: Uuid,
    run: &EvalRun,
    req: &ExportEvalRunDatasetRequest,
) -> anyhow::Result<()> {
    db.update_eval_run_dataset(
        dataset_id,
        UpdateEvalRunDatasetRow {
            status: Some("running".to_string()),
            ..Default::default()
        },
    )
    .await?;

    let (body, count) = build_dataset_ndjson(db, run, req).await?;

    db.update_eval_run_dataset(
        dataset_id,
        UpdateEvalRunDatasetRow {
            status: Some("completed".to_string()),
            body: Some(body),
            record_count: Some(count as i64),
            ..Default::default()
        },
    )
    .await?;

    tracing::info!(dataset_id = %dataset_id, records = count, "Dataset export completed");
    Ok(())
}
