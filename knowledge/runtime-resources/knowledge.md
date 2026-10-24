---
type: Design
title: Unified Knowledge
description: Unified versioned file collections with optional indexing, agent mounts, metadata and progressive disclosure.
tags:
  - everruns
  - knowledge
  - filesystem
  - okf
---

# Unified Knowledge

## Status and scope

**Implementation, 2026-10-23.** Replaces the two undeployed product surfaces with
one capability, management API and UI. Implementation lives in the
[Knowledge domain](../../crates/server/src/domains/knowledge/mod.rs),
[capability](../../crates/capabilities/src/capabilities/knowledge.rs),
[neutral runtime port](../../crates/contracts/src/knowledge.rs) and
[Bashkit extension](../../crates/integrations/src/bashkit/knowledge.rs).

The delivered scope covers optional incremental indexing, managed files/folders,
public Git and existing GitHub connections, binary originals, Markkit extraction,
metadata and OKF navigation, agent catalog pagination without indexing,
lazy session-authorized mounts including shared
workspaces, exact grep, indexed shell/tool retrieval, automatic progressive
disclosure, built-in docs and organization BYOK. Management uses organization
settings permissions; attaching a collection explicitly authorizes runtime reads.
Collection configuration and source attachments participate in entity history,
manager context and configuration restore. Indexer creation records only public
configuration and its credential-binding reference. File writes, deletions and
metadata retain immutable file versions; deletion and read/index flags live in
the file catalog. These operations, sync jobs and changing index counts do not
create collection configuration revisions. Configuration restore preserves
file contents and current source progress.
[Product documentation](../../docs/capabilities/knowledge.md) owns operational limits.

The remaining sections describe contracts and extension directions, not a claim
that every connector or extractor exists. PDF/OCR/transcription are outside
Markkit's current supported formats; originals remain valid files. OKF metadata
validation is paginated and lossless; complete authored-link linting and archive
packaging are separate from retaining an OKF tree. Source refresh is explicit,
versions have indefinite retention, and terabyte indexing throughput requires
representative backend qualification. Collection-specific reader grants and
provider polling are future extensions to the current organization/binding model.

One **Knowledge** feature owns collections of files, their original hierarchy,
metadata, source revisions and optional search indexes. Curated notes, uploads,
OKF bundles and connected repositories enter the same model. Indexing is a
derived capability of a collection; it is never required to create, mount or
read one. An index is rebuildable and is not the source of the knowledge.

The companion [indexing and storage design](knowledge-indexing.md) owns incremental
processing, publication, storage, organization indexers and scale qualification.
The older [Knowledge Bases](knowledge-bases.md), [Knowledge Indexes](knowledge-indexes.md)
and [OKF adoption](okf-adoption.md) concepts describe the existing split implementation.

## Grounding in current Everruns

Reviewed against OSS `origin/main` at `310d40221` on 2026-10-19:

| Existing boundary | Reuse and gap |
| --- | --- |
| [Knowledge capabilities](../../crates/capabilities/src/capabilities/knowledge_base.rs) and [index capability](../../crates/capabilities/src/capabilities/knowledge_index.rs) | Separate bindings and search tools; consolidate behind one collection binding and retrieval service. |
| [Index sync](../../crates/server/src/domains/knowledge_indexes/source_sync/mod.rs) | GitHub checkout, text-extension filtering, fresh document/chunk IDs, re-embedding every run and namespace replacement. Stored hashes do not make this incremental. Markkit is not used. |
| [Index repository](../../crates/server/src/storage/repositories/knowledge_indexes/mod.rs) and [retrieval](../../crates/server/src/domains/knowledge_indexes/search/mod.rs) | PostgreSQL stores chunk text and retrieval hydrates it there. Originals and a mountable tree are not the index substrate. |
| [Filesystem contract](../../crates/contracts/src/runtime/session_files.rs) and [MountFs](../../crates/contracts/src/runtime/mount_fs.rs) | Existing read, list, stat, grep and path authority. Add a knowledge-backed mount, not a parallel path resolver. |
| [Bashkit adapter](../../crates/integrations/src/bashkit/mod.rs) | Already implements `SearchCapable`/`SearchProvider` for recursive grep. Ranked retrieval needs a separate host-supplied operation. |
| [BlobStore](../../crates/server/src/storage/blob_store.rs), [VectorStore](../../crates/contracts/src/vector_store.rs), [platform selection](../../crates/server/src/platform.rs) | Useful independent byte/search boundaries. Current blob reads buffer whole objects; the platform selects one vector-store handle from deployment configuration. |
| [OKF parser/exporter](../../crates/server/src/domains/knowledge_bases/okf.rs) and [static data knowledge](../../crates/capabilities/src/capabilities/data_knowledge.rs) | Reuse parsing/hardening experience. Current mapping/export and scaffold still contain v0.1 assumptions; preserving complete v0.2 metadata needs work. |

