---
type: Design
title: Knowledge Indexing and Storage
description: Incremental publication, object-backed content and organization search-provider bindings for unified Knowledge.
tags:
  - everruns
  - knowledge
  - indexing
  - storage
---

# Knowledge Indexing and Storage

## Status and boundaries

**Implementation, 2026-10-23.** Companion to [Unified Knowledge](knowledge.md).
Originals, derived text and embedding-cache artifacts use segmented blob storage
with a local development fallback. An explicit Knowledge-only S3 selection reuses the
existing object-storage configuration independently of the workspace file/image
backend. This lets a deployment retain inline PostgreSQL files during its separate
object-storage backfill without putting new Knowledge bytes on ephemeral disk.
See [content storage selection](../../crates/server/src/storage/blob_store.rs) and
[storage initialization](../../crates/server/src/setup/storage_init.rs). The selected
store survives background-pool handles; invalid explicit configuration fails startup
and object errors never fall back to local files. No organization feature flag or
credential grant is changed by backend selection. PostgreSQL owns metadata, leases, publication
pointers and optional rebuildable lexical postings. It stores neither canonical
chunk text nor binary originals. Turbopuffer supports lexical and hybrid modes,
including organization-owned encrypted credential references with no fallback
when a selected credential is invalid.

[Implementation](../../crates/server/src/domains/knowledge/indexing.rs) skips
unchanged Git blobs, retains immutable versions and caches embeddings by model
profile and passage hash. Domain writes use the current cancel-safe transaction boundary, including
composed-command rollback. Configuration rebuilds retain the published generation
until all current files have projections. Per-file updates move their projection
only after acknowledged search writes and lease/revision checks.

The sections below remain the target contract. Qualification targets are not
benchmarks already passed. Current Git fetches stream bounded shallow snapshots directly into libgit2,
not a durable incremental object cache. The initial worker resumes at file/job
boundaries and retries snapshots; it does not resume in the middle of a file.
Automated orphan cleanup, remote generation GC, aggregate organization storage
quotas, streaming extraction above the current per-file budget, provider-profile
change notifications and representative end-to-end throughput qualification remain extension
work. Catalog qualification uses a synthetic million-file corpus (the weekly
`knowledge_catalog` bench, with a per-page time budget); streaming and
extraction budgets are tested independently of embedding costs. Configuring a collection for another model/account advances its generation;
provider model edits outside that workflow do not automatically enqueue rebuilds.

## Storage ownership

Use three independent layers:

| Layer | Owns | Does not own |
| --- | --- | --- |
| PostgreSQL control/catalog | Collections, sources, hierarchy, file/version identities, bounded metadata, permissions, jobs, checkpoints, projection manifests and content references. | Original binary payloads or the authoritative passage text of every chunk. |
| Content/blob store | Immutable original bytes, extracted representations, location maps and durable fragment manifests; versioned artifacts required for citation resolution. | Filesystem authorization, directory query semantics or ranking. |
| Search backends | Rebuildable lexical postings, embeddings, selected filter metadata, and passage text required by the backend for search/snippets. | The only copy of source content or historical evidence. |

This follows the existing [object-storage separation](object-storage.md): keep
the searchable directory catalog separate from opaque content bytes. Reuse
[BlobStore](../../crates/server/src/storage/blob_store.rs) and its tenant-scoped
keys/recovery conventions rather than inventing a text-only knowledge database.
Extend the byte boundary with bounded streaming/range reads, resumable writes
and checksums as required; the current whole-object `Vec<u8>` contract alone
does not qualify large-file support. Authorization stays at Everruns' proxy.

S3-compatible object storage is the recommended production content backend.
An embedder may supply local disk or in-memory implementations for development.
Do not require another database merely to browse an unindexed collection.
Content-addressing and extraction/embedding caches are tenant-scoped; identical
private files across orgs cannot become a side channel or shared authorization.

Chunk/fragment text is stored in immutable representation objects, with durable
IDs and location manifests. Search engines may duplicate that text for BM25 or
snippets. PostgreSQL may retain small references or measured hot-path metadata,
but not a mandatory row containing every passage. This avoids coupling binary
and corpus growth to primary-database WAL, backups and row transfer.

Do not replace one chunk row with one tiny object per chunk. Pack fragment text
and location maps into bounded seekable objects per document or shard; fetch
selected ranges, cache authorized immutable fragments, and avoid reading a
complete huge document to hydrate one hit. Benchmark object-request cost and
latency before choosing the packing size. Chunk manifests may themselves live
in object storage; only query-critical lookup/catalog state belongs in SQL.

Search results must carry enough authorized version references to hydrate from
this substrate. Evolve the existing
[VectorStore contract](../../crates/contracts/src/vector_store.rs) and
[retrieval service](../../crates/server/src/domains/knowledge_indexes/search/mod.rs),
which currently hydrate text from PostgreSQL. Preserve vendor-neutral host
injection and existing embedding-provider resolution. A lexical backend must
accept records without fabricated embeddings; regex acceleration advertises
its own capabilities independently of ranked text/vector retrieval.

