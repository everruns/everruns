# Object Storage Specification

## Abstract

Everruns stores workspace file content and image artifacts as PostgreSQL
`BYTEA` by default. This spec defines an **optional, S3-compatible blob
backend** that offloads the *bytes* of those payloads to an object store while
keeping all metadata, structure, and policy logic in PostgreSQL.

The backend is opt-in (`STORAGE_BLOB_BACKEND=s3`), preserves the existing
performance and multitenancy guarantees, and keeps Everruns as the **proxy** for
all reads and writes: clients and workers never talk to the object store
directly.

## Status

Implemented as an additive backend. Default deployments (`STORAGE_BLOB_BACKEND`
unset or `db`) are unchanged: content stays inline in PostgreSQL and the sidecar
tables introduced here are never written.

## Motivation

PostgreSQL `BYTEA` is simple and transactional but couples blob growth to the
primary datastore: large workspace files and images inflate row storage, WAL,
backups, and the connection-bound transfer path. The workspace spec
(`specs/workspace.md`, Decision 1) explicitly deferred object storage as
follow-up "for large files." This spec is that follow-up.

Goals:

- **Same performance goals.** Keep PostgreSQL lean; stream large blobs from the
  object store instead of through row storage. Metadata operations (listing,
  stat, quota, tree) stay single-query against indexed columns.
- **Multitenant isolation.** Tenant scope is encoded in every object key and
  enforced by the existing org/workspace authorization layer.
- **Proxy, not redirect.** Everruns fetches bytes and serves them. No S3
  presigned URLs are handed to browsers or workers; the existing
  `/v1/images/*` proxy and worker gRPC/internal-presign seams are untouched.
- **Optional and reversible.** A deployment can run on PostgreSQL only, or
  enable S3 without reshaping the data model.

## Design

### Seam: blob-content offload (metadata stays in PostgreSQL)

We considered implementing `SessionFileSystem` directly on S3 (keys as paths)
and rejected it: S3 has no efficient directory listing, no server-side grep, no
atomic compare-and-set, and no byte-quota accounting. The workspace filesystem
relies on all four. Instead we keep the proven PostgreSQL metadata layer and
offload only the opaque content bytes.

```
SessionFileSystem / image endpoints  (unchanged contracts)
        │
WorkspaceFileService / image repo     (tree, quotas, grep orchestration — unchanged)
        │
PostgreSQL metadata  ──────────────┐  workspace_files / images rows (path, size, flags, hash)
        │                          │
   BlobStore (optional)  ──────────┘  bytes only, keyed per row, when STORAGE_BLOB_BACKEND=s3
```

### `BlobStore`