Keep source links authoritative as code changes. The table is the starting gap
analysis, not a second API or schema reference.

## Collection, source, file and representation

A collection is the user-facing resource, scoped to an organization with explicit
reader grants through the common resource-access contract. It contains one or more sources,
each under an explicit collection-relative root. A manually edited collection
is a managed source, not another product. Two sources cannot silently overwrite
the same path; overlapping roots are rejected initially.

A file has a stable identity, a path, a source identity and immutable content
versions. Renames change paths without inventing new content where the provider
supplies identity or reliable rename evidence. Git path identity is a fallback;
identical hashes alone cannot distinguish a rename from two legitimate copies.

The original bytes are canonical, including for Markdown. Derived representations
may include extracted Markdown, text, previews, OCR, transcripts and eventually
modality-specific fragments and embeddings. Each records its producer/version,
input version, warnings and location mapping. A binary is a valid file even when
no extractor supports it. Storage support, extraction support and semantic-search
support are distinct states shown in the UI and API.

Source-backed files are read-only; managed files are editable through management
APIs. Agent knowledge mounts default to read-only in both cases. Editing a
curated note creates a file version and schedules the same incremental pipeline.
Writable agent context remains [Memory](memory.md); this proposal does not rebuild
Memory or turn external knowledge into agent-editable memory. Reuse storage and
source-fetch machinery without maintaining two copies of a source solely for search.

## Optional indexing and explicit exclusions

Offer three indexing choices: **off**, **lexical**, and **hybrid**. Lexical indexing
does not require an embedding model. Hybrid indexing selects an organization
indexer and an embedding profile. Default a newly created collection to off
unless the creation flow explicitly selects indexing and its costs.

Keep these independent:

* **Ingestion rules** select which provider files are retained at all.
* **Indexing rules** select which retained files and representations enter search.
* **Read grants** decide who can enumerate or read retained content.

Collection defaults may be narrowed by source/path/MIME rules and per-file
overrides. Organization restrictions and explicit denies win. Report the effective
decision and reason: excluded, unsupported, pending, partial, ready or failed.
An indexing exclusion is not an access restriction: a permitted agent may still
read or grep an unindexed mounted file. A file that must be inaccessible needs
an access deny or ingestion exclusion.

Turning indexing off stops serving its search projection immediately, cancels
new index work and schedules projection cleanup; original content stays readable.
Turning it on backfills retained versions without downloading the source again.
Expose source-sync freshness and index freshness separately.

## Connections and source adapters

Use the existing [GitHub model](../integrations/github-model.md), scoped
[resource access](../security/resource-access.md) and connection resolver.
Knowledge must not introduce another GitHub login, token store or implicit
identity-selection order. Each source attachment names its authorized connection
and background acting identity; sync requests only repository content read access.
Connection ownership, collection ownership and permission to share retained data
are separate decisions. An attachment never manufactures a grant.

The current background sync path already uses a repository-scoped GitHub request
through `get_github_token_for_owner`; replace the legacy owner-field coupling
with an explicit, durable attachment through the same scoped-access machinery.
Do not infer personal authority from a management actor or borrow another member's
connection. Jobs reauthorize when they run and resolve short-lived credentials then.

Source adapters expose paginated enumeration, byte fetching, revision/change
tokens, deletion evidence and available metadata/ACL information. Delta support
is advertised, not assumed. GitHub is the first complete adapter; generic Git,
uploads and OKF import must use the same ingestion boundary. Ship at least one
non-GitHub path before declaring it provider-independent. Drive/OneDrive,
Dropbox, object stores and web sources are follow-on adapters, not GitHub-shaped
configuration interpreted by another name.

