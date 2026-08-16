---
type: Specification
title: "Object Storage Specification"
description: "Optional S3-compatible blob backend for file/image content."
tags:
  - everruns
  - runtime-resources
---
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
(`knowledge/runtime-resources/workspace.md`, Decision 1) explicitly deferred object storage as
follow-up "for large files." This spec is that follow-up.

Goals:

- **Same performance goals.** Keep PostgreSQL lean; stream large blobs from the
  object store instead of through row storage. Metadata operations (listing,
  stat, quota, tree) stay single-query against indexed columns.
- **Multitenant isolation.** Tenant scope is encoded in every object key and
  enforced by the existing org/workspace authorization layer.
- **Proxy, not redirect.** Everruns fetches bytes and serves them. No S3
  presigned URLs are handed to browsers or workers; the existing
  `/v1/images/*` proxy and worker gRPC/internal-presign boundaries are untouched.
- **Optional and reversible.** A deployment can run on PostgreSQL only, or
  enable S3 without reshaping the data model.

## Design

### Boundary: blob-content offload (metadata stays in PostgreSQL)

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
serves AWS S3, SeaweedFS, Cloudflare R2, and any S3-compatible endpoint, plus an
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
  time. No content read required (`knowledge/runtime-resources/workspace.md`, Decision 6).
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
HMAC-signed `/internal/images/{id}` endpoint, both proxy through Everruns, so
no S3 URL ever leaves the control plane.

## Disaster-recovery metadata

Every stored object carries **object user-metadata** so the bucket is
self-describing. This is pure redundancy, it is **never read on the hot path**;
runtime reads/writes ignore it. Its only purpose is partial recovery if the
PostgreSQL metadata store is lost or corrupted: a tool can walk the bucket and
rebuild the `workspace_files` / `images` rows from the objects alone.

Metadata is set through object_store's portable `Attribute::Metadata`, so the
keys are backend-neutral (`everruns-kind`, `everruns-recovery`). On any
S3-protocol backend, AWS S3 and every S3-compatible store (SeaweedFS, R2,
...), these surface on the wire as `x-amz-meta-<key>`; a native GCS/Azure
backend would map them to that provider's convention. Nothing is AWS-specific.

Each object sets:

- `Content-Type`, the object's media type (when known).
- `everruns-kind`, `workspace_file` | `image` | `image_thumbnail`.
- `everruns-recovery`, base64(JSON) of the owning-row fields. base64 keeps the
  value header-safe regardless of unicode in paths/filenames.

### Exposure

This metadata is only visible to callers with bucket credentials (the control
plane). Everruns never presigns object URLs or forwards object response headers
to clients, `get()` returns raw bytes only, so these values never reach end
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

## Garbage collection

Object stores are not transactional with PostgreSQL, so a blob object can
outlive its metadata: an interrupted delete (row gone, object left) or a crash
between `put()` and the best-effort object cleanup leaks an object. Such orphans
are never *served*, reads always go through the sidecar pointer rows, but they
accumulate as storage cost. A periodic GC sweep reconciles bucket contents
against the live pointers and reclaims orphans.

**Sweep.** A background task (`crates/server/src/blob_gc.rs`, spawned from
`app_builder.rs` next to event retention) runs every
`STORAGE_BLOB_GC_INTERVAL_SECONDS` (default 6h). Each pass:

1. Enumerates **all live keys** from `workspace_file_blobs.blob_key` and
   `image_blobs.{data_key, thumbnail_key}` into a set. If this query fails, the
   whole sweep aborts, it never deletes without a reliable picture of what is
   live (fail-closed).
2. Lists the bucket under the two tenant-scoped prefixes (`workspaces/`,
   `images/`) via `BlobStore::list_with_prefix`, which returns *relative* keys
   (deployment prefix stripped) directly comparable to the sidecar columns.
   Objects outside this deployment's prefix are never listed, so a shared bucket
   is safe.
3. Deletes an object **iff** it has no live pointer **and** its server-reported
   last-modified time is at or before `now − grace`
   (`STORAGE_BLOB_GC_GRACE_SECONDS`, default 24h). The grace period is the core
   safety mechanism: a freshly written object may have its row committed
   slightly after the object lands, and the sweep may race an in-flight create,
   so recently-written objects are never touched.
4. Caps deletions at `STORAGE_BLOB_GC_MAX_DELETES_PER_RUN` (default 10000) to
   bound the work a single run performs; remaining orphans are reclaimed on the
   next pass. The per-run cap is consumed per delete *attempt* (not just
   successes), so transient delete failures cannot push a sweep past the cap. A
   delete failure is non-fatal (logged, retried next sweep).
5. Caps the number of objects listed per prefix at
   `STORAGE_BLOB_GC_MAX_LIST_PER_RUN` (default 100000) so the sweep's memory is
   bounded regardless of bucket size. Buckets larger than the cap are reconciled
   across multiple sweeps in lexicographic key-order windows.

**Safety invariants.** An object with a live pointer is never deleted; an object
younger than the grace period is never deleted; any listing or pointer-
enumeration error fails closed (skip deletion, log).

**No-op backends.** GC only runs when the object-storage (`s3`) backend is
configured. The inline (`db`) backend and the in-memory dev backend keep bytes
inline in PostgreSQL and have no external objects, so the task short-circuits.

**Metrics.** `everruns_blob_gc_orphans_deleted_total` (orphans deleted) and
`everruns_blob_gc_bytes_reclaimed_total` (bytes reclaimed) are per-instance
counters; each pass also logs a summary (listed, deleted, bytes, live pointers).

## Configuration