A narrow byte-blob contract (`crates/server/src/storage/blob_store.rs`):
`put(key, bytes)`, `get(key) -> Option<bytes>`, `delete(key)`. It is implemented
over [`object_store`](https://docs.rs/object_store), so the same code path
serves AWS S3, MinIO, Cloudflare R2, and any S3-compatible endpoint, plus an
in-memory backend used in tests to exercise the production offload path without
a network dependency.

### Where bytes live

| Payload | Metadata (always PostgreSQL) | Bytes when `db` | Bytes when `s3` |
|---|---|---|---|
| Workspace file | `workspace_files` row (path, size_bytes, flags, timestamps) | `workspace_files.content` | object `workspaces/{workspace_id}/files/{file_id}`; pointer + hash in `workspace_file_blobs` |
| Image | `images` row (filename, content_type, size, metadata) | `images.data` / `images.thumbnail_data` | objects `images/org-{org_id}/{image_id}/{data,thumb}`; pointers in `image_blobs` |

Sidecar tables (`workspace_file_blobs`, `image_blobs`) are keyed by the owning
row id with `ON DELETE CASCADE`. A row is offloaded **iff** a sidecar row
exists; the repository fills `content`/`data` from the blob store on read. This
keeps the migration purely additive and leaves every existing query mapping
untouched.

### Preserved logic

- **Quotas.** Enforced on `size_bytes` in `workspace_files`, written at offload
  time. No content read required (`specs/workspace.md`, Decision 6).
- **Compare-and-set.** `edit_file` stale-write rejection compares the SHA-256
  recorded in `workspace_file_blobs.content_sha256` instead of round-tripping
  bytes. The hash is updated atomically with the row under the same lock.
- **grep.** When offloaded, the candidate query drops the in-database
  `convert_from(content,'UTF8') ~ pattern` predicate (content is empty in the
  row) and selects on `size_bytes`/path only; the existing Rust-side line scan
  fetches each candidate from the blob store and matches it. The TM-DOS-008
  per-file (512 KiB) and total-scan (5 MiB) caps still bound the work.
- **move/copy/delete.** Move keeps the same object (only the path changes).
  Copy duplicates the blob under the new file id. Delete removes the object,
  then the row (cascade clears the sidecar).

### Multitenancy

Object keys embed the tenant partition (`workspace_id` for files, `org_id` for
images), and the same ids are duplicated in the object's recovery metadata (see
below). Authorization is unchanged and enforced above the storage layer
(`get_workspace(org_id, …) → 404`, image org scoping). The blob backend never
mixes keys across callers, and a per-deployment `STORAGE_S3_PREFIX` allows one
bucket to host multiple isolated Everruns deployments.

### Proxy / worker path

Workers have no object-store credentials. They read and write workspace files
through the control plane (gRPC), which performs the offload transparently.
Images keep their existing flow: `GET /v1/images/{id}` streams bytes the server
fetched from PostgreSQL or the blob store, and workers fetch via the
HMAC-signed `/internal/images/{id}` endpoint — both proxy through Everruns, so
no S3 URL ever leaves the control plane.

## Disaster-recovery metadata

Every stored object carries **object user-metadata** so the bucket is
self-describing. This is pure redundancy — it is **never read on the hot path**;
runtime reads/writes ignore it. Its only purpose is partial recovery if the
PostgreSQL metadata store is lost or corrupted: a tool can walk the bucket and
rebuild the `workspace_files` / `images` rows from the objects alone.

Metadata is set through object_store's portable `Attribute::Metadata`, so the
keys are backend-neutral (`everruns-kind`, `everruns-recovery`). On any
S3-protocol backend — AWS S3 and every S3-compatible store (SeaweedFS, MinIO,
R2, ...) — these surface on the wire as `x-amz-meta-<key>`; a native GCS/Azure
backend would map them to that provider's convention. Nothing is AWS-specific.

Each object sets:

- `Content-Type` — the object's media type (when known).
- `everruns-kind` — `workspace_file` | `image` | `image_thumbnail`.
- `everruns-recovery` — base64(JSON) of the owning-row fields. base64 keeps the
  value header-safe regardless of unicode in paths/filenames.

### Exposure

This metadata is only visible to callers with bucket credentials (the control
plane). Everruns never presigns object URLs or forwards object response headers
to clients — `get()` returns raw bytes only — so these values never reach end
users. Anyone who can read the metadata can already read the object bytes and
the key (which encodes the tenant ids), so it adds no exposure beyond what
already exists. `everruns-recovery` carries paths/filenames/hashes at the same
sensitivity as the data; deployments that treat those as sensitive should keep
the bucket private (required) and enable bucket-side encryption (SSE/KMS).

Recovery record (JSON, `v:1`):

| kind | fields |
|------|--------|
| `workspace_file` | `workspace_id`, `file_id`, `path`, `size_bytes`, `content_sha256` |
| `image` | `org_id`, `image_id`, `filename`, `content_type`, `size_bytes` |
| `image_thumbnail` | `org_id`, `image_id`, `content_type` |

