---
title: "Adopt agent examples and list provider accounts from the CLI"
description: "The everruns CLI and MCP can list and adopt agent examples such as pr-reviewer, and list an organization's provider accounts without their credentials."
date: 2026-10-12
kind: improvement
significance: minor
products: [platform, cloud, cli]
surfaces: [cli, agent]
links:
  docs: [/reference/cli/agents/, /reference/cli/orgs/]
---

Agent examples could only be adopted from the UI. `everruns agents examples
list` shows them, and `adopt` creates an agent from one by name (see
[`everruns agents`](/reference/cli/agents/)). Organization admins can list
sandbox provider and web search accounts with `everruns orgs provider-accounts
list` (see [`everruns orgs`](/reference/cli/orgs/)); it returns names, providers
and ids, never credentials.

```bash
everruns agents examples adopt pr-reviewer --reason 'Review pull requests'
everruns orgs provider-accounts list --org org_01h9
```

A flag renamed for the CLI now also accepts its parameter's own kebab-case
spelling, so `--agent-id` works where `--agent` is documented.
