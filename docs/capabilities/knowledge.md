---
title: Knowledge
description: Attach versioned file collections to agents, browse originals and extracted text, and optionally index them for keyword or hybrid retrieval.
---

Knowledge brings notes, uploaded files, repository trees and Everruns documentation into one collection model. Indexing is optional. Files remain available to browse, read and grep when indexing is off.

Knowledge is an Adoption feature, available on development and hosted deployments and disabled by default. An organization owner/admin or platform operator can enable the organization's **Knowledge** feature. Then open **Knowledge** and create a collection. A collection opens on a file browser: upload files or a folder with its relative paths, or create Markdown files with optional OKF frontmatter, on its **Files** tab, and connect a source on **Sources**. Search accounts for indexed collections live on the **Accounts** tab of the Knowledge page. GitHub sources reuse existing organization or virtual-user GitHub connections. Public HTTPS Git repositories work without a GitHub connection. Source roots must not overlap, and a connected source cannot overwrite a managed file.

## Attach to an agent

Add the **Knowledge** capability and select collections in the agent editor, or use this capability reference:

```json
{
  "ref": "knowledge",
  "config": {
    "collections": ["everruns-docs"],
    "automatic": false,
    "limit": 8
  }
}
```

`everruns-docs` is predefined, requires no credentials, and contains a snapshot of the documentation bundled with the server. Indexing starts off. Managers may enable it like any other collection.

The agent sees read-only mounts:

- `/knowledge/index.md`: the attached collection catalog.
- `/knowledge/<collection-id>/`: original files, with their hierarchy preserved.
- `/knowledge-text/<collection-id>/`: extracted text where available.

The existing **Data Knowledge** capability keeps its read-only `/knowledge/{tables,business,queries}` scaffold. Its index and the collection catalog share `/knowledge/index.md`; collection ID subtrees always use live collection permissions, including when a collection is detached. Directory listings and grep scoped to a Knowledge path include both sources.

Read indexes first, then individual files as needed. An authored `index.md` is preserved; collections without one receive a bounded generated index. Use `browse_knowledge` or `knowledge files` to page through larger directories without enabling indexing. Request metadata only when needed; metadata pages are separately bounded. Knowledge bindings belong to the calling session, including when several sessions share a workspace. Sharing workspace files does not share Knowledge attachments.

## Ownership and attachment scope

Collections, their indexes and search-provider accounts belong to an organization. Users manage them through organization permissions; a personal GitHub connection supplies source credentials, not a private user index.

Attach collections through the **Knowledge** capability on a harness, agent or session. Configuration follows the existing capability precedence: harness, then agent, then session. A higher layer replaces the lower layer's configuration for that capability, so list every collection the session should retain. Collections are shared rather than copied or reindexed for each agent or session.

Runtime tools, Bashkit search, automatic retrieval and mounts use the calling session's effective collection bindings and live file visibility. There are no separate user-, agent- or session-owned Knowledge indexes, and an agent does not automatically search every collection in its organization. Sharing a workspace does not share its sessions' Knowledge bindings.

## Search and grep

Choose **Off**, **Keyword**, or **Hybrid** in collection settings. Hybrid needs an enabled embedding model and a persistent Turbopuffer backend. Keyword indexing works with Turbopuffer or the local PostgreSQL full-text fallback. PostgreSQL stores catalog rows, fragment offsets and local lexical postings, not canonical passage text or binary payloads. Collections using a remote indexer keep their postings there.

The `search_knowledge` tool accepts a query, optional collection subset, path prefix, user metadata filter, result limit and `lexical_only` flag. Requested collections are always intersected with the session's current bindings. Returned passages carry immutable file/version IDs, source locators and extracted-text offsets.

Bashkit uses the same retrieval service:

```bash
knowledge files --collection kn_0123456789abcdef0123456789abcdef --path /policies --limit 50
knowledge search --limit 5 'refund policy'
knowledge search --collection kn_0123456789abcdef0123456789abcdef --lexical 'retention'
knowledge search --metadata '{"team":"support"}' 'refund policy'
grep -n 'retention' /knowledge/kn_0123456789abcdef0123456789abcdef/policies.md
```

Exact grep is independent of the index. Scope it to `/knowledge-text` to search extracted documents. Scans and returned data are bounded; narrow the path if a scan exceeds its budget.

