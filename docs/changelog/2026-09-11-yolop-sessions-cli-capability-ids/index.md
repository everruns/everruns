---
title: "Session search and self-configuration move to CLI commands"
description: "Yolop 0.18.0 replaces search_sessions and the configuration tools with yolop sessions, mcp and connectors commands and drops the yolop_ ID prefix."
date: 2026-09-11
kind: breaking
significance: notable
products: [yolop]
surfaces: [cli, agent]
release: "0.18.0"
links:
  docs: ["https://github.com/everruns/yolop#cli-essentials"]
  prs: [everruns/yolop#691, everruns/yolop#693]
---

Yolop 0.18.0 removes several model tools and routes the same work through CLI
commands that the agent runs in its shell:

- `search_sessions` is replaced by `yolop sessions search` and
  `yolop sessions list`. The `session_history` capability ID is now `sessions`.
- `list_mcp_servers`, `set_reasoning_effort`, `search_models`, `set_model`,
  `set_provider` and the connector tools are removed. Use `yolop mcp`, the
  provider and model commands, and the new
  `yolop connectors list|get|connect|disconnect`.
- Capability IDs drop the `yolop_` prefix: `yolop_approval` is now `approval`.

**Upgrade note:** settings overrides that use the old capability IDs fail
validation. Rename them to the bare IDs.

See [CLI essentials](https://github.com/everruns/yolop#cli-essentials), [#691](https://github.com/everruns/yolop/pull/691) and [#693](https://github.com/everruns/yolop/pull/693).
