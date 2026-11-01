---
title: "Daytona commands reject missing working directories"
description: "Multi-line Daytona commands now stop entirely when their requested working directory is missing, rather than running later lines elsewhere."
date: 2026-10-31
kind: fix
significance: minor
products: [platform]
surfaces: [agent]
links:
  docs: [/capabilities/daytona/]
---

[Daytona sandbox commands](/capabilities/daytona/) now reject the entire command
when its requested working directory does not exist. Previously, only the first
line was guarded; later lines could run in the sandbox's default directory.

For example, neither line runs when `/missing` is absent:

```json
{
  "command": "printf first\nprintf second",
  "cwd": "/missing"
}
```

Trailing shell comments also work without consuming the command wrapper. The
persistent shell stays available after a command exits. No configuration change
is required.
