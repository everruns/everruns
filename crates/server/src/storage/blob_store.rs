// Pluggable opaque-blob backend (specs/object-storage.md).
//
// The default deployment stores file/image bytes inline in PostgreSQL. When an
// S3-compatible object store is configured, the *bytes* move to the object
// store while all metadata (file tree, sizes, quotas, content hashes) stays in
// PostgreSQL. The control plane is always the proxy: clients and workers never
// talk to the object store directly, so org isolation and the existing
// presigned-URL/gRPC seams are unchanged.
//
// The blob backend is intentionally a narrow key/value byte interface so the
// same seam serves both the workspace filesystem and image artifacts.

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use everruns_config::{env_bool, env_opt_string, env_string};
use futures::StreamExt;
use object_store::{
    Attribute, Attributes, ObjectStore, ObjectStoreExt, PutOptions, PutPayload,
    path::Path as ObjectPath,
};
use std::borrow::Cow;
use std::sync::Arc;
use uuid::Uuid;

/// Narrow byte-blob contract addressed by an opaque, slash-delimited key.
///
/// Keys are relative; implementations apply their own deployment-level prefix
/// (e.g. an S3 key prefix) on top. Implementations must be multi-tenant safe by
/// construction — callers embed the tenant scope (`workspace_id`, `org_id`,
/// resource id) in the key, and the store never mixes keys across callers.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Human-readable backend name for diagnostics/logging.
    fn backend_name(&self) -> &'static str;

    /// Store `bytes` at `key`, overwriting any existing object. `metadata` is
    /// attached as object user-metadata for disaster recovery (not read on the
    /// hot path).
    async fn put(&self, key: &str, bytes: Vec<u8>, metadata: &BlobMetadata) -> Result<()>;

    /// Fetch the bytes stored at `key`. Returns `Ok(None)` when absent.
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Delete the object at `key`. A missing object is not an error (idempotent).
    async fn delete(&self, key: &str) -> Result<()>;

    /// List objects whose key starts with `prefix` (a relative key prefix, as
    /// passed to [`put`](BlobStore::put) / [`delete`](BlobStore::delete)).
    ///
    /// Returns objects with their *relative* keys (the deployment-level prefix
    /// applied by the implementation is stripped back off), so the result is
    /// directly comparable to the keys callers store in the sidecar pointer
    /// tables. Used by the blob garbage collector to reconcile bucket contents
    /// against PostgreSQL pointers (`specs/object-storage.md`).
    ///
    /// Backends with no external objects (none today — only the object_store
    /// backend implements `BlobStore`) may return an empty list.
    ///
    /// `limit` bounds how many objects are materialized in one call so a very
    /// large bucket cannot drive unbounded memory in the GC; `None` means no
    /// bound. The GC passes a configurable cap and reconciles in
    /// lexicographic-key windows across sweeps.
    async fn list_with_prefix(&self, prefix: &str, limit: Option<usize>)
    -> Result<Vec<BlobObject>>;
}

/// A single object surfaced by [`BlobStore::list_with_prefix`].
#[derive(Debug, Clone)]
pub struct BlobObject {
    /// Relative object key (deployment prefix stripped), comparable to the
    /// sidecar `blob_key` / `data_key` / `thumbnail_key` columns.
    pub key: String,
    /// Server-reported last-modified time. The GC grace period is measured
    /// against this so recently-written (possibly in-flight) objects are never
    /// deleted.
    pub last_modified: DateTime<Utc>,
    /// Object size in bytes, used to report bytes reclaimed.
    pub size_bytes: u64,
}

/// Shared, cheaply-cloneable handle to the active blob backend.
pub type SharedBlobStore = Arc<dyn BlobStore>;

/// User-metadata key carrying the object kind.
const META_KIND_KEY: &str = "everruns-kind";
/// User-metadata key carrying the base64(JSON) recovery record.
const META_RECOVERY_KEY: &str = "everruns-recovery";

