---
type: Specification
title: "Open Knowledge Format (OKF) Adoption"
description: "Open Knowledge Format (OKF) import/export adoption for Knowledge Bases."
tags:
  - everruns
  - runtime-resources
---
# Open Knowledge Format (OKF) Adoption

## Abstract

[Open Knowledge Format (OKF)][okf] is a vendor-neutral interchange format for
the metadata, context, and curated insight that surrounds data and systems. An
OKF *bundle* is a directory tree of markdown files with YAML frontmatter,
distributable as a git repo, tarball, or subdirectory. It is deliberately
minimal: the only required field is `type`, links between concepts are plain
markdown links, and consumers must degrade gracefully on anything missing.

OKF is, for everruns, an **import/export boundary** for the existing
[Knowledge Bases](knowledge-bases.md) feature and the `data_knowledge`
capability, not a new storage architecture. Our database-backed model
(Postgres FTS, stable `kb_`/`kbe_` IDs, org scoping, lifecycle, optional
embeddings) remains the runtime ground truth and is strictly better for serving
agents than loose files. OKF lets that knowledge round-trip to and from the
"metadata as code" world that data teams and other agents already live in.

This spec is the durable design intent for OKF adoption. It is delivered across
several PRs (see Delivery Plan). It captures *why* and *what*; field-level OKF
detail lives in the upstream [SPEC.md][okf-spec] and is not duplicated here.

## Motivation

OKF and everruns Knowledge Bases independently converged on the same pattern:
curated, agent-consumable knowledge expressed as small markdown documents with
a light type taxonomy. The article that prompted this work ([Google Cloud][okf])
even cites `AGENTS.md`/`CLAUDE.md`, conventions this repo already uses, as
inspiration. Adopting OKF gives us:

* **Interop in.** Ingest bundles produced by others, including Google's
  reference enrichment agent (auto-walks BigQuery → OKF) and the published
  sample bundles (GA4 e-commerce, Stack Overflow, Bitcoin).
* **Knowledge as code.** Let users keep KB content in a git repo and sync it
  in, reviewed like code.
* **Interop out.** Hand an agent's curated working set to any OKF consumer as a
  portable bundle, with no everruns account or SDK required.
* **A shared vocabulary.** Align our internal terminology with an emerging open
  standard rather than inventing a parallel one.

## Terminology Mapping

OKF terminology is adopted as the **public/interchange** vocabulary. Internal
identifiers (`knowledge_base`, `knowledge_entry`, `kind`) are unchanged to avoid
a churny rename; docs and UI copy lead with OKF terms where they help users.

| OKF concept            | everruns equivalent                                             | Notes |
|------------------------|-----------------------------------------------------------------|-------|
| Bundle                 | A Knowledge Base (`kb_…`)                                       | One bundle ⇆ one KB on import/export. |
| Concept document       | A Knowledge Entry (`kbe_…`)                                     | One `.md` file ⇆ one entry. |
| `type` (frontmatter)   | `kind` (`note`/`table`/`business`/`query`/`runbook`)            | See Type Mapping. |
| `title`                | `title`                                                         | 1:1. |
| `description`          | first paragraph of `body` / dedicated field                    | Stored at head of `body`; see Gap Closure. |
| `tags`                 | `tags`                                                          | Lowercased on import; ≤ 16 retained. |
| `timestamp`            | `updated_at`                                                    | Read on import for ordering; we own `updated_at`. |
| `resource` (frontmatter) | `resource` (new column, Gap Closure)                          | Also the primary idempotency key. |
| Markdown links         | entry relationships (new, Gap Closure)                          | Resolved to `kbe_` IDs on import; emitted bundle-relative on export. |
| `index.md`             | derived on export; ignored as a concept on import              | Reserved filename, no entry created. |
| `log.md`               | derived from audit log on export; ignored on import            | Reserved filename, no entry created. |
| `# Citations` section  | preserved verbatim in `body`                                   | No structured model in v1. |
| `okf_version`          | emitted as `"0.1"` in root `index.md`; tolerated on import     | |

### Type Mapping

OKF `type` is a free-form short string; our `kind` is a closed enum. Mapping is
lossy and **must round-trip the original**: the raw OKF `type` is preserved so
export reproduces it.

Import (`type` → `kind`), case-insensitive substring match, default `note`:

| `type` contains            | `kind`     |
|----------------------------|------------|
| `table`, `dataset`, `view` | `table`    |
| `metric`, `business`, `kpi`, `definition` | `business` |
| `query`, `sql`             | `query`    |
| `playbook`, `runbook`, `procedure` | `runbook` |
| (anything else)            | `note`     |

To make export faithful, the original `type` string is stored as a reserved
tag `okf:type=<raw>` (lowercased, excluded from the ≤16 user-tag budget and
hidden in the UI). On export, if present it takes precedence over the `kind`→
`type` default. This keeps the schema change minimal while preserving fidelity.
If a cleaner home for it emerges in Gap Closure (e.g. a `source_type` column),
migrate to that; the reserved-tag approach is the floor, not the ceiling.

## Gap Closure

OKF features we do not model today, and how each is closed (all additive; no
breaking migration):

