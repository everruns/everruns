//! A daemon whose store is a bucket: `serve start --store s3://bucket/prefix`.
//!
//! The data directory stays the working copy (serve's catalog and the
//! engine's SQLite files); the bucket holds the truth. A daemon started on
//! another machine with the same `--store` takes over: it waits out the old
//! daemon's lease, rebuilds the data directory from the bucket, and boot
//! resume carries on the turns the old one was running.
//!
//! Decisions (actor-based design, step 5, storage option C):
//! - One lease per daemon, an object in the bucket taken with conditional
//!   writes (`lease.json`). The bucket must support conditional create and
//!   overwrite (S3, R2, GCS, MinIO, Tigris do).
//! - The manifest (`manifest.json`) names each database's snapshot and the
//!   page segments after it, and is only ever written with a conditional
//!   update. It is the fence: once another daemon has taken over and written
//!   it, the old daemon's next write fails and the old daemon stops.
//! - Every SQLite file under the data directory is copied by changed pages
//!   (see [`pages`]); every other file (the agents' workspace) whole, under
//!   its content hash, when it changes. The copy loop checks every
//!   `sync_interval` (200 ms by default) and ships what changed. That is not a copy per commit before it
//!   is acknowledged: a crash loses at most that window plus one upload, and
//!   resume (step 1) carries a turn on from what reached the log.
//! - A segment run longer than its snapshot is folded into a new snapshot,
//!   and objects the manifest no longer names are deleted after it is
//!   written.
//! - Attaching makes the data directory a copy of the bucket: files the
//!   bucket does not have are removed. A bucket with no manifest yet starts
//!   from what the directory holds.
//! - The data directory's absolute path must be the same on every machine:
//!   a session's workspace binding records it.
//! - Symbolic links and empty directories are not copied.

mod lease;
mod pages;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail};
use object_store::aws::AmazonS3Builder;
use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt, PutMode, PutOptions, UpdateVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use self::lease::{Lease, lost_race, version_of};
use self::pages::{Fingerprint, Pages};

/// How long the daemon's lease lasts; it is renewed every third of that.
pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(15);

/// How often the copy loop looks for changed databases.
pub const DEFAULT_SYNC_INTERVAL: Duration = Duration::from_millis(200);

/// Segments per database before they are folded into a new snapshot, even
/// when they are still smaller than it.
const MAX_SEGMENTS: usize = 256;

/// Where the copy loop keeps its consistent copies, inside the data dir.
const MIRROR_DIR: &str = ".bucket";

/// A bucket a daemon keeps its data in. See the module docs.
#[derive(Clone)]
pub struct Bucket {
    store: Arc<dyn ObjectStore>,
    prefix: ObjectPath,
    lease_ttl: Duration,
    sync_interval: Duration,
}

impl std::fmt::Debug for Bucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bucket")
            .field("store", &self.store.to_string())
            .field("prefix", &self.prefix.as_ref())
            .finish_non_exhaustive()
    }
}

impl Bucket {
    /// A bucket from a URL: `s3://bucket/prefix`, with credentials, region
    /// and endpoint from the standard `AWS_*` environment variables
    /// (`AWS_ENDPOINT` and `AWS_ALLOW_HTTP` for MinIO and other S3-compatible
    /// stores), or `memory://prefix`, which lives only as long as the process.
    pub fn parse(url: &str) -> crate::Result<Self> {
        if let Some(rest) = url.strip_prefix("s3://") {
            let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
            if bucket.is_empty() {
                bail!("`{url}` names no bucket; use s3://bucket/prefix");
            }
            let store = AmazonS3Builder::from_env()
                .with_bucket_name(bucket)
                .build()?;
            return Ok(Self::new(Arc::new(store), prefix));
        }
        if let Some(prefix) = url.strip_prefix("memory://") {
            return Ok(Self::new(Arc::new(InMemory::new()), prefix));
        }
        Err(anyhow!(
            "unsupported store `{url}`: use s3://bucket/prefix (S3 or an S3-compatible store)"
        ))
    }

    /// A bucket over any object store with conditional writes, keeping
    /// everything under `prefix`.
    pub fn new(store: Arc<dyn ObjectStore>, prefix: &str) -> Self {
        Self {
            store,
            prefix: ObjectPath::from(prefix.trim_matches('/')),
            lease_ttl: DEFAULT_LEASE_TTL,
            sync_interval: DEFAULT_SYNC_INTERVAL,
        }
    }