/// Disaster-recovery metadata attached to every stored object as object
/// user-metadata.
///
/// Set via object_store's portable `Attribute::Metadata` — the key is just
/// `everruns-kind` / `everruns-recovery`. On an S3-protocol backend (AWS S3 and
/// every S3-compatible store: SeaweedFS, R2, ...) this surfaces on the
/// wire as `x-amz-meta-<key>`; a native GCS/Azure backend would map the same
/// attribute to its own convention. Nothing here is AWS-specific.
///
/// This is **never read on the hot path** — runtime reads/writes ignore it. It
/// exists purely for redundancy: the bucket becomes self-describing, so a
/// recovery tool can walk the objects and rebuild the PostgreSQL rows (paths,
/// owners, sizes, hashes) after a metadata-store loss. It is only visible to
/// holders of the bucket credentials (the control plane), who can already read
/// the bytes and the key, so it adds no exposure beyond what already exists.
/// See `specs/object-storage.md`.
#[derive(Debug, Clone)]
pub struct BlobMetadata {
    /// Object kind: `workspace_file` | `image` | `image_thumbnail`.
    pub kind: &'static str,
    /// Object content type, when known. Also set as the object's `Content-Type`.
    pub content_type: Option<String>,
    /// Full recovery record (JSON) — the owning-row fields needed to
    /// reconstruct the database row from the object alone.
    pub recovery: serde_json::Value,
}

impl BlobMetadata {
    pub fn new(kind: &'static str, recovery: serde_json::Value) -> Self {
        Self {
            kind,
            content_type: None,
            recovery,
        }
    }

    pub fn with_content_type(mut self, content_type: Option<String>) -> Self {
        self.content_type = content_type;
        self
    }

    /// Render into object_store attributes (Content-Type + DR user metadata).
    fn to_attributes(&self) -> Attributes {
        let mut attrs = Attributes::new();
        if let Some(ct) = &self.content_type {
            attrs.insert(Attribute::ContentType, ct.clone().into());
        }
        attrs.insert(
            Attribute::Metadata(Cow::Borrowed(META_KIND_KEY)),
            self.kind.into(),
        );
        // base64 keeps the value header-safe regardless of unicode in paths/filenames.
        let recovery = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&self.recovery).unwrap_or_default());
        attrs.insert(
            Attribute::Metadata(Cow::Borrowed(META_RECOVERY_KEY)),
            recovery.into(),
        );
        attrs
    }
}

// ============================================================================
// Tenant-scoped key derivation
// ============================================================================

/// Object key for a workspace file's content blob.
///
/// `workspace_id` is the per-tenant partition (org ownership is enforced above
/// this layer), `file_id` keys the logical file, and `content_sha256` makes each
/// revision immutable. This prevents object-store overwrites from racing ahead
/// of the PostgreSQL sidecar pointer that selects the current revision.
pub fn workspace_file_key(workspace_id: Uuid, file_id: Uuid, content_sha256: &str) -> String {
    format!("workspaces/{workspace_id}/files/{file_id}/sha256/{content_sha256}")
}

/// Object key for an image's primary data blob.
pub fn image_data_key(org_id: i64, image_id: Uuid) -> String {
    format!("images/org-{org_id}/{image_id}/data")
}

/// Object key for an image's thumbnail blob.
pub fn image_thumbnail_key(org_id: i64, image_id: Uuid) -> String {
    format!("images/org-{org_id}/{image_id}/thumb")
}

/// Hex-encoded SHA-256 of `bytes`, used as the compare-and-set token for
/// offloaded file content (the bytes themselves live in the blob store, so CAS
/// compares hashes recorded in PostgreSQL).
pub fn content_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

// ============================================================================
// object_store-backed implementation (S3-compatible + in-memory)
// ============================================================================

/// A [`BlobStore`] backed by any [`object_store::ObjectStore`].
///
/// Production uses the AWS S3 (S3-compatible, incl. SeaweedFS/R2/etc.) backend;
/// tests use object_store's in-memory backend so the exact production code path
/// (prefixing, not-found handling) is exercised without a network dependency.
pub struct ObjectStoreBlobStore {
    inner: Arc<dyn ObjectStore>,
    prefix: String,
    backend_name: &'static str,
}

impl ObjectStoreBlobStore {
    /// Wrap an arbitrary object store with an optional key prefix.
    pub fn new(inner: Arc<dyn ObjectStore>, prefix: &str, backend_name: &'static str) -> Self {
        Self {
            inner,
            prefix: prefix.trim_matches('/').to_string(),
            backend_name,
        }
    }

    /// In-memory backend, for dev/test exercising of the offload code path.
    pub fn in_memory() -> Self {
        Self::new(
            Arc::new(object_store::memory::InMemory::new()),
            "",
            "memory",
        )
    }

