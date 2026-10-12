---
title: "Org admins link the Everruns GitHub App, agents choose it explicitly"
description: "Org admins link Everruns GitHub App installations in Settings, and each agent chooses between its own GitHub App and a linked account."
date: 2026-10-12
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/how-to/summarize-github-pull-requests/]
  prs: [everruns/everruns#PRNUM]
---

The shared Everruns GitHub App now belongs to the organization. An org admin links a GitHub
account in **Settings > Organization > Everruns GitHub App**, and an agent without a GitHub
identity can pick **Use the Everruns app** next to **Create this agent's own GitHub App**.

```bash
# The linked account ids are in `everruns_app_installations`
everruns agents github get --agent-id "$AGENT_ID"
everruns agents github use-everruns-app --agent-id "$AGENT_ID" --connection-id "$LINK_ID"
```

The agent then comments as the shared bot, and its pull request triggers fire from the shared
App's webhook. Nothing falls back to the shared App: unlinking an account removes it from every
agent that chose it, and members can no longer attach an installation to an agent directly.

See [Summarize GitHub pull requests](/how-to/summarize-github-pull-requests/).
