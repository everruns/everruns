---
title: "An agent's GitHub App gets its own section, name and avatar step"
description: "GitHub moves out of Triggers into its own section on the agent's Integrations tab, new Apps take the agent's name, and a step helps set the App's avatar."
date: 2026-10-12
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/how-to/summarize-github-pull-requests/]
  prs: [everruns/everruns#4665]
---

**In the UI:** the agent's **Integrations** tab now has a **GitHub** section
between **Channels** and **Triggers**. It shows the agent's bot login, such as
`pr-summarizer[bot]`, whether it is the agent's own App, the account and
repositories it is installed on, and links to add and list GitHub triggers.

GitHub offers no way to set an App's logo for it, so a **Set the avatar** step
offers the agent's avatar as a download and links to the App's settings page.

New Apps are named after the agent, with a random suffix only when the name is
taken.

**In the API:** `GET /v1/agents/{agent_id}/github` adds three fields:

```json
{
  "kind": "agent_app",
  "bot_login": "pr-summarizer[bot]",
  "app_settings_url": "https://github.com/organizations/acme/settings/apps/pr-summarizer"
}
```

See [Summarize GitHub pull requests](/how-to/summarize-github-pull-requests/).
