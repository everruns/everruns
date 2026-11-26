---
title: "Agent exports and GitHub sync keep portraits"
description: "Agent packages now carry portrait images for the root agent and bundled team members, preserving branding through ZIP export, import and GitHub sync."
date: 2026-11-25
kind: fix
significance: minor
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
links:
  docs: [/reference/agent-package/]
---

Agent exports now include portraits instead of leaving branding behind.
ZIP packages carry the root agent's image and portraits for every bundled team
member. Imports create destination avatar variants from those images; deployment
IDs and image URLs are not copied.

GitHub sources use the same [agent package format](/reference/agent-package/):

```toml
schema_version = 1
name = "support"
instructions = "Help customers."
avatar = "branding/support.png"
```

Two-way sync publishes local portrait changes as image assets. Existing authored
paths and unrelated repository files are preserved. Pull-only sources keep
portrait edits in GitHub. Older packages that omit `avatar` preserve existing
destination branding; `avatar = false` explicitly removes it.
