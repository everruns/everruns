---
title: "Web search answers cite their sources, sentence by sentence"
description: "The Web Citations capability links each sentence an agent backs with a web search result to that page, from the web_search tool or the model's own search."
date: 2026-10-12
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/capabilities/citation-web/]
  prs: [everruns/everruns#4625]
---

Add **Web Citations** (`citation_web`) to an agent that has Web Search, and each sentence the
model backs with a search result links to that page. Chat shows the numbered source chips,
hover preview and **Sources** strip that retrieval citations already use.

```json
{
  "capabilities": [
    {
      "ref": "web_search"
    },
    {
      "ref": "citation_web"
    }
  ]
}
```

It reads the URLs and result ids the model cites from `web_search` results, and the citations
OpenAI, Anthropic, Gemini and OpenRouter return when the model searches on its own. Matching is
exact, and a citation that cannot be placed on the answer's text is dropped. The annotations are
on the message for the API and evals, and Citation Verification checks them like any other
citation.

See [Web Citations](/capabilities/citation-web/).
