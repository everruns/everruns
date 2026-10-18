---
title: "Connecting GitHub as yourself acts as you, not the shared bot"
description: "Your own GitHub connection now signs you in to GitHub and acts as you; shared Everruns App installations move to organization links."
date: 2026-10-18
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/how-to/summarize-github-pull-requests/]
  prs: [everruns/everruns#4815]
---

Connecting GitHub under your own connections now signs you in to GitHub, and tools that act as
you use your own GitHub access. Previously that connection installed the shared Everruns App, so
your actions showed up as the shared bot. Tokens refresh on their own; if GitHub refuses a
refresh, connect GitHub again.

```text
Before: GET /v1/user/connections/github/authorize → install the Everruns App (acts as everruns[bot])
After:  GET /v1/user/connections/github/authorize → sign in to GitHub (acts as you)
```

Existing shared App installations move to organization links: each one becomes a link in the
organization of whoever installed it first, and an agent that used one now uses that link. If you
connected GitHub for yourself before this change, connect it again to act as yourself. Org admins
manage links in **Settings > Organization > Everruns GitHub App**.
