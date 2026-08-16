---
type: Specification
title: "Knowledge Bases Specification"
description: "Curated organization knowledge."
tags:
  - everruns
  - runtime-resources
---
# Knowledge Bases Specification

## Abstract

Knowledge Bases are org-scoped, named collections of curated text **entries**
(facts, table docs, business rules, validated SQL templates, runbooks, etc.)
that users manage through a dedicated UI and that agents can search through a
`search_knowledge` tool. Knowledge Bases interoperate with the [Open Knowledge Format](okf-adoption.md)
(OKF) for import/export ("knowledge as code"); see `knowledge/runtime-resources/okf-adoption.md`.

Knowledge Bases sit alongside one adjacent primitive:

* **Memory** (`knowledge/runtime-resources/memory.md`), org-scoped, named stores mountable into
  session workspaces. Files-only today; emphasizes file fidelity and direct
  file edits. Knowledge Bases may fold into Memory as a `structured` surface
  later, see open questions in `knowledge/runtime-resources/memory.md`.

Knowledge Bases fill the curation-first slot: small, structured, human-edited
entries that an agent should consult **before** generating output, with stable
citation IDs.

This spec is the durable design intent for [EVE-423]. Implementation is
delivered across multiple PRs:

* Foundational PR, entity, ID schema, DB schema, capability registration,
  CRUD HTTP API. The `data_knowledge` capability remains the runtime
  ground-truth surface and is unaffected.
* Follow-up PR, agent-facing `search_knowledge` tool, capability config UI,
  KB management UI page, and migration path from `/knowledge/**` starter
  files into a default Knowledge Base.

## Motivation

`data_knowledge` mounts a readonly `/knowledge/{tables,business,queries}`
scaffold and the data analyst harness greps the files. That works as a
workaround, but:

* No org-level UI; entries can only be added by editing capability code.
* No semantic or hybrid search; agents fall back to keyword grep.
* No dedicated tool name (`search_knowledge`); agents discover the directory
  via system-prompt hints alone.
* No stable citation IDs that survive renames or filesystem changes.

Curated knowledge is a first-class product surface. It needs the same lifecycle
guarantees (active/archived/deleted) and org-scoping as Memory, and a dedicated
agent-facing affordance (typed search tool, structured responses).

## Concepts

| Name              | Description                                                       |
|-------------------|-------------------------------------------------------------------|
| **Knowledge Base** | Org-scoped, named container for curated entries. Public ID prefix: `kb_`. |
| **Knowledge Entry** | A single curated unit: title, body (markdown), kind, tags. Public ID prefix: `kbe_`. |
| **Kind**          | Lightweight taxonomy: `note` (default), `table`, `business`, `query`, `runbook`. |
| **Capability config** | `knowledge_base` capability binds an agent/harness to one or more KBs. |

Boundary with adjacent primitives:

| Question                           | Memory           | Knowledge Base |
|------------------------------------|------------------|----------------|
| Who writes the content?            | User (files)     | User (entries) |
| Primary access pattern             | Filesystem reads | Search         |
| Granularity                        | Files/dirs       | Short docs     |
| Mounted into `/workspace`?         | Yes              | No             |
| Stable citation IDs?               | Path-based       | Yes (`kbe_`)   |

## Lifecycle

Knowledge Bases follow the standard building-block lifecycle from
`knowledge/foundations/models.md`:

* `active`, assignable and editable.
* `archived`, read-only, hidden from default lists, not assignable to new
  capability bindings.
* `deleted`, tombstone; detail/list APIs return 404 except for historical
  references.

Knowledge Entries inherit the lifecycle of their parent KB. A KB hard delete
cascades to its entries.

## Data Model

### `knowledge_bases`

| Column                    | Type        | Notes                                                  |
|---------------------------|-------------|--------------------------------------------------------|
| `id`                      | UUID PK     | Internal primary key.                                  |
| `org_id`                  | BIGINT FK   | Organization scope.                                    |
| `public_id`               | TEXT        | `kb_<32-hex>`. Unique per `org_id`.                    |
| `name`                    | VARCHAR     | Unique within `org_id` while not deleted.              |
| `description`             | TEXT?       |                                                        |
| `owner_principal_id`      | TEXT?       | Principal that created the knowledge base.             |
| `resolved_owner_user_id`  | UUID?       | Resolved user id, if known.                            |
| `status`                  | VARCHAR     | `active` / `archived` / `deleted`.                     |
| `created_at` / `updated_at` | TIMESTAMPTZ |                                                       |
| `archived_at` / `deleted_at` | TIMESTAMPTZ? |                                                     |
| `embedding_model_id`      | UUID FK?    | FK → `llm_models.id`. Optional embedding model for hybrid retrieval. `NULL` = keyword search only. See "Embedding Configuration" below. |

