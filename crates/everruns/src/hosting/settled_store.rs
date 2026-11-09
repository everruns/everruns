//! The epoch's actor store, in step with the events committing behind.
//!
//! Decision: some events are held for a later batch or commit behind the
//! work (see `everruns_core::host::HostEventEmitter`). The driver reads the
//! log's tail and gives up the lease through this store, so it first
//! commits them: a tail read sees every event the epoch emitted, and the
//! lease is released or handed off only once the log holds them (a released
//! lease refuses later commits).
use std::time::Duration;

use everruns_core::host::HostEventEmitter;
use everruns_core::host::session_actor::store::{
    ActorJournal, ActorLeaseRenewal, ActorRelease, ActorRenewal,
};

use super::*;

pub(super) struct SettledStore {
    pub(super) inner: Arc<dyn SessionActorStore>,
    pub(super) emitter: Arc<HostEventEmitter>,
}

impl SettledStore {
    async fn commit_events(&self) {
        if let Err(error) = self.emitter.flush().await {
            tracing::debug!(%error, "events not committed before the lease was given up");
        }
    }
}

#[async_trait]
impl SessionActorStore for SettledStore {
    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>> {
        self.inner
            .renew_actor_lease(session_id, holder, fence, ttl, consumed_seq)
            .await
    }

    async fn renew_actor_leases(
        &self,
        holder: &str,
        ttl: Duration,
        leases: &[ActorLeaseRenewal],
    ) -> Result<Vec<Result<Option<ActorRenewal>>>> {
        self.inner.renew_actor_leases(holder, ttl, leases).await
    }

    async fn release_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<ActorRelease> {
        self.commit_events().await;
        self.inner
            .release_actor(session_id, holder, fence, consumed_inbox_seq)
            .await
    }

    async fn hand_off_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<Option<ActorRelease>> {
        self.commit_events().await;
        self.inner
            .hand_off_actor(session_id, holder, fence, consumed_inbox_seq)
            .await
    }

    async fn read_actor_journal(
        &self,
        session_id: SessionId,
        after_sequence: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        self.emitter.flush().await?;
        self.inner
            .read_actor_journal(session_id, after_sequence, limit)
            .await
    }
}
