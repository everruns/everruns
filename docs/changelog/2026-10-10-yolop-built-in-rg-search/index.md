---
title: "Code search works on machines without ripgrep"
description: "Yolop ships a built-in rg search as yolop utils rg and, when no system ripgrep is found, as an agent tool backed by the same engine as its search tool."
date: 2026-10-10
kind: feature
significance: notable
products: [yolop]
surfaces: [cli, agent]
release: "0.21.0"
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/features/tools.md"]
  prs: [everruns/yolop#839]
---

Yolop now includes a ripgrep-compatible search. `yolop utils rg` supports a
common flag subset (`-n`, `-i`, `-S`, `-F`, `-w`, `-g`, `-l`, `-c`, `--files`,
`-C`, `-A`, `-B`, `--hidden`, `--no-ignore` and more) with rg-style exit codes:

```bash
yolop utils rg -n hello .
yolop utils rg --files -g '*.txt' .
```

When no `rg` is found on `PATH` at startup, the agent also gets an `rg` tool on
the same engine: read-only, scoped to the workspace and with bounded output.
Machines with ripgrep installed keep using it.

See [Tools](https://github.com/everruns/yolop/blob/main/docs/features/tools.md) and [#839](https://github.com/everruns/yolop/pull/839).