`UNIQUE(org_id, public_id)` and `UNIQUE(org_id, lower(name)) WHERE status != 'deleted'`.

### `knowledge_entries`

| Column          | Type        | Notes                                                  |
|-----------------|-------------|--------------------------------------------------------|
| `id`            | UUID PK     |                                                        |
| `kb_id`         | UUID FK     | `ON DELETE CASCADE`.                                   |
| `public_id`     | TEXT        | `kbe_<32-hex>`. Unique within `kb_id`.                 |
| `title`         | VARCHAR     |                                                        |
| `body`          | TEXT        | Markdown body. Hard cap: 64 KiB.                       |
| `kind`          | VARCHAR     | `note` / `table` / `business` / `query` / `runbook`.   |
| `tags`          | TEXT[]      | Free-form tags, lowercase, ≤ 16 entries.               |
| `created_at` / `updated_at` | TIMESTAMPTZ |                                            |

`UNIQUE(kb_id, public_id)`. A GIN index on `to_tsvector('english', title || ' ' || body)`
backs keyword search; a per-KB index on `(kb_id, kind)` backs kind filters.

## Capability: `knowledge_base`

* **ID:** `knowledge_base`
* **Name:** `Knowledge Base`
* **Category:** `Knowledge`
* **Icon:** `library`
* **Dependencies:** none (does not require `session_file_system`)
* **Features:** `knowledge`
* **Risk:** `Low`, read-only entry retrieval scoped to org-owned content.

### Config Schema

```json
{
  "bases": ["kb_abc123...", "kb_def456..."],
  "kinds": ["table", "business", "query"]
}
```

* `bases`, IDs of Knowledge Bases the agent can search. Empty/null = no
  bases bound; the tool returns an empty result set.
* `kinds`, optional default kind filter applied when the agent does not
  pass `kind` explicitly.

### Validation Rules

The foundation PR enforces structural shape only:

* Each `bases[i]` must match the `kb_<32-hex>` ID format.
* `bases[]` rejects duplicates.
* `kinds[i]` must be one of the allowed kind values.

Domain-level cross-validation (cross-org references, archived/deleted KBs)
runs at tool dispatch time in the follow-up PR that ships
`search_knowledge`. Errors must not leak existence of other-org KBs.

## Agent-facing Tool: `search_knowledge`

Ships in the follow-up PR. The contract is:

```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "description": "Keyword search across entry title and body" },
    "kind": { "type": "string", "enum": ["note", "table", "business", "query", "runbook"] },
    "tags": { "type": "array", "items": { "type": "string" } },
    "limit": { "type": "integer", "minimum": 1, "maximum": 25, "default": 10 }
  },
  "required": ["query"],
  "additionalProperties": false
}
```

Response shape:

```json
{
  "results": [
    {
      "id": "kbe_...",
      "kb_id": "kb_...",
      "title": "...",
      "kind": "table",
      "tags": ["orders", "fact"],
      "snippet": "...",
      "score": 0.83
    }
  ]
}
```

V1 uses keyword search (PostgreSQL `tsvector` + `plainto_tsquery`) restricted
to KBs in the capability config. Embedding-backed hybrid retrieval is a
follow-up; the response shape is forward-compatible.

## API

REST endpoints (see `knowledge/execution/apis.md` for conventions and OpenAPI exposure):

* `GET    /v1/knowledge-bases`
* `POST   /v1/knowledge-bases`
* `GET    /v1/knowledge-bases/{kb_id}`
* `PATCH  /v1/knowledge-bases/{kb_id}`
* `DELETE /v1/knowledge-bases/{kb_id}`, archive per lifecycle
* `GET    /v1/knowledge-bases/{kb_id}/entries`
* `POST   /v1/knowledge-bases/{kb_id}/entries`
* `GET    /v1/knowledge-bases/{kb_id}/entries/{entry_id}`
* `PATCH  /v1/knowledge-bases/{kb_id}/entries/{entry_id}`
* `DELETE /v1/knowledge-bases/{kb_id}/entries/{entry_id}`

