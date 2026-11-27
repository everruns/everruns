use std::time::Duration;

use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{Event, EventContext, SessionActivatedData};

use super::*;
use crate::state::{SessionState, ShardState};

fn events(session: SessionId, from: i32, to: i32) -> Vec<Event> {
    (from..=to)
        .map(|seq| {
            let mut event = if seq % 3 == 0 {
                // A context event, so the context range is not empty.
                Event::new(
                    session,
                    EventContext::default(),
                    everruns_core::SessionStartedData {
                        harness_id: everruns_contracts::typed_id::HarnessId::new(),
                        agent_id: None,
                        model_id: None,
                    },
                )
            } else {
                Event::new(
                    session,
                    EventContext::default(),
                    SessionActivatedData {
                        turn_id: everruns_contracts::typed_id::TurnId::new(),
                        input_message_id: everruns_contracts::typed_id::MessageId::new(),
                    },
                )
            };
            event.sequence = Some(seq);
            event
        })
        .collect()
}

/// Objects in the store (each one write).
async fn objects(store: &Store) -> usize {
    use futures::TryStreamExt;
    let all: Vec<_> = store.inner.list(None).try_collect().await.unwrap();
    all.len()
}

fn ids(events: &[Event]) -> Vec<String> {
    events.iter().map(|e| e.id.to_string()).collect()
}

#[test]
fn a_pack_id_names_its_object_and_a_legacy_segment_serializes_as_before() {
    let id = PackId {
        shard: 3,
        inc: 2,
        writer: 7,
        stamp: 1_732_670_000_123_004,
    };
    assert_eq!(id.to_string().parse::<PackId>().unwrap(), id);
    assert!("3.2.7".parse::<PackId>().is_err());
    assert!("3.2.7.1.9".parse::<PackId>().is_err());
    let store = Store::open("memory://").unwrap();
    assert_eq!(PackId::from_path(&store.pack_key(id)), Some(id));

    let legacy = SegRef::legacy(1, 22, 0);
    assert_eq!(
        serde_json::to_string(&legacy).unwrap(),
        r#"{"from":1,"to":22}"#
    );
    let packed = SegRef {
        pack: Some(id),
        events: (10, 200),
        context: (210, 40),
        ..legacy
    };
    let json = serde_json::to_string(&packed).unwrap();
    assert_eq!(
        json,
        r#"{"from":1,"to":22,"p":"3.2.7.1732670000123004","e":[10,200],"c":[210,40]}"#
    );
    assert_eq!(serde_json::from_str::<SegRef>(&json).unwrap(), packed);
}

/// Every read path returns the same events from a pack as from legacy
/// objects, for a session whose history mixes both.
#[tokio::test]
async fn a_session_reads_the_same_history_from_packs_and_legacy_segments() {
    let store = Store::open("memory://").unwrap();
    let (a, b) = (SessionId::new(), SessionId::new());
    let all_a = events(a, 1, 30);
    let all_b = events(b, 1, 12);
    // a: 1..=10 legacy, 11..=20 and 21..=30 in one pack next to b's.
    let legacy = store.put_segment(a, &all_a[..10], 0).await.unwrap();
    let mut pack = PackBuilder::new(PackId::new(0, 0, 1));
    let a2 = pack.add(&all_a[10..20], 0).unwrap();
    let b1 = pack.add(&all_b, 0).unwrap();
    let a3 = pack.add(&all_a[20..], 0).unwrap();
    store.put_pack(pack).await.unwrap();
    assert_eq!(
        objects(&store).await,
        3,
        "three segments, one pack beside a legacy pair"
    );
    let chain = [legacy, a2, a3];
    assert_eq!(
        ids(&store.load(a, &chain, 1, 30).await.unwrap()),
        ids(&all_a)
    );
    assert_eq!(
        ids(&store.load(a, &chain, 8, 23).await.unwrap()),
        ids(&all_a[7..23])
    );
    assert_eq!(
        ids(&store.load(b, &[b1], 1, 12).await.unwrap()),
        ids(&all_b)
    );
    let context: Vec<Event> = all_a
        .iter()
        .filter(|e| crate::store::is_context(&e.event_type))
        .cloned()
        .collect();
    assert_eq!(
        ids(&store.load_context(a, &chain, 1, 30).await.unwrap()),
        ids(&context)
    );
    // Through a cache, twice: the second read is a hit.
    let cached = store.clone().with_cache(1 << 20);
    for _ in 0..2 {
        assert_eq!(
            ids(&cached.load(a, &chain, 11, 30).await.unwrap()),
            ids(&all_a[10..])
        );
    }
    assert!(cached.cache_stats().0 >= 2);
}

