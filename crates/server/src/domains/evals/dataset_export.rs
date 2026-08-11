// Async dataset export job. See `knowledge/evaluation/dataset-export.md` (Phase 2).
//
// Phase 1 built the reward-labeled NDJSON synchronously inside the export
// command. Phase 2 makes the export a handle: the command persists a dataset
// row and spawns this fire-and-forget background job (mirroring
// `spawn_eval_run`). The job reconstructs each surviving case's model-view
// messages, serializes one NDJSON record per case via the pure `dataset`
// helpers, and stores the produced NDJSON back on the row. If the server
// crashes mid-export the row stays non-terminal for diagnosis.

use std::sync::{Arc, LazyLock};

use everruns_core::capabilities::compaction::{CompactionConfig, build_model_view_messages};
use everruns_core::message_retriever::MessageRetriever;
use everruns_platform::eval::EvalRun;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use super::dataset::{self, DatasetFormat, ExportEvalRunDatasetRequest};
use crate::storage::StorageBackend;
use crate::storage::message_store::DbMessageRetriever;
use crate::storage::models::UpdateEvalRunDatasetRow;

const MAX_CONCURRENT_DATASET_EXPORTS: usize = 4;
pub const MAX_DATASET_EXPORT_BYTES: usize = crate::atif::ATIF_EXPORT_MAX_BYTES;
static DATASET_EXPORT_CONCURRENCY: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_DATASET_EXPORTS)));

pub fn try_acquire_export_permit() -> Option<OwnedSemaphorePermit> {
    DATASET_EXPORT_CONCURRENCY.clone().try_acquire_owned().ok()
}

fn ensure_line_fits(current_bytes: usize, line_bytes: usize) -> anyhow::Result<()> {
    let next_bytes = current_bytes
        .checked_add(line_bytes)
        .and_then(|size| size.checked_add(1))
        .ok_or_else(|| anyhow::anyhow!("dataset export size overflow"))?;
    if next_bytes > MAX_DATASET_EXPORT_BYTES {
        anyhow::bail!("dataset export exceeds the {MAX_DATASET_EXPORT_BYTES}-byte limit");
    }
    Ok(())
}

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
    //
    // Every format — including ATIF — folds the model-view messages, so ATIF
    // training rows never contain content the model did not read (masked/dropped
    // tool results). Only the whole-session export (`?format=atif`) still folds
    // the raw event log, which is a debug/backup surface. See
    // knowledge/evaluation/dataset-export.md (Model-view faithfulness) and knowledge/evaluation/atif-adoption.md.
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
        let record = if req.format == DatasetFormat::Atif {
            crate::atif::build_case_record_from_messages(
                run,
                result,
                &messages,
                crate::atif::AtifOptions {
                    redact_content: req.redaction.redact_content,
                },
            )
        } else {
            dataset::build_record(req.format, run, result, &messages, &req.redaction)
        };
        let line = serde_json::to_string(&record)?;
        ensure_line_fits(body.len(), line.len())?;
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
    permit: OwnedSemaphorePermit,
) {
    tokio::spawn(async move {
        let _permit = permit;
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

#[cfg(test)]
mod tests {
    use super::{MAX_DATASET_EXPORT_BYTES, ensure_line_fits};

    #[test]
    fn dataset_export_size_is_bounded() {
        assert!(ensure_line_fits(MAX_DATASET_EXPORT_BYTES - 2, 1).is_ok());
        assert!(ensure_line_fits(MAX_DATASET_EXPORT_BYTES - 1, 1).is_err());
        assert!(ensure_line_fits(usize::MAX, 1).is_err());
    }
}
