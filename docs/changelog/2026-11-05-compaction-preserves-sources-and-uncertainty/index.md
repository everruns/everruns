---
title: "Compaction keeps claim sources and uncertainty"
description: "Local summaries preserve user intent, evidence, working assumptions and later corrections across repeated compaction, with Active Skills retained."
date: 2026-11-05
kind: improvement
significance: minor
products: [framework, platform, cloud]
surfaces: [agent]
links:
  docs: [/advanced/compaction/]
---

Local [context compaction](/advanced/compaction/) now asks the summarizer to keep
claim sources, unresolved hypotheses, and later confirmations or corrections.
Instructions found in tool output remain attributed to that source. Explicit user
adoption keeps its scope, including assumptions accepted only for testing.

For example, enable local summarization with:

```json
{
  "ref": "compaction",
  "config": {
    "strategy": "summarization"
  }
}
```

The same guidance applies to the local fallback in `auto`. Repeated summaries
retain the full previous summary, including late qualifiers and Active Skills.
Results still depend on the model and available evidence. Provider-native opaque
compaction is unchanged.
