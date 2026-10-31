//! Durable receipts for deployment-owned index operations. Organization-owned
//! indexer accounts never install this recorder; a selected BYOK account cannot
//! silently fall back to a managed service or incur its charges.
use super::*;
use anyhow::{Context, Result};
use async_trait::async_trait;
use everruns_turbopuffer::{Usage, UsageRecorder};

pub(super) struct IndexUsageRecorder {
    pub db: Arc<crate::storage::StorageBackend>,
    pub org_id: i64,
    pub collection_id: Uuid,
}

#[async_trait]
impl UsageRecorder for IndexUsageRecorder {
    async fn record(&self, source_ref: &str, usage: Usage) -> Result<()> {
        let written = i64::try_from(usage.written_bytes).context("index receipt overflow")?;
        let queried = i64::try_from(usage.queried_bytes).context("index receipt overflow")?;
        let returned = i64::try_from(usage.returned_bytes).context("index receipt overflow")?;
        // A control-plane/background pool is intentional: a supplier operation
        // already happened and a surrounding command rollback cannot undo it.
        // The command may already hold a request connection; using that pool
        // again can deadlock receipt writes when concurrent queries saturate it.
        sqlx::query("INSERT INTO knowledge_index_usage(id,org_id,collection_id,source_ref,written_bytes,queried_bytes,returned_bytes) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(source_ref) DO NOTHING")
            .bind(Uuid::now_v7()).bind(self.org_id).bind(self.collection_id).bind(source_ref)
            .bind(written).bind(queried).bind(returned).execute(self.db.background_pool()).await?;
        Ok(())
    }
}
