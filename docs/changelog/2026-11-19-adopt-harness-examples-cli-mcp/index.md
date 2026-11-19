---
title: "Agents can find and adopt the Coding harness without the console"
description: "The everruns CLI and MCP list and adopt harness examples such as Coding, and the Everruns skill tells agents to review examples before building from scratch."
date: 2026-11-19
kind: improvement
significance: minor
products: [platform, cloud, cli]
surfaces: [cli, agent]
links:
  docs: [/reference/cli/harnesses/]
---

Coding, the Yolop-based harness, is an example that an organization adopts,
not a built-in. Until now only the console's Harnesses → Examples page could
adopt it, so an agent working over MCP listed the org's harnesses, did not
find Coding, and settled for Sandbox Worker.

`everruns harnesses examples list` shows the harness examples and `adopt`
adds one to the organization by name (see
[`everruns harnesses`](/reference/cli/harnesses/)). Over MCP the same
operations are `list_harness_examples` and `adopt_harness_example`.

```bash
everruns harnesses examples adopt coding --reason 'Coding agents need the Coding harness'
```

The Everruns skill, the MCP server's instructions, and the `list_harnesses`
description now say that ready-made agents and harnesses live in the examples
until adopted, and that reviewing them is the first step before writing a new
one.