| Variable | Default | Description |
|---|---|---|
| `STORAGE_BLOB_BACKEND` | `db` | `db` keeps bytes inline (current behavior); `s3` offloads to object storage. |
| `STORAGE_S3_BUCKET` |, | Required for `s3`. Target bucket. |
| `STORAGE_S3_REGION` |, | Bucket region (or compatible region label). |
| `STORAGE_S3_ENDPOINT` |, | Custom endpoint for S3-compatible stores (SeaweedFS, R2). Unset for AWS. |
| `STORAGE_S3_ACCESS_KEY_ID` |, | Static access key. Omit to use the AWS credential chain (IAM role/instance). |
| `STORAGE_S3_SECRET_ACCESS_KEY` |, | Static secret key. |
| `STORAGE_S3_PREFIX` | (empty) | Key prefix isolating deployments within a bucket. |
| `STORAGE_S3_ALLOW_HTTP` | `false` | Allow plaintext HTTP (local/dev only, e.g. SeaweedFS over HTTP). |
| `STORAGE_S3_FORCE_PATH_STYLE` | `true` | Path-style requests (required by SeaweedFS; harmless on AWS). |
| `STORAGE_BLOB_GC_INTERVAL_SECONDS` | `21600` (6h) | Interval between GC sweeps. `0` disables GC. Only effective with the `s3` backend. |
| `STORAGE_BLOB_GC_GRACE_SECONDS` | `86400` (24h) | Safety grace period; orphans younger than this are never deleted. |
| `STORAGE_BLOB_GC_MAX_DELETES_PER_RUN` | `10000` | Per-sweep deletion cap to bound work (consumed per delete attempt). |
| `STORAGE_BLOB_GC_MAX_LIST_PER_RUN` | `100000` | Per-sweep cap on objects listed per prefix, bounding GC memory; larger buckets reconcile across sweeps. |

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

Any S3-compatible store (AWS S3, SeaweedFS, Cloudflare R2, ...) works
unchanged, only the endpoint and credentials differ.

## Testing

- `BlobStore` round-trip, idempotent delete, prefix isolation, listing (relative
  keys + deployment-prefix stripping), key derivation, and content hashing are
  unit-tested against object_store's in-memory backend (the production code
  path), with no network dependency.
- The GC reconciliation logic (live-pointer kept, orphan-older-than-grace
  deleted, orphan-within-grace kept, grace boundary, per-run cap) is unit-tested
  as a pure function, plus an end-to-end list→reconcile→delete sweep against the
  in-memory blob store (no database).

### CI coverage of the offload paths

The PostgreSQL repository offload paths
(`crates/server/src/storage/repositories/{session_files,skills}.rs`) are covered
by a backend-agnostic integration suite,
`crates/server/tests/blob_offload_integration_test.rs`. It runs the full offload
lifecycle against a real PostgreSQL database plus a `BlobStore` and asserts the
offloaded invariants: the inline `content`/`data` column is empty, the sidecar
pointer and `content_sha256` are recorded, bytes round-trip on read, delete
removes the backing object (and cascades the sidecar), update/CAS semantics hold,
move keeps the same object while copy duplicates it, and the cross-tenant
image-delete guard rejects an out-of-org delete.

The suite selects its blob backend from the environment, so the **same
assertions** run against both backends:

- **In-memory object_store backend** (default) in the `Integration Tests
  (PostgreSQL)` CI job, exercises the offload code path on every PostgreSQL PR
  with no S3/SeaweedFS container (it still uses the PostgreSQL service container).
- **Real S3-compatible store** in the dedicated `Integration Tests (S3 Blob
  Backend)` CI job, which starts SeaweedFS via a `docker run` step alongside the
  PostgreSQL service container, sets `STORAGE_BLOB_BACKEND=s3` + `STORAGE_S3_*`
  (with `STORAGE_S3_ALLOW_HTTP` and path-style addressing for the local
  endpoint), and re-runs the identical suite against the AWS S3 client path. The
  job is gated on a narrow `object_storage` path filter so it only runs when the
  offload code, its sidecar migration, or its harness changes.

The live-pointer enumeration query and the full GC `spawn_blob_gc_task` path
require PostgreSQL and are not unit-tested without a database, consistent with
the rest of the storage layer; end-to-end offload + GC against SeaweedFS remains
available for local smoke testing (see *Local development*).

## Non-goals / follow-ups

- **Streaming I/O.** The current contract buffers whole blobs in memory, matching
  the existing inline path and per-file size caps. Range/streaming reads are a
  later enhancement.
- **Migration of existing data.** Enabling `s3` offloads newly written content;
  a backfill job to move pre-existing inline content is follow-up work. Reads
  transparently serve inline or offloaded content regardless.
- **Per-backend encryption.** Object-store server-side encryption is configured
  on the bucket; envelope encryption of blobs is out of scope here.

## Source Index

- `crates/server/src/storage/blob_store.rs`, `BlobStore`, `ObjectStoreBlobStore`,
  config, key derivation, content hashing, prefix listing (`list_with_prefix`).
- `crates/server/src/blob_gc.rs`, orphan reconciliation sweep, grace period,
  per-run cap, metrics; spawned from `app_builder.rs`.
- `crates/server/migrations/071_object_storage_blobs.sql`, sidecar tables.
- `crates/server/src/storage/repositories/session_files.rs`, file offload.
- `crates/server/src/storage/repositories/skills.rs`, image offload.
- `knowledge/runtime-resources/workspace.md`, workspace filesystem model and quotas.
- `knowledge/runtime-resources/file-store.md`, `SessionFileSystem` boundary.
