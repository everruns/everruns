---
title: Sync Agent source continuously from GitHub
description: Connect an Agent to a GitHub repository package, review its changes, and apply future updates automatically while keeping source fields read-only.
date: 2026-10-23
kind: feature
significance: notable
products: [platform]
surfaces: [ui, api, agent]
links:
  docs: [/how-to/define-agents-as-files/]
---

Agents can now follow a package in a GitHub repository continuously. Open an
Agent, select **Source → Sync from GitHub**, choose the repository, branch and
package path, then review changes before enabling sync. For example, point at
`agents/support/` containing:

```toml
schema_version = 1
name = "support"
instructions_file = "instructions.md"
```

![Agent Source panel showing its GitHub repository, applied commit and sync status](./source@2x.png)

Pushes and periodic checks apply valid updates automatically. The Source panel
shows the applied commit and sync errors. Invalid updates retain the last working
definition. Synced fields are read-only across Everruns; edit the repository or
disconnect to restore local editing. See
[Define agents as files](/how-to/define-agents-as-files/#sync-an-agent-continuously-from-github).

## Update 2026-10-24

Connect an unlinked Agent from **More actions → Sync from GitHub**. The Source panel is shown only for a connected source or while its status is loading or unavailable. Connected source status and controls remain visible. See [Agent actions and searchable Playground setup](/changelog/2026-10-24-agent-actions-playground-search/).

## Update 2026-10-25

Two-way sync is now available as an opt-in. Existing bindings remain read-only.
See [two-way Agent sync](/changelog/2026-10-25-two-way-agent-github-sync/).
