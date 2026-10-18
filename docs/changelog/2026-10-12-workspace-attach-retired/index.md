---
title: "Sessions no longer attach to another workspace"
description: "Every session now gets its own workspace: create-session refuses workspace_id, and POST /v1/workspaces and everruns workspaces create are removed."
date: 2026-10-12
kind: breaking
significance: notable
products: [platform, cloud]
surfaces: [api, cli]
links:
  docs: [/reference/cli/workspaces/]
  prs: [everruns/everruns#4673]
---

Every session now gets its own workspace. Creating a session with
`workspace_id` returns `400`, and `POST /v1/workspaces` and
`everruns workspaces create` are removed. Attaching was gated only by the
org-wide workspace permission, so a new session on any agent could be pointed
at another session's files.

```http
POST /v1/sessions
{"agent_id": "agt_...", "workspace_id": "wsp_..."}

400 Bad Request
workspace_id is no longer supported: every session gets its own workspace.
Copy files between sessions through the Files API instead.
```

Sessions that were attached to a shared workspace before this change keep it
and its files. Listing, reading, renaming and archiving workspaces, and the
`/v1/workspaces/{id}/fs` file routes, are unchanged. To move files between
sessions, read them from one session and write them to the other through the
Files API.

See [`everruns workspaces`](/reference/cli/workspaces/) and [#4673](https://github.com/everruns/everruns/pull/4673).
