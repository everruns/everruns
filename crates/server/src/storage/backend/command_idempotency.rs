use anyhow::Result;

use super::StorageBackend;
use crate::storage::command_idempotency::{
    ClaimIdempotencyKey, IdempotencyClaim, IdempotencyKeyScope,
};

impl StorageBackend {
    /// Claims an idempotency key for one command request; see
    /// `crate::storage::command_idempotency`.
    pub async fn claim_command_idempotency_key(
        &self,
        input: &ClaimIdempotencyKey,
    ) -> Result<IdempotencyClaim> {
        dispatch!(self, claim_command_idempotency_key, input)
    }

    /// Stores the response of the request that claimed the key.
    pub async fn complete_command_idempotency_key(
        &self,
        scope: &IdempotencyKeyScope,
        response: &[u8],
    ) -> Result<()> {
        dispatch!(self, complete_command_idempotency_key, scope, response)
    }

    /// Forgets an in-flight key after its command failed, so a retry runs it.
    pub async fn release_command_idempotency_key(&self, scope: &IdempotencyKeyScope) -> Result<()> {
        dispatch!(self, release_command_idempotency_key, scope)
    }
}
