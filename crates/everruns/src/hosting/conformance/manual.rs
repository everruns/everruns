use crate::hosting::SessionHosting;
use crate::{MessageId, SessionId};

// Disable the process scheduler for explicitly owned activations.
// Deployment harnesses separately qualify independent wakeup delivery.
#[derive(Clone)]
pub(super) struct Manual<H>(pub H);
#[async_trait::async_trait]
impl<H: SessionHosting> SessionHosting for Manual<H> {
    async fn resolve_waits(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: crate::TurnId,
        approvals: Vec<crate::hosting::ApprovalCommit>,
        results: Vec<crate::hosting::spi::events::ToolCompletedData>,
    ) -> crate::hosting::spi::error::Result<()> {
        self.0
            .resolve_waits(id, command, target, approvals, results)
            .await
    }
    async fn submit_approvals(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: crate::TurnId,
        decisions: Vec<crate::execution_api::ToolApprovalAnswer>,
    ) -> crate::hosting::spi::error::Result<crate::execution_api::SubmitToolApprovalsResponse> {
        self.0
            .submit_approvals(id, command, target, decisions)
            .await
    }
    async fn configure(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        patch: crate::hosting::SessionConfiguration,
    ) -> crate::hosting::spi::error::Result<()> {
        self.0.configure(id, command, patch).await
    }
    async fn seed_history(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        messages: Vec<crate::hosting::spi::RuntimeMessage>,
    ) -> crate::hosting::spi::error::Result<bool> {
        self.0.seed_history(id, command, messages).await
    }
    async fn query_snapshot(
        &self,
        id: SessionId,
    ) -> crate::hosting::spi::error::Result<crate::hosting::SessionQuerySnapshot> {
        self.0.query_snapshot(id).await
    }
    async fn approve_many(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        approvals: Vec<crate::hosting::ApprovalCommit>,
    ) -> crate::hosting::spi::error::Result<()> {
        self.0.approve_many(id, command, approvals).await
    }
    async fn create(
        &self,
        id: SessionId,
        binding: &crate::hosting::SessionBinding,
    ) -> crate::hosting::spi::error::Result<()> {
        self.0.create(id, binding).await
    }
    async fn status(
        &self,
        id: SessionId,
    ) -> crate::hosting::spi::error::Result<crate::hosting::HostingStatus> {
        self.0.status(id).await
    }
    async fn admit(
        &self,
        id: SessionId,
        message: crate::hosting::spi::RuntimeMessage,
        cancelled: bool,
    ) -> crate::hosting::spi::error::Result<crate::hosting::AdmissionReceipt> {
        self.0.admit(id, message, cancelled).await
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
    async fn resolve(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: crate::TurnId,
        results: Vec<crate::hosting::spi::events::ToolCompletedData>,
    ) -> crate::hosting::spi::error::Result<()> {
        self.0.resolve(id, command, target, results).await
    }
    async fn cancel(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        input: MessageId,
    ) -> crate::hosting::spi::error::Result<()> {
        self.0.cancel(id, command, input).await
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
