---
title: Retrieval Citations
description: Attach claim-level citations to an agent's answer from its knowledge retrieval results, so each grounded sentence links back to the source that supports it.
---

| | |
|---|---|
| **ID** | `citation_retrieval` |
| **Category** | Knowledge |
| **Features** | `citations` |
| **Dependencies** | None |

Turn the sources an agent retrieves into **claim-level provenance** on its answer. When the agent grounds a reply in a knowledge search, `citation_retrieval` links each grounded sentence to the passage that backs it, and the UI renders those links as inline numbered chips with a hover preview and a deduped **Sources** strip.

It contributes **no tools** and **no system prompt**, and it never rewrites the model's answer. After the agent responds, it inspects the turn's retrieval results, aligns each retrieved passage to the sentence it best supports, and attaches a citation there. Alignment is deterministic token overlap, no extra model call, so it is model- and provider-agnostic.

## How it looks

Grounded sentences get an inline numbered chip. Hovering a chip previews the source (title, snippet, and link); a deduped **Sources** strip sits below the message. When [Citation Verification](/capabilities/citation-verification/) is also enabled, each source carries a faithfulness badge.

![An assistant message with inline numbered citation chips on grounded sentences, a hover popover previewing the cited source's title, snippet, and URL, and a "Sources" strip below the message listing three deduped sources with verified / unsupported / unverified badges.](./citations-ui.png)

## Feed

Reads the results of the agent's knowledge retrieval tools:

| Tool | Source shape |
|---|---|
| `search_index` | Knowledge-index chunks (`kchk_…`), `source_uri`, `document_title`, `snippet`, `location` |
| `search_knowledge` | Knowledge-base entries (`kbe_…`), `resource`, `title`, `snippet` |

Both shapes are normalized into the shared citation envelope. A retrieval result with no usable snippet is skipped, there is nothing to align a claim to.

> **This capability is a no-op on its own.** It cites what the agent retrieves, so it only produces citations when the agent also has a retrieval capability (a knowledge index or knowledge base) that surfaces `search_index` / `search_knowledge` results. With no retrieval feed, there is nothing to cite and the answer is unchanged.

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `min_overlap` | number (0–1) | `0.5` | Minimum token-overlap ratio for a retrieved passage to attach to a sentence. Overlap is `shared tokens / passage tokens`, so `0.5` means at least half the passage's distinctive words appear in the sentence. |

Raise `min_overlap` for stricter, higher-precision attachment (fewer chips, each more defensible); lower it for broader coverage when the model paraphrases retrieved text.

## Notes

- **No answer rewrite**: the streamed answer is never changed; annotations attach to sentence spans after generation.
- **Alignment is lexical**: a sentence that paraphrases a source heavily enough to fall below `min_overlap` will not be cited. This favors precision over recall.
- **Enabled by default**: `citation_retrieval` is part of the generic (default) harness, so any agent with a retrieval feed gets citations automatically.
- **Persistence**: annotations ride the message in the event log, so they survive reload, forking, and session export, no separate store.
- **Org scoping**: sources are derived only from already-authorized retrieval results, so a citation can never reference a document the requesting org cannot read.

## See Also

- [Citation Verification](/capabilities/citation-verification/), stamp a faithfulness verdict on each citation
- [Capabilities Overview](/capabilities/)
