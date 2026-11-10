//! What one epoch already knows about its own authority state.
//!
//! Decision: the epoch's holder is the only writer of its receipts and its
//! timer while the lease holds (every write is fenced), so a value it wrote
//! need not be read back, and a timer it set need not be set again. A
//! receipt riding on the next lease write counts as written: if that write
//! fails, so does the epoch. A claim that returns the stored wake time seeds
//! the timer. Data plane v2 step 1 counts each saved call as a saved round
//! trip to a replicated authority's leader.
//!
//! For the same reason a receipt this epoch read, present or absent, keeps
//! the value it read until this epoch writes it, and a prefix it listed
//! keeps the keys it listed plus the ones it writes. A turn looks up the
//! same receipts many times (its outcome, its failures, provider retries);
//! data plane v2 step 4 measured 11 record reads per turn on PostgreSQL
//! hosting before this. An insert-if-absent that finds a key this epoch
//! had not seen forgets what it knew about it: something it did not know
//! of is there.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use super::*;

pub(crate) struct EpochCache {
    inner: Arc<dyn SessionEpoch>,
    /// Receipts this epoch read or wrote: `None` for one it read as absent.
    known: Mutex<Known>,
    /// The wake time this epoch last stored; `None` until it stores one.
    scheduled: Mutex<Option<Option<i64>>>,
}

impl EpochCache {
    pub(crate) fn new(inner: Arc<dyn SessionEpoch>) -> Self {
        Self {
            scheduled: Mutex::new(inner.claimed_wake_at()),
            inner,
            known: Mutex::default(),
        }
    }

    fn known(&self) -> std::sync::MutexGuard<'_, Known> {
        self.known.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// A listing shorter than this is whole: the smallest receipt scan bound an
/// authority applies (PostgreSQL hosting's) is 1000.
const COMPLETE_LISTING: usize = 1000;

#[derive(Default)]
struct Known {
    records: HashMap<String, Option<Value>>,
    /// Prefixes whose every key is in `records`.
    listed: HashSet<String>,
}

impl Known {
    fn wrote(&mut self, key: &str, value: Value) {
        self.records.insert(key.to_owned(), Some(value));
    }

