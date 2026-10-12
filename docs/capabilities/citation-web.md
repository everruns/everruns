---
title: Web Citations
description: Attach claim-level citations to an agent's answer from its web search results, whether the web_search tool or the model's own search found them.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `citation_web` |
| **Category** | Knowledge |
| **Features** | `citations` |
| **Dependencies** | None |

Turn the pages an agent found with [Web Search](/capabilities/web-search/) into **claim-level provenance** on its answer. Each sentence the model backed with a search result links to that page, and the chat renders the links as the same inline numbered chips, hover preview, and **Sources** strip that [Retrieval Citations](/capabilities/citation-retrieval/) use.

It contributes **no tools** and **no system prompt**, and it never rewrites the model's answer. After the agent responds, it reads the turn's search results and the citations the model provider returned, and attaches a citation to each cited sentence. No extra model call is made.

## Feed

| Source | How a citation is found |
|---|---|
| `web_search` function results (free search, Brave, Parallel) | The tool asks the model to cite each claim with a result's `url` or `id` (`w3f9a1c`). A URL in the answer that matches a result, or a result id, cites the sentence it sits in. A citation alone after its sentence, such as `Source: https://…`, cites the sentence before it. |
| Native search: Anthropic | Text blocks with `web_search_result_location` citations; the block is the cited claim. |
| Native search: OpenAI, OpenRouter | `url_citation` annotations on the answer text. |
| Native search: Gemini | Grounding supports and their sources. |
| `brave_web_search` results | Matched by URL, like `web_search` results. |

Matching is exact, never fuzzy. A URL matches when it names the same page: scheme and host compare without case, and a `#fragment` or trailing slash does not count. The path and query must be equal. A provider's citation keeps its place only while the text it covered is still there in the final answer; otherwise it is dropped rather than attached to the wrong words.

Each citation carries the page `url`, its title, a snippet (the result's snippet, or the provider's quoted text), and the result id as `external_id`. Native citations get the same id the `web_search` tool would give the page.

> **This capability is a no-op on its own.** It cites what the agent searched, so it only produces citations when the agent also has [Web Search](/capabilities/web-search/) (or `brave_web_search`). Answers without search results are unchanged.

## Configuration

None.

## Notes

- **Opt-in**: add `citation_web` next to `web_search`. The built-in harness presets turn on Web Search but not citations.
- **Verification**: pair it with [Citation Verification](/capabilities/citation-verification/) to stamp a faithfulness verdict on each citation, exactly as for retrieval citations.
- **No answer rewrite**: the streamed answer is never changed; annotations attach to spans after generation.
- **Persistence**: annotations ride the message in the event log, so they survive reload, forking, and session export.
- **Untrusted sources**: a citation links to a public web page the search returned; the page content is not checked beyond what the verification capability does.

## See Also

- [Web Search](/capabilities/web-search/)
- [Retrieval Citations](/capabilities/citation-retrieval/)
- [Citation Verification](/capabilities/citation-verification/)
- [Capabilities Overview](/capabilities/)
