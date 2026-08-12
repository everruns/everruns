---
type: Specification
title: "Knowledge Indexes Specification"
description: "Source-backed, embedded, citable knowledge indexes."
tags:
  - everruns
  - runtime-resources
---
# Knowledge Indexes Specification

## Abstract

Knowledge Indexes are org-scoped, named collections that **connect an external
knowledge source** (a GitHub repository today; Dropbox / OneDrive / generic Git
/ web later), **sync** its documents, **chunk and embed** them, and expose
**semantic + hybrid search with citations** to agents. They are the
search-optimized, source-backed sibling of curated Knowledge Bases.

This spec is the durable design intent. Implementation lands across the phases
in [Phasing](#phasing); phase 1 ships the foundation (entity, IDs, schema,
vendor-neutral vector-store abstraction, capability registration).

## Motivation

Two existing primitives sit next to this gap:

* **Knowledge Bases** (`knowledge/runtime-resources/knowledge-bases.md`) — human-curated short
  entries, keyword search, optional per-KB embedding model. Curation-first;
  no external sources, no chunking, no large-corpus retrieval.
* **Memory** (`knowledge/runtime-resources/memory.md`) — org-scoped file stores mounted into
  `/workspace`, including **source-backed memories** synced from GitHub / Git.
  Filesystem-first; the agent reads/greps files, there is no semantic retrieval
  or chunk-level citation.

Neither covers "connect a large external corpus, keep it in sync, and let an
agent retrieve and cite the most relevant passages." A Dropbox folder, a
OneDrive drive, or a documentation repository should be searchable by meaning,
returning passages with stable, linkable provenance. Knowledge Index fills that
slot.

The pieces this builds on already exist:

* **Embeddings** — `ServiceKind::Embeddings` + `EmbeddingsDriver`, resolved via
  `resolve_service(org, ServiceKind::Embeddings, binding)` (`knowledge/foundations/providers.md`).
* **Source sync** — the background sync worker pattern from source-backed
  Memory (`crates/server/src/domains/memory/source_sync.rs`): claim → snapshot →
  complete/fail with optimistic claim-timestamp concurrency, sanitized errors,
  byte/file limits, secret-free source config.
* **Connections** — GitHub App with on-demand 1-hour installation tokens and the
  `ConnectorPlugin` registry for future OAuth sources
  (`crates/server/specs/user-connections.md`).

## Concepts

| Name | Description |
|---|---|
| **Knowledge Index** | Org-scoped, source-backed, embedded, searchable collection. Public ID prefix: `kidx_`. |
| **Index Document** | One ingested source document (a repo file today). Public ID prefix: `kidoc_`. |
| **Index Chunk** | A token-windowed passage of a document; the unit that is embedded, retrieved, and **cited**. Public ID prefix: `kchk_`. |
| **Source** | The external origin of documents, addressed by `source_type` + `source_config`. |
| **Syncout** | The sync pipeline: enumerate → extract text → chunk → embed → upsert; prune removed documents. |
| **Vector store** | The external engine holding embeddings + BM25 text per namespace. Turbopuffer is the reference backend. |
| **Capability config** | The `knowledge_index` capability binds an agent/harness to one or more indexes. |

### Boundary with adjacent primitives

| Question | Knowledge Base | Memory (source-backed) | **Knowledge Index** |
|---|---|---|---|
| Content origin | Human-curated entries | Synced from Git/GitHub | **Synced from a connector** |
| Granularity | Short entries | Whole files | **Chunks** |
| Primary access | Search | Filesystem mount | **Semantic + hybrid search** |
| Retrieval | keyword (+opt embed) | grep / read | **vector KNN ⊕ BM25 (RRF)** |
| Embeddings | Optional | None | **Required** |
| Citations | `kbe_` IDs | file path | **chunk → document `source_uri` + location** |
| Mounted into `/workspace`? | No | Yes | No |

## Lifecycle

Knowledge Indexes follow the standard building-block lifecycle from
`knowledge/foundations/models.md`: `active` → `archived` (read-only, hidden from default lists,
not assignable to new capability bindings) → `deleted` (tombstone; detail/list
APIs return 404). Documents and chunks inherit the parent index lifecycle; a
hard delete cascades and drops the index's vector namespace.

`sync_status` is an orthogonal axis tracking the Syncout pipeline:
`idle` → `pending` → `syncing` → `synced` / `failed`. A failed sync preserves
the previously indexed content and records a sanitized `last_sync_error`.

## Data Model

