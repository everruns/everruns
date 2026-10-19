use crate::hosting::SessionHosting;
use crate::{MessageId, SessionId};

// Disable the process scheduler for explicitly owned activations.
// Deployment harnesses separately qualify independent wakeup delivery.
#[derive(Clone)]
pub(super) struct Manual<H>(pub H);
#[async_trait::async_trait]
impl<H: SessionHosting> SessionHosting for Manual<H> {
    async fn command(
        &self,
        session: crate::SessionId,
        command: crate::hosting::spi::session_commands::SessionCommand,
    ) -> crate::hosting::spi::error::Result<serde_json::Value> {
        self.0.command(session, command).await
    }

    async fn query_snapshot(
        &self,
        id: SessionId,
    ) -> crate::hosting::spi::error::Result<crate::hosting::SessionQuerySnapshot> {
        self.0.query_snapshot(id).await
    }
    async fn status(
        &self,
        id: SessionId,
    ) -> crate::hosting::spi::error::Result<crate::hosting::HostingStatus> {
        self.0.status(id).await
    }
    async fn receipt(
        &self,
        id: SessionId,
        message: MessageId,
    ) -> crate::hosting::spi::error::Result<Option<crate::hosting::AdmissionReceipt>> {
        self.0.receipt(id, message).await
    }
    fn reader(&self, id: SessionId) -> crate::hosting::spi::HostBackends {
        self.0.reader(id)
    }
    async fn record(
        &self,
        id: SessionId,
        key: &str,
    ) -> crate::hosting::spi::error::Result<Option<serde_json::Value>> {
        self.0.record(id, key).await
    }
    async fn claim(
        &self,
        id: SessionId,
        holder: &str,
        ttl: std::time::Duration,
    ) -> crate::hosting::spi::error::Result<Option<crate::hosting::SessionActivation>> {
        self.0.claim(id, holder, ttl).await
    }
}