    /// Override [`DEFAULT_LEASE_TTL`].
    #[must_use]
    pub fn with_lease_ttl(mut self, ttl: Duration) -> Self {
        self.lease_ttl = ttl;
        self
    }

    /// Override [`DEFAULT_SYNC_INTERVAL`].
    #[must_use]
    pub fn with_sync_interval(mut self, interval: Duration) -> Self {
        self.sync_interval = interval;
        self
    }

    fn path(&self, name: &str) -> ObjectPath {
        self.prefix.clone().join(name)
    }

    fn object(&self, key: &str) -> ObjectPath {
        key.split('/')
            .fold(self.prefix.clone(), |path, part| path.join(part))
    }

    /// Take the bucket's lease, waiting out a daemon that stopped renewing
    /// it, and rebuild `data_dir` from the bucket. Boot the server on
    /// `data_dir` after this returns. A bucket with nothing in it yet starts
    /// from whatever `data_dir` already holds.
    pub async fn attach(&self, data_dir: &Path) -> crate::Result<Attached> {
        let holder = uuid::Uuid::new_v4().to_string();
        let lease_path = self.path("lease.json");
        let lease = lease::acquire(&*self.store, &lease_path, &holder, self.lease_ttl).await?;
        let fence = lease.fence;

        std::fs::create_dir_all(data_dir)?;
        let (manifest, version) = match self.read_manifest().await? {
            Some((manifest, version)) => (manifest, Some(version)),
            None => (Manifest::default(), None),
        };
        let mut files = BTreeMap::new();
        if version.is_some() {
            let (databases, plain) = scan(data_dir)?;
            for name in databases.iter().chain(&plain) {
                if !manifest.files.contains_key(name) && !manifest.blobs.contains_key(name) {
                    let path = data_dir.join(name);
                    remove_if_present(&path)?;
                    for suffix in ["-wal", "-shm", "-journal"] {
                        remove_if_present(&pages::sidecar(&path, suffix))?;
                    }
                }
            }
        }
        let mut blobs = BTreeMap::new();
        for (name, blob) in &manifest.blobs {
            let bytes = self
                .store
                .get(&self.object(&blob_key(&blob.sha256)))
                .await?
                .bytes()
                .await?;
            let path = data_dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &bytes)?;
            blobs.insert(name.clone(), Seen { stat: stat(&path) });
        }
        for (name, entry) in &manifest.files {
            let bytes = self.restore(entry).await?;
            let path = data_dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            for suffix in ["-wal", "-shm", "-journal"] {
                remove_if_present(&pages::sidecar(&path, suffix))?;
            }
            std::fs::write(&path, &bytes)?;
            files.insert(
                name.clone(),
                Tracked {
                    pages: Pages::of(&bytes)?,
                    fingerprint: None,
                },
            );
        }

        // Claim the manifest at once, so a daemon this one took over from
        // fails its next write.
        let mut sync = Copier {
            bucket: self.clone(),
            data_dir: data_dir.to_path_buf(),
            manifest: Manifest {
                version: MANIFEST_VERSION,
                fence,
                ..manifest
            },
            version,
            files,
            blobs,
        };
        sync.write_manifest(fence).await?;

