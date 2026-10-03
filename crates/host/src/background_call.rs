//! Durable re-attachment for provider background responses (EVE-1134).
//!
//! The response id lives in the turn's native-async checkpoint, the
//! tenant-scoped, lease-fenced row a durable host already keeps per turn. Each
//! journal operation takes the lease, reads or writes the record and releases
//! it at once, so it never holds the turn across a minutes-long call and the
//! worker that reclaims a dead worker's task can read the record immediately.
//! Hosts without a shared store (embedded, in-memory) get no journal and keep
//! the in-process behaviour.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::background_call::{
    BackgroundCallContext, BackgroundResponseJournal, BackgroundResponseRecord,
};
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_core::native_async_store::{NativeAsyncLease, NativeAsyncStore};

pub(crate) struct CheckpointBackgroundJournal {
    store: Arc<dyn NativeAsyncStore>,
    org_id: i64,
    session_id: SessionId,
    turn_id: TurnId,
}

impl CheckpointBackgroundJournal {
    fn lease(&self) -> NativeAsyncLease {
        NativeAsyncLease {
            org_id: self.org_id,
            session_id: self.session_id,
            turn_id: self.turn_id,
            owner: uuid::Uuid::new_v4(),
        }
    }
}

#[async_trait]
impl BackgroundResponseJournal for CheckpointBackgroundJournal {
    async fn load(&self) -> Result<Option<BackgroundResponseRecord>> {
        let lease = self.lease();
        let checkpoint = self.store.acquire(lease).await?;
        self.store.release(lease).await?;
        Ok(checkpoint.background_response)
    }

    async fn store(&self, record: Option<&BackgroundResponseRecord>) -> Result<()> {
        let lease = self.lease();
        let mut checkpoint = self.store.acquire(lease).await?;
        if checkpoint.background_response.as_ref() != record {
            checkpoint.background_response = record.cloned();
            self.store.save(lease, &checkpoint).await?;
        }
        self.store.release(lease).await
    }
}

/// The background context for one Reason activity of a durable turn.
pub(crate) fn context<A: crate::RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    turn_id: TurnId,
) -> BackgroundCallContext {
    let mut context = BackgroundCallContext::default();
    if let Some(store) = adapter.native_async_store() {
        context = context.with_journal(Arc::new(CheckpointBackgroundJournal {
            store,
            org_id,
            session_id,
            turn_id,
        }));
    }
    if let Some(cancel) = adapter.turn_cancel_requested() {
        context = context.with_cancel(cancel);
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::error::AgentLoopError;
    use everruns_contracts::native_async::NativeAsyncCheckpoint;
    use std::sync::Mutex;

    /// One turn's row with the same fencing rule as the database store: a live
    /// lease held by another owner refuses access.
    #[derive(Default)]
    struct OneRow {
        checkpoint: Mutex<NativeAsyncCheckpoint>,
        owner: Mutex<Option<uuid::Uuid>>,
    }

    impl OneRow {
        fn owned_by(&self, lease: NativeAsyncLease) -> Result<()> {
            match *self.owner.lock().unwrap() {
                Some(owner) if owner == lease.owner => Ok(()),
                _ => Err(AgentLoopError::store("fence lost")),
            }
        }
    }

    #[async_trait]
    impl NativeAsyncStore for OneRow {
        async fn acquire(&self, lease: NativeAsyncLease) -> Result<NativeAsyncCheckpoint> {
            let mut owner = self.owner.lock().unwrap();
            if owner.is_some_and(|owner| owner != lease.owner) {
                return Err(AgentLoopError::store("lease held"));
            }
            *owner = Some(lease.owner);
            Ok(self.checkpoint.lock().unwrap().clone())
        }
        async fn load(&self, lease: NativeAsyncLease) -> Result<NativeAsyncCheckpoint> {
            self.owned_by(lease)?;
            Ok(self.checkpoint.lock().unwrap().clone())
        }
        async fn renew(&self, lease: NativeAsyncLease) -> Result<()> {
            self.owned_by(lease)
        }
        async fn save(
            &self,
            lease: NativeAsyncLease,
            checkpoint: &NativeAsyncCheckpoint,
        ) -> Result<()> {
            self.owned_by(lease)?;
            *self.checkpoint.lock().unwrap() = checkpoint.clone();
            Ok(())
        }
        async fn release(&self, lease: NativeAsyncLease) -> Result<()> {
            self.owned_by(lease)?;
            *self.owner.lock().unwrap() = None;
            Ok(())
        }
    }

    fn journal(store: Arc<OneRow>) -> CheckpointBackgroundJournal {
        CheckpointBackgroundJournal {
            store,
            org_id: 1,
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
        }
    }

    fn record() -> BackgroundResponseRecord {
        BackgroundResponseRecord {
            response_id: "resp_1".into(),
            request_fingerprint: "fp".into(),
        }
    }

    #[tokio::test]
    async fn record_survives_the_writer_and_keeps_native_state() {
        let row = Arc::new(OneRow::default());
        row.checkpoint.lock().unwrap().completed_responses = 3;
        // The worker that saved the record is gone; a new one reads it.
        journal(row.clone()).store(Some(&record())).await.unwrap();
        let reader = journal(row.clone());
        assert_eq!(reader.load().await.unwrap(), Some(record()));
        assert_eq!(row.checkpoint.lock().unwrap().completed_responses, 3);
        assert!(row.owner.lock().unwrap().is_none(), "no lease is kept");

        reader.store(None).await.unwrap();
        assert_eq!(reader.load().await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_held_lease_is_reported_not_overwritten() {
        let row = Arc::new(OneRow::default());
        *row.owner.lock().unwrap() = Some(uuid::Uuid::new_v4());
        let journal = journal(row.clone());
        assert!(journal.store(Some(&record())).await.is_err());
        assert!(journal.load().await.is_err());
        assert!(row.checkpoint.lock().unwrap().background_response.is_none());
    }
}