/// One store pass of many sessions writes one object, and the segments it
/// returns read back what the sessions hold.
#[tokio::test]
async fn a_store_pass_writes_one_pack_for_every_session() {
    let store = Store::open("memory://").unwrap();
    let shared = crate::sm::Shared::new(store.clone(), 4);
    let sessions: Vec<SessionId> = (0..40).map(|_| SessionId::new()).collect();
    {
        let mut state = shared.state.write();
        for id in &sessions {
            state.sessions.insert(
                *id,
                SessionState {
                    events: events(*id, 1, 9),
                    ..Default::default()
                },
            );
        }
    }
    let stored = shared.flush(512, Duration::ZERO, 2).await;
    assert_eq!(stored.len(), sessions.len());
    assert_eq!(objects(&store).await, 1, "one pack for 40 sessions");
    for f in stored {
        assert!(!f.compact);
        let pack = f.seg.pack.expect("written to a pack");
        assert_eq!((pack.shard, pack.writer), (4, 2));
        let held = shared.state.read().sessions[&f.session].events.clone();
        assert_eq!(
            ids(&store.load(f.session, &[f.seg], 1, 9).await.unwrap()),
            ids(&held)
        );
    }
}

fn state_naming(session: SessionId, segments: Vec<SegRef>) -> bytes::Bytes {
    let mut state = ShardState::default();
    state.sessions.insert(
        session,
        SessionState {
            segments,
            ..Default::default()
        },
    );
    serde_json::to_vec(&state).unwrap().into()
}

async fn pack_with_one(store: &Store, session: SessionId) -> SegRef {
    let mut pack = PackBuilder::new(PackId::new(0, 0, 1));
    let seg = pack.add(&events(session, 1, 5), 0).unwrap();
    store.put_pack(pack).await.unwrap();
    seg
}

/// Age every pack in the file store at `dir` past `window` plus the grace.
fn age_packs(dir: &std::path::Path, window: Duration) {
    let old = std::time::SystemTime::now() - window - PACK_GRACE - Duration::from_secs(3600);
    let mut stack = vec![dir.join("packs")];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let file = std::fs::File::options().write(true).open(&path).unwrap();
                file.set_modified(old).unwrap();
            }
        }
    }
}

/// The sweep deletes an old pack nothing names, and keeps one a kept
/// snapshot or a restore pin names, and every young one.
#[tokio::test]
async fn the_sweep_deletes_only_old_packs_no_restore_point_names() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&format!("file://{}", dir.path().display())).unwrap();
    let session = SessionId::new();
    let named = pack_with_one(&store, session).await;
    let pinned = pack_with_one(&store, session).await;
    let orphan = pack_with_one(&store, session).await;
    store
        .put_snapshot(
            0,
            "s1",
            0,
            10,
            1,
            [
                bytes::Bytes::from_static(b"{}"),
                state_naming(session, vec![named]),
            ],
        )
        .await
        .unwrap();
    store.pin_packs([&pinned]).await.unwrap();
    let window = Duration::from_secs(3600);

    let young = store
        .sweep_packs(window, crate::node::now_ms())
        .await
        .unwrap();
    assert_eq!((young.deleted, young.kept), (0, 3), "young packs stay");

    age_packs(dir.path(), window);
    let swept = store
        .sweep_packs(window, crate::node::now_ms())
        .await
        .unwrap();
    assert_eq!(swept.skipped, None);
    assert_eq!((swept.deleted, swept.kept), (1, 2));
    assert!(store.load(session, &[named], 1, 5).await.is_ok());
    assert!(store.load(session, &[pinned], 1, 5).await.is_ok());
    assert!(store.load(session, &[orphan], 1, 5).await.is_err());
}

/// A snapshot stored after packs exist but without its sidecar (an older
/// node took it) may name any pack: the sweep then deletes nothing.
#[tokio::test]
async fn a_snapshot_without_its_pack_refs_stops_the_sweep() {
    let store = Store::open("memory://").unwrap();
    let session = SessionId::new();
    let orphan = pack_with_one(&store, session).await;
    store
        .put_object("shards/0/snapshots/i0-n10-t1-old.snap", b"{}".to_vec())
        .await
        .unwrap();
    let window = Duration::from_secs(3600);
    let later = crate::node::now_ms() + (window + PACK_GRACE).as_millis() as i64 + 60_000;
    let swept = store.sweep_packs(window, later).await.unwrap();
    assert!(swept.skipped.is_some());
    assert_eq!(swept.deleted, 0);
    assert!(store.load(session, &[orphan], 1, 5).await.is_ok());
}
