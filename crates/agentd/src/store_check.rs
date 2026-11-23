//! `store-check`: the object store operations the shard's safety rests on,
//! run against the real store a deployment uses (`app store-check --store
//! s3://...`), under a scratch prefix of their own.
//!
//! Decision: unit tests run these on `memory://` and `file://`, where every
//! operation is atomic and immediately visible; a real S3 (or an
//! S3-compatible store such as RustFS or MinIO) may differ in what it
//! offers, and the difference is invisible to those tests. This command
//! is how load runs and the live fleet's setup check the real one before
//! trusting it:
//! - a create-only write (`claim_incarnation`) that two racers cannot both
//!   win;
//! - a segment written again after it was retired outliving its mark
//!   (create, retire, replace in one process);
//! - a restore taking a retirement back (`Store::unretire`);
//! - the shipped log listing back what was written.

use std::collections::HashSet;
use std::time::Duration;

use everruns_contracts::typed_id::SessionId;
use serde::Serialize;

use crate::store::{SegRef, Store};

/// One check's outcome.
#[derive(Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Run every check against `url` under a new scratch prefix, then delete
/// what they wrote. Returns each check's outcome; the caller fails on any.
pub async fn run(url: &str) -> anyhow::Result<Vec<Check>> {
    let scratch = format!(
        "{}/store-check-{}",
        url.trim_end_matches('/'),
        crate::node::now_ms()
    );
    let store = Store::open(&scratch)?;
    let mut checks = Vec::new();
    checks.push(claim_once(&store).await);
    checks.push(rewrite_outlives_mark(&store).await);
    checks.push(unretire(&store).await);
    checks.push(shipped_lists_back(&store).await);
    let _ = store.delete_prefix("").await;
    Ok(checks)
}

fn check(name: &'static str, result: anyhow::Result<String>) -> Check {
    match result {
        Ok(detail) => Check {
            name,
            ok: true,
            detail,
        },
        Err(error) => Check {
            name,
            ok: false,
            detail: format!("{error:#}"),
        },
    }
}

async fn claim_once(store: &Store) -> Check {
    check(
        "create-only write has one winner",
        async {
            let racers =
                futures::future::join_all((0..4).map(|_| store.claim_incarnation(9, 2))).await;
            let won: Vec<u64> = racers.into_iter().collect::<anyhow::Result<_>>()?;
            let distinct: HashSet<u64> = won.iter().copied().collect();
            anyhow::ensure!(
                distinct.len() == won.len(),
                "two racers claimed the same incarnation: {won:?}"
            );
            Ok(format!("claimed {won:?}"))
        }
        .await,
    )
}

fn seg(from: i32, to: i32) -> SegRef {
    SegRef {
        from,
        to,
        branch: 1,
    }
}

async fn rewrite_outlives_mark(store: &Store) -> Check {
    check(
        "a segment written after its retirement stays",
        async {
            let session = SessionId::new();
            let (rewritten, retired) = (seg(1, 5), seg(6, 9));
            for s in [rewritten, retired] {
                store
                    .put(store.segment_key(session, s), b"[]".to_vec())
                    .await?;
            }
            store
                .retire_segments(session, &[rewritten, retired])
                .await?;
            // Object times are whole seconds on S3.
            tokio::time::sleep(Duration::from_millis(1100)).await;
            store
                .put(store.segment_key(session, rewritten), b"[]".to_vec())
                .await?;
            let later = crate::node::now_ms() + 3 * 3_600_000;
            let done = store
                .collect(0, true, Duration::from_secs(3600), later)
                .await?;
            anyhow::ensure!(
                store
                    .get(store.segment_key(session, rewritten))
                    .await
                    .is_ok(),
                "the segment written after its mark was deleted"
            );
            anyhow::ensure!(
                store
                    .get(store.segment_key(session, retired))
                    .await
                    .is_err(),
                "the retired segment was not collected"
            );
            Ok(format!("collected {}", done.segments))
        }
        .await,
    )
}

async fn unretire(store: &Store) -> Check {
    check(
        "a restore takes a retirement back",
        async {
            let session = SessionId::new();
            let s = seg(10, 20);
            store
                .put(store.segment_key(session, s), b"[]".to_vec())
                .await?;
            store.retire_segments(session, &[s]).await?;
            let taken = store.unretire(&HashSet::from([(session, s)])).await?;
            anyhow::ensure!(taken == 1, "took back {taken} marks, not 1");
            let later = crate::node::now_ms() + 3 * 3_600_000;
            store
                .collect(0, true, Duration::from_secs(3600), later)
                .await?;
            anyhow::ensure!(
                store.get(store.segment_key(session, s)).await.is_ok(),
                "a segment taken back from retirement was deleted"
            );
            Ok("kept".into())
        }
        .await,
    )
}

async fn shipped_lists_back(store: &Store) -> Check {
    check(
        "the shipped log lists back",
        async {
            store.put_log(9, 2, 1, 4, b"[]".to_vec()).await?;
            store.put_log(9, 2, 5, 9, b"[]".to_vec()).await?;
            let listed = store.shipped(9, 2).await?;
            anyhow::ensure!(listed == vec![(1, 4), (5, 9)], "listed {listed:?}");
            Ok(format!("{listed:?}"))
        }
        .await,
    )
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn every_check_passes_on_the_file_store() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("file://{}", dir.path().display());
        let checks = super::run(&url).await.unwrap();
        assert_eq!(checks.len(), 4);
        for c in &checks {
            assert!(c.ok, "{c:?}");
        }
    }
}