        let (lost_tx, lost) = watch::channel(None);
        let inner = Arc::new(Inner {
            bucket: self.clone(),
            lease_path,
            lease: Mutex::new(lease),
            sync: Mutex::new(sync),
            lost: lost_tx,
        });
        let tasks = vec![
            tokio::spawn(renew_loop(inner.clone())),
            tokio::spawn(sync_loop(inner.clone())),
        ];
        Ok(Attached {
            inner,
            tasks,
            lost,
            fence,
        })
    }

    async fn read_manifest(&self) -> crate::Result<Option<(Manifest, UpdateVersion)>> {
        match self.store.get(&self.path("manifest.json")).await {
            Ok(got) => {
                let version = version_of(got.meta.e_tag.clone(), got.meta.version.clone());
                let manifest: Manifest = serde_json::from_slice(&got.bytes().await?)?;
                if manifest.version != MANIFEST_VERSION {
                    bail!(
                        "the bucket's manifest is version {}, this serve reads {MANIFEST_VERSION}",
                        manifest.version
                    );
                }
                Ok(Some((manifest, version)))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    async fn restore(&self, entry: &FileEntry) -> crate::Result<Vec<u8>> {
        let mut bytes = self
            .store
            .get(&self.object(&entry.snapshot))
            .await?
            .bytes()
            .await?
            .to_vec();
        for segment in &entry.segments {
            let segment = self.store.get(&self.object(segment)).await?.bytes().await?;
            pages::apply_segment(&mut bytes, &segment)?;
        }
        Ok(bytes)
    }

    async fn put_new(&self, key: &str, body: Vec<u8>) -> crate::Result<()> {
        self.store
            .put_opts(
                &self.object(key),
                body.into(),
                PutOptions {
                    mode: PutMode::Create,
                    ..PutOptions::default()
                },
            )
            .await?;
        Ok(())
    }
}

/// A daemon attached to a [`Bucket`]: it holds the lease, and a background
/// loop copies the data directory's databases to the bucket. Dropping it
/// stops both without releasing the lease, as a crash would; call
/// [`Attached::detach`] on a clean shutdown.
pub struct Attached {
    inner: Arc<Inner>,
    tasks: Vec<JoinHandle<()>>,
    lost: watch::Receiver<Option<String>>,
    fence: u64,
}

impl std::fmt::Debug for Attached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attached")
            .field("fence", &self.fence)
            .finish_non_exhaustive()
    }
}

impl Attached {
    /// The lease's fence: it grows with every daemon that takes the bucket.
    pub fn fence(&self) -> u64 {
        self.fence
    }

    /// Copy whatever changed to the bucket now.
    pub async fn sync(&self) -> crate::Result<()> {
        if let Some(reason) = self.lost.borrow().clone() {
            bail!("this daemon lost the store: {reason}");
        }
        self.inner.sync_once().await
    }

    /// Resolves when this daemon lost the store (another daemon took the
    /// lease or the manifest), with the reason. The daemon must stop then.
    pub async fn lost(&self) -> String {
        let mut lost = self.lost.clone();
        loop {
            if let Some(reason) = lost.borrow_and_update().clone() {
                return reason;
            }
            if lost.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    /// Clean shutdown: copy what is left and release the lease, so the next
    /// daemon need not wait it out.
    pub async fn detach(self) -> crate::Result<()> {
        for task in &self.tasks {
            task.abort();
        }
        self.sync().await?;
        let lease = self.inner.lease.lock().await;
        lease
            .release(&*self.inner.bucket.store, &self.inner.lease_path)
            .await
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

struct Inner {
    bucket: Bucket,
    lease_path: ObjectPath,
    lease: Mutex<Lease>,
    sync: Mutex<Copier>,
    lost: watch::Sender<Option<String>>,
}

impl Inner {
    fn lose(&self, reason: String) {
        eprintln!("serve: lost the store: {reason}");
        self.lost.send_if_modified(|lost| {
            if lost.is_none() {
                *lost = Some(reason);
                true
            } else {
                false
            }
        });
    }

    async fn sync_once(&self) -> crate::Result<()> {
        let (fence, valid_until) = {
            let lease = self.lease.lock().await;
            (lease.fence, lease.valid_until)
        };
        if tokio::time::Instant::now() >= valid_until {
            bail!("the lease expired before it could be renewed");
        }
        let mut sync = self.sync.lock().await;
        match sync.step(fence).await {
            Err(err) if is_fenced(&err) => {
                self.lose(err.to_string());
                Err(err)
            }
            other => other,
        }
    }
}

async fn renew_loop(inner: Arc<Inner>) {
    let ttl = inner.bucket.lease_ttl;
    loop {
        tokio::time::sleep(ttl / 3).await;
        let mut lease = inner.lease.lock().await;
        match lease
            .renew(&*inner.bucket.store, &inner.lease_path, ttl)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                inner.lose("another daemon took the lease".into());
                return;
            }
            Err(err) if tokio::time::Instant::now() >= lease.valid_until => {
                inner.lose(format!("the lease could not be renewed: {err}"));
                return;
            }
            Err(err) => eprintln!("serve: renewing the store lease failed, retrying: {err}"),
        }
    }
}

async fn sync_loop(inner: Arc<Inner>) {
    loop {
        tokio::time::sleep(inner.bucket.sync_interval).await;
        if inner.lost.borrow().is_some() {
            return;
        }
        if let Err(err) = inner.sync_once().await {
            if inner.lost.borrow().is_some() {
                return;
            }
            eprintln!("serve: copying to the store failed, retrying: {err}");
        }
    }
}

/// The manifest's conditional write failed: another daemon wrote it.
#[derive(Debug)]
struct Fenced;

impl std::fmt::Display for Fenced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another daemon wrote the store's manifest")
    }
}