Use retention references for live snapshots, active sessions and citations.
Garbage-collect unreferenced blobs, abandoned projections and retired generations
after a grace period, with retryable cleanup jobs. Deletion/access revocation
overrides historical citation readability; retained evidence is never a bypass.
Backups need both catalog and content manifests; search indexes can be rebuilt.

## Incremental work and publication

Incremental means avoiding unchanged work at each stage, not merely storing a
hash. Current [sync](../../crates/server/src/domains/knowledge_indexes/source_sync/mod.rs)
re-embeds all documents, allocates new IDs, commits replacement SQL rows and then
deletes/recreates the active vector namespace. That is the behavior to replace.

1. **Discover changes.** Enumerate source pages or consume a provider delta
   token. Record stable provider IDs, revisions, paths, metadata/ACL changes and
   deletion events. For Git, compare committed trees and fetch changed blobs;
   webhook delivery is a wake hint, not the sole source of correctness. Persist
   completed work/checkpoints; expired cursors fall back to a reconciliation scan.
2. **Retain changed originals.** Stream and hash new content into immutable
   objects. Provider revisions can avoid downloads; content hashes avoid duplicate
   extraction when a provider reports a superficial revision change. Metadata-only
   updates do not require embedding the same bytes again.
3. **Produce representations.** Cache by input digest plus extractor identity,
   version and configuration. Changing Markkit options invalidates the affected
   extraction, not unrelated sources. Store warnings and bounded partial output.
4. **Produce fragments.** Prefer structural boundaries such as headings,
   paragraphs, code sections and sheet rows, with bounded token windows as a
   fallback. Cache by representation version and chunker profile. Future image,
   page or time-based fragments need not pretend to be text passages.
5. **Embed changed fragments.** Cache by normalized fragment digest, model
   identity/version, dimensions and preprocessing profile. A changed description
   or path should update metadata without paying for unchanged embeddings.
   Distinct repeated passages need disambiguated identities even when they share
   a cached vector. Model/dimension changes create a separate index generation.
6. **Stage search changes.** Upsert new fragment versions and prepare tombstones
   idempotently. Preserve unchanged fragment identities and reference their
   already-published records; do not clone the full corpus for a small update.
7. **Publish.** Advance an authoritative visible file/projection manifest only
   after its blobs and required search writes are acknowledged and validated.
   Commit the source checkpoint only after the corresponding work is durable.
   A reconciler repairs retries and orphaned staged writes.

Use durable scheduled activities, bounded per-file/batch work and fenced leases
through the existing worker/claim patterns. Persist the stage outcome so a crash
after embedding retries publication without paying for embedding again. Outbox
and job records coordinate the stores; no cross-database transaction is assumed.
Leases must be renewed and stale workers fenced at publication as well as at
claim time. Repeated or out-of-order source events converge on the newest
authorized revision.

Delete a missing file only with an explicit provider tombstone or a successfully
completed, authoritative full enumeration. A partial listing, timeout or rate
limit cannot imply that the rest of the corpus was deleted. Deletion removes it
from visible catalogs/search immediately through tombstones; physical projection
and blob cleanup may follow asynchronously. Permission reductions are applied
before any new results can expose stale permissions.

The default consistency unit is a document version, with source and indexed
checkpoints reported separately. Sync may expose a newer original while indexing
lags; a hit always names the version that was actually indexed. A caller needing
a coherent source snapshot explicitly pins one and its compatible projection.
Mounts pin a version for an operation/turn; refresh occurs at an explicit boundary.
Catalog snapshots should use persistent/delta manifests rather than rewriting a
million-file listing for each change.

Queries use only published generations and recheck permissions, current deny
state and manifest membership before hydration. Backends must support generation
filters/visibility or an equivalent staged-namespace strategy. Filtering stale
hits after top-k is insufficient for complete ranking: overfetch/refill against
published membership and report degraded coverage if a budget prevents completion.

Ordinary sync never deletes the active namespace. Reindexing into a new model or
backend builds a replacement in the background, verifies it, then swaps the
active reference. Keep the previous generation for retry/rollback and collect it
later. A failed search write must not publish SQL state that promises unavailable
hits. Document-level failure is visible without making the entire retained source
unreadable; expose ready/failed/unsupported counts and indexing coverage.

## Citations and retrieval

A citation resolves an immutable original version, its representation/fragment
and a real locator: original text lines, extracted-text offsets, page/slide/cell
coordinates or media time when supplied by the extractor. Keep the source's
moving URL separate from its immutable revision URL. Git citations use a commit
SHA and full repository path, including a selected source subdirectory.

