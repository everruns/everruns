//! Renewals that arrive while one is in flight go out together; each lease
//! gets its own answer back.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Semaphore;

/// Records the batches it was sent; renewals wait for a permit.
struct Batches {
    sent: Mutex<Vec<Vec<ActorLeaseRenewal>>>,
    permits: Semaphore,
    fail: AtomicBool,
}

impl Batches {
    fn new(open: bool) -> Arc<Self> {
        Arc::new(Self {
            sent: Mutex::new(Vec::new()),
            permits: Semaphore::new(if open { Semaphore::MAX_PERMITS } else { 0 }),
            fail: AtomicBool::new(false),
        })
    }

    fn sizes(&self) -> Vec<usize> {
        self.sent.lock().unwrap().iter().map(Vec::len).collect()
    }
}

/// Fence 1 is held, 2 is lost, 3 is a deleted session.
fn answer(lease: &ActorLeaseRenewal) -> Result<Option<ActorRenewal>> {
    match lease.fence {
        1 => Ok(Some(ActorRenewal {
            inbox_seq: lease.consumed_seq + 1,
            cancel_inbox_seq: None,
        })),
        2 => Ok(None),
        _ => Err(AgentLoopError::SessionNotFound(lease.session_id)),
    }
}

#[async_trait]
impl SessionActorStore for Batches {
    async fn renew_actor_lease(
        &self,
        _session_id: SessionId,
        _holder: &str,
        _fence: u64,
        _ttl: Duration,
        _consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>> {
        unreachable!("renewals arrive batched")
    }

    async fn renew_actor_leases(
        &self,
        holder: &str,
        ttl: Duration,
        leases: &[ActorLeaseRenewal],
    ) -> Result<Vec<Result<Option<ActorRenewal>>>> {
        assert_eq!(holder, "worker-a");
        assert_eq!(ttl, TTL);
        self.sent.lock().unwrap().push(leases.to_vec());
        self.permits.acquire().await.unwrap().forget();
        if self.fail.load(Ordering::SeqCst) {
            return Err(AgentLoopError::store("connection reset"));
        }
        Ok(leases.iter().map(answer).collect())
    }

    async fn release_actor(
        &self,
        _session_id: SessionId,
        _holder: &str,
        _fence: u64,
        _consumed_inbox_seq: i64,
    ) -> Result<ActorRelease> {
        unreachable!()
    }

    async fn read_actor_journal(
        &self,
        _session_id: SessionId,
        _after_sequence: Option<i32>,
        _limit: u32,
    ) -> Result<ActorJournal> {
        unreachable!()
    }
}

const TTL: Duration = Duration::from_secs(10);

async fn renew(
    store: &Arc<BatchedRenewals>,
    session_id: SessionId,
    fence: u64,
) -> Result<Option<ActorRenewal>> {
    store
        .renew_actor_lease(session_id, "worker-a", fence, TTL, 7)
        .await
}

#[tokio::test]
async fn a_lone_renewal_goes_out_at_once() {
    let inner = Batches::new(true);
    let store = Arc::new(BatchedRenewals::new(inner.clone()));
    let renewed = renew(&store, SessionId::new(), 1).await.unwrap();
    assert_eq!(renewed.map(|r| r.inbox_seq), Some(8));
    assert_eq!(inner.sizes(), vec![1]);
}

#[tokio::test]
async fn renewals_behind_one_in_flight_share_the_next_call() {
    let inner = Batches::new(false);
    let store = Arc::new(BatchedRenewals::new(inner.clone()));
    let first = tokio::spawn({
        let store = store.clone();
        async move { renew(&store, SessionId::new(), 1).await }
    });
    while inner.sizes().is_empty() {
        tokio::task::yield_now().await;
    }
    let behind: Vec<_> = (0..5)
        .map(|_| {
            let store = store.clone();
            tokio::spawn(async move { renew(&store, SessionId::new(), 1).await })
        })
        .collect();
    // Let the five queue up behind the call in flight.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    inner.permits.add_permits(2);
    assert!(first.await.unwrap().unwrap().is_some());
    for renewal in behind {
        assert!(renewal.await.unwrap().unwrap().is_some());
    }
    assert_eq!(inner.sizes(), vec![1, 5], "two calls for six renewals");
}

#[tokio::test]
async fn each_lease_gets_its_own_answer() {
    let inner = Batches::new(false);
    let store = Arc::new(BatchedRenewals::new(inner.clone()));
    let lead = tokio::spawn({
        let store = store.clone();
        async move { renew(&store, SessionId::new(), 1).await }
    });
    while inner.sizes().is_empty() {
        tokio::task::yield_now().await;
    }
    let deleted = SessionId::new();
    let held = tokio::spawn({
        let store = store.clone();
        async move { renew(&store, SessionId::new(), 1).await }
    });
    let lost = tokio::spawn({
        let store = store.clone();
        async move { renew(&store, SessionId::new(), 2).await }
    });
    let gone = tokio::spawn({
        let store = store.clone();
        async move { renew(&store, deleted, 3).await }
    });
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    inner.permits.add_permits(2);
    lead.await.unwrap().unwrap();
    assert!(held.await.unwrap().unwrap().is_some());
    assert_eq!(lost.await.unwrap().unwrap(), None);
    assert!(matches!(
        gone.await.unwrap(),
        Err(AgentLoopError::SessionNotFound(id)) if id == deleted
    ));
    assert_eq!(inner.sizes(), vec![1, 3]);
}

#[tokio::test]
async fn a_failed_batch_fails_each_renewal_and_the_next_one_goes_out() {
    let inner = Batches::new(true);
    inner.fail.store(true, Ordering::SeqCst);
    let store = Arc::new(BatchedRenewals::new(inner.clone()));
    let error = renew(&store, SessionId::new(), 1).await.unwrap_err();
    assert!(!matches!(error, AgentLoopError::SessionNotFound(_)));

    inner.fail.store(false, Ordering::SeqCst);
    assert!(renew(&store, SessionId::new(), 1).await.unwrap().is_some());
    assert_eq!(inner.sizes(), vec![1, 1]);
}

#[test]
fn ticks_land_on_multiples_of_the_period() {
    let period = Duration::from_millis(3_000);
    let epoch = everruns_contracts::rt::Instant::now() - Duration::from_millis(7_500);
    let wait = super::super::lease::until_next_tick(epoch, period);
    // 7.5 s in: the next tick is at 9 s.
    assert!(
        wait <= Duration::from_millis(1_500) && wait > Duration::from_millis(1_400),
        "{wait:?}"
    );
}
