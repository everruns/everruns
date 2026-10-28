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
/// The lease is rechecked at each admission; holding this value is not proof
/// that the lease remains live. Other RPCs must fail closed for this scope.
#[derive(Clone, Debug)]
pub struct ActorJournalScope {
    pub(super) org_id: i64,
    pub(super) session_id: SessionId,
    pub(super) holder: String,
    pub(super) fence: i64,
}

impl ActorJournalScope {
    /// Scope already resolved by a trusted assignment/identity adapter.
    /// This validates shape only, not the caller's identity or authority.
    pub fn new(
        org_id: i64,
        session_id: SessionId,
        holder: String,
        fence: i64,
    ) -> Result<Self, Status> {
        if org_id <= 0 || holder.trim().is_empty() || fence <= 0 {
            return Err(Status::invalid_argument("Invalid actor journal scope"));
        }
        Ok(Self {
            org_id,
            session_id,
            holder,
            fence,
        })
    }

    pub(super) async fn admit_read(
        &self,
        db: &crate::storage::StorageBackend,
        session_id: SessionId,
    ) -> Result<(), Status> {
        // Selectors never replace the scope supplied by the trusted adapter.
        // Refuse foreign and absent IDs identically, before touching their rows.
        if session_id != self.session_id {
            return Err(denied());
        }
        let allowed = db
            .actor_journal_read_allowed(self.org_id, self.session_id, &self.holder, self.fence)
            .await
            .map_err(|error| {
                super::worker::support::internal_status(
                    "Failed to authorize worker assignment",
                    error,
                )
            })?;
        if !allowed {
            return Err(denied());
        }
        Ok(())
    }
}