Creating shared knowledge from a private connection must disclose and authorize
the collection's reader scope. Provider permission-removal events suppress affected
content through the common access layer. Connection disconnect stops sync;
retention and continuing access to authorized snapshots follow an explicit source
policy, not an accidental fallback to cached credentials or retained bytes.

## Extraction with Markkit

[Markkit](https://github.com/everruns/markkit) is the default document extraction
library. Fetch content through the source adapter, then pass bytes and MIME/name
hints to Markkit with network fetching disabled in the extractor. Keep raw
UTF-8 code and Markdown faithful rather than routing them through HTML conversion.

At review time Markkit supports HTML, DOCX and spreadsheets; PDF remains an
unsupported placeholder. Enable the required format features explicitly. Preserve
its detected format, title, warnings and truncation statistics with the resulting
representation. Run CPU-bound extraction off async executor threads and enforce
input, archive expansion, cell, output and processing-time budgets.

PDF, OCR, image understanding and audio/video transcription are explicit later
extractor capabilities, preferably added through Markkit where appropriate.
Unsupported originals remain mounted and discoverable by path/metadata. A preview
or extracted text is never presented as the original, and extracted line numbers
must not be reported as source page/slide/cell locations without a real mapping.

## Filesystem and Bashkit access

Preserve source-relative paths, filenames, case and relative links. For sources
with virtual hierarchies, store provider IDs and original names alongside a
deterministic collision-safe path mapping. Source `index.md` and `log.md` remain
ordinary retained navigation files even though OKF reserves them as non-concepts.

Mount a collection lazily through `SessionFileSystem` and `MountFs`. Enumeration
reads the metadata catalog; opening a file fetches only its bytes. Do not copy
the whole collection into session files, Bashkit memory or a sandbox at startup.
Use separate raw and derived mount roots so generated content cannot shadow a
real source filename. Both are discoverable from the collection descriptor.

File tools and Bashkit must see the same mounted paths and versions. Real
container/sandbox shells additionally need a read-only filesystem bridge or
explicit bounded materialization; a virtual mount alone does not make files
appear on a remote machine. Qualify that host separately.

Grep retains literal/regex line-match semantics. It searches original text by
default; extracted document text is an explicit derived view, with links back
to the original binary and its extraction warnings. Grep works with indexing
off using a bounded scan. An exact-search accelerator may select candidates,
but must verify matches and preserve complete-match/truncation semantics.
BM25 or semantic top-k results cannot substitute for recursive grep.

Reuse Bashkit's existing search-provider seam for accelerated exact search.
For ranked retrieval, register a host-provided `knowledge search` extension
with lexical/semantic/hybrid modes, path and metadata filters, bounded output,
cursors and revision-bearing citations. The host extension implements this spelling. Its neutral host interface belongs in Bashkit; Everruns
supplies the authorized collection resolver, not vendor credentials to scripts.
The typed search tool, shell command and automatic retrieval call one service.
Use the existing [command tree](../execution/command-tree.md) and
[tools in shell](../execution/tools-in-shell.md) adapters rather than independent
authorization or retrieval implementations.

## Progressive disclosure, including automatic retrieval

Initial context contains only attached collection names, descriptions, available
operations, mount roots and freshness. Large catalogs are themselves paginated
or searched; attaching many collections cannot dump all file metadata into a prompt.
The agent can then request directory summaries, document metadata, passages,
surrounding sections and finally bounded original/derived file content.

Honor authored OKF directory indexes first. Generate missing directory listings
from paths and descriptions; optional generated summaries are versioned derived
content, attributed as generated and invalidated when their dependencies change.
Never overwrite an authored index with a generated one.

Automatic disclosure is a configurable pre-reasoning retrieval step over only
attached, permitted collections. Use the current request and explicit task state,
with limits on collections queried, candidates, expanded neighbors, tokens,
latency and embedding spend. A small matching passage may expand to its heading,
parent overview or one linked concept within that budget. Do not ingest whole
directories automatically. Deduplicate against content already in context.

Record queries, selected immutable versions, citations, warnings and usage as
retrieval evidence. Reuse it on replay rather than rerunning a changing search
as if it were deterministic. Surface stale/partial results and retrieval errors;
allow on-demand file access when optional retrieval fails. Automatic retrieval
never enables indexing, exports content to a new provider or widens access.
External text remains evidence, including when it resembles agent instructions.

## OKF and general metadata

Target [upstream OKF v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md).
Preserve original frontmatter, body, hierarchy, links and unknown keys/types;
normalized searchable fields are a projection. Keep descriptions separate from
body text and do not force free-form types into a closed entry-kind taxonomy.
Retain provenance, generated/verified attribution, lifecycle/freshness metadata
and resource references without treating them as authorization or executable code.

Support metadata on arbitrary binaries through provider attributes and managed
sidecars. Keep source metadata, user annotations and generated metadata in
separate namespaces with explicit precedence and provenance. Index only selected
bounded fields; retaining arbitrary metadata does not imply arbitrary backend
filter schemas. Unknown or malformed optional metadata warns without losing bytes.

OKF-aware import/export preserves reserved navigation files and relative links.
An ordinary file collection need not be valid OKF. Export retained files as a
file bundle; export curated OKF concepts as OKF, without claiming that binary
assets themselves are OKF concept documents. Never use `resource` alone as a
unique concept identity: multiple concepts can describe the same asset.

## Predefined everruns-docs

Provide a built-in collection with stable catalog name **everruns-docs**. Resolve
deployment-local resource IDs by name, following existing built-in resource
patterns. Its source is public product documentation under `everruns/everruns/docs`,
with an explicit revision and canonical docs URLs; internal engineering knowledge
and private cloud operations are not part of this corpus.

Ship a versioned file snapshot with the distribution so file reading and grep
work without a GitHub connection or embedding credential. Offer explicit refresh
to the selected product release or main; release documentation is the default
for a released self-hosted version. Display source revision and freshness.
Generated API/CLI reference may be added as separately attributed public inputs.

Make it available in the collection picker and attach it to the platform's
Everruns-help experience. Arbitrary customer agents get it only when selected.
Hosted search is optional; enable local or org-configured indexing through the
same controls as any other collection. Sharing one immutable public snapshot
must never merge private collection metadata or queries across organizations.

## Delivery and success criteria

Deliver PR-sized vertical slices; the phases are dependencies, not deployment claims:

1. Consolidate collection lifecycle, bindings and managed/connected source intake.
   Reuse existing IDs internally where useful; do not freeze undeployed public
   names solely to preserve the split. Define a dev-data conversion/reset path
   and update affected API/CLI/UI contracts together. Do not edit frozen migrations.
2. Establish immutable file/blob versions, Markkit representations, lazy mounts,
   exact grep and OKF metadata fidelity. Ship the built-in docs snapshot here.
3. Add lexical/hybrid organization indexers and the incremental publication
   pipeline, including BYOK. Mount/read must remain useful without either.
4. Unify tool/shell retrieval and staged disclosure; qualify automatic retrieval
   with citations, budgets, replay and cross-surface authorization parity.
5. Qualify large-source behavior and a second remote source adapter. Roll out
   the single feature only after the required acceptance cases pass.

Required acceptance cases include an unindexed mounted collection; a changed
file among many unchanged files; original DOCX/spreadsheet/binary round-trips;
unknown OKF metadata and navigation preservation; exact grep without embeddings;
equivalent tool/shell permissions; scoped GitHub connection reuse; separate org
search accounts; offline docs access; and measured prompt/retrieval budgets.
Failure, deletion, ACL removal, duplicate events and worker restarts must not
produce mixed-version citations or reveal another org's content.

## Requirement coverage

| Request | Owning contract |
| --- | --- |
| 1–2: indexable or unindexed, single feature | Optional indexing; collection model. |
| 3: incremental indexing | [Indexing pipeline](knowledge-indexing.md#incremental-work-and-publication). |
| 4: GitHub and other connections | Connections and source adapters. |
| 5–6: non-text and file structure | Representations; extraction; filesystem. |
| 7–9: mounts, grep, Bashkit indexed search | Filesystem and Bashkit access. |
| 10: large sources | [Scale qualification](knowledge-indexing.md#large-source-qualification). |
| 11: predefined everruns-docs | Predefined everruns-docs. |
| 12: progressive and automatic disclosure | Progressive disclosure. |
| 13: OKF and metadata | OKF and general metadata. |
| 14: org indexer BYOK | [Organization indexers](knowledge-indexing.md#organization-indexers-and-byok). |
| 15: chunk and binary storage | [Storage ownership](knowledge-indexing.md#storage-ownership). |
| 16: Markkit | Extraction with Markkit. |
