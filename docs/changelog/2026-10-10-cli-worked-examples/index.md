---
title: "Everruns CLI help shows a worked example for every command"
description: "Every everruns command now prints a worked example in --help, command groups describe themselves, and a generated reference documents each command group."
date: 2026-10-10
kind: improvement
significance: notable
products: [cli]
surfaces: [cli]
links:
  docs: [/reference/cli/, /features/cli/]
  prs: [everruns/everruns#4433]
---

`everruns <noun> <verb> --help` now ends with a worked example, every command
group describes itself, and about 150 more flags have a help line. Commands
whose flat form took a bare ID now take it in the tree form too.

```bash
everruns mcp-servers tools --help
```

The same help is rendered into a new **Reference → CLI Commands** section, one
page per command group generated from the binary itself, so the docs and the
installed CLI say the same thing.

See the [CLI reference](/reference/cli/), [CLI](/features/cli/) and [#4433](https://github.com/everruns/everruns/pull/4433).