    fn forget(&mut self, key: &str) {
        self.records.remove(key);
        self.listed
            .retain(|prefix| !key.starts_with(prefix.as_str()));
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
    fn live_deltas(&self) -> Option<Arc<dyn super::LiveDeltaSink>> {
        self.inner.live_deltas()
    }
    fn requires(&self) -> Vec<String> {
        self.inner.requires()
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
        {
            let known = self.known();
            if let Some(value) = known.records.get(key) {
                return Ok(value.clone());
            }
            if known
                .listed
                .iter()
                .any(|prefix| key.starts_with(prefix.as_str()))
            {
                return Ok(None);
            }
        }
        let value = self.inner.record(key).await?;
        self.known()
            .records
            .entry(key.to_owned())
            .or_insert_with(|| value.clone());
        Ok(value)
    }
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        {
            let known = self.known();
            if known.listed.contains(prefix) {
                let mut records: Vec<_> = known
                    .records
                    .iter()
                    .filter(|(key, _)| key.starts_with(prefix))
                    .filter_map(|(key, value)| Some((key.clone(), value.clone()?)))
                    .collect();
                records.sort_by(|a, b| a.0.cmp(&b.0));
                return Ok(records);
            }
        }
        let records = self.inner.records(prefix).await?;
        let mut known = self.known();
        for (key, value) in &records {
            known
                .records
                .entry(key.clone())
                .or_insert_with(|| Some(value.clone()));
        }
        // Keys under the prefix this epoch read as absent stay absent, when
        // the listing cannot have been cut at an authority's scan bound.
        if records.len() < COMPLETE_LISTING {
            known.listed.insert(prefix.to_owned());
        }
        Ok(records)
    }
    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
        // An insert-if-absent of a key this epoch knows is there finds it
        // present; the authority would answer the same.
        if absent && matches!(self.known().records.get(key), Some(Some(_))) {
            return Ok(false);
        }
        let committed = self.inner.commit(key, value.clone(), absent).await?;
        let mut known = self.known();
        if committed {
            known.wrote(key, value);
        } else {
            known.forget(key);
        }
        Ok(committed)
    }
    async fn commit_with_lease(&self, key: &str, value: Value) -> Result<()> {
        if matches!(self.known().records.get(key), Some(Some(_))) {
            return Ok(());
        }
        self.inner.commit_with_lease(key, value.clone()).await?;
        self.known().wrote(key, value);
        Ok(())
    }
    async fn suspend(&self, reason: &str) -> Result<()> {
        self.inner.suspend(reason).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An authority of receipts that counts its reads.
    #[derive(Default)]
    struct Receipts {
        records: Mutex<BTreeMap<String, Value>>,
        reads: AtomicUsize,
    }

    #[async_trait]
    impl SessionEpoch for Receipts {
        async fn schedule(&self, _: Option<i64>) -> Result<()> {
            Ok(())
        }
        fn backends(&self) -> HostBackends {
            HostBackends::in_memory()
        }
        fn actor_store(&self) -> Arc<dyn SessionActorStore> {
            unreachable!("receipts only")
        }
        fn observe_fence(&self, _: &ActorFence) {}
        async fn record(&self, key: &str) -> Result<Option<Value>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.records.lock().unwrap().get(key).cloned())
        }
        async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .records
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect())
        }
        async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
            let mut records = self.records.lock().unwrap();
            if absent && records.contains_key(key) {
                return Ok(false);
            }
            records.insert(key.to_owned(), value);
            Ok(true)
        }
        async fn suspend(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_receipt_read_once_is_not_read_again() {
        let authority = Arc::new(Receipts::default());
        authority
            .commit("outcome/a", serde_json::json!(1), true)
            .await
            .unwrap();
        let epoch = EpochCache::new(authority.clone());
        for _ in 0..3 {
            assert_eq!(
                epoch.record("outcome/a").await.unwrap(),
                Some(serde_json::json!(1))
            );
            assert_eq!(epoch.record("outcome/b").await.unwrap(), None);
        }
        assert_eq!(authority.reads.load(Ordering::SeqCst), 2);

        // The epoch's own write replaces what it read as absent.
        assert!(
            epoch
                .commit("outcome/b", serde_json::json!(2), true)
                .await
                .unwrap()
        );
        assert_eq!(
            epoch.record("outcome/b").await.unwrap(),
            Some(serde_json::json!(2))
        );
        assert_eq!(authority.reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_listed_prefix_answers_its_keys_and_this_epochs_writes() {
        let authority = Arc::new(Receipts::default());
        authority
            .commit("failure/t/1", serde_json::json!("x"), true)
            .await
            .unwrap();
        let epoch = EpochCache::new(authority.clone());
        assert_eq!(epoch.records("failure/t/").await.unwrap().len(), 1);
        assert_eq!(epoch.record("failure/t/2").await.unwrap(), None);
        epoch
            .commit("failure/t/0", serde_json::json!("y"), true)
            .await
            .unwrap();
        let listed = epoch.records("failure/t/").await.unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            vec!["failure/t/0", "failure/t/1"]
        );
        assert_eq!(authority.reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_key_found_present_unexpectedly_is_read_again() {
        let authority = Arc::new(Receipts::default());
        let epoch = EpochCache::new(authority.clone());
        assert_eq!(epoch.records("retry/").await.unwrap(), vec![]);
        // Stored behind this epoch's back (a test of the fallback only).
        authority
            .records
            .lock()
            .unwrap()
            .insert("retry/x".into(), serde_json::json!(5));
        assert!(
            !epoch
                .commit("retry/x", serde_json::json!(6), true)
                .await
                .unwrap()
        );
        assert_eq!(
            epoch.record("retry/x").await.unwrap(),
            Some(serde_json::json!(5))
        );
        assert_eq!(epoch.records("retry/").await.unwrap().len(), 1);
        assert_eq!(authority.reads.load(Ordering::SeqCst), 3);
    }
}
