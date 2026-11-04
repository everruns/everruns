---
title: Sync Agents with GitHub through your account or your organization
description: Agent source sync now connects GitHub inline and reads through your own GitHub or the organization's, separate from the Agent's runtime GitHub identity.
date: 2026-10-30
kind: improvement
significance: notable
products: [platform, cloud]
release: "0.50.1"
surfaces: [ui, api]
links:
  docs: [/how-to/define-agents-as-files/]
---

**Sync with GitHub** no longer sends you to the Agent's Integrations tab. The
dialog asks which GitHub account to sync through: **My GitHub account** (your own
sign-in, two-way commits carry your name) or **Organization GitHub** (the Everruns
App an admin linked, which keeps working when people leave). Connect either one in
a popup and pick a repository without leaving the page.

Sync is now separate from the Agent's own GitHub identity, which only controls how
the Agent appears on GitHub while it runs. Changing that identity no longer breaks
a source. Existing sources keep working; select **Change source** to move them to
your or the organization's GitHub. See
[Define agents as files](/how-to/define-agents-as-files/#sync-an-agent-continuously-from-github).

From the CLI, list the GitHub accounts and repositories a source can sync through:

```bash
everruns agents source accounts list --agent-id agent_01h9
```
