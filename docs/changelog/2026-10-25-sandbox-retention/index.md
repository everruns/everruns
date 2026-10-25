---
title: Paused Sandboxes are deleted after three days
description: Paused Sandboxes are now deleted after a retention period you set per template, and Sandboxes of stuck Sessions no longer keep running.
date: 2026-10-25
kind: improvement
significance: minor
products: [platform]
surfaces: [ui, api]
links:
  docs: [/features/sandbox-templates/]
  prs: [everruns/everruns#4915]
---

Paused Sandboxes used to stay with the provider until their Session was deleted, which never
happens for Sessions started by triggers. Everruns now deletes a paused Sandbox after three days
without use. Pick 1, 7, or 30 days, or **Never**, under **Delete after paused** on the
[Sandbox Template](/features/sandbox-templates/). The Session stays; its next run gets a fresh
Sandbox.

```json
{
  "lifecycle": {
    "idle_after_seconds": 900,
    "delete_after_seconds": 604800
  }
}
```

Set `delete_after_seconds` to `null` to keep paused Sandboxes.

A Sandbox whose Session is stuck mid-turn is now paused after six hours without use, instead of
running until someone stops it.