KB list supports `?search=` and `?include_archived=` query parameters; entry
list supports `?kind=` and `?search=`.

## UI

Follow-up PR. Shape:

* Top-level **Knowledge Bases** page, list, search, archive toggle, create
  button. Lives next to **Volumes** in the building-blocks navigation.
* **Knowledge Base detail**: editable name/description, entry list grouped
  by kind, archive/delete actions.
* **Entry editor**: title, kind, tags, markdown body editor with preview.
* **Capability config UI** for `knowledge_base`, multi-select KBs and
  optional kind filter.

## Migration from `/knowledge/**` Starter Files

`data_knowledge` keeps mounting the existing scaffold so existing harnesses
do not regress. The follow-up PR adds an opt-in import action:

1. New org auto-provisioning seeds a default Knowledge Base named "Default"
   bound to the data analyst harness.
2. A one-shot `POST /v1/knowledge-bases/{kb_id}/import_starter_files` action
   walks the in-memory `data_knowledge` scaffold, creating one entry per
   README/file with `kind` derived from the directory (`tables/` → `table`,
   `business/` → `business`, `queries/` → `query`).
3. After successful import, the data analyst harness can be reconfigured to
   bind the `knowledge_base` capability instead of (or alongside)
   `data_knowledge`.

The import is idempotent on `(kb_id, title)` so repeated runs do not create
duplicates.

## Security

See `knowledge/security/threat-model.md` for the canonical entries.

* **Cross-org reference:** Capability config validation MUST reject KB IDs
  from other orgs. Errors must not leak existence.
* **Content size:** Entry body is hard-capped at 64 KiB to bound storage,
  search-index size, and tool response payloads.
* **Search injection:** Tokenized search uses parameterized `tsquery`/`ILIKE`
  patterns; user query strings are never interpolated.
* **Audit:** Knowledge Base CRUD (create, update, archive, delete) and
  capability binding changes are audited via the standard audit log domain.

## Permissions

KB CRUD is org-scoped. Standard org policies (`org.member`, `org.admin`)
apply. Entry CRUD inherits the parent KB policy. Capability binding is
gated by the same policy that gates capability configuration on the parent
agent or harness.

## Testing

* Typed-ID parsing/serialization for `KnowledgeBaseId` and `KnowledgeEntryId`.
* Storage parity (in-memory and Postgres) for `knowledge_bases` and
  `knowledge_entries`.
* CRUD lifecycle round-trip (create → list → update → archive → list with
  `include_archived`).
* Duplicate-name conflict on active rows.
* Cross-org isolation: a KB created in org A is not visible to org B.
* Capability config validation: empty config is valid; cross-org or archived
  KBs are rejected.
* API CRUD permissions and org scoping.
* OpenAPI export updated when API surface changes.
* Manual test cases (`test_cases/knowledge-bases/`), added with the UI PR.

## Embedding Configuration

**Decision (phase 6):** Embedding-backed hybrid retrieval is configured **per knowledge base**, not org-wide. The `embedding_model_id` column on `knowledge_bases` links to an embedding model in the org's model catalog.

* `NULL` means the KB uses keyword search only (current default). No embedding provider is required.
* When set, the referenced model must exist in the org and its provider driver must declare `ServiceKind::Embeddings`. The `EmbeddingsDriver` trait (implemented by `OpenAIEmbeddingsDriver` in phase 6) provides the `embed()` call used during hybrid retrieval.
* Different KBs in the same org can use different embedding models (e.g., multilingual embeddings for a multilingual KB).
* The embedding model is validated on create/update: the referenced model UUID must exist in the org's `models` table and its provider driver must declare `ServiceKind::Embeddings`.

This design is explicit and flexible: orgs pay only for the embedding providers they configure, and different KBs can use different providers independently.

## Open Questions

* Should KBs support per-KB ACLs (team-scoped), or is org scope sufficient?
* ~~Should embedding-backed retrieval be opt-in per KB, or org-wide?~~ **Resolved:** per-KB via `embedding_model_id` (phase 6, knowledge/foundations/providers.md).
* Should entries support attachments (images, files), or only inline
  markdown? Memory already supports image content parts.
* Should `data_knowledge` remain after the migration path is in place, or
  be deprecated in favor of the default Knowledge Base?

[EVE-423]: https://linear.app/everruns/issue/EVE-423
