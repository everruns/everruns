//! Lease-fenced journals the worker keeps through the control plane: native
//! async tool checkpoints and OpenAI Agents API orchestration state.

use super::*;

impl GrpcAdapter {
    async fn native_async_operation(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
        operation: proto::native_async_journal_request::Operation,
        checkpoint_json: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let response = self
            .client
            .inner
            .client()
            .native_async_journal(proto::NativeAsyncJournalRequest {
                operation: operation as i32,
                org_id: lease.org_id,
                session_id: Some(uuid_to_proto(lease.session_id.uuid())),
                turn_id: Some(uuid_to_proto(lease.turn_id.uuid())),
                owner: Some(uuid_to_proto(lease.owner)),
                checkpoint_json,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(response.checkpoint_json)
    }
}

#[async_trait]
impl everruns_core::native_async_store::NativeAsyncStore for GrpcAdapter {
    async fn acquire(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
    ) -> Result<everruns_contracts::native_async::NativeAsyncCheckpoint> {
        let bytes = self
            .native_async_operation(
                lease,
                proto::native_async_journal_request::Operation::Acquire,
                vec![],
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|error| AgentLoopError::store(error.to_string()))
    }
    async fn load(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
    ) -> Result<everruns_contracts::native_async::NativeAsyncCheckpoint> {
        let bytes = self
            .native_async_operation(
                lease,
                proto::native_async_journal_request::Operation::Load,
                vec![],
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|error| AgentLoopError::store(error.to_string()))
    }
    async fn renew(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
    ) -> Result<()> {
        self.native_async_operation(
            lease,
            proto::native_async_journal_request::Operation::Renew,
            vec![],
        )
        .await?;
        Ok(())
    }
    async fn save(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
        checkpoint: &everruns_contracts::native_async::NativeAsyncCheckpoint,
    ) -> Result<()> {
        let bytes = serde_json::to_vec(checkpoint)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        self.native_async_operation(
            lease,
            proto::native_async_journal_request::Operation::Save,
            bytes,
        )
        .await?;
        Ok(())
    }
    async fn release(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
    ) -> Result<()> {
        self.native_async_operation(
            lease,
            proto::native_async_journal_request::Operation::Release,
            vec![],
        )
        .await?;
        Ok(())
    }
}

impl GrpcAdapter {
    async fn agents_api_operation(
        &self,
        lease: everruns_core::agents_api_store::AgentsApiLease,
        operation: proto::agents_api_journal_request::Operation,
        checkpoint_json: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let response = self
            .client
            .inner
            .client()
            .agents_api_journal(proto::AgentsApiJournalRequest {
                operation: operation as i32,
                org_id: lease.org_id,
                session_id: Some(uuid_to_proto(lease.session_id.uuid())),
                owner: Some(uuid_to_proto(lease.owner)),
                checkpoint_json,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(response.checkpoint_json)
    }
}

#[async_trait]
impl everruns_core::agents_api_store::AgentsApiStore for GrpcAdapter {
    async fn acquire(
        &self,
        lease: everruns_core::agents_api_store::AgentsApiLease,
    ) -> Result<everruns_core::agents_api_store::AgentsApiCheckpoint> {
        let bytes = self
            .agents_api_operation(
                lease,
                proto::agents_api_journal_request::Operation::Acquire,
                vec![],
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|error| AgentLoopError::store(error.to_string()))
    }
    async fn renew(&self, lease: everruns_core::agents_api_store::AgentsApiLease) -> Result<()> {
        self.agents_api_operation(
            lease,
            proto::agents_api_journal_request::Operation::Renew,
            vec![],
        )
        .await?;
        Ok(())
    }
    async fn save(
        &self,
        lease: everruns_core::agents_api_store::AgentsApiLease,
        checkpoint: &everruns_core::agents_api_store::AgentsApiCheckpoint,
    ) -> Result<()> {
        let bytes = serde_json::to_vec(checkpoint)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        self.agents_api_operation(
            lease,
            proto::agents_api_journal_request::Operation::Save,
            bytes,
        )
        .await?;
        Ok(())
    }
    async fn release(&self, lease: everruns_core::agents_api_store::AgentsApiLease) -> Result<()> {
        self.agents_api_operation(
            lease,
            proto::agents_api_journal_request::Operation::Release,
            vec![],
        )
        .await?;
        Ok(())
    }
}
