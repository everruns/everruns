//! Installations, schedules, leased resources, session tasks, and apps.

use super::*;

impl StorageBackend {
    // ============================================
    // Cross-Org Resource Resolution
    //
    // Lookup-by-public_id helpers that return the owning org without requiring
    // the caller to know it. Used only by the authenticated
    // GET /v1/resolve-org endpoint, which gates the result by the caller's
    // org memberships to preserve the 404-vs-403 enumeration guarantee.
    // See knowledge/security/multitenancy.md (Cross-Org Resource Resolution).
    // ============================================

    // ============================================
    // Session Tasks
    // ============================================

    // Per-task push-notification configs (EVE-682).

    /// Full retention prune (EVE-580): delete a bounded batch of terminal
    /// session tasks older than `now - ttl` (rows + messages), then remove
    /// each pruned task's recorded internal artifact subtree through the
    /// existing session-file deletion seam (which clears backing blobs for the
    /// object-storage backend). Returns the number of tasks pruned.
    ///
    /// Ordering: rows commit first, artifacts after, so a crash leaks at worst
    /// a dangling blob (reclaimed by blob GC) rather than a row pointing at a
    /// deleted artifact. Artifact deletion is best-effort and never fails the
    /// prune. Shared by the in-process Direct worker adapter and the gRPC
    /// `PruneTerminalSessionTasks` server handler.
    pub async fn prune_terminal_session_tasks_with_artifacts(
        &self,
        ttl: chrono::Duration,
        limit: i64,
    ) -> Result<usize> {
        // Defensive bound on a destructive query. Postgres treats `LIMIT <= 0`
        // (a negative value) as unbounded (`LIMIT ALL`), so a misconfigured or
        // legacy caller passing `limit <= 0` could turn this bounded retention
        // pass into an unlimited delete. Clamp to a positive, capped batch here
        // — the single chokepoint every caller (Direct adapter + gRPC handler)
        // funnels through — regardless of the caller's input. EVE-580 review.
        let limit = limit.clamp(1, MAX_RETENTION_PRUNE_LIMIT);
        let cutoff = chrono::Utc::now() - ttl;
        let pruned = self.prune_terminal_session_tasks(cutoff, limit).await?;

        for (session_id, task_id, result_path) in &pruned {
            let Some(result_path) = result_path.as_deref() else {
                continue;
            };
            let Some(dir) = task_artifact_delete_root(result_path) else {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    result_path = %result_path,
                    "Retention prune: skipped non-task artifact result path"
                );
                continue;
            };
            if let Err(e) = self
                .delete_session_file_recursive(session_id.uuid(), dir)
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    result_path = %result_path,
                    error = %e,
                    "Retention prune: failed to delete task artifacts (best-effort; blob GC will reclaim)"
                );
            }
        }

        Ok(pruned.len())
    }
}
