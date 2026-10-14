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

Agents now search with one `web_search` tool, whichever provider answers. Keyless
free search is the fallback. Automatic uses a model's native search first,
billed by that provider and counted on the model call.

**In the UI:** **Registers > Web search** lists providers and organization
defaults: provider, Automatic order, fallback, result count and blocked domains.
Agents choose how to search; chat shows each attempt and its response. Duplicate
search tools get a warning.

**In code:** use `WebSearch::auto()` or a Platform capability config:

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

Hosts paying for search can install per-call wallet permits: stable engine
identity, fenced dispatch, sanitized settlement, result replay and journal
receipts for callback recovery. Uncertain spend stays with host reconciliation.

See [Web Search](/capabilities/web-search/).
