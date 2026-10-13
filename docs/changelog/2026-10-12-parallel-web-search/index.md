---
title: "Parallel joins Brave as a Web Search provider"
description: "Connect a Parallel API key as a search account, and the web_search tool sends searches to Parallel's Search API, with excerpts the model can quote."
date: 2026-10-12
kind: feature
significance: minor
products: [framework, platform, cloud]
surfaces: [ui, api, code]
release: "0.48.0"
links:
  docs: [/capabilities/web-search/, /integrations/parallel/]
  prs: [everruns/everruns#4635]
---

[Parallel](/integrations/parallel/) is now a Web Search provider next to Brave Search. Connect a Parallel API key as an
organization search account in **Registers > Web search**, then pick it in an agent's Web search
settings or place it in the Automatic order. The agent keeps its one `web_search` tool; searches
go to Parallel's Search API in its fast mode and come back with longer excerpts the model can
quote. Allowed sites, blocked sites and freshness are applied by Parallel itself.

In the Framework, `WebSearch::parallel()` reads the `PARALLEL_API_KEY` environment variable, and
Automatic tries Parallel after Brave when a key is set:

```rust ignore
let agent = Agent::builder()
    .capability(WebSearch::parallel().limit(5))
    .build()?;
```

See [Web Search](/capabilities/web-search/).
