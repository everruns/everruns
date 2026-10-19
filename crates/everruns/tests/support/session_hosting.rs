use everruns::hosting::SessionHosting;
use everruns::{MessageId, SessionId};

/// Simulate independent alarm delivery. The test owns and can kill an executor
/// without also killing the authority, just like a durable host deployment.
#[derive(Clone)]
pub struct External<H>(pub H);
#[async_trait::async_trait]
impl<H: SessionHosting> SessionHosting for External<H> {
    async fn command(
        &self,
        session: everruns::SessionId,
        command: everruns::hosting::spi::session_commands::SessionCommand,
    ) -> everruns::hosting::spi::error::Result<serde_json::Value> {
        self.0.command(session, command).await
    }

    async fn query_snapshot(
        &self,
        id: SessionId,
    ) -> everruns_contracts::error::Result<everruns::hosting::SessionQuerySnapshot> {
        self.0.query_snapshot(id).await
    }
    async fn status(
        &self,
        id: SessionId,
    ) -> everruns_contracts::error::Result<everruns::hosting::HostingStatus> {
        self.0.status(id).await
    }
    async fn receipt(
        &self,
        id: SessionId,
        message: MessageId,
    ) -> everruns_contracts::error::Result<Option<everruns::hosting::AdmissionReceipt>> {
        self.0.receipt(id, message).await
    }
    fn reader(&self, id: SessionId) -> everruns_core::host::HostBackends {
        self.0.reader(id)
    }
    async fn record(
        &self,
        id: SessionId,
        key: &str,
    ) -> everruns_contracts::error::Result<Option<serde_json::Value>> {
        self.0.record(id, key).await
    }
    async fn claim(
        &self,
        id: SessionId,
        holder: &str,
        ttl: std::time::Duration,
    ) -> everruns_contracts::error::Result<Option<everruns::hosting::SessionActivation>> {
        self.0.claim(id, holder, ttl).await
    }
}
