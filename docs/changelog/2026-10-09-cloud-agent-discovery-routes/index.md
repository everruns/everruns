---
title: app.everruns.com serves llms.txt and auth.md for AI agents
description: AI agents and crawlers can discover Everruns Cloud from app.everruns.com itself through llms.txt, auth.md, Link headers, robots.txt and a sitemap.
date: 2026-10-09
kind: improvement
significance: minor
products: [cloud]
surfaces: [agent]
links:
  docs: [/getting-started/use-in-ai-tools/]
---

An agent pointed at [app.everruns.com](https://app.everruns.com) can now find
out how to use it from the app host itself. `/llms.txt`, `/auth.md`,
`/robots.txt` and `/sitemap.xml` are served there, a request for `/` with
`Accept: text/markdown` returns `llms.txt`, and `/` carries `Link` headers
pointing to the API catalog, `llms.txt`, `auth.md` and the OpenAPI document.

```bash
curl -H "Accept: text/markdown" https://app.everruns.com/
```

MCP connect dialogs also show the Everruns icon for the app.

See [Use in AI Tools](/getting-started/use-in-ai-tools/).
