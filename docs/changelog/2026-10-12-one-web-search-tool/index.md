---
title: "Web search: one tool, search providers and organization defaults"
description: "Agents search the web with one web_search tool. Admins connect search providers and set defaults, each agent picks its own, and free search is the fallback."
date: 2026-10-12
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/capabilities/web-search/]
  prs: [everruns/everruns#4551, everruns/everruns#4584, everruns/everruns#4605]
---

Agents now search with one `web_search` tool, whichever provider answers. The Conversation and
Worker harnesses include it, and keyless free search answers whenever the chosen provider cannot.

**In the UI:** **Registers > Web search** lists the providers (model provider search with its
price, connected Brave Search accounts, free search) and holds the organization defaults: the
default provider, the Automatic order, fallback, results per search and blocked domains. In the
agent editor, **Web search** picks a provider and says how the agent will search. Chat shows each
search as one row with the provider that answered, and a details view with every attempt and the
raw response. An agent with two search tools gets a duplicate-search warning.

**In code:** `WebSearch::auto()` in the Framework.

See [Web Search](/capabilities/web-search/).