Full DDL: `crates/server/migrations/074_knowledge_indexes.sql`. **Embedding
vectors are not stored in Postgres** — Postgres is the management source of
truth; vectors and BM25 text live in the external vector store (see
[Vector store](#vector-store)).

### `knowledge_indexes`

| Column | Notes |
|---|---|
| `id` UUID PK | Internal primary key. |
| `org_id` BIGINT FK | Organization scope. |
| `public_id` TEXT | `kidx_<32-hex>`. Unique per `org_id`. |
| `name` VARCHAR | Unique within `org_id` while not deleted (case-insensitive). |
| `description` TEXT? | |
| `source_type` VARCHAR | `github` (v1); `git` reserved. |
| `source_config` JSONB | Non-secret source coordinates. Never holds credentials. |
| `embedding_model_id` UUID FK → `models(id)` | Required. Must resolve to a model whose driver declares `ServiceKind::Embeddings`. |
| `vector_dim` INT? | Embedding dimension, recorded on first successful sync. |
| `vector_namespace` TEXT? | Vector-store namespace assigned at creation (see naming below). |
| `owner_principal_id` TEXT? / `resolved_owner_user_id` UUID? | Creator; resolved at the domain layer. |
| `status` VARCHAR | `active` / `archived` / `deleted`. |
| `sync_status` VARCHAR | `idle` / `pending` / `syncing` / `synced` / `failed`. |
| `last_synced_at` / `last_sync_error` | Last successful sync / sanitized failure reason. |
| timestamps | `created_at`, `updated_at`, `archived_at`, `deleted_at`. |

`source_config` mirrors source-backed Memory exactly so the GitHub path reuses
the same coordinates and token resolution. Example:

```json
{ "provider": "github", "repository": "owner/repo", "branch": "main", "root_folder": "docs" }
```

GitHub repository input accepts `owner/repo` and canonical `https://github.com`
repository URLs with an optional trailing slash or `.git` suffix. The API
normalizes accepted input to `owner/repo`; sync applies the same normalization
to historical stored configs so they recover on retry. Public repositories do
not require a GitHub connection. Private repositories use the owner's resolved
connection at sync time. Invalid coordinates and clone failures expose
credential-safe, actionable categories rather than raw provider errors.

### `knowledge_index_documents`

| Column | Notes |
|---|---|
| `id` UUID PK | |
| `index_id` UUID FK | `ON DELETE CASCADE`. |
| `public_id` TEXT | `kidoc_<32-hex>`. Unique within `index_id`. |
| `source_uri` TEXT | Stable per-source locator (e.g. `github://owner/repo@main/docs/x.md`). Unique within `index_id`. |
| `title` / `mime_type` / `content_hash` / `size_bytes` | Document metadata; `content_hash` drives incremental re-embedding. |
| `chunk_count` INT | Denormalized count for management/UI. |
| `last_seen_at` TIMESTAMPTZ | Set each sync pass; documents not seen are pruned. |
| timestamps | |

### `knowledge_index_chunks`

| Column | Notes |
|---|---|
| `id` UUID PK | |
| `document_id` UUID FK | `ON DELETE CASCADE`. |
| `index_id` UUID FK | Denormalized for index-scoped queries; `ON DELETE CASCADE`. |
| `public_id` TEXT | `kchk_<32-hex>`. **The stable citation ID.** Unique within `index_id`. |
| `ordinal` INT | Position within the document. |
| `text` TEXT | The chunk passage (also written to the vector store for BM25). |
| `location` JSONB | Provenance within the document: line / char / page ranges. |
| `token_count` INT? | |
| `created_at` | |

## Vector store

Embeddings live in an **external vector database**, abstracted behind a
vendor-neutral `VectorStore` trait selected through `HostComposition`
(`crates/platform/src/vector_store.rs`), mirroring how `SessionFileStore` is
pluggable. OSS depends on no vendor SDK at the core layer.

Backends:

* **In-memory** (`InMemoryVectorStore`) — brute-force cosine, used by dev mode
  and the in-memory storage-parity tests. No external dependency.
* **Turbopuffer** (reference production backend) — namespace-oriented serverless
  vector + BM25 engine. Multitenancy is first-class: each index is its own
  **namespace**, org-prefixed for isolation.

Turbopuffer is **opt-in** and lives in the `everruns-turbopuffer` crate. The
in-memory store stays the default; the server activates Turbopuffer only when
`TURBOPUFFER_API_KEY` is set (regional endpoint via `TURBOPUFFER_BASE_URL`,
defaulting to a sensible region). The backend maps each `index_namespace`
string onto one Turbopuffer namespace (namespace-per-index), enables ANN + BM25
via the upsert schema, and serves hybrid queries with a server-side
multi-query + reciprocal-rank-fusion (`rerank_by: ["RRF"]`). `$dist` is
normalized to `VectorMatch.score` so ordering is best-first regardless of mode:
raw ANN `cosine_distance` (lower is closer) is negated, while BM25 and RRF
relevance scores (higher is better) pass through. Turbopuffer hosts
(`*.turbopuffer.com`) are on the embedded system egress allowlist so outbound
sync calls are permitted when the allowlist is enabled. The API key is sent only
in the `Authorization` header and never logged or surfaced in errors.

### Multitenancy and naming

The store is **multitenant and multi-index** by construction:

* **Multi-index** — one namespace per Knowledge Index. The namespace is recorded
  on `knowledge_indexes.vector_namespace` at creation and never reused.
* **Multitenant** — the namespace name is org-prefixed:
  `org_{org_id}__{public_id}` (e.g. `org_1__kidx_<hex>`). Cross-org reads are
  structurally impossible because every query targets a single, org-derived
  namespace; the resolved namespace is always checked against the index's
  `org_id` before any call.

### Stored shape

Each point is keyed by the chunk `public_id` (`kchk_…`) and carries:

* `vector` — the embedding (`vector_dim` from the index's embedding model).
* `text` — the chunk passage, enabling Turbopuffer BM25.
* attributes — `{ index_id, document_id }` for filtering and bulk delete.

### Operations (trait surface)

`upsert(namespace, records)`, `query(namespace, QueryRequest)`,
`delete_by_document(namespace, document_id)`,
`delete_namespace(namespace)`. The trait is intentionally minimal; richer
filtering is additive.

### Consistency

Postgres and the vector store are dual-written. Reconciliation rules:

* On document re-sync, chunks are replaced atomically in Postgres, then
  `delete_by_document` + `upsert` runs against the namespace. The vector store
  is treated as a rebuildable projection: a full re-sync always reconciles it.
* On index hard delete, the Postgres cascade runs, then `delete_namespace`.
* A best-effort vector-store failure during sync marks `sync_status = failed`
  with previous content preserved; it never corrupts the Postgres source of
  truth.

## Syncout pipeline

A `KnowledgeSourceConnector` enumerates documents from the configured source
(incrementally where the source supports it) and yields `(source_uri, bytes,
metadata)`. The pipeline then, per document:

1. **Extract** text by MIME type (markdown / text / code / HTML in v1; PDF /
   Office later).
2. **Chunk** with a token-aware windowing strategy and overlap.
3. **Embed** chunk batches via the index's `EmbeddingsDriver`.
4. **Upsert** chunks into Postgres and the vector-store namespace; stamp
   `last_seen_at`.

After the pass, documents whose `last_seen_at` is older than the sync start are
pruned (Postgres cascade + `delete_by_document`).

The worker reuses the source-backed Memory pattern: a background poll task
claims the next `pending` index (`claim_next_index_sync`), runs the pipeline,
and calls `complete_index_sync` / `fail_index_sync` guarded by the claim
timestamp so concurrent or stale workers cannot clobber each other. GitHub
credentials are resolved **only at sync time** from the owner's short-lived
connection token (`UserConnectionResolver`), never persisted in `source_config`
(THREAT[TM-API-018]).

Creation atomically enters `pending`, so every valid new index receives an
initial sync without a second client request. Editing source coordinates or the
embedding model also re-enters `pending`; metadata-only edits preserve the
current sync state. Manual "Sync now" is an idempotent retry. Periodic and
webhook-driven (GitHub push) sync remain later work.

The embedding model must be enabled, advertise the embeddings capability, and
belong to an active org-scoped provider whose driver declares the embeddings
service. All invalid or inaccessible references return the same validation
error. Existing invalid indexes are never silently reassigned: management UI
surfaces the incompatible selection, disables retry, and lets an admin repair
it by choosing a valid model; saving that repair queues a sync.

## Retrieval and citations

Agent-facing tool `search_index` (follow-up vertical slice):

```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string" },
    "indexes": { "type": "array", "items": { "type": "string" } },
    "top_k": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10 }
  },
  "required": ["query"],
  "additionalProperties": false
}
```

Flow: embed the query → run a hybrid query (vector KNN + BM25) against each
bound index namespace → fuse with reciprocal-rank fusion → hydrate chunk text /
citation metadata from Postgres. Response keeps the `search_knowledge` citation
shape so the UI renders both uniformly:

```json
{
  "results": [
    {
      "id": "kchk_...",
      "index_id": "kidx_...",
      "document_title": "...",
      "source_uri": "github://owner/repo@main/docs/x.md",
      "location": { "line_start": 12, "line_end": 34 },
      "snippet": "...",
      "score": 0.81
    }
  ]
}
```

`id` (`kchk_`) + `source_uri` + `location` give the agent a stable, linkable
citation that survives re-sync as long as the passage persists.

## Capability: `knowledge_index`

* **ID:** `knowledge_index`
* **Name:** `Knowledge Index`
* **Category:** `Knowledge`
* **Icon:** `library`
* **Dependencies:** none
* **Features:** `knowledge`
* **Risk:** `Medium` — retrieval surfaces untrusted external content into the
  agent context (prompt-injection vector), unlike the org-curated Knowledge Base.

### Config Schema

```json
{ "indexes": ["kidx_abc123...", "kidx_def456..."], "top_k": 10 }
```

* `indexes` — IDs of Knowledge Indexes the agent can search. Empty/null = no
  indexes bound; the tool returns an empty result set.
* `top_k` — optional default result cap (1–50).

Phase-1 validation enforces structural shape only (`kidx_<32-hex>` format, no
duplicates, `top_k` bounds). Cross-org / archived-index rejection runs at tool
dispatch time in the retrieval slice and must not leak existence of other-org
indexes.

## API

REST endpoints (conventions in `knowledge/execution/apis.md`), shipped with the management
slice:

* `GET/POST /v1/knowledge-indexes`
* `GET/PATCH/DELETE /v1/knowledge-indexes/{index_id}` — DELETE archives per lifecycle
* `POST /v1/knowledge-indexes/{index_id}/sync` — enqueue a manual sync
* `GET /v1/knowledge-indexes/{index_id}/documents`

List supports `?search=` and `?include_archived=`.

## Security

Canonical entries live in `knowledge/security/threat-model.md`.

* **Prompt injection (headline).** Indexed documents are untrusted external
  content surfaced into the agent context. Retrieved passages are data, never
  instructions; the capability is `Medium` risk and tool output is subject to
  the standard distillation / guardrail paths.
* **Secret handling.** `source_config` never stores credentials; GitHub tokens
  are minted on demand at sync time and never persisted or logged.
* **Cross-org isolation.** Every vector-store call targets an org-derived
  namespace; capability config validation rejects other-org index IDs without
  leaking existence.
* **Storage / cost.** Chunks + embeddings are unbounded by source size; sync
  enforces per-file / total byte and document-count limits, and the feature
  participates in per-org storage quotas (EVE-510). Embedding spend is bounded
  by those limits and the configured embedding model.
* **Audit.** Index CRUD, sync triggers, and capability binding changes are
  audited via the standard audit log domain.

## Permissions

Index CRUD is org-scoped under standard org policies (`org.member` /
`org.admin`), matching Knowledge Base permissions. Capability binding is gated
by the same policy that gates capability configuration on the parent agent or
harness.

## Testing

* Typed-ID parsing/serialization for `KnowledgeIndexId`, `KnowledgeIndexDocumentId`,
  `KnowledgeIndexChunkId`.
* `VectorStore` contract tests against `InMemoryVectorStore` (upsert/query
  ordering, document delete, namespace delete, cosine ranking).
* Capability config validation: empty is valid; malformed IDs, duplicates, and
  out-of-range `top_k` are rejected.
* Storage parity (in-memory and Postgres) for the three tables (management slice).
* Sync claim/complete/fail concurrency with stale-claim rejection (sync slice).
* Create-to-ingestion state transitions, invalid embedding-model rejection,
  source-subfolder ingestion, failure display, and management-UI polling.
* Cross-org isolation (namespace derivation + config validation).
* Hybrid retrieval ranking and citation hydration (retrieval slice).
* OpenAPI export updated when the API surface lands.

## Phasing

PR-sized slices, each leaving the tree green:

1. **Foundation** — entity IDs, Postgres schema, `VectorStore` trait +
   `InMemoryVectorStore`, `knowledge_index` capability registration + config
   validation. No retrieval, no sync yet.
2. **Management + storage** — domain, repositories (in-memory + Postgres),
   CRUD API, namespace assignment, OpenAPI.
3. **Syncout** — GitHub source connector, chunking, embedding, the background
   sync worker, Turbopuffer backend wired through `HostComposition` and
   `just start-all`.
4. **Retrieval** — `search_index` tool, hybrid query + RRF, citation hydration.
5. **UI** — list/detail/sync-status pages and capability config UI.
6. **Later** — GitHub push webhooks, scheduling, richer extractors (PDF/Office),
   and additional sources (Dropbox, OneDrive, generic Git).

## Open Questions

* Should an index be able to target an existing source-backed Memory as its
  document substrate (sync once → index), decoupling connect/sync from
  embed/search?
* Per-index ACLs (team-scoped) or org scope only, mirroring the Knowledge Base
  open question?
* Should `search_index` and `search_knowledge` unify into one tool with a shared
  citation surface once both ship?
* Chunking strategy knobs (size/overlap) — per-index config or platform default?