1. **`resource` URI.** Add nullable `resource TEXT` to `knowledge_entries`.
   Surfaced in API/UI and used as the primary import idempotency key.
2. **Relationships / links.** OKF concepts link via markdown links forming a
   graph richer than the directory tree. Add a `knowledge_entry_links` table
   (`from_entry_id`, `to_entry_id`, nullable `label`) populated on import by
   resolving bundle-relative/relative links to `kbe_` IDs (unresolved links are
   dropped, per OKF "consumers must tolerate broken links"). Exposed read-only
   in the entry API and re-emitted as markdown links on export. The
   `search_knowledge` tool may later traverse links for one-hop expansion.
3. **`index.md` / `log.md`.** Reserved files; never imported as entries.
   Derived on export only, `index.md` from KB structure (grouped by `kind`
   with descriptions), `log.md` from the audit log for the KB.
4. **Citations.** Preserved verbatim inside `body`; no structured model in v1.
5. **Raw `type` preservation.** Via the `okf:type=` reserved tag (see Type
   Mapping), pending a possible dedicated column.

Open: whether `resource` should be UNIQUE per KB. Leaning **no** (OKF does not
require uniqueness, and two concepts may legitimately point at one asset);
idempotency instead keys on `(kb_id, resource)` when `resource` is present and
falls back to `(kb_id, source_path)` otherwise (see Importer).

## Importer

**Decision: on-demand, idempotent sync in v1, not a background watcher.**

The importer parses an OKF bundle (uploaded tarball or a referenced git repo
URL) and upserts entries into a target KB. It is re-runnable and idempotent, so
re-importing an updated bundle converges the KB to the bundle state without
duplicates. This delivers most of the value of a "live" connector with zero new
infrastructure (no credential storage, git polling, or webhook plumbing).

* **Idempotency key:** `(kb_id, resource)` when frontmatter `resource` is
  present; otherwise `(kb_id, source_path)` where `source_path` is the
  bundle-relative file path. The source path is recorded (reserved tag
  `okf:path=<path>`) so path-keyed entries remain stable across re-imports.
* **Per-file mapping:** frontmatter → fields per Terminology Mapping; body
  stored verbatim; reserved files (`index.md`, `log.md`) skipped; links queued
  for a second resolution pass after all entries exist.
