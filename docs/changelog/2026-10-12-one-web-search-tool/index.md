---
title: "Web search: one tool, search providers and organization defaults"
description: "Agents search the web with one web_search tool. Admins connect search providers and set defaults, each agent picks its own, and free search is the fallback."
date: 2026-10-12
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.48.0"
links:
  docs: [/capabilities/web-search/]
  prs: [everruns/everruns#4551, everruns/everruns#4584, everruns/everruns#4605, everruns/everruns#4611, everruns/everruns#4614, everruns/everruns#4621]
---

Agents now search with one `web_search` tool, whichever provider answers. Worker harnesses
include it, and keyless free search answers whenever the chosen provider cannot.
On OpenAI, Anthropic, Gemini and OpenRouter models with their own search, Automatic uses it
first, billed by the provider and counted on the model call's cost.

**In the UI:** **Registers > Web search** lists the providers (model provider search with its
price, Brave Search accounts, free search) and holds the organization defaults: default provider,
Automatic order, fallback, results per search and blocked domains. In the agent editor, **Web
search** picks a provider and says how the agent will search. Chat shows each search as one row,
with every attempt and the raw response a click away. Two search tools on one agent get a warning.

**In code:** `WebSearch::auto()` in the Framework, or the capability config on a Platform agent:

```json
{
  "ref": "web_search",
  "config": {
    "provider": "brave",
    "blocked_domains": [
      "pinterest.com"
    ]
  }
}
```

**For hosts that pay for search:** the hosted provider can use per-call wallet
permits. The SDK forwards the engine's stable call identity, starts an owned
permit before dispatch, and reports a sanitized outcome for settlement. Hosts
can replay a completed result without paying for another request and repair a
lost callback from the tool's billing receipt. Cancellation leaves uncertain
spend for host reconciliation.

See [Web Search](/capabilities/web-search/).
