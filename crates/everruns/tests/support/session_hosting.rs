use everruns::hosting::SessionHosting;
use everruns::{MessageId, SessionId};

/// Simulate independent alarm delivery. The test owns and can kill an executor
/// without also killing the authority, just like a durable host deployment.
#[derive(Clone)]
pub struct External<H>(pub H);
#[async_trait::async_trait]
impl<H: SessionHosting> SessionHosting for External<H> {
    async fn resolve_waits(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: everruns::TurnId,
        approvals: Vec<everruns::hosting::ApprovalCommit>,
        results: Vec<everruns_core::events::ToolCompletedData>,
    ) -> everruns_contracts::error::Result<()> {
        self.0
            .resolve_waits(id, command, target, approvals, results)
            .await
    }
    async fn submit_approvals(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: everruns::TurnId,
        decisions: Vec<everruns_contracts::execution_api::ToolApprovalAnswer>,
    ) -> everruns_contracts::error::Result<
        everruns_contracts::execution_api::SubmitToolApprovalsResponse,
    > {
        self.0
            .submit_approvals(id, command, target, decisions)
            .await
    }
    async fn configure(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        patch: everruns::hosting::SessionConfiguration,
    ) -> everruns_contracts::error::Result<()> {
        self.0.configure(id, command, patch).await
    }
    async fn seed_history(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        messages: Vec<everruns_core::RuntimeMessage>,
    ) -> everruns_contracts::error::Result<bool> {
        self.0.seed_history(id, command, messages).await
    }
    async fn query_snapshot(
        &self,
        id: SessionId,
    ) -> everruns_contracts::error::Result<everruns::hosting::SessionQuerySnapshot> {
        self.0.query_snapshot(id).await
    }
    async fn approve_many(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        approvals: Vec<everruns::hosting::ApprovalCommit>,
    ) -> everruns_contracts::error::Result<()> {
        self.0.approve_many(id, command, approvals).await
    }
    async fn create(
        &self,
        id: SessionId,
        binding: &everruns::hosting::SessionBinding,
    ) -> everruns_contracts::error::Result<()> {
        self.0.create(id, binding).await
    }
    async fn status(
        &self,
        id: SessionId,
    ) -> everruns_contracts::error::Result<everruns::hosting::HostingStatus> {
        self.0.status(id).await
    }
    async fn admit(
        &self,
        id: SessionId,
        message: everruns_core::RuntimeMessage,
        cancelled: bool,
    ) -> everruns_contracts::error::Result<everruns::hosting::AdmissionReceipt> {
        self.0.admit(id, message, cancelled).await
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
    async fn resolve(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: everruns::TurnId,
        results: Vec<everruns_core::events::ToolCompletedData>,
    ) -> everruns_contracts::error::Result<()> {
        self.0.resolve(id, command, target, results).await
    }
    async fn cancel(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        input: MessageId,
    ) -> everruns_contracts::error::Result<()> {
        self.0.cancel(id, command, input).await
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
