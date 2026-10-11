//! Session actor leases and journal reads for the in-process worker
//! (DEV_MODE): the same repository calls the gRPC handlers in
//! `grpc_service/worker/actors.rs` make, without the wire.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_worker::actor_lease_store::{
    ActorJournal, ActorLeaseStore, ActorRelease, ActorRenewal, LeasedActor,
};

use super::store_error;
use crate::services::EventService;
use crate::storage::{DEFAULT_ACTOR_LEASES_PER_ORG, StorageBackend};

fn fence_to_db(fence: u64) -> Result<i64> {
    i64::try_from(fence).map_err(|_| AgentLoopError::store(format!("fence {fence} out of range")))
}

/// [`ActorLeaseStore`] on this process's storage backend.
pub(super) struct DirectActorLeaseStore {
    pub(super) db: Arc<StorageBackend>,
}

#[async_trait]
impl ActorLeaseStore for DirectActorLeaseStore {
    async fn lease_actors(
        &self,
        holder: &str,
        limit: u32,
        ttl: Duration,
        max_leased_per_org: Option<u32>,
    ) -> Result<Vec<LeasedActor>> {
        let max_per_org = max_leased_per_org
            .filter(|cap| *cap > 0)
            .map_or(DEFAULT_ACTOR_LEASES_PER_ORG, i64::from);
        let rows = self
            .db
            .lease_session_actors(holder, i64::from(limit), ttl, max_per_org)
            .await
            .map_err(|e| store_error(format!("Failed to lease actors: {e}")))?;
        rows.into_iter()
            .map(|row| {
                Ok(LeasedActor {
                    session_id: row.session_id,
                    org_id: row.org_id,
                    fence: u64::try_from(row.fence)
                        .map_err(|_| store_error(format!("negative fence {}", row.fence)))?,
                    inbox_seq: row.inbox_seq,
                    cancel_requested: row.cancel_requested,
                })
            })
            .collect()
    }

    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
    ) -> Result<Option<ActorRenewal>> {
        Ok(self
            .db
            .renew_session_actor_lease(session_id, holder, fence_to_db(fence)?, ttl)
            .await
            .map_err(|e| store_error(format!("Failed to renew actor lease: {e}")))?
            .map(|row| ActorRenewal {
                inbox_seq: row.inbox_seq,
                cancel_requested: row.cancel_requested,
            }))
    }

    async fn release_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<ActorRelease> {
        Ok(
            match self
                .db
                .release_session_actor(session_id, holder, fence_to_db(fence)?, consumed_inbox_seq)
                .await
                .map_err(|e| store_error(format!("Failed to release actor: {e}")))?
            {
                Some(still_runnable) => ActorRelease::Released { still_runnable },
                None => ActorRelease::LeaseLost,
            },
        )
    }

    async fn read_actor_journal(
        &self,
        session_id: SessionId,
        after_sequence: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        let journal = self
            .db
            .read_session_actor_journal(session_id, after_sequence, i64::from(limit))
            .await
            .map_err(|e| store_error(format!("Failed to read actor journal: {e}")))?;
        Ok(ActorJournal {
            events: journal
                .events
                .into_iter()
                .map(EventService::row_to_event)
                .collect(),
            has_more: journal.has_more,
            head_sequence: journal.head_sequence,
        })
    }
}
