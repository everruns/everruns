// Durable leased-resource cleanup activity.
//
// Decision: Cleanup runs as a standalone durable activity scheduled by the
// control plane so it survives restarts and inherits durable scheduler
// observability (schedule executions, task history, worker heartbeats).
// Decision: The leased-resource model is generic and reusable by any subsystem
// that creates provider-owned state needing eventual cleanup.
// Decision: Cleanup completion/failure uses compare-and-set transitions keyed
// by the claim timestamp so stale cleanup attempts cannot overwrite a newer
// lease refresh.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::worker_adapters::WorkerAdapters;
use everruns_capabilities::leased_cleanup::cleanup_resource;

/// Durable activity input for leased-resource cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeasedResourceCleanupInput {
    /// Max resources to claim per cleanup run.
    pub batch_size: u32,
    /// How long a `cleaning` claim may be stale before another worker reclaims it.
    pub stale_after_seconds: u32,
    /// Backoff before retrying a failed cleanup attempt.
    pub retry_after_seconds: u32,
}

impl Default for LeasedResourceCleanupInput {
    fn default() -> Self {
        Self {
            batch_size: 25,
            stale_after_seconds: 5 * 60,
            retry_after_seconds: 60,
        }
    }
}

#[derive(Debug, Serialize)]
struct CleanupOutcome {
    resource_id: String,
    provider: String,
    resource_type: String,
    external_id: String,
    status: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct CleanupSummary {
    claimed: usize,
    released: usize,
    failed: usize,
    skipped: usize,
    outcomes: Vec<CleanupOutcome>,
}

/// Execute one leased-resource cleanup pass using the worker adapters.
pub async fn execute_cleanup_activity<A: WorkerAdapters>(
    adapters: &A,
    input: &LeasedResourceCleanupInput,
) -> Result<serde_json::Value> {
    let resources = adapters
        .claim_due_leased_resources(input.batch_size, input.stale_after_seconds)
        .await?;

    let mut summary = CleanupSummary {
        claimed: resources.len(),
        released: 0,
        failed: 0,
        skipped: 0,
        outcomes: Vec::with_capacity(resources.len()),
    };

    for (org_id, resource) in resources {
        let Some(expected_cleanup_started_at) = resource.cleanup_started_at else {
            summary.skipped += 1;
            summary.outcomes.push(CleanupOutcome {
                resource_id: resource.id.to_string(),
                provider: resource.provider.clone(),
                resource_type: resource.resource_type.clone(),
                external_id: resource.external_id.clone(),
                status: "skipped".to_string(),
                detail: "Resource claim missing cleanup_started_at".to_string(),
            });
            continue;
        };

        // The claim spans orgs; each lease's session storage and provider
        // connections are its own org's.
        let storage_store = adapters.storage_store(org_id);
        let resolver = adapters.connection_resolver(org_id);
        match cleanup_resource(&resource, resolver.as_ref(), storage_store.as_ref()).await {
            Ok(detail) => {
                let updated = adapters
                    .mark_leased_resource_released(resource.id, expected_cleanup_started_at)
                    .await?;
                if updated {
                    info!(
                        resource_id = %resource.id,
                        provider = %resource.provider,
                        resource_type = %resource.resource_type,
                        external_id = %resource.external_id,
                        "Leased resource cleanup released resource"
                    );
                } else {
                    info!(
                        resource_id = %resource.id,
                        provider = %resource.provider,
                        resource_type = %resource.resource_type,
                        external_id = %resource.external_id,
                        "Leased resource cleanup skipped stale release transition"
                    );
                }
                if updated {
                    summary.released += 1;
                } else {
                    summary.skipped += 1;
                }
                summary.outcomes.push(CleanupOutcome {
                    resource_id: resource.id.to_string(),
                    provider: resource.provider.clone(),
                    resource_type: resource.resource_type.clone(),
                    external_id: resource.external_id.clone(),
                    status: if updated { "released" } else { "skipped" }.to_string(),
                    detail,
                });
            }
            Err(error) => {
                let updated = adapters
                    .mark_leased_resource_cleanup_failed(
                        resource.id,
                        expected_cleanup_started_at,
                        input.retry_after_seconds,
                        &error.to_string(),
                    )
                    .await?;
                if updated {
                    warn!(
                        resource_id = %resource.id,
                        provider = %resource.provider,
                        resource_type = %resource.resource_type,
                        external_id = %resource.external_id,
                        error = %error,
                        "Leased resource cleanup failed; resource scheduled for retry"
                    );
                } else {
                    info!(
                        resource_id = %resource.id,
                        provider = %resource.provider,
                        resource_type = %resource.resource_type,
                        external_id = %resource.external_id,
                        error = %error,
                        "Leased resource cleanup skipped stale failure transition"
                    );
                }
                if updated {
                    summary.failed += 1;
                } else {
                    summary.skipped += 1;
                }
                summary.outcomes.push(CleanupOutcome {
                    resource_id: resource.id.to_string(),
                    provider: resource.provider.clone(),
                    resource_type: resource.resource_type.clone(),
                    external_id: resource.external_id.clone(),
                    status: if updated { "failed" } else { "skipped" }.to_string(),
                    detail: error.to_string(),
                });
            }
        }
    }

    info!(
        claimed = summary.claimed,
        released = summary.released,
        failed = summary.failed,
        skipped = summary.skipped,
        "Leased resource cleanup pass completed"
    );

    Ok(serde_json::to_value(summary)?)
}
