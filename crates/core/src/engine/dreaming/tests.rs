use super::{runner::*, store::InMemoryDreamingStore, *};
use crate::{
    error::{AgentLoopError, Result},
    typed_id::SessionId,
};
use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Mutex;
struct Host {
    store: Arc<InMemoryDreamingStore>,
    calls: AtomicUsize,
    writes: Mutex<Vec<String>>,
    fail_commit: Mutex<bool>,
    priced: bool,
    canceled: AtomicBool,
    cancel_after_generate: bool,
    scope: String,
    output: DreamOutput,
}
impl Host {
    fn new() -> Self {
        Self {
            store: Arc::default(),
            calls: AtomicUsize::new(0),
            writes: Mutex::default(),
            fail_commit: Mutex::new(false),
            priced: true,
            canceled: AtomicBool::new(false),
            cancel_after_generate: false,
            scope: "shared-memory".into(),
            output: DreamOutput {
                notes: vec![DreamNote {
                    text: "Run the integration suite before publishing.".into(),
                    kind: "procedure".into(),
                    evidence: vec!["u1".into()],
                }],
            },
        }
    }
}
#[async_trait]
impl DreamingHost for Host {
    async fn cancelled(&self, _: &DreamJob) -> Result<bool> {
        Ok(self.canceled.load(Ordering::SeqCst))
    }
    fn store(&self) -> Arc<dyn DreamingStore> {
        self.store.clone()
    }
    async fn destination(&self, _: SessionId, d: &DreamDestination) -> Result<DreamMemoryScope> {
        if d.mount != "/memory/user" {
            return Err(AgentLoopError::config("unauthorized Memory"));
        }
        Ok(DreamMemoryScope {
            key: self.scope.clone(),
            private: true,
        })
    }
    async fn existing(&self, _: SessionId, _: &DreamDestination) -> Result<String> {
        Ok(String::new())
    }
    async fn maximum_cost(&self, _: &DreamJob, _: &str) -> Result<Option<f64>> {
        Ok(self.priced.then_some(0.01))
    }
    async fn generate(&self, _: &DreamJob, _: &str) -> Result<DreamOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.cancel_after_generate {
            self.canceled.store(true, Ordering::SeqCst);
        }
        Ok(self.output.clone())
    }
    async fn commit(&self, job: &DreamJob, _: &DreamDestination, _: &DreamOutput) -> Result<()> {
        if *self.fail_commit.lock().await {
            return Err(AgentLoopError::store("injected write failure"));
        }
        if job.config.commit_mode == DreamCommitMode::Automatic {
            let mut w = self.writes.lock().await;
            if !w.contains(&job.operation_id) {
                w.push(job.operation_id.clone())
            }
        }
        Ok(())
    }
    async fn activity(&self, _: SessionId, _: DreamActivity) -> Result<()> {
        Ok(())
    }
}
fn config() -> DreamingConfig {
    let mut c = DreamingConfig::new(DreamDestination {
        mount: "/memory/user".into(),
        path: "/learned".into(),
        instructions: String::new(),
    });
    c.trigger.min_new_turns = 1;
    c.trigger.mode = DreamTriggerMode::AfterTurn;
    c
}
fn evidence() -> Vec<DreamEvidence> {
    vec![DreamEvidence {
        id: "u1".into(),
        role: "user".into(),
        text: "Run integration checks before publishing".into(),
    }]
}
#[tokio::test]
async fn restart_after_generation_reuses_output_and_commits_once() {
    let h = Host::new();
    let id = SessionId::new();
    enqueue(&h, 1, id, config(), evidence(), 100, false)
        .await
        .unwrap();
    *h.fail_commit.lock().await = true;
    assert!(step(&h, 1, id, 100).await.is_err());
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    *h.fail_commit.lock().await = false;
    assert!(step(&h, 1, id, 701).await.unwrap());
    assert!(!step(&h, 1, id, 1402).await.unwrap());
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.writes.lock().await.len(), 1);
    assert!(
        !enqueue(&h, 1, id, config(), evidence(), 1500, false)
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn concurrent_executors_only_one_claims() {
    let h = Host::new();
    let id = SessionId::new();
    enqueue(&h, 1, id, config(), evidence(), 100, false)
        .await
        .unwrap();
    let (a, b) = tokio::join!(step(&h, 1, id, 100), step(&h, 1, id, 100));
    assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn quota_is_shared_across_sessions_but_isolates_tenants() {
    let h = Host::new();
    let mut c = config();
    c.budget.max_runs_per_day = 1;
    for (org, allowed) in [(1, true), (1, false), (2, true)] {
        let id = SessionId::new();
        enqueue(&h, org, id, c.clone(), evidence(), 100, false)
            .await
            .unwrap();
        assert_eq!(step(&h, org, id, 100).await.is_ok(), allowed);
    }
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn unknown_prices_fail_before_a_billable_call() {
    let mut h = Host::new();
    h.priced = false;
    let id = SessionId::new();
    enqueue(&h, 1, id, config(), evidence(), 100, false)
        .await
        .unwrap();
    assert!(step(&h, 1, id, 100).await.is_err());
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn proposals_never_enter_active_memory() {
    let h = Host::new();
    let mut c = config();
    c.commit_mode = DreamCommitMode::Propose;
    let id = SessionId::new();
    enqueue(&h, 1, id, c, evidence(), 100, false).await.unwrap();
    step(&h, 1, id, 100).await.unwrap();
    assert!(h.writes.lock().await.is_empty());
    let row = h.store.get(1, &job_key(id)).await.unwrap().unwrap();
    let state: DreamSessionState = serde_json::from_str(&row.value).unwrap();
    assert_eq!(state.job.unwrap().proposals.len(), 1);
}
#[tokio::test]
async fn idle_and_manual_triggers_do_not_run_early() {
    let h = Host::new();
    let id = SessionId::new();
    let mut c = config();
    c.trigger.mode = DreamTriggerMode::Manual;
    assert!(
        !enqueue(&h, 1, id, c.clone(), evidence(), 100, false)
            .await
            .unwrap()
    );
    assert!(enqueue(&h, 1, id, c, evidence(), 100, true).await.unwrap());
    assert!(step(&h, 1, id, 100).await.unwrap());
    let id = SessionId::new();
    let mut c = config();
    c.trigger.mode = DreamTriggerMode::Idle;
    enqueue(&h, 1, id, c, evidence(), 100, false).await.unwrap();
    assert!(!step(&h, 1, id, 159).await.unwrap());
    assert!(step(&h, 1, id, 160).await.unwrap());
}
#[tokio::test]
async fn manual_flushes_idle_work_and_debouncing_preserves_operation_identity() {
    let h = Host::new();
    let id = SessionId::new();
    let mut c = config();
    c.trigger.mode = DreamTriggerMode::Idle;
    assert!(
        enqueue(&h, 1, id, c.clone(), evidence(), 100, false)
            .await
            .unwrap()
    );
    let before: DreamSessionState =
        serde_json::from_str(&h.store.get(1, &job_key(id)).await.unwrap().unwrap().value).unwrap();
    let mut newer = evidence();
    newer.push(DreamEvidence {
        id: "u2".into(),
        role: "user".into(),
        text: "Remember UTC too.".into(),
    });
    assert!(
        enqueue(&h, 1, id, c.clone(), newer.clone(), 105, false)
            .await
            .unwrap()
    );
    assert!(!step(&h, 1, id, 110).await.unwrap());
    assert!(enqueue(&h, 1, id, c, newer, 110, true).await.unwrap());
    let flushed: DreamSessionState =
        serde_json::from_str(&h.store.get(1, &job_key(id)).await.unwrap().unwrap().value).unwrap();
    assert_eq!(
        before.job.unwrap().operation_id,
        flushed.job.unwrap().operation_id
    );
    assert!(step(&h, 1, id, 110).await.unwrap());
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn workspace_paths_cannot_masquerade_as_memory() {
    let h = Host::new();
    let mut c = config();
    c.destinations[0].mount = "/workspace".into();
    assert!(
        enqueue(&h, 1, SessionId::new(), c, evidence(), 100, false)
            .await
            .is_err()
    );
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn fabricated_evidence_and_invalid_memory_are_rejected() {
    for note in [
        DreamNote {
            text: "invented".into(),
            kind: "fact".into(),
            evidence: vec!["missing".into()],
        },
        DreamNote {
            text: String::new(),
            kind: "fact".into(),
            evidence: vec!["u1".into()],
        },
        DreamNote {
            text: "x".repeat(MAX_MEMORY_BYTES + 1),
            kind: "fact".into(),
            evidence: vec!["u1".into()],
        },
    ] {
        assert!(validate_output(&DreamOutput { notes: vec![note] }, &evidence()).is_err());
    }
}
#[test]
fn empty_output_is_a_valid_no_op() {
    assert!(validate_output(&DreamOutput { notes: vec![] }, &evidence()).is_ok());
}

#[tokio::test]
async fn successful_turn_counter_is_deduplicated_and_does_not_invalidate_claims() {
    let h = Host::new();
    let id = SessionId::new();
    let mut c = config();
    c.trigger.min_new_turns = 2;
    record_turn(h.store.as_ref(), 1, id, "turn1".into())
        .await
        .unwrap();
    record_turn(h.store.as_ref(), 1, id, "turn1".into())
        .await
        .unwrap();
    assert!(
        !enqueue(&h, 1, id, c.clone(), evidence(), 100, false)
            .await
            .unwrap()
    );
    record_turn(h.store.as_ref(), 1, id, "turn2".into())
        .await
        .unwrap();
    assert!(
        enqueue(&h, 1, id, c.clone(), evidence(), 100, false)
            .await
            .unwrap()
    );
    *h.fail_commit.lock().await = true;
    assert!(step(&h, 1, id, 100).await.is_err());
    record_turn(h.store.as_ref(), 1, id, "turn3".into())
        .await
        .unwrap();
    *h.fail_commit.lock().await = false;
    assert!(step(&h, 1, id, 701).await.unwrap());
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    let mut next = evidence();
    next.push(DreamEvidence {
        id: "u2".into(),
        role: "user".into(),
        text: "new preference".into(),
    });
    assert!(
        !enqueue(&h, 1, id, c.clone(), next.clone(), 702, false)
            .await
            .unwrap()
    );
    record_turn(h.store.as_ref(), 1, id, "turn4".into())
        .await
        .unwrap();
    assert!(enqueue(&h, 1, id, c, next, 703, false).await.unwrap());
}

#[tokio::test]
async fn cancellation_before_and_during_generation_prevents_memory_commit() {
    for during in [false, true] {
        let mut h = Host::new();
        h.cancel_after_generate = during;
        h.canceled.store(!during, Ordering::SeqCst);
        let id = SessionId::new();
        enqueue(&h, 1, id, config(), evidence(), 100, false)
            .await
            .unwrap();
        assert!(step(&h, 1, id, 100).await.unwrap());
        assert_eq!(h.calls.load(Ordering::SeqCst), usize::from(during));
        assert!(h.writes.lock().await.is_empty());
        assert!(!step(&h, 1, id, 701).await.unwrap());
    }
}

#[tokio::test]
async fn one_activation_drains_every_configured_destination() {
    let h = Host::new();
    let id = SessionId::new();
    let mut c = config();
    c.destinations.push(DreamDestination {
        mount: "/memory/user".into(),
        path: "/procedures".into(),
        instructions: String::new(),
    });
    enqueue(&h, 1, id, c, evidence(), 100, false).await.unwrap();
    assert!(run_session(&h, 1, id).await.unwrap());
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    let row = h.store.get(1, &job_key(id)).await.unwrap().unwrap();
    let state: DreamSessionState = serde_json::from_str(&row.value).unwrap();
    let job = state.job.unwrap();
    assert!(job.completed);
    assert_eq!(job.note_count, 2);
    assert!(!run_session(&h, 1, id).await.unwrap());
}

#[tokio::test]
async fn new_turn_cannot_replace_a_partially_processed_operation() {
    let h = Host::new();
    let id = SessionId::new();
    let mut c = config();
    c.trigger.mode = DreamTriggerMode::Idle;
    c.destinations.push(DreamDestination {
        mount: "/memory/user".into(),
        path: "/procedures".into(),
        instructions: String::new(),
    });
    enqueue(&h, 1, id, c.clone(), evidence(), 100, false)
        .await
        .unwrap();
    assert!(step(&h, 1, id, 160).await.unwrap());
    let before = h.store.get(1, &job_key(id)).await.unwrap().unwrap();
    let mut newer = evidence();
    newer.push(DreamEvidence {
        id: "u2".into(),
        role: "user".into(),
        text: "New experience.".into(),
    });
    assert!(!enqueue(&h, 1, id, c, newer, 161, false).await.unwrap());
    assert_eq!(
        before.value,
        h.store.get(1, &job_key(id)).await.unwrap().unwrap().value
    );
    assert!(step(&h, 1, id, 161).await.unwrap());
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    let row = h.store.get(1, &job_key(id)).await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<DreamSessionState>(&row.value)
            .unwrap()
            .job
            .unwrap()
            .note_count,
        2
    );
}
