---
title: "Compaction reserves context for tool schemas"
description: "Context checks now include model-visible tool schemas, reserve them before trimming history, and explain when fixed instructions and tools cannot fit."
date: 2026-11-13
kind: fix
significance: minor
products: [framework, platform]
surfaces: [agent]
links:
  docs: [/advanced/compaction/]
---

[Context compaction](/advanced/compaction/) now includes model-visible tool
schemas in its proactive budget. Large active MCP catalogs can trigger compaction
before message history alone reaches the threshold. Deferred full schemas stay
outside the estimate until offered to the model.

History trimming reserves room for tool declarations and system instructions.
Cache-stable schema replay can restart from the current catalog under pressure.
When current declarations and instructions alone exceed the estimated context
window, the error explains that callers must reduce that fixed input or select a
larger-context model. Estimates remain approximate; provider overflow recovery
still handles differences in tokenization and request formatting.
