//! Server-side scope for the first assignment-authorized worker operation.
//!
//! This is not a credential or an identity verifier. A trusted server identity
//! adapter may wrap it in canonical worker authority only after verifying an
//! executor's assignment. Never construct it from caller metadata or the shared
//! worker bearer. No production adapter issues this scope yet.

use everruns_contracts::typed_id::SessionId;
use tonic::Status;

#[cfg(test)]
mod tests;

fn denied() -> Status {
    Status::permission_denied("Worker assignment does not permit this operation")
}

/// A server-resolved actor assignment permitted to read its journal only.
///
/// Executor and grant references are not credentials or identity proofs. The
/// stored authorization and lease are rechecked at each admission; retaining
/// this value never preserves a revoked grant. Other RPCs must fail closed.
#[derive(Clone, Debug)]
pub struct ActorJournalScope {
    pub(super) grant_id: uuid::Uuid,
    pub(super) executor_id: uuid::Uuid,
    pub(super) org_id: i64,
    pub(super) session_id: SessionId,
    pub(super) holder: String,
    pub(super) fence: i64,
}

impl ActorJournalScope {
    /// Scope already resolved by a trusted assignment/identity adapter.
    /// This validates shape only, not the caller's identity or authority. The
    /// adapter must bind the executor to verified identity, never metadata.
    pub fn new(
        grant_id: uuid::Uuid,
        executor_id: uuid::Uuid,
        org_id: i64,
        session_id: SessionId,
        holder: String,
        fence: i64,
    ) -> Result<Self, Status> {
        if grant_id.is_nil()
            || executor_id.is_nil()
            || org_id <= 0
            || holder.trim().is_empty()
            || holder.len() > 256
            || fence <= 0
        {
            return Err(Status::invalid_argument("Invalid actor journal scope"));
        }
        Ok(Self {
            grant_id,
            executor_id,
            org_id,
            session_id,
            holder,
            fence,
        })
    }

    pub(super) async fn read_journal(
        &self,
        db: &crate::storage::StorageBackend,
        session_id: SessionId,
        after_sequence: Option<i32>,
        limit: i64,
    ) -> Result<crate::storage::ActorJournalRows, Status> {
        if session_id != self.session_id {
            return Err(denied());
        }
        let binding = crate::storage::WorkerJournalCapabilityBinding {
            grant_id: self.grant_id,
            executor_id: self.executor_id,
            org_id: self.org_id,
            session_id: self.session_id,
            holder: self.holder.clone(),
            fence: self.fence,
        };
        db.read_worker_assignment_journal(
            &binding,
            crate::storage::WorkerJournalReadIdentity::Assignment,
            after_sequence,
            limit,
        )
        .await
        .map_err(|error| {
            super::worker::support::internal_status("Failed to authorize worker assignment", error)
        })?
        .ok_or_else(denied)
    }
}