With `automatic: true`, Everruns retrieves a small amount of relevant evidence for the incoming user message before model execution. Evidence is retained by input ID for replay, checked against live visibility, and added as source data. Retrieval has a five-second timeout and does not prevent a turn from running when unavailable. Query embedding calls use session usage events; ingestion embeddings are attributed to the organization.

## Content and metadata

Original bytes are retained for every format. Markkit extracts HTML, DOCX and spreadsheets. Text and code retain their original UTF-8 representation. Markdown frontmatter, including OKF fields and unknown extensions, is preserved alongside user annotations. Links and file paths remain source-relative. OKF remains a directory of original files: concept Markdown, reserved `index.md` navigation, `log.md` history and binary assets all round-trip through uploads/downloads or a Git source. Unknown frontmatter fields remain intact, including BOM/CRLF files; there is no conversion into fixed entry kinds. `GET /v1/knowledge/{collection_id}/okf` validates a page of OKF v0.2 metadata and reserved-file structure. Follow `next_cursor` to validate the rest of a large catalog. This checks metadata requirements, not every Markdown link or authored index entry.

Unsupported binaries, including PDFs, images, audio and video, remain downloadable. This release does not provide PDF extraction, OCR, transcription or multimodal embeddings. Files exceeding the extraction budget remain stored and report that state rather than disappearing.

Collection rules independently exclude paths from ingestion, indexing or agent reads. File controls can disable indexing or agent reads without removing the original. Managers can annotate connected files; their source content stays read-only.

## Incremental processing and storage

Sync compares Git blob identities before uploading content artifacts. Changed files receive immutable versions; unchanged files retain their version and citations. Extraction is reused for metadata-only edits, and embeddings are cached by organization, model profile and passage hash. Source deletions apply only after a successful complete traversal. Leased jobs renew during source fetches and indexing, retry interrupted work, and index generations publish after the catalog's visibility checks pass.

Uploads and downloads stream in bounded segments. Content uses the configured S3-compatible blob store; local development falls back to `KNOWLEDGE_STORAGE_ROOT` (default `.local/knowledge`). Use shared object storage for multiple server replicas. Back up the database and content store together.

Organization search accounts reference encrypted, write-only organization service credentials. A revoked or invalid selected credential fails closed; it never falls back to the deployment account. Embedding provider credentials are configured separately in the model catalog.

Embedding usage preserves the serving provider account for background indexing
and query-time calls, so host-managed usage stays distinct from your own keys.
Deployment-owned index backends can also record the resource quantities reported
by the search provider. Your hosting service controls the prices and credit
policy; self-hosted Knowledge does not require a billing service.

## Current bounds

File catalogs paginate at up to 500 rows; pass `tree=true` to list one folder at a time (its files plus one entry per subfolder). Agent directory listings cap at 500 children, file reads at 1 MiB, extraction at 8 MiB, and exact grep at 5 MiB of scanned content, 5,000 candidate files and a 512 KiB per-file ceiling. Streamed originals have a 1 TiB per-file transfer ceiling; this is a safety limit, not a tested capacity claim. Git fetching streams shallow snapshot packs directly into the object store, with the existing 512 MiB pack limit. A synthetic million-file catalog and bounded streamed-original fixture qualify catalog/transfer behavior; these do not establish end-to-end terabyte indexing throughput. Source refresh is explicit. Retained versions currently have no automatic expiry; monitor content storage and keep it with database backups. Additional source adapters and document extractors can use the same collection boundary.

Management APIs are under `/v1/knowledge` and `/v1/knowledge-indexers`; the generated command catalog exposes the corresponding `everruns knowledge` commands. Organization settings permissions govern management, and explicit agent/session bindings govern runtime reads. A read-only personal access token can browse, download, validate metadata and search; file changes, source refresh and configuration require a full-access token.

## Deployment storage

Knowledge originals and extracted text need shared, persistent content storage on
hosted replicas. `KNOWLEDGE_STORAGE_BACKEND=s3` uses the existing `STORAGE_S3_*`
configuration for Knowledge alone, even while `STORAGE_BLOB_BACKEND=db` keeps
workspace files and image bytes inline in PostgreSQL. It does not backfill or
move existing files. The default `inherit` uses the existing shared blob backend
or the local development fallback. Do not use the local fallback on ephemeral
hosted containers. An invalid explicit S3 configuration fails startup; storage
errors never silently switch to local disk. Organization adoption and collection
permissions remain separate.
