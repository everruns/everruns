---
title: Parallel answers yes, which one and how much from the web
description: The Parallel capability moved to Parallel's current MCP server, adding web_decisions for probabilities, picks and scores with citations, plus image search.
date: 2026-11-21
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/integrations/parallel/]
---

Agents with [Parallel](/integrations/parallel/) used to get two tools: search and
fetch. The capability now talks to Parallel's current MCP server, so the same
setup also brings `web_decisions` and `image_search`, free or on your Parallel
key.

`web_decisions` takes a subject and named questions, and answers each with a
calibrated number instead of prose:

```json
{
  "open_source": {
    "type": "noul",
    "noul": 0.998
  },
  "language": {
    "type": "choice",
    "choice": "rust",
    "probabilities": {
      "rust": 0.999,
      "go": 0.0002,
      "unknown": 0.0005
    }
  }
}
```

The pages it relied on come back as citations. An agent can ask up to 100
questions in one call, so screening a list of companies or vendors is one call
per row, not a research loop.
