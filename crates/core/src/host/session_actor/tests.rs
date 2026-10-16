use super::lease::Lease;
use super::store::*;
use super::*;
use everruns_contracts::error::{AgentLoopError, Result};
use std::sync::atomic::{AtomicU8, Ordering};

struct Deletable(AtomicU8);

#[async_trait]
impl SessionActorStore for Deletable {
    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        _holder: &str,
        _fence: u64,
        _ttl: Duration,
        _consumed: i64,
    ) -> Result<Option<ActorRenewal>> {
        if self.0.load(Ordering::SeqCst) == 2 {
            return std::future::pending().await;
        }
        if self.0.load(Ordering::SeqCst) == 1 {
            Err(AgentLoopError::session_not_found(session_id))
        } else {
            Ok(Some(ActorRenewal {
                inbox_seq: 1,
                cancel_inbox_seq: None,
            }))
        }
    }
    async fn release_actor(
        &self,
        _id: SessionId,
        _holder: &str,
        _fence: u64,
        _consumed: i64,
    ) -> Result<ActorRelease> {
        unreachable!("the test only probes ownership")
    }
    async fn read_actor_journal(
        &self,
        _id: SessionId,
        _after: Option<i32>,
        _limit: u32,
    ) -> Result<ActorJournal> {
        unreachable!("the test only probes ownership")
    }
}

/// EVE-1235: unrelated read failures become a quiet stop only when the
/// authority confirms that the session was deleted. Moved with the driver.
#[tokio::test]
async fn a_failure_after_the_session_was_deleted_reads_as_the_delete() {
    let store = Arc::new(Deletable(AtomicU8::new(0)));
    let actor = LeasedActor {
        session_id: SessionId::new(),
        org_id: 1,
        fence: 1,
        inbox_seq: 1,
        cancel_inbox_seq: None,
        consumed_seq: 0,
        next_input_seq: 1,
        attempts: 1,
    };
    let lease = Lease::new(
        store.clone(),
        "worker".into(),
        &actor,
        Duration::from_secs(3),
        3,
    );
    let unrelated = AgentLoopError::store("User message not found: message_x");
    assert!(!super::turn::session_deleted(&lease, &unrelated).await);
    store.0.store(1, Ordering::SeqCst);
    assert!(super::turn::session_deleted(&lease, &unrelated).await);
}

#[tokio::test(start_paused = true)]
async fn a_hung_authority_renewal_marks_the_driver_lease_lost() {
    let store = Arc::new(Deletable(AtomicU8::new(2)));
    let actor = LeasedActor {
        session_id: SessionId::new(),
        org_id: 1,
        fence: 1,
        inbox_seq: 1,
        cancel_inbox_seq: None,
        consumed_seq: 0,
        next_input_seq: 1,
        attempts: 1,
    };
    let lease = Lease::new(store, "worker".into(), &actor, Duration::from_secs(1), 3);
    let result = tokio::time::timeout(Duration::from_secs(2), lease.renew())
        .await
        .expect("renewal has a bound");
    assert!(result.is_err());
    assert!(lease.is_lost());
}
