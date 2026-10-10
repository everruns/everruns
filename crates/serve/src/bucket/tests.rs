use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use everruns::sqlite as rusqlite;
use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};

use super::*;
use crate::app::Mode;
use crate::host::{Host, NewSession};
use crate::wire_tests::app;

const TTL: Duration = Duration::from_millis(400);

fn bucket(store: &Arc<InMemory>) -> Bucket {
    Bucket::new(store.clone() as Arc<dyn ObjectStore>, "apps/demo")
        .with_lease_ttl(TTL)
        .with_sync_interval(Duration::from_secs(3600))
}

fn insert(dir: &Path, name: &str, rows: usize) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = rusqlite::open(path).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, body TEXT);",
    )
    .unwrap();
    for i in 0..rows {
        conn.execute(
            "INSERT INTO t (body) VALUES (?1)",
            [format!("{i} {}", "x".repeat(300))],
        )
        .unwrap();
    }
}

fn count(dir: &Path, name: &str) -> i64 {
    rusqlite::open(dir.join(name))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
        .unwrap()
}

async fn manifest(store: &InMemory) -> Manifest {
    let got = store
        .get(&ObjectPath::from("apps/demo/manifest.json"))
        .await
        .unwrap();
    serde_json::from_slice(&got.bytes().await.unwrap()).unwrap()
}

#[tokio::test]
async fn another_machine_rebuilds_the_data_dir_from_the_bucket() {
    let store = Arc::new(InMemory::new());
    let first_dir = tempfile::tempdir().unwrap();
    let first = bucket(&store).attach(first_dir.path()).await.unwrap();
    assert_eq!(first.fence(), 1);
    insert(first_dir.path(), "serve.db", 20);
    insert(first_dir.path(), "everruns/local.db", 5);
    first.sync().await.unwrap();
    // Small changes travel as page segments on top of the snapshot.
    insert(first_dir.path(), "serve.db", 3);
    first.sync().await.unwrap();
    let written = manifest(&store).await;
    assert_eq!(written.files["serve.db"].segments.len(), 1);
    first.detach().await.unwrap();

    // Released: the next daemon takes over at once, on an empty directory.
    let second_dir = tempfile::tempdir().unwrap();
    let started = tokio::time::Instant::now();
    let second = bucket(&store).attach(second_dir.path()).await.unwrap();
    assert!(started.elapsed() < TTL);
    assert_eq!(second.fence(), 2);
    assert_eq!(count(second_dir.path(), "serve.db"), 23);
    assert_eq!(count(second_dir.path(), "everruns/local.db"), 5);
    assert!(!second_dir.path().join(MIRROR_DIR).join("serve.db").exists());
}

#[tokio::test]
async fn a_killed_daemons_lease_is_waited_out_and_its_last_copy_restored() {
    let store = Arc::new(InMemory::new());
    let first_dir = tempfile::tempdir().unwrap();
    let first = bucket(&store).attach(first_dir.path()).await.unwrap();
    insert(first_dir.path(), "serve.db", 10);
    first.sync().await.unwrap();
    // Killed: nothing released, and the last write never reached the bucket.
    insert(first_dir.path(), "serve.db", 1);
    drop(first);

    let second_dir = tempfile::tempdir().unwrap();
    let started = tokio::time::Instant::now();
    let second = bucket(&store).attach(second_dir.path()).await.unwrap();
    assert!(started.elapsed() >= TTL / 2, "waited out the lease");
    assert_eq!(count(second_dir.path(), "serve.db"), 10);
    assert_eq!(second.fence(), 2);
}

#[tokio::test]
async fn a_live_daemon_keeps_the_bucket() {
    let store = Arc::new(InMemory::new());
    let first_dir = tempfile::tempdir().unwrap();
    let _first = bucket(&store).attach(first_dir.path()).await.unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let err = bucket(&store).attach(second_dir.path()).await.unwrap_err();
    assert!(err.to_string().contains("held by another daemon"), "{err}");
}

