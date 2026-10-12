---
title: "Docs moved to everruns.com/docs"
description: "The documentation now lives at everruns.com/docs, and every docs.everruns.com link, Markdown file and schema redirects to the same page there."
date: 2026-10-12
kind: improvement
significance: notable
products: [cloud, platform, framework]
surfaces: [agent]
links:
  docs: [/getting-started/use-in-ai-tools/]
  prs: [everruns/everruns#4596]
---

The documentation moved from docs.everruns.com to
[everruns.com/docs](https://everruns.com/docs/). Every old address, including
`llms.txt`, the per-page Markdown files, the OpenAPI schema and the agent
schema, answers with a permanent redirect to the same path under the new
address, so existing links and bookmarks keep working.

Agents get the same treatment across everruns.com: a request for a docs page
with `Accept: text/markdown` returns the page as Markdown, as the rest of the
site already does.

The docs also share the everruns.com header and footer now, and the docs
search finds pages from the rest of everruns.com, such as product and
comparison pages, alongside the documentation.
