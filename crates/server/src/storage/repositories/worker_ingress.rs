//! Uncached protocol floor. Empty means the existing unprotected worker plane.
use super::Database;

impl Database {
    /// No public or worker API writes this record. Missing schema/database is
    /// an error, never evidence that the legacy plane is allowed.
    pub(crate) async fn worker_ingress_minimum_version(&self) -> anyhow::Result<Option<i32>> {
        Ok(sqlx::query_scalar(
            "SELECT minimum_version FROM worker_ingress_protection_floor WHERE singleton",
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Direct/in-process workers have no scoped protocol authority. This is a
    /// startup refusal, not a guard on already captured direct store handles.
    pub(crate) async fn require_legacy_worker_hosting(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.worker_ingress_minimum_version().await?.is_none(),
            "Protected worker ingress does not support legacy in-process execution"
        );
        Ok(())
    }
}
