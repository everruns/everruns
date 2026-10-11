---
title: "Yolop profiles can define a whole agent"
description: "A Yolop profile can now carry its own capabilities, MCP servers, skills directory and standing instructions, so one profile flag selects an agent with a job."
date: 2026-08-17
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.16.0"
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/session-coordination.md"]
  prs: [everruns/yolop#586]
---

A named profile used to switch only execution settings, such as the provider.
It can now also decide what the agent can do:

- `capabilities` and `capabilities_mode`: capability entries layered after the
  global list or replacing it, including which extensions are enabled.
- `mcp` and `mcp_mode`: MCP servers merged by name over the global set, or
  replacing it.
- `instructions` or `instructions_file`: text appended to the system prompt.
- `skills_dir`: an extra skills scope, by default `profiles/<name>/skills/`.

Start a session with `yolop --profile <name>`. Credentials stay global.

See [Session coordination](https://github.com/everruns/yolop/blob/main/docs/session-coordination.md) for a profile example and [#586](https://github.com/everruns/yolop/pull/586).