Stable file and passage identity must survive a no-op sync. A changed passage
creates a new evidence version rather than silently retargeting old citations.
Renames do not rewrite historical references. Historical references resolve only
while their retained version and live access policy permit it. Integrate with
the existing [citation rendering direction](citations.md), without claiming that
new citation UI or modality support already exists.

Unify explicit tool, shell and automatic retrieval behind one authorized query
service. Apply collection/path/metadata/access filters before ranking where
possible, then verify against the live catalog. Fuse ranks across collections
and different backends; their raw relevance scores are not comparable. Reuse a
query embedding only across compatible model profiles and authorized provider
bindings. Account for query-time and background embedding/index costs separately.

Exact grep is a separate operation. Search ranking may improve navigation, but
it cannot prove that a regex has no other matches. Preserve bounded-scan behavior
and explicit coverage/truncation until an exact backend can supply that guarantee.

## Organization indexers and BYOK

An organization configures one or more **indexer connections**: a search-provider
account, approved endpoint/region and encrypted credential reference, plus
advertised capabilities. Each indexed collection binds to one of these or an
explicit platform-managed default. The embedding model/provider is a separate
binding. Bringing a Turbopuffer key does not configure or pay for embeddings.

Reuse [scoped resources](../security/resource-access.md) and
[bound credential storage](../security/bound-secrets.md) for organization-owned
credentials and grants. Keep keys out of source configuration, tools, mounts,
job payloads, logs and model context. Resolve the selected account on each job
or query through a host-injected provider factory. Provider-specific validation,
rotation, destination restrictions and audit belong in that adapter.

Current [platform selection](../../crates/server/src/platform.rs) creates a
deployment-level Turbopuffer instance from environment variables. Keep that as
an explicit managed/default account option; it is not organization BYOK.
Replace the single globally selected handle with a tenant/account-aware resolver,
without introducing vendor SDKs into the core contracts.

Derive namespaces from deployment, org, collection and index generation. Namespace
prefixes supplement authorization; possession of a provider key or a string
prefix is not proof of collection access. The resolver checks org membership,
grants, destination, provider capabilities and revocation. Missing/revoked keys
fail closed for that account; never silently switch to a hosted key, another
region or another org's account. Existing permitted file reading can continue.

A key rotation within the same account does not require re-embedding. Changing
account, region or provider builds a new projection from retained representations
and compatible embeddings where policy permits. Explain which data leaves
Everruns: lexical text, selected metadata and/or vectors according to the mode.
Track org indexing budgets and quotas independently from session LLM budgets.
Configuration and qualification remain OSS-owned; private deployment values and
cloud operational procedures stay in their owning private repository.

## Large-source qualification

Large sources need paginated catalog queries, streaming intake and lazy reads,
not larger constants around whole-corpus vectors. Partition work by source and
file; bound queues, batch size, concurrency, memory, temporary disk and output.
Implement retries with backoff, provider rate-limit handling, resumable transfers,
fair scheduling across orgs and cancellation. Progress must survive restarts.

Keep existing per-operation safety budgets for reads, extraction and grep.
An exceptional file should become an explicit per-file unsupported/over-budget
outcome rather than silently disappear or fail all unrelated files. Account for
retained original, derived and search bytes independently; surface skipped,
partial and failed counts. Metadata listings and one-file reads cannot download
the collection. Large binary access needs bounded ranges/download streams rather
than giant base64 tool results.

Proposed qualification envelope: a synthetic million-file catalog and a streamed
terabyte-scale source fixture, including large binaries, plus real mixed-format
samples. These are test goals, not advertised capacity. Metadata and streaming
load generators can establish bounds without paying to embed a terabyte.
Benchmark ingestion, filtered search, object hydration and exact-scan behavior
separately; final latency/cost targets require measurements on representative
backend accounts and hosts.

Release evidence must show:

* A no-op sync performs no extraction or document embedding; delta-capable
  sources also avoid downloading unchanged originals. Snapshot-only adapters
  may relist, but must still reuse downstream work.
* Changing one small file processes that file's changed fragments without
  rewriting the full namespace or corpus manifest. Metadata-only changes reuse
  embeddings, and renames preserve known file identity.
* Job memory is bounded by configured batch/file buffers, independent of total
  corpus size; opening one mount does not materialize every file.
* Crashes between blob upload, embedding, search writes and publication preserve
  the previous visible version and resume idempotently. Incomplete enumeration
  never prunes unseen files.
* Turning indexing off removes searchable hits while retained files remain
  readable; denied files are absent from list, grep, search and hydration.
* Two orgs with separate Turbopuffer accounts remain isolated, rotation avoids
  re-embedding, and revocation cannot fall back to deployment credentials.
* Citations survive no-op sync and reindexing, remain tied to immutable bytes,
  and stop resolving when access/retention policy requires it.

Remaining measured choices are the packed-representation layout, the first
production exact-search accelerator, and backend-specific visibility mechanics.
Require evidence for these before claiming large-corpus readiness. The chosen
backend must meet the contracts above rather than weakening them implicitly.