#[tokio::test]
async fn a_daemon_that_was_taken_over_stops_writing() {
    let store = Arc::new(InMemory::new());
    let first_dir = tempfile::tempdir().unwrap();
    let first = bucket(&store).attach(first_dir.path()).await.unwrap();
    insert(first_dir.path(), "serve.db", 4);
    first.sync().await.unwrap();

    // The first daemon stalls past its lease (simulated by clearing it), and
    // a second one takes the bucket.
    store
        .put(
            &ObjectPath::from("apps/demo/lease.json"),
            PutPayload::from(r#"{"holder":null,"fence":1,"expires_at_ms":0}"#),
        )
        .await
        .unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let second = bucket(&store).attach(second_dir.path()).await.unwrap();
    assert_eq!(second.fence(), 2);

    insert(first_dir.path(), "serve.db", 1);
    let err = first.sync().await.unwrap_err();
    assert!(err.to_string().contains("another daemon"), "{err}");
    let reason = tokio::time::timeout(Duration::from_secs(1), first.lost())
        .await
        .unwrap();
    assert!(reason.contains("another daemon"), "{reason}");
    assert_eq!(manifest(&store).await.fence, 2);
    assert_eq!(count(second_dir.path(), "serve.db"), 4);
}

#[tokio::test]
async fn segments_fold_into_a_new_snapshot_once_they_outgrow_it() {
    let store = Arc::new(InMemory::new());
    let dir = tempfile::tempdir().unwrap();
    // Forty syncs can outlast the short test lease on a slow runner, and this
    // test is about compaction, not leases.
    let attached = bucket(&store)
        .with_lease_ttl(Duration::from_secs(60))
        .attach(dir.path())
        .await
        .unwrap();
    insert(dir.path(), "serve.db", 1);
    attached.sync().await.unwrap();
    let first = manifest(&store).await.files["serve.db"].snapshot.clone();
    for _ in 0..40 {
        insert(dir.path(), "serve.db", 20);
        attached.sync().await.unwrap();
    }
    let entry = manifest(&store).await.files["serve.db"].clone();
    assert_ne!(entry.snapshot, first);
    assert!(entry.segment_bytes <= entry.snapshot_bytes);
    // The old snapshot is gone.
    assert!(
        store
            .get(&ObjectPath::from(format!("apps/demo/{first}")))
            .await
            .is_err()
    );
    attached.detach().await.unwrap();

    let elsewhere = tempfile::tempdir().unwrap();
    let _again = bucket(&store).attach(elsewhere.path()).await.unwrap();
    assert_eq!(count(elsewhere.path(), "serve.db"), 801);
}

#[tokio::test]
async fn the_copy_loop_ships_changes_on_its_own() {
    let store = Arc::new(InMemory::new());
    let dir = tempfile::tempdir().unwrap();
    let attached = Bucket::new(store.clone() as Arc<dyn ObjectStore>, "apps/demo")
        .with_lease_ttl(TTL)
        .with_sync_interval(Duration::from_millis(20))
        .attach(dir.path())
        .await
        .unwrap();
    insert(dir.path(), "serve.db", 2);
    for _ in 0..200 {
        if manifest(&store).await.files.contains_key("serve.db") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(manifest(&store).await.files.contains_key("serve.db"));
    // The renew loop keeps the lease past its first life.
    tokio::time::sleep(TTL * 2).await;
    attached.sync().await.unwrap();
}

#[test]
fn only_s3_and_memory_urls_are_stores() {
    assert!(Bucket::parse("memory://apps/demo").is_ok());
    assert!(Bucket::parse("file:///tmp/x").is_err());
    assert!(Bucket::parse("s3:///prefix").is_err());
}

#[tokio::test]
async fn plain_files_travel_whole_and_deletions_follow() {
    let store = Arc::new(InMemory::new());
    let dir = tempfile::tempdir().unwrap();
    let attached = bucket(&store).attach(dir.path()).await.unwrap();
    let workspace = dir.path().join("everruns/workspace");
    std::fs::create_dir_all(workspace.join("notes")).unwrap();
    std::fs::write(workspace.join("notes/a.md"), "alpha").unwrap();
    std::fs::write(workspace.join("b.md"), "alpha").unwrap();
    std::fs::write(workspace.join("c.md"), "gone soon").unwrap();
    attached.sync().await.unwrap();
    std::fs::remove_file(workspace.join("c.md")).unwrap();
    std::fs::write(workspace.join("b.md"), "beta").unwrap();
    attached.sync().await.unwrap();
    let written = manifest(&store).await;
    assert_eq!(written.blobs.len(), 2);
    // "gone soon" is no longer named, so its blob is deleted; "alpha" stays.
    let blobs = store
        .list_with_delimiter(Some(&ObjectPath::from("apps/demo/blobs")))
        .await
        .unwrap();
    assert_eq!(blobs.objects.len(), 2);
    attached.detach().await.unwrap();

    // A stale directory becomes a copy of the bucket.
    std::fs::write(workspace.join("stale.md"), "old").unwrap();
    let _again = bucket(&store).attach(dir.path()).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.join("notes/a.md")).unwrap(),
        "alpha"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("b.md")).unwrap(),
        "beta"
    );
    assert!(!workspace.join("c.md").exists());
    assert!(!workspace.join("stale.md").exists());
}

