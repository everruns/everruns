---
title: "Choose where an agent's GitHub App lives, and manage it from the CLI"
description: "Connect GitHub suggests the organizations your team already uses for an agent's App, an uninstalled App can start over, and the CLI manages it."
date: 2026-10-12
kind: improvement
significance: minor
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
links:
  docs: [/how-to/summarize-github-pull-requests/]
  prs: [everruns/everruns#4579, everruns/everruns#4592]
---

An agent's GitHub App can only be installed on the account that created it, so
**Connect GitHub** on the agent's Integrations tab now asks where to create it:
**Personal account**, one of the GitHub organizations your Everruns organization
already connected (from other agents' Apps and GitHub connections), or **Another
organization…**. This replaces the free-text organization field.

If the App was created on the wrong account, the card now has **… > Start
over**, which forgets the uninstalled App so the next connect can choose again.

The same connection is now available from the CLI and the MCP `execute` tool:

```bash
everruns agents github get --agent-id agent_01h9
everruns agents github connect --agent-id agent_01h9 --owner-org acme
everruns agents github disconnect --agent-id agent_01h9
```

`connect` prints the install link once the App exists; creating the App still
finishes in the browser. `disconnect` uninstalls an installed App, or starts over
when it was never installed.
