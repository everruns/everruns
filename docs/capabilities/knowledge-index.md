---
title: Knowledge Index
description: Bind an agent to Knowledge Indexes, source-backed collections that are synced, chunked, and embedded, and give it a search_index tool that returns cited passages.
---

| | |
|---|---|
| **ID** | `knowledge_index` |
| **Category** | Knowledge |
| **Tools** | `search_index` (only when `indexes` is set) |
| **Features** | `knowledge` |
| **Dependencies** | None |
| **Risk** | Medium |

A Knowledge Index connects an external source, a GitHub repository today,
syncs its documents, splits them into chunks, and embeds each chunk. The
`knowledge_index` capability binds an agent to one or more indexes and adds
the `search_index` tool, which searches by meaning and returns passages with
the location they came from.

Use it for a large corpus that lives elsewhere and changes there, such as a
documentation repository. For short entries people curate by hand, use
[Knowledge Base](/capabilities/knowledge-base/).

The risk level is Medium because retrieved passages are external content that
enters the agent's context. The tool description tells the model that passages
are data, not instructions.

## Configuration

```json
{
  "ref": "knowledge_index",
  "config": {
    "indexes": ["kidx_0193f0c2a1b27c4e8f5d6a7b8c9d0e1f"],
    "top_k": 10
  }
}
```

| Field | Description |
|---|---|
| `indexes` | Knowledge Index IDs the agent may search, each `kidx_` followed by 32 lowercase hex characters. Duplicates are rejected. |
| `top_k` | Optional default result count, 1 to 50. Defaults to 10. |

With no `indexes`, the capability contributes no tool.

## `search_index`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `query` | string | Yes | Natural-language query |
| `indexes` | string[] | No | A subset of the configured index IDs. IDs outside the configured set are ignored, so the model cannot widen its access. |
| `top_k` | integer | No | 1 to 50; overrides the configured default |

Each result is a citation:

| Field | Description |
|---|---|
| `id` | Chunk ID (`kchk_...`), stable while the passage persists across syncs |
| `index_id` | The Knowledge Index it came from |
| `document_title` | Title of the source document, when known |
| `source_uri` | Locator for the document, for example `github://owner/repo@main/docs/x.md` |
| `location` | Position within the document, such as a line range |
| `snippet` | The start of the passage |
| `score` | Relevance, higher is better; use it for ordering only |

Each search embeds the query once per bound index, because indexes can use
different embedding models. Every embedding call is recorded as an
`llm.generation` event tagged `embedding`, so query-time spend counts toward
session usage and budgets like any other model call.

## Managing indexes

Indexes are managed on the **Knowledge Indexes** page of the UI or through the
`/v1/knowledge-indexes` API. An index needs:

- a GitHub source (`repository`, plus optional `branch` and `root_folder`),
  read through the organization's GitHub connection
- an embedding model from the organization's model catalog whose provider
  supports embeddings

Trigger a sync with `POST /v1/knowledge-indexes/{index_id}/sync` and list the
ingested documents with `GET /v1/knowledge-indexes/{index_id}/documents`. The
[API reference](/api/) has the request shapes.

## See also

- [Knowledge Base](/capabilities/knowledge-base/), curated entries with keyword search
- [Memory](/capabilities/memory/), source-backed files mounted into the workspace
- [Retrieval Citations](/capabilities/citation-retrieval/), citations built from `search_index` results
