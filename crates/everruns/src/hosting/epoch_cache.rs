//! What one epoch already knows about its own authority state.
//!
//! Decision: the epoch's holder is the only writer of its receipts and its
//! timer while the lease holds (every write is fenced), so a value it wrote
//! need not be read back, and a timer it set need not be set again. Only
//! successful writes are remembered: a key this epoch never wrote is always
//! read from the authority. A receipt riding on the next lease write counts
//! as written: if that write fails, so does the epoch. A claim that returns the stored wake time seeds
//! the timer. Data plane v2 step 1 counts each saved call as a
//! saved round trip to a replicated authority's leader.
use std::collections::HashMap;
use std::sync::Mutex;

use super::*;

pub(crate) struct EpochCache {
    inner: Arc<dyn SessionEpoch>,
    written: Mutex<HashMap<String, Value>>,
    /// The wake time this epoch last stored; `None` until it stores one.
    scheduled: Mutex<Option<Option<i64>>>,
}

impl EpochCache {
    pub(crate) fn new(inner: Arc<dyn SessionEpoch>) -> Self {
        Self {
            scheduled: Mutex::new(inner.claimed_wake_at()),
            inner,
            written: Mutex::default(),
        }
    }

    fn written(&self) -> std::sync::MutexGuard<'_, HashMap<String, Value>> {
        self.written.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[async_trait]
impl SessionEpoch for EpochCache {
    fn renew_poke(&self) -> Option<Arc<tokio::sync::Notify>> {
        self.inner.renew_poke()
    }
    fn hand_off(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        self.inner.hand_off()
    }
    fn pushes_admissions(&self) -> bool {
        self.inner.pushes_admissions()
    }
    fn claimed_wake_at(&self) -> Option<Option<i64>> {
        self.inner.claimed_wake_at()
    }
    async fn schedule(&self, wake_at: Option<i64>) -> Result<()> {
        if *self.scheduled.lock().unwrap_or_else(|p| p.into_inner()) == Some(wake_at) {
            return Ok(());
        }
        self.inner.schedule(wake_at).await?;
        *self.scheduled.lock().unwrap_or_else(|p| p.into_inner()) = Some(wake_at);
        Ok(())
    }
    fn backends(&self) -> HostBackends {
        self.inner.backends()
    }
    fn actor_store(&self) -> Arc<dyn SessionActorStore> {
        self.inner.actor_store()
    }
    fn observe_fence(&self, fence: &ActorFence) {
        self.inner.observe_fence(fence)
    }
    async fn record(&self, key: &str) -> Result<Option<Value>> {
        if let Some(value) = self.written().get(key) {
            return Ok(Some(value.clone()));
        }
        self.inner.record(key).await
    }
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        self.inner.records(prefix).await
    }
    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
        // An insert-if-absent of a key this epoch already wrote finds it
        // present; the authority would answer the same.
        if absent && self.written().contains_key(key) {
            return Ok(false);
        }
        let committed = self.inner.commit(key, value.clone(), absent).await?;
        if committed {
            self.written().insert(key.to_owned(), value);
        }
        Ok(committed)
    }
    async fn commit_with_lease(&self, key: &str, value: Value) -> Result<()> {
        if self.written().contains_key(key) {
            return Ok(());
        }
        self.inner.commit_with_lease(key, value.clone()).await?;
        self.written().insert(key.to_owned(), value);
        Ok(())
    }
    async fn suspend(&self, reason: &str) -> Result<()> {
        self.inner.suspend(reason).await
    }
}