* **Conformance:** accept any bundle where every non-reserved `.md` has
  parseable frontmatter with a non-empty `type`. Tolerate unknown types,
  unknown keys, missing optional fields, and broken links (warn, don't fail).
* **Sync semantics:** default `upsert` (create/update matched, leave others).
  An explicit `prune` option archives KB entries that originated from a prior
  import (tracked via `okf:path`/`resource`) and are absent from the new
  bundle, giving full mirror behavior when desired.
* **Surface (implemented):** `POST /v1/knowledge-bases/{kb_id}/okf_import`
  accepting JSON with either inline `files` (`[{path, content}]`) or a base64
  `.tar.gz` `bundle_base64`, plus an optional `prune` flag. Synchronous.

**Future "live" / git mode** (explicitly out of v1, *not implemented*): a
`{ "git_url": "...", "ref": "..." }` input and/or a scheduled task
(`knowledge/operations/scheduled-tasks.md`) or git webhook that calls the same idempotent sync
on a cadence. The idempotency-key design above is what makes this a drop-in
follow-up rather than a rewrite.

Security: bundle parsing runs untrusted input, enforce the 64 KiB body cap per
entry, per-file and total decompressed caps (gzip-bomb defense), path traversal
rejection on both inline `files[].path` and tar entries, and the cross-org
guarantees from `knowledge/security/threat-model.md`. A future git-URL input would obey
`knowledge/operations/network-access.md` (no network egress in v1).

## Exporter

`GET /v1/knowledge-bases/{kb_id}/okf_export` streams a conformant OKF bundle as
a tarball (`.tar.gz`). Layout:

* One `.md` per entry under a `kind`-derived subdirectory
  (`tables/`, `business/`, `queries/`, `runbooks/`, `notes/`); filename derived
  from `okf:path` when present, else slugified title.
* Frontmatter from fields, with `type` reconstructed (raw `okf:type` if present,
  else `kind`→`type` default), `resource`, `tags` (reserved `okf:*` tags
  stripped), and `timestamp` from `updated_at`.
* Relationships re-emitted as a `# Related` section of bundle-relative links.
* Root `index.md` with `okf_version: "0.1"` frontmatter and a grouped concept
  listing; `log.md` derived from the audit log.

Round-trip property (tested): export → import into a fresh KB reproduces
entries, kinds, raw types, tags, resources, and resolvable links.

## `data_knowledge` as OKF Consumer

The `data_knowledge` capability mounts a readonly `/knowledge/{tables,business,
queries}` scaffold (`crates/platform/src/capabilities/data_knowledge.rs`). It is
made an OKF consumer so the mounted tree *is* a conformant OKF bundle and the
same content is portable:

* Mounted files gain YAML frontmatter (`type`, `title`, `description`) and the
  scaffold gains `index.md` files for progressive disclosure.
* When the capability is bound to a KB (existing config direction in
  `knowledge-bases.md`), the mount is rendered from that KB via the exporter's
  bundle layout instead of static READMEs, so agents reading `/knowledge/**`
  see live curated content as OKF.
* The capability `system_prompt_addition` is updated to tell agents the mount is
  an OKF bundle (frontmatter + `index.md` navigation), not just loose files.

Existing harnesses that grep `/knowledge/**` keep working; OKF framing is
additive.

## Showcase Agent

A demonstrable end-to-end use case for the landing announcement:

* Seed a KB by importing a published OKF sample bundle (e.g. the GA4
  e-commerce bundle) via the importer.
* An agent bound to `knowledge_base` (and/or `data_knowledge` rendering that KB)
  answers a business question, citing entries by `kbe_` ID and traversing a
  join-path link between concept documents.
* Captured as a manual UI test under `test_cases/okf/` per
  `knowledge/evaluation/test-cases.md`, and as a screen recording for the announcement.

## Landing Announcement

Final step posts to `everruns/landing`: an announcement of OKF support with the
showcase example and, ideally, the screen recording of the agent. Requires
adding the `everruns/landing` repo to the working scope.

## Delivery Plan

Each item is an independently reviewable change (committed PR-sized). Status
reflects what has shipped on the OKF-adoption branch.

1. ✅ **This spec + terminology**: `knowledge/runtime-resources/okf-adoption.md`, README index entry,
   cross-link from `knowledge-bases.md`.
2. ◑ **Gap closure**: shipped: additive migration `076` adds `resource` to
   entries (models, storage parity, API, OpenAPI); `index.md`/`log.md` reserved
   semantics handled by importer/exporter. **Deferred:** the
   `knowledge_entry_links` relationship table (export emits a flat bundle; links
   are not yet modeled).
3. ✅ **Importer**: `POST …/okf_import`: parser, idempotent upsert/prune,
   inline + base64 tarball, hardening, unit + idempotency tests.
4. ✅ **Exporter**: `GET …/okf_export`: bundle writer + root `index.md`,
   round-trip test.
5. ◑ **`data_knowledge` consumer**: shipped: OKF-framed static mount
   (`index.md` navigation + `okf_version`, prompt framing). **Deferred:**
   rendering the mount from a bound KB (needs session-mount-time DB access).
6. ◑ **Showcase**: shipped: how-to guide, API round-trip test case, Data
   Analyst showcase test case, and the `search_knowledge` runtime tool (below)
   so agents actually consume imported OKF knowledge. **Pending:** a live
   end-to-end run + recording need a running stack.
7. ⏳ **Landing announcement**: `everruns/landing` PR + example + recording.
   Requires `everruns/landing` access and a running stack for the recording.

### `search_knowledge` runtime tool (shipped)

The agent-facing `search_knowledge` tool lets an agent *read* imported OKF
knowledge. Implemented as a cross-crate slice:

* Platform trait `KnowledgeStore` + `KnowledgeSearchHit` result type
  (`crates/platform/src/knowledge_store.rs`), resolved through the typed
  `ToolContext` extension boundary owned by `everruns-core`.
* Server impl over `StorageBackend`
  (`crates/server/src/knowledge_store.rs`), wired via `DirectWorkerAdapters`. It
  resolves each KB public id within the caller's org (cross-org ids silently
  skipped, no existence leak) and calls `search_knowledge_entries`. Public
  org-id → internal id uses `org_internal_id_from_public`.
* The tool on `KnowledgeBaseCapability::tools_with_config`
  (`crates/platform/src/capabilities/knowledge_base.rs`), reading `bases`/`kinds`
  from config, unit-tested against a mock `KnowledgeStore`.
* The embedded `InProcessRuntime` path is not yet wired (returns no store), so
  the tool is currently active on the server/worker execution path.

## Security

Inherits `knowledge/security/threat-model.md`. New surface area:

* **Untrusted bundle import:** per-file + total decompressed caps (gzip-bomb
  defense), body 64 KiB cap, path-traversal rejection on inline `files[].path`
  and tar entries, frontmatter parsing hardened against malformed YAML.
* **Git URL fetch (future, not implemented):** a future git-URL input would be
  subject to `knowledge/operations/network-access.md` allowlist/blocklist. v1 has no network
  egress, only inline files and base64 tarballs.
* **Cross-org:** import/export are org-scoped like all KB CRUD; errors never
  leak existence of other-org KBs.
* **Export disclosure:** export reflects only the requesting org's KB content;
  `log.md`/audit-derived content is filtered to org-visible audit entries.

## Open Questions

* Should `resource` be unique per KB? (Leaning no; see Gap Closure.)
* Is the reserved-tag (`okf:type=`, `okf:path=`) approach acceptable long-term,
  or should raw type / source path become first-class columns?
* Should live (scheduled/webhook) sync ship as a fast follow, or wait for
  demand? (v1 is on-demand by decision above.)
* Should relationship links influence `search_knowledge` ranking / one-hop
  expansion in v1, or strictly later?

[okf]: https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing
[okf-spec]: https://github.com/GoogleCloudPlatform/knowledge-catalog/tree/main/okf