Combined with the key itself (which encodes tenant + id), this is enough to
reconstruct the row and re-link the blob. The record is forward-versioned
(`v`) so the format can evolve.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `STORAGE_BLOB_BACKEND` | `db` | `db` keeps bytes inline (current behavior); `s3` offloads to object storage. |
| `STORAGE_S3_BUCKET` | — | Required for `s3`. Target bucket. |
| `STORAGE_S3_REGION` | — | Bucket region (or compatible region label). |
| `STORAGE_S3_ENDPOINT` | — | Custom endpoint for S3-compatible stores (MinIO, R2). Unset for AWS. |
| `STORAGE_S3_ACCESS_KEY_ID` | — | Static access key. Omit to use the AWS credential chain (IAM role/instance). |
| `STORAGE_S3_SECRET_ACCESS_KEY` | — | Static secret key. |
| `STORAGE_S3_PREFIX` | (empty) | Key prefix isolating deployments within a bucket. |
| `STORAGE_S3_ALLOW_HTTP` | `false` | Allow plaintext HTTP (local MinIO only). |
| `STORAGE_S3_FORCE_PATH_STYLE` | `true` | Path-style requests (required by MinIO; harmless on AWS). |

Credentials are read once at startup. The backend is selected per process, so a
deployment runs entirely on `db` or `s3`.

## Local development

SeaweedFS provides an S3-compatible endpoint for local testing. `just seaweedfs`
runs the `weed` binary as a native process (no Docker), via
`scripts/lib/seaweedfs.sh`, mirroring how PostgreSQL/Valkey/NATS run locally.
SeaweedFS speaks the S3 API, so the exact same client code path as AWS S3 is
exercised. Point the server at it with:

```bash
export STORAGE_BLOB_BACKEND=s3
export STORAGE_S3_BUCKET=everruns-dev
export STORAGE_S3_ENDPOINT=http://127.0.0.1:8333
export STORAGE_S3_REGION=us-east-1
export STORAGE_S3_ACCESS_KEY_ID=everruns
export STORAGE_S3_SECRET_ACCESS_KEY=everruns-secret
export STORAGE_S3_ALLOW_HTTP=true
```

Any S3-compatible store (AWS S3, SeaweedFS, MinIO, Cloudflare R2, ...) works
unchanged — only the endpoint and credentials differ.

## Testing

- `BlobStore` round-trip, idempotent delete, prefix isolation, key derivation,
  and content hashing are unit-tested against object_store's in-memory backend
  (the production code path), with no network dependency.
- End-to-end offload (PostgreSQL + SeaweedFS) is validated manually; the PostgreSQL
  repository paths are not unit-tested without a database, consistent with the
  rest of the storage layer.

## Non-goals / follow-ups

- **Blob garbage collection.** Interrupted deletes (row gone, object left) leak
  objects. A periodic GC that reconciles sidecar pointers against bucket
  contents is follow-up work; orphans are harmless besides storage cost.
- **Streaming I/O.** The current contract buffers whole blobs in memory, matching
  the existing inline path and per-file size caps. Range/streaming reads are a
  later enhancement.
- **Migration of existing data.** Enabling `s3` offloads newly written content;
  a backfill job to move pre-existing inline content is follow-up work. Reads
  transparently serve inline or offloaded content regardless.
- **Per-backend encryption.** Object-store server-side encryption is configured
  on the bucket; envelope encryption of blobs is out of scope here.

## Source Index

- `crates/server/src/storage/blob_store.rs` — `BlobStore`, `ObjectStoreBlobStore`,
  config, key derivation, content hashing.
- `crates/server/migrations/071_object_storage_blobs.sql` — sidecar tables.
- `crates/server/src/storage/repositories/session_files.rs` — file offload.
- `crates/server/src/storage/repositories/skills.rs` — image offload.
- `specs/workspace.md` — workspace filesystem model and quotas.
- `specs/file-store.md` — `SessionFileSystem` seam.