    /// Build an S3-compatible backend from configuration.
    pub fn s3_from_config(config: &BlobStoreConfig) -> Result<Self> {
        use object_store::aws::AmazonS3Builder;

        // Start from the environment so AWS deployments can use IAM-role /
        // instance credentials, then override with explicit STORAGE_S3_* values
        // for S3-compatible endpoints (SeaweedFS, R2, ...) that pass static creds.
        let mut builder = AmazonS3Builder::from_env().with_bucket_name(&config.bucket);
        if let Some(region) = &config.region {
            builder = builder.with_region(region);
        }
        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        if let Some(key) = &config.access_key_id {
            builder = builder.with_access_key_id(key);
        }
        if let Some(secret) = &config.secret_access_key {
            builder = builder.with_secret_access_key(secret);
        }
        if config.allow_http {
            builder = builder.with_allow_http(true);
        }
        // SeaweedFS and most non-AWS S3 implementations require path-style requests.
        if config.force_path_style {
            builder = builder.with_virtual_hosted_style_request(false);
        }

        let store = builder
            .build()
            .context("failed to build S3 object store from STORAGE_S3_* configuration")?;
        Ok(Self::new(Arc::new(store), &config.prefix, "s3"))
    }

    fn object_path(&self, key: &str) -> ObjectPath {
        let key = key.trim_start_matches('/');
        let full = if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{}", self.prefix, key)
        };
        ObjectPath::from(full)
    }

    /// Strip the deployment-level prefix from a full object key so it matches
    /// the relative keys stored in the sidecar pointer tables. Returns `None`
    /// for objects that fall outside this deployment's prefix (defensive: a
    /// shared bucket may host other deployments — never GC those).
    fn strip_prefix<'a>(&self, full: &'a str) -> Option<&'a str> {
        if self.prefix.is_empty() {
            return Some(full);
        }
        full.strip_prefix(&self.prefix)
            .map(|rest| rest.trim_start_matches('/'))
    }
}

#[async_trait]
impl BlobStore for ObjectStoreBlobStore {
    fn backend_name(&self) -> &'static str {
        self.backend_name
    }

    async fn put(&self, key: &str, bytes: Vec<u8>, metadata: &BlobMetadata) -> Result<()> {
        let path = self.object_path(key);
        let opts = PutOptions {
            attributes: metadata.to_attributes(),
            ..Default::default()
        };
        self.inner
            .put_opts(&path, PutPayload::from(bytes), opts)
            .await
            .with_context(|| format!("blob put failed for key {key}"))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let path = self.object_path(key);
        match self.inner.get(&path).await {
            Ok(result) => {
                let bytes = result
                    .bytes()
                    .await
                    .with_context(|| format!("blob read failed for key {key}"))?;
                Ok(Some(bytes.to_vec()))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(anyhow::Error::from(e).context(format!("blob get failed for key {key}"))),
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.object_path(key);
        match self.inner.delete(&path).await {
            Ok(()) => Ok(()),
            Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => {
                Err(anyhow::Error::from(e).context(format!("blob delete failed for key {key}")))
            }
        }
    }

    async fn list_with_prefix(
        &self,
        prefix: &str,
        limit: Option<usize>,
    ) -> Result<Vec<BlobObject>> {
        // Build the full (prefixed) listing path. An empty relative prefix
        // lists the whole deployment scope.
        let list_path = self.object_path(prefix);
        let mut stream = self.inner.list(Some(&list_path));
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            let meta = item.with_context(|| format!("blob list failed for prefix {prefix:?}"))?;
            let full = meta.location.as_ref();
            // Fail closed on objects outside our prefix (foreign deployments
            // sharing the bucket): skip rather than risk deleting them.
            let Some(rel) = self.strip_prefix(full) else {
                continue;
            };
            if !rel.starts_with(prefix) {
                continue;
            }
            out.push(BlobObject {
                key: rel.to_string(),
                last_modified: meta.last_modified,
                size_bytes: meta.size,
            });
            // Stop once the caller's bound is reached so memory stays bounded
            // regardless of bucket size.
            if let Some(max) = limit
                && out.len() >= max
            {
                break;
            }
        }
        Ok(out)
    }
}

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the S3-compatible blob backend, read from `STORAGE_S3_*`.
#[derive(Debug, Clone)]
pub struct BlobStoreConfig {
    pub bucket: String,
    pub region: Option<String>,
    pub endpoint: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub prefix: String,
    pub allow_http: bool,
    pub force_path_style: bool,
}

