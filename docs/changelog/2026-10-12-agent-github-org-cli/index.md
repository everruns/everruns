---
title: "Start over on an agent's GitHub App, and manage it from the CLI"
description: "An agent's GitHub App that was never installed can be removed to create it in an organization, and everruns agents github manages the connection."
date: 2026-10-12
kind: improvement
significance: minor
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
links:
  docs: [/how-to/summarize-github-pull-requests/]
  prs: [everruns/everruns#4579, everruns/everruns#4592]
---

An agent's GitHub App can only be installed on the account that created it. If it
was created on a personal account by mistake, the GitHub card on the agent's
Integrations tab now has **… > Start over**, which forgets the uninstalled App and
brings back the organization field.

The same connection is now available from the CLI and the MCP `execute` tool:

```bash
everruns agents github get --agent-id agent_01h9
everruns agents github connect --agent-id agent_01h9 --owner-org acme
everruns agents github disconnect --agent-id agent_01h9
```

`connect` prints the install link once the App exists; creating the App still
finishes in the browser. `disconnect` uninstalls an installed App, or starts over
when it was never installed.