impl std::error::Error for Fenced {}

fn is_fenced(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Fenced>().is_some()
}

const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    /// The lease fence of the daemon that last wrote it.
    fence: u64,
    /// Databases, by path relative to the data directory.
    files: BTreeMap<String, FileEntry>,
    /// Every other file, by path relative to the data directory.
    #[serde(default)]
    blobs: BTreeMap<String, BlobEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BlobEntry {
    sha256: String,
    bytes: u64,
}

fn blob_key(sha256: &str) -> String {
    format!("blobs/{sha256}")
}

/// A plain file as last copied.
struct Seen {
    stat: Option<(u64, std::time::SystemTime)>,
}

fn stat(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileEntry {
    snapshot: String,
    snapshot_bytes: u64,
    segments: Vec<String>,
    segment_bytes: u64,
    /// The next object number, for unique names.
    next: u64,
}

struct Tracked {
    pages: Pages,
    /// The file as last copied; `None` forces a copy.
    fingerprint: Option<Fingerprint>,
}

/// The copy loop's state.
struct Copier {
    bucket: Bucket,
    data_dir: PathBuf,
    manifest: Manifest,
    version: Option<UpdateVersion>,
    files: BTreeMap<String, Tracked>,
    blobs: BTreeMap<String, Seen>,
}

impl Copier {
    async fn write_manifest(&mut self, fence: u64) -> crate::Result<()> {
        self.manifest.fence = fence;
        let mode = match &self.version {
            Some(version) => PutMode::Update(version.clone()),
            None => PutMode::Create,
        };
        let put = self
            .bucket
            .store
            .put_opts(
                &self.bucket.path("manifest.json"),
                serde_json::to_vec_pretty(&self.manifest)?.into(),
                PutOptions {
                    mode,
                    ..PutOptions::default()
                },
            )
            .await;
        match put {
            Ok(put) => {
                self.version = Some(version_of(put.e_tag, put.version));
                Ok(())
            }
            Err(err) if lost_race(&err) => Err(Fenced.into()),
            Err(err) => Err(err.into()),
        }
    }

    /// Copy every database that changed, then write the manifest once.
    async fn step(&mut self, fence: u64) -> crate::Result<()> {
        let mut retired = Vec::new();
        let (databases, plain) = scan(&self.data_dir)?;
        let mut changed = self.copy_plain(&plain, &mut retired).await?;
        for name in databases {
            let path = self.data_dir.join(&name);
            let fingerprint = pages::fingerprint(&path);
            if self
                .files
                .get(&name)
                .is_some_and(|tracked| tracked.fingerprint == Some(fingerprint))
            {
                continue;
            }
            let mirror = self.data_dir.join(MIRROR_DIR).join(&name);
            let bytes = tokio::task::spawn_blocking(move || pages::copy(&path, &mirror)).await??;
            let tracked = self.files.entry(name.clone()).or_insert_with(|| Tracked {
                pages: Pages::default(),
                fingerprint: None,
            });
            let changes = match self.manifest.files.get(&name) {
                Some(_) => tracked.pages.changes(&bytes)?,
                None => None,
            };
            let entry = self.manifest.files.get(&name).cloned();
            let next = entry.as_ref().map_or(0, |entry| entry.next);
            match (entry, changes) {
                (Some(_), Some(changes)) if changes.changed.is_empty() => {
                    tracked.pages.accept(changes);
                }
                (Some(mut entry), Some(changes))
                    if entry.segments.len() < MAX_SEGMENTS
                        && entry.segment_bytes
                            + u64::from(changes.page_size) * changes.changed.len() as u64
                            <= entry.snapshot_bytes =>
                {
                    let key = format!("files/{name}/{fence:010}-{next:010}.seg");
                    let segment = pages::encode_segment(&bytes, &changes);
                    entry.segment_bytes += segment.len() as u64;
                    self.bucket.put_new(&key, segment).await?;
                    entry.segments.push(key);
                    entry.next = next + 1;
                    self.manifest.files.insert(name.clone(), entry);
                    tracked.pages.accept(changes);
                    changed = true;
                }
                (old, _) => {
                    let key = format!("files/{name}/{fence:010}-{next:010}.snap");
                    let size = bytes.len() as u64;
                    tracked.pages = Pages::of(&bytes)?;
                    self.bucket.put_new(&key, bytes).await?;
                    if let Some(old) = old {
                        retired.push(old.snapshot);
                        retired.extend(old.segments);
                    }
                    self.manifest.files.insert(
                        name.clone(),
                        FileEntry {
                            snapshot: key,
                            snapshot_bytes: size,
                            segments: Vec::new(),
                            segment_bytes: 0,
                            next: next + 1,
                        },
                    );
                    changed = true;
                }
            }
            tracked.fingerprint = Some(fingerprint);
        }
        let gone: Vec<String> = self
            .manifest
            .files
            .keys()
            .filter(|name| !self.data_dir.join(name).exists())
            .cloned()
            .collect();
        for name in gone {
            if let Some(old) = self.manifest.files.remove(&name) {
                retired.push(old.snapshot);
                retired.extend(old.segments);
            }
            self.files.remove(&name);
            changed = true;
        }
        if changed {
            self.write_manifest(fence).await?;
            for key in retired {
                // Best effort: an object left behind costs storage, not data.
                let _ = self.bucket.store.delete(&self.bucket.object(&key)).await;
            }
        }
        Ok(())
    }
}

impl Copier {
    /// Copy the plain files that changed; `true` when the manifest did.
    /// Blobs no file names any more go to `retired`.
    async fn copy_plain(
        &mut self,
        plain: &[String],
        retired: &mut Vec<String>,
    ) -> crate::Result<bool> {
        let mut changed = false;
        let mut dropped = Vec::new();
        for name in plain {
            let path = self.data_dir.join(name);
            let now = stat(&path);
            if self.blobs.get(name).is_some_and(|seen| seen.stat == now) {
                continue;
            }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };
            let sha256 = hex::encode(Sha256::digest(&bytes));
            if self.manifest.blobs.get(name).map(|blob| &blob.sha256) != Some(&sha256) {
                let size = bytes.len() as u64;
                match self.bucket.put_new(&blob_key(&sha256), bytes).await {
                    Ok(()) => {}
                    Err(err)
                        if err
                            .downcast_ref::<object_store::Error>()
                            .is_some_and(|err| {
                                matches!(err, object_store::Error::AlreadyExists { .. })
                            }) => {}
                    Err(err) => return Err(err),
                }
                if let Some(old) = self.manifest.blobs.insert(
                    name.clone(),
                    BlobEntry {
                        sha256: sha256.clone(),
                        bytes: size,
                    },
                ) {
                    dropped.push(old.sha256);
                }
                changed = true;
            }
            self.blobs.insert(name.clone(), Seen { stat: now });
        }
        let present: std::collections::BTreeSet<&String> = plain.iter().collect();
        let gone: Vec<String> = self
            .manifest
            .blobs
            .keys()
            .filter(|name| !present.contains(name))
            .cloned()
            .collect();
        for name in gone {
            if let Some(old) = self.manifest.blobs.remove(&name) {
                dropped.push(old.sha256);
            }
            self.blobs.remove(&name);
            changed = true;
        }
        // A blob another file still names stays.
        let named: std::collections::BTreeSet<&String> = self
            .manifest
            .blobs
            .values()
            .map(|blob| &blob.sha256)
            .collect();
        dropped.sort();
        dropped.dedup();
        retired.extend(
            dropped
                .iter()
                .filter(|sha| !named.contains(sha))
                .map(|sha| blob_key(sha)),
        );
        Ok(changed)
    }
}

/// Files under `dir` as `/`-separated relative paths: SQLite databases, and
/// every other regular file. Skips the copy loop's mirror and SQLite's
/// sidecar files.
fn scan(dir: &Path) -> crate::Result<(Vec<String>, Vec<String>)> {
    let mut databases = Vec::new();
    let mut plain = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = match std::fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                if path != dir.join(MIRROR_DIR) {
                    pending.push(path);
                }
            } else if kind.is_file() {
                let relative = path.strip_prefix(dir).unwrap_or(&path);
                let name = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                if ["-wal", "-shm", "-journal"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
                {
                    continue;
                }
                if path.extension().is_some_and(|ext| ext == "db") {
                    databases.push(name);
                } else {
                    plain.push(name);
                }
            }
        }
    }
    databases.sort();
    plain.sort();
    Ok((databases, plain))
}

fn remove_if_present(path: &Path) -> crate::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests;
