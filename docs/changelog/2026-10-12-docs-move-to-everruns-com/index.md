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
`llms.txt`, the per-page Markdown files and the schemas, permanently redirects
to the same path there, so existing links keep working. A request with
`Accept: text/markdown` returns any docs page as Markdown; see
[Use in AI tools](/getting-started/use-in-ai-tools/) for the text sets.

```bash
curl -H "Accept: text/markdown" https://everruns.com/docs/framework/
curl -sI https://docs.everruns.com/llms.txt   # 301 to everruns.com/docs/llms.txt
```

The docs share the everruns.com header, footer and search, and were redesigned
to match. A docs bar with the sections, search and a light, dark or system
theme switch stays at the top. Articles open with a breadcrumb, a summary,
reading time and last update, with "On this page" beside the text. The docs
home starts with search and the three ways to run Everruns.