/// A serve host on a bucket: a session, its log and its workspace written
/// on one machine are there on another after the first was killed. The data
/// directory has the same path on both, as a deployment's would.
#[tokio::test]
async fn a_session_survives_its_daemon_being_killed_and_started_elsewhere() {
    let store = Arc::new(InMemory::new());
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");
    let (id, before) = {
        let attached = bucket(&store).attach(&data_dir).await.unwrap();
        let host = Host::new(app(), Mode::Dev, Some(data_dir.clone())).unwrap();
        let id = host.create_session(NewSession::default()).await.unwrap();
        let turn = host.send(&id, "before".into()).await.unwrap().wait().await;
        assert!(turn.unwrap().success);
        let before = host.events_after(&id, 0).await.unwrap().len();
        attached.sync().await.unwrap();
        // Killed: no detach.
        (id, before)
    };
    // Another machine: nothing of the first one's disk.
    std::fs::remove_dir_all(&data_dir).unwrap();

    let _attached = bucket(&store).attach(&data_dir).await.unwrap();
    let host = Host::new(app(), Mode::Dev, Some(data_dir.clone())).unwrap();
    assert_eq!(host.events_after(&id, 0).await.unwrap().len(), before);
    let turn = host.send(&id, "after".into()).await.unwrap().wait().await;
    assert!(turn.unwrap().success);
    assert!(host.events_after(&id, 0).await.unwrap().len() > before);
}

/// The lease and manifest against a real S3-compatible bucket, when
/// `SERVE_TEST_S3_URL` names one (`s3://bucket`, with `AWS_*` credentials
/// and endpoint in the environment): conditional create, conditional
/// update, and the precondition failure that fences out an old daemon.
#[tokio::test]
async fn takeover_against_a_real_bucket() {
    let Some(url) = std::env::var("SERVE_TEST_S3_URL")
        .ok()
        .filter(|url| !url.is_empty())
    else {
        eprintln!("SERVE_TEST_S3_URL not set; skipping the real-bucket takeover");
        return;
    };
    let url = format!(
        "{}/serve-test-{}",
        url.trim_end_matches('/'),
        uuid::Uuid::new_v4()
    );
    let bucket = || {
        Bucket::parse(&url)
            .unwrap()
            .with_lease_ttl(Duration::from_secs(2))
            .with_sync_interval(Duration::from_secs(3600))
    };
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");

    let first = bucket().attach(&data_dir).await.unwrap();
    insert(&data_dir, "serve.db", 30);
    std::fs::create_dir_all(data_dir.join("everruns/workspace")).unwrap();
    std::fs::write(data_dir.join("everruns/workspace/a.md"), "alpha").unwrap();
    first.sync().await.unwrap();
    insert(&data_dir, "serve.db", 2);
    first.sync().await.unwrap();
    drop(first);
    std::fs::remove_dir_all(&data_dir).unwrap();

    // The killed daemon's lease is waited out.
    let second = bucket().attach(&data_dir).await.unwrap();
    assert_eq!(second.fence(), 2);
    assert_eq!(count(&data_dir, "serve.db"), 32);
    assert_eq!(
        std::fs::read_to_string(data_dir.join("everruns/workspace/a.md")).unwrap(),
        "alpha"
    );

    // A live holder keeps the bucket.
    let elsewhere = root.path().join("elsewhere");
    let err = bucket().attach(&elsewhere).await.unwrap_err();
    assert!(err.to_string().contains("held by another daemon"), "{err}");

    // A stalled holder is fenced out by the next one.
    let store = Bucket::parse(&url).unwrap();
    store
        .store
        .put(
            &store.path("lease.json"),
            PutPayload::from(r#"{"holder":null,"fence":2,"expires_at_ms":0}"#),
        )
        .await
        .unwrap();
    let third = bucket().attach(&elsewhere).await.unwrap();
    assert_eq!(third.fence(), 3);
    insert(&data_dir, "serve.db", 1);
    let err = second.sync().await.unwrap_err();
    assert!(err.to_string().contains("another daemon"), "{err}");
    third.detach().await.unwrap();
}
