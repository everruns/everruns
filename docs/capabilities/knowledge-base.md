---
title: Knowledge Base
description: Bind an agent to curated organization Knowledge Bases and give it a search_knowledge tool for table docs, business rules, query templates, and runbooks.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `knowledge_base` |
| **Category** | Knowledge |
| **Tools** | `search_knowledge` |
| **Features** | `knowledge` |
| **Dependencies** | None |
| **Risk** | Low |

A Knowledge Base is a named, organization-scoped collection of short curated
entries. Each entry has a title, a Markdown body, a kind, and tags. The
`knowledge_base` capability binds an agent to one or more Knowledge Bases and
adds the `search_knowledge` tool, so the agent can look up entries before it
answers and cite them by their `kbe_` ID.

Use it for knowledge people write and maintain by hand: table documentation,
metric definitions, validated SQL, and runbooks. For a large corpus synced
from a repository, use [Knowledge Index](/capabilities/knowledge-index/). For
files the agent should read and edit, use [Memory](/capabilities/memory/).

## Configuration

```json
{
  "ref": "knowledge_base",
  "config": {
    "bases": ["kb_0193f0c2a1b27c4e8f5d6a7b8c9d0e1f"],
    "kinds": ["table"]
  }
}
```

| Field | Description |
|---|---|
| `bases` | Knowledge Base IDs the agent may search, each `kb_` followed by 32 lowercase hex characters. Duplicates are rejected. |
| `kinds` | Optional entry kinds: `note`, `table`, `business`, `query`, `runbook`. When the list has exactly one kind, it is used as the filter for calls that do not pass `kind`. |

With no `bases`, the tool is still present and returns an empty result.

The capability also adds a short system prompt telling the agent to consult
`search_knowledge` before answering data questions and to cite the entries it
uses.

## `search_knowledge`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `query` | string | Yes | Keywords matched against entry titles and bodies |
| `kind` | string | No | One of the five entry kinds |
| `tags` | string[] | No | Return only entries that carry every listed tag |
| `limit` | integer | No | 1 to 25, default 10 |

The search is keyword full-text search (PostgreSQL `plainto_tsquery` over title
and body), ranked by text relevance and restricted to the configured bases. A
base that does not exist in the caller's organization is skipped silently.

Each result carries the entry `id` (`kbe_...`), its `kb_id`, `title`, `kind`,
`tags`, a `snippet` of up to 400 characters of the body, and the optional
`resource` URI.

## Managing Knowledge Bases

Knowledge Bases and their entries are managed through the
`/v1/knowledge-bases` API: create a base, then add, update, and delete entries
under `/v1/knowledge-bases/{kb_id}/entries`. Bases can also be imported and
exported as Open Knowledge Format bundles; see
[Share knowledge with OKF](/how-to/share-knowledge-with-okf/). The
[API reference](/api/) has the request shapes.

## See also

- [Knowledge Index](/capabilities/knowledge-index/), semantic search over a synced source with passage citations
- [Data Knowledge](/capabilities/data-knowledge/), a read-only OKF scaffold for the Data Analyst harness
- [Retrieval Citations](/capabilities/citation-retrieval/), citations built from `search_knowledge` results