impl BlobStoreConfig {
    /// Load S3 configuration from `STORAGE_S3_*` environment variables.
    pub fn from_env() -> Result<Self> {
        let bucket = env_opt_string("STORAGE_S3_BUCKET")
            .context("STORAGE_BLOB_BACKEND=s3 requires STORAGE_S3_BUCKET to be set")?;
        Ok(Self {
            bucket,
            region: env_opt_string("STORAGE_S3_REGION"),
            endpoint: env_opt_string("STORAGE_S3_ENDPOINT"),
            access_key_id: env_opt_string("STORAGE_S3_ACCESS_KEY_ID"),
            secret_access_key: env_opt_string("STORAGE_S3_SECRET_ACCESS_KEY"),
            prefix: env_string("STORAGE_S3_PREFIX", ""),
            allow_http: env_bool("STORAGE_S3_ALLOW_HTTP", false),
            // Default to path-style: it is the safe choice for S3-compatible
            // endpoints (SeaweedFS) and still works against AWS S3.
            force_path_style: env_bool("STORAGE_S3_FORCE_PATH_STYLE", true),
        })
    }
}

/// Resolve the configured blob backend.
///
/// - `STORAGE_BLOB_BACKEND=db` (default) → `Ok(None)`: bytes stay inline in
///   PostgreSQL (current behavior).
/// - `STORAGE_BLOB_BACKEND=s3` → `Ok(Some(..))`: bytes offload to S3.
pub fn blob_store_from_env() -> Result<Option<SharedBlobStore>> {
    let backend = env_string("STORAGE_BLOB_BACKEND", "db");
    match backend.as_str() {
        "db" | "" => Ok(None),
        "s3" => {
            let config = BlobStoreConfig::from_env()?;
            let store = ObjectStoreBlobStore::s3_from_config(&config)?;
            tracing::info!(
                bucket = %config.bucket,
                prefix = %config.prefix,
                endpoint = ?config.endpoint,
                "Object-storage blob backend enabled (S3-compatible)"
            );
            Ok(Some(Arc::new(store)))
        }
        other => {
            anyhow::bail!("unknown STORAGE_BLOB_BACKEND={other:?} (expected \"db\" or \"s3\")")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> BlobMetadata {
        BlobMetadata::new("test", serde_json::json!({ "v": 1 }))
    }

    #[tokio::test]
    async fn in_memory_round_trips_bytes() {
        let store = ObjectStoreBlobStore::in_memory();
        store
            .put("workspaces/a/files/b", b"hello".to_vec(), &meta())
            .await
            .unwrap();
        let got = store.get("workspaces/a/files/b").await.unwrap();
        assert_eq!(got.as_deref(), Some(b"hello".as_slice()));
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let store = ObjectStoreBlobStore::in_memory();
        assert!(store.get("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let store = ObjectStoreBlobStore::in_memory();
        store.put("k", b"v".to_vec(), &meta()).await.unwrap();
        store.delete("k").await.unwrap();
        assert!(store.get("k").await.unwrap().is_none());
        // Deleting an absent key must not error.
        store.delete("k").await.unwrap();
    }

    #[tokio::test]
    async fn put_overwrites() {
        let store = ObjectStoreBlobStore::in_memory();
        store.put("k", b"one".to_vec(), &meta()).await.unwrap();
        store.put("k", b"two".to_vec(), &meta()).await.unwrap();
        assert_eq!(
            store.get("k").await.unwrap().as_deref(),
            Some(b"two".as_slice())
        );
    }

    #[tokio::test]
    async fn list_with_prefix_returns_relative_keys() {
        let store = ObjectStoreBlobStore::in_memory();
        store
            .put("workspaces/a/files/1", b"x".to_vec(), &meta())
            .await
            .unwrap();
        store
            .put("workspaces/a/files/2", b"yy".to_vec(), &meta())
            .await
            .unwrap();
        store
            .put("images/org-1/i/data", b"zzz".to_vec(), &meta())
            .await
            .unwrap();

        let mut ws = store.list_with_prefix("workspaces/", None).await.unwrap();
        ws.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(
            ws.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
            vec!["workspaces/a/files/1", "workspaces/a/files/2"]
        );
        assert_eq!(ws[0].size_bytes, 1);
        assert_eq!(ws[1].size_bytes, 2);

        // Empty prefix lists everything in scope.
        let all = store.list_with_prefix("", None).await.unwrap();
        assert_eq!(all.len(), 3);

        // `limit` bounds the number of objects materialized.
        let capped = store.list_with_prefix("", Some(2)).await.unwrap();
        assert_eq!(capped.len(), 2);
    }

    #[tokio::test]
    async fn list_with_prefix_excludes_sibling_prefixes() {
        let store = ObjectStoreBlobStore::in_memory();
        store
            .put("workspaces/a/files/1", b"x".to_vec(), &meta())
            .await
            .unwrap();
        store
            .put("workspaces-old/a/files/2", b"yy".to_vec(), &meta())
            .await
            .unwrap();
        store
            .put("images/org-1/i/data", b"zzz".to_vec(), &meta())
            .await
            .unwrap();
        store
            .put("images-old/org-1/i/data", b"wwww".to_vec(), &meta())
            .await
            .unwrap();

        let ws = store.list_with_prefix("workspaces/", None).await.unwrap();
        assert_eq!(
            ws.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
            vec!["workspaces/a/files/1"]
        );

        let imgs = store.list_with_prefix("images/", None).await.unwrap();
        assert_eq!(
            imgs.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
            vec!["images/org-1/i/data"]
        );
    }

    #[tokio::test]
    async fn list_with_prefix_strips_deployment_prefix() {
        let inner: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let store = ObjectStoreBlobStore::new(inner.clone(), "deploy-1", "memory");
        let other = ObjectStoreBlobStore::new(inner, "deploy-2", "memory");

        store
            .put("workspaces/a/files/1", b"x".to_vec(), &meta())
            .await
            .unwrap();
        other
            .put("workspaces/a/files/2", b"yy".to_vec(), &meta())
            .await
            .unwrap();

        // Listing under deploy-1 returns relative keys and never sees deploy-2.
        let listed = store.list_with_prefix("workspaces/", None).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, "workspaces/a/files/1");
    }

    #[tokio::test]
    async fn prefix_is_applied_and_isolated() {
        let inner: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let prefixed = ObjectStoreBlobStore::new(inner.clone(), "tenant-1", "memory");
        let other = ObjectStoreBlobStore::new(inner, "tenant-2", "memory");

        prefixed.put("file", b"a".to_vec(), &meta()).await.unwrap();
        // A different prefix must not see the first tenant's object.
        assert!(other.get("file").await.unwrap().is_none());
        assert_eq!(
            prefixed.get("file").await.unwrap().as_deref(),
            Some(b"a".as_slice())
        );
    }

    #[tokio::test]
    async fn recovery_metadata_is_attached_to_object() {
        let inner: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let store = ObjectStoreBlobStore::new(inner.clone(), "", "memory");
        let m = BlobMetadata::new(
            "workspace_file",
            serde_json::json!({ "v": 1, "path": "/notes.txt", "size_bytes": 5 }),
        )
        .with_content_type(Some("text/plain".to_string()));
        store.put("k", b"hello".to_vec(), &m).await.unwrap();

        // The object is self-describing: Content-Type + DR user metadata can be
        // recovered from the object alone (no database).
        let got = inner.get(&ObjectPath::from("k")).await.unwrap();
        let attrs = got.attributes.clone();
        assert_eq!(
            attrs.get(&Attribute::ContentType).map(|v| v.as_ref()),
            Some("text/plain")
        );
        let kind = attrs
            .get(&Attribute::Metadata(Cow::Borrowed(META_KIND_KEY)))
            .map(|v| v.as_ref().to_string())
            .unwrap();
        assert_eq!(kind, "workspace_file");
        let recovery_b64 = attrs
            .get(&Attribute::Metadata(Cow::Borrowed(META_RECOVERY_KEY)))
            .map(|v| v.as_ref().to_string())
            .unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(recovery_b64)
            .unwrap();
        let record: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(record["path"], "/notes.txt");
        assert_eq!(record["size_bytes"], 5);
    }

    #[test]
    fn keys_are_tenant_scoped() {
        let ws = Uuid::nil();
        let file = Uuid::from_u128(1);
        assert_eq!(
            workspace_file_key(ws, file, "abc123"),
            "workspaces/00000000-0000-0000-0000-000000000000/files/00000000-0000-0000-0000-000000000001/sha256/abc123"
        );
        assert_eq!(
            image_data_key(7, file),
            "images/org-7/00000000-0000-0000-0000-000000000001/data"
        );
        assert!(image_thumbnail_key(7, file).ends_with("/thumb"));
    }

    #[test]
    fn content_sha256_matches_known_vector() {
        // SHA-256 of "abc".
        assert_eq!(
            content_sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
