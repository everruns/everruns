---
title: Sandbox Worker Harness
description: Worker plus a full sandbox shell, for agents that need real processes, packages and builds.
---

**Sandbox Worker** extends [Worker](/built-ins/harnesses/worker/) and requires a full sandbox: a
container or a managed provider such as Daytona, E2B or Modal. The Agent's Sandbox policy chooses
which one.

| | |
|---|---|
| **Name** | `sandbox-worker` |
| **Parent** | `worker` |
| **Sandbox policy** | Required; must name a container or managed Sandbox Template |
| **Primary Sandbox** | Whatever the Agent's policy selects |

Use it for coding agents, data work that installs packages, browser automation and long builds.
The [Coding](/how-to/customize-a-harness/) harness example builds on it.

Sandbox Worker never falls back to Bashkit:

- Saving an Agent whose Sandbox policy can only resolve to Bashkit is rejected.
- An Agent with no Sandbox policy saves, but starting a Session on it fails with a message that
  names the missing policy.

Choose [Bashkit Worker](/built-ins/harnesses/bashkit-worker/) for a virtual shell that needs no
provider. Custom Harnesses that inherit from Sandbox Worker inherit the requirement too.

Sandbox Worker is a Platform harness. In the Framework, express the same need as an Environment
requirement on your harness and bind an Environment that provides it.

See [Sandbox Templates](/features/sandbox-templates/) for configuring providers.
