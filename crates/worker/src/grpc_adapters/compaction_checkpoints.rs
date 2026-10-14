//! Compaction checkpoints over the control plane's gRPC store.

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_internal_protocol::proto;

use super::checkpoint_availability::is_encryption_not_configured;
use super::{GrpcAdapter, grpc_status_to_error, proto_uuid_to_uuid, uuid_to_proto};

#[async_trait]
impl crate::core::CompactionCheckpointStore for GrpcAdapter {
    async fn get_latest(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<crate::core::CompactionCheckpoint>> {
        let mut client = self.client.inner.client();
        let response = client
            .get_compaction_checkpoint(proto::GetCompactionCheckpointRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                provider_type: provider_type.to_string(),
                model: model.to_string(),
            })
            .await;
        let response = match response {
            Ok(response) => response.into_inner(),
            // No checkpoint encryption on the control plane: no checkpoint to
            // resume from, and later turns get no store (EVE-1261).
            Err(status) if is_encryption_not_configured(&status) => {
                self.client.checkpoints.mark_unavailable();
                return Ok(None);
            }
            Err(status) => return Err(grpc_status_to_error(status)),
        };
        response
            .checkpoint
            .map(|checkpoint| {
                Ok(crate::core::CompactionCheckpoint {
                    id: proto_uuid_to_uuid(checkpoint.id.as_ref())?,
                    session_id: proto_uuid_to_uuid(checkpoint.session_id.as_ref())?.into(),
                    source_sequence: checkpoint.source_sequence,
                    provider_type: checkpoint.provider_type,
                    model: checkpoint.model,
                    format_version: checkpoint.format_version,
                    payload: serde_json::from_slice(&checkpoint.payload_json).map_err(|error| {
                        AgentLoopError::store(format!("invalid checkpoint payload: {error}"))
                    })?,
                })
            })
            .transpose()
    }

    async fn install(&self, checkpoint: crate::core::CompactionCheckpoint) -> Result<bool> {
        let mut client = self.client.inner.client();
        let payload_json = serde_json::to_vec(&checkpoint.payload)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        Ok(client
            .install_compaction_checkpoint(proto::InstallCompactionCheckpointRequest {
                checkpoint: Some(proto::CompactionCheckpoint {
                    id: Some(uuid_to_proto(checkpoint.id)),
                    session_id: Some(uuid_to_proto(checkpoint.session_id.uuid())),
                    source_sequence: checkpoint.source_sequence,
                    provider_type: checkpoint.provider_type,
                    model: checkpoint.model,
                    format_version: checkpoint.format_version,
                    payload_json,
                }),
            })
            .await
            .map_err(|status| self.client.checkpoint_status_to_error(status))?
            .into_inner()
            .installed)
    }

    async fn get_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<crate::core::ProactiveCompactionAttempt>> {
        Ok(self
            .proactive_compaction_attempts
            .get(session_id, provider_type, model)
            .await)
    }

    async fn record_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        attempt: crate::core::ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.proactive_compaction_attempts
            .record(session_id, provider_type, model, attempt)
            .await;
        Ok(())
    }
}
