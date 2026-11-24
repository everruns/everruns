//! What building a snapshot costs in memory on top of the state it copies.
//!
//! Decision: a check in CI, not only the 75k run. A snapshot used to hold
//! the live state, a copy and the encoded copy at once, on every replica at
//! the same entry; the 75k run on the Phase 2 image (saas run 45077134741)
//! found it when all three nodes of a shard ran out of memory together, an
//! hour and $5 after the change. Here a counting allocator measures the
//! peak a build adds over the live heap, for a follower (copy to the file
//! only) and for the leader (which reads the file back for the object
//! store once the copy is gone). Each must stay near one copy of the
//! unstored events; the old build added one and a half.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use everruns_agentd::sm::{Shared, StateMachine};
use everruns_agentd::state::SessionState;
use everruns_agentd::store::Store;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::events::{Event, EventContext, SessionActivatedData};
use openraft::RaftSnapshotBuilder;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const SESSIONS: usize = 2_000;
const TAIL: i32 = 40;

fn session(id: SessionId) -> SessionState {
    let events = (1..=TAIL)
        .map(|seq| {
            let mut event = Event::new(
                id,
                EventContext::default(),
                SessionActivatedData {
                    turn_id: TurnId::new(),
                    input_message_id: MessageId::new(),
                },
            );
            event.sequence = Some(seq);
            event
        })
        .collect();
    SessionState {
        events,
        ..Default::default()
    }
}

/// The heap a build adds at its peak, over what was live before it.
async fn added_by_build(machine: &mut StateMachine) -> usize {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    machine.build_snapshot().await.unwrap();
    PEAK.load(Ordering::Relaxed) - before
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_snapshot_costs_about_one_copy_of_the_unstored_events() {
    // Decision: a store on disk, not `memory://`. The in-memory store joins
    // an upload's parts into one more copy of the snapshot, on the heap
    // this test counts, whenever the upload task happens to run; on a slow
    // runner that landed inside the leader's measurement (main 81a8f692:
    // 121 MB over a 118 MB bound). A real store streams the parts.
    let objects = tempfile::tempdir().unwrap();
    let store = Store::open(&format!("file://{}", objects.path().display())).unwrap();
    let shared = Shared::new(store.clone(), 0);
    let dir = tempfile::tempdir().unwrap();
    let mut machine = StateMachine::open(dir.path(), shared.clone()).unwrap();

    let empty = LIVE.load(Ordering::Relaxed);
    {
        let mut state = shared.state.write();
        for _ in 0..SESSIONS {
            let id = SessionId::new();
            state.sessions.insert(id, session(id));
        }
    }
    let held = LIVE.load(Ordering::Relaxed) - empty;

    let follower = added_by_build(&mut machine).await;
    // A new leader copies its current snapshot at once; wait until that
    // copy is stored (not a fixed sleep) so it is not counted against the
    // build.
    shared.set_leading(true);
    for _ in 0..1_000 {
        if !store.snapshot_copies(0).await.unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        store.snapshot_copies(0).await.unwrap().len(),
        1,
        "the new leader copied its current snapshot"
    );
    // The upload frees its buffer just after the copy is listed; freed
    // during the build it only lowers what the build is charged.
    let leader = added_by_build(&mut machine).await;
    println!(
        "state {} KB; a build adds {} KB on a follower, {} KB on the leader",
        held / 1024,
        follower / 1024,
        leader / 1024
    );
    // One copy, give or take the write buffer and the snapshot's own maps.
    // The old build measured 1.5 times the state here (copy and encoding).
    let bound = held + held / 4;
    assert!(
        follower <= bound,
        "a follower's build added {follower} bytes over a {held}-byte state"
    );
    assert!(
        leader <= bound,
        "the leader's build added {leader} bytes over a {held}-byte state"
    );
}
