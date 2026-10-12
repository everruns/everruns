---
title: "Parallel answers Web Search"
description: "Connect a Parallel API key as a search account, and the web_search tool sends searches to Parallel's Search API, with excerpts the model can quote."
date: 2026-10-12
kind: feature
significance: minor
products: [framework, platform, cloud]
surfaces: [ui, api, code]
links:
  docs: [/capabilities/web-search/, /integrations/parallel/]
  prs: [everruns/everruns#4635]
---

Parallel is now a Web Search provider next to Brave Search. Connect a Parallel API key as an
organization search account in **Registers > Web search**, then pick it in an agent's Web search
settings or place it in the Automatic order. The agent keeps its one `web_search` tool; searches
go to Parallel's Search API in its fast mode and come back with longer excerpts the model can
quote. Allowed sites, blocked sites and freshness are applied by Parallel itself.

In the Framework, `WebSearch::parallel()` uses the `PARALLEL_API_KEY` environment variable, and
Automatic tries it after Brave when a key is set.

See [Web Search](/capabilities/web-search/).
