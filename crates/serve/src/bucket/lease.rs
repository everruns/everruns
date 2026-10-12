//! The daemon's lease: one object in the bucket, taken and renewed with
//! conditional writes.
//!
//! Decisions:
//! - Expiry is wall time in milliseconds, as another machine reads it.
//! - A held lease is waited out for one lease life before giving up, so a
//!   daemon started elsewhere takes over from one that was killed and could
//!   not release it. A live holder renews, and the newcomer then fails.
//! - Every take raises the fence. Release clears the holder and keeps the
//!   fence, so the next holder's fence is still higher.

use std::time::Duration;

use anyhow::bail;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt, PutMode, PutOptions, UpdateVersion};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

#[derive(Debug, Serialize, Deserialize)]
struct LeaseRecord {
    holder: Option<String>,
    fence: u64,
    expires_at_ms: i64,
}

/// A lease this daemon holds.
#[derive(Debug)]
pub(super) struct Lease {
    holder: String,
    pub(super) fence: u64,
    version: UpdateVersion,
    /// When this process stops trusting the lease, by its own clock.
    pub(super) valid_until: Instant,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

pub(super) fn version_of(e_tag: Option<String>, version: Option<String>) -> UpdateVersion {
    UpdateVersion { e_tag, version }
}

async fn read(
    store: &dyn ObjectStore,
    path: &ObjectPath,
) -> crate::Result<Option<(LeaseRecord, UpdateVersion)>> {
    match store.get(path).await {
        Ok(got) => {
            let version = version_of(got.meta.e_tag.clone(), got.meta.version.clone());
            let record = serde_json::from_slice(&got.bytes().await?)?;
            Ok(Some((record, version)))
        }
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Whether a conditional write lost a race (someone else wrote first).
pub(super) fn lost_race(err: &object_store::Error) -> bool {
    matches!(
        err,
        object_store::Error::AlreadyExists { .. } | object_store::Error::Precondition { .. }
    )
}

async fn write(
    store: &dyn ObjectStore,
    path: &ObjectPath,
    mode: PutMode,
    record: &LeaseRecord,
) -> Result<UpdateVersion, object_store::Error> {
    let body = serde_json::to_vec(record).map_err(|err| object_store::Error::Generic {
        store: "lease",
        source: err.into(),
    })?;
    let put = store
        .put_opts(
            path,
            body.into(),
            PutOptions {
                mode,
                ..PutOptions::default()
            },
        )
        .await?;
    Ok(version_of(put.e_tag, put.version))
}

/// Take the lease at `path` for `holder`, waiting out a holder that stopped
/// renewing for up to about one `ttl`.
pub(super) async fn acquire(
    store: &dyn ObjectStore,
    path: &ObjectPath,
    holder: &str,
    ttl: Duration,
) -> crate::Result<Lease> {
    // A quarter of a lease life on top, for clocks that disagree a little.
    let deadline = Instant::now() + ttl + ttl / 4;
    loop {
        let now = now_ms();
        let (mode, fence) = match read(store, path).await? {
            None => (PutMode::Create, 1),
            Some((record, version)) if record.holder.is_none() || record.expires_at_ms <= now => {
                (PutMode::Update(version), record.fence + 1)
            }
            Some((record, _)) => {
                let left = Instant::now();
                if left >= deadline {
                    bail!(
                        "the store is held by another daemon (lease fence {}); stop it, or wait for its lease to expire",
                        record.fence
                    );
                }
                let expires_in =
                    Duration::from_millis(u64::try_from(record.expires_at_ms - now).unwrap_or(0));
                let poll = expires_in
                    .min(deadline - left)
                    .min(ttl / 10)
                    .max(Duration::from_millis(10));
                tokio::time::sleep(poll).await;
                continue;
            }
        };
        let taken_at = Instant::now();
        let record = LeaseRecord {
            holder: Some(holder.to_string()),
            fence,
            expires_at_ms: now + ms(ttl),
        };
        match write(store, path, mode, &record).await {
            Ok(version) => {
                return Ok(Lease {
                    holder: holder.to_string(),
                    fence,
                    version,
                    valid_until: taken_at + ttl,
                });
            }
            Err(err) if lost_race(&err) => continue,
            Err(err) => return Err(err.into()),
        }
    }
}

impl Lease {
    /// Extend the lease by `ttl`. `Ok(false)` when another daemon took it.
    pub(super) async fn renew(
        &mut self,
        store: &dyn ObjectStore,
        path: &ObjectPath,
        ttl: Duration,
    ) -> crate::Result<bool> {
        let started = Instant::now();
        let record = LeaseRecord {
            holder: Some(self.holder.clone()),
            fence: self.fence,
            expires_at_ms: now_ms() + ms(ttl),
        };
        match write(store, path, PutMode::Update(self.version.clone()), &record).await {
            Ok(version) => {
                self.version = version;
                self.valid_until = started + ttl;
                Ok(true)
            }
            Err(err) if lost_race(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    /// Give the lease up, keeping its fence. A lease another daemon already
    /// took is left alone.
    pub(super) async fn release(
        &self,
        store: &dyn ObjectStore,
        path: &ObjectPath,
    ) -> crate::Result<()> {
        let record = LeaseRecord {
            holder: None,
            fence: self.fence,
            expires_at_ms: 0,
        };
        match write(store, path, PutMode::Update(self.version.clone()), &record).await {
            Ok(_) => Ok(()),
            Err(err) if lost_race(&err) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;

    #[tokio::test]
    async fn a_held_lease_passes_on_only_when_released_or_expired() {
        let store = InMemory::new();
        let path = ObjectPath::from("app/lease.json");
        let ttl = Duration::from_millis(300);

        let mut ours = acquire(&store, &path, "a", ttl).await.unwrap();
        assert_eq!(ours.fence, 1);
        assert!(ours.renew(&store, &path, ttl).await.unwrap());

        // A live holder keeps it: the newcomer waits one lease life, then fails.
        let renewing = async {
            for _ in 0..6 {
                tokio::time::sleep(ttl / 3).await;
                assert!(ours.renew(&store, &path, ttl).await.unwrap());
            }
        };
        let (taken, ()) = tokio::join!(acquire(&store, &path, "b", ttl), renewing);
        assert!(
            taken
                .unwrap_err()
                .to_string()
                .contains("held by another daemon")
        );

        ours.release(&store, &path).await.unwrap();
        let theirs = acquire(&store, &path, "b", ttl).await.unwrap();
        assert_eq!(theirs.fence, 2);
        assert!(!ours.renew(&store, &path, ttl).await.unwrap());

        // A holder that stops renewing is waited out.
        let started = Instant::now();
        let third = acquire(&store, &path, "c", ttl).await.unwrap();
        assert_eq!(third.fence, 3);
        assert!(started.elapsed() >= ttl / 2);
    }
}
