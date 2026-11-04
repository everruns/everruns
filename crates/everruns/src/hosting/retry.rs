//! Provider retries use the fenced session authority. Lease-expiry recovery
//! covers owner loss between storing a checkpoint, arming and releasing.
use super::*;
use everruns_contracts::durable_retry::{RetryCheckpoint, RetryHost};

pub(super) struct ProviderRetries {
    pub engine: crate::Engine,
    pub session: SessionId,
    pub epoch: Arc<dyn SessionEpoch>,
}

#[async_trait]
impl RetryHost for ProviderRetries {
    async fn now_ms(&self) -> Result<i64> {
        Ok(self.engine.session_status(self.session).await?.now_ms)
    }
    async fn load(&self, operation: &str) -> Result<Option<RetryCheckpoint>> {
        self.epoch
            .record(&format!("provider_retry/{operation}"))
            .await?
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| spi::error::AgentLoopError::store("invalid provider retry checkpoint"))
    }
    async fn save(&self, operation: &str, checkpoint: &RetryCheckpoint) -> Result<()> {
        self.epoch
            .commit(
                &format!("provider_retry/{operation}"),
                serde_json::to_value(checkpoint).map_err(|_| {
                    spi::error::AgentLoopError::store("invalid provider retry checkpoint")
                })?,
                false,
            )
            .await?;
        Ok(())
    }
    async fn schedule(&self, at_ms: i64) -> Result<()> {
        self.epoch.schedule(Some(at_ms)).await
    }
}
