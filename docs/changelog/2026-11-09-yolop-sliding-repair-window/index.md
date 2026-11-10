---
title: "Yolop stops cutting off long tasks at ten minutes"
description: "Yolop now restarts the continuation time window on every productive turn, so long tasks that keep working no longer stop at 600 seconds for approval."
date: 2026-11-09
kind: fix
significance: notable
products: [yolop]
surfaces: [cli, operations]
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/features/tools.md#plans-and-completion"]
  prs: [everruns/yolop#931]
---

Yolop no longer measures the continuation time budget from the first repair. Each continuation turn that runs tools without an unrecoverable failure restarts the `continuation_max_seconds` window, so a long task that keeps passing checks and committing work is not interrupted at ten minutes with an approval prompt. Idle loops and failed turns stay on the original clock, and the turn and token counters remain cumulative, so runaway sessions are still bounded.

To give a long task more room without changing the default, raise the window:

```bash
yolop config set continuation_max_seconds 3600
```

See [Plans and completion](https://github.com/everruns/yolop/blob/main/docs/features/tools.md#plans-and-completion).
